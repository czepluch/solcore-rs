//! Runs tests written as Solcore contracts: deploys several contracts on one
//! in-memory chain and calls each public `test*()` function of a test contract.
//! Calls to forge-std's `VM_ADDRESS` are answered as cheatcodes.

use std::collections::BTreeMap;

use nameres::Db as _;
use revm::{
    Context, InspectCommitEvm, InspectEvm, Inspector, MainBuilder, MainContext,
    context::TxEnv,
    context_interface::{
        ContextTr,
        result::{ExecutionResult, Output},
    },
    database::InMemoryDB,
    interpreter::{CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult},
    primitives::{Address, Bytes, TxKind, U256, address, hardfork::SpecId, keccak256},
    state::AccountInfo,
};
use vfs::{AnalysisHost, Workspace, WorkspaceFileChange};

use crate::{
    backend::{self, Backend},
    execution::{
        ACCOUNTS, CALLER, DEPLOYMENT_GAS_LIMIT, MEMORY_LIMIT, STARTING_BALANCE,
        transaction_gas_limit,
    },
    sandbox, test_backend,
};

const TOKEN_TEST_SOURCE: &str = include_str!("contract_tests/token_test.sol");
const GUARDED_TEST_SOURCE: &str = include_str!("contract_tests/guarded_test.sol");
const TOKEN_CLIENT_SOURCE: &str = include_str!("contract_tests/token_client.sol");
const EXTERNAL_CALLS_SOURCE: &str = include_str!("contract_tests/external_calls.sol");
const CHEATCODES_SOURCE: &str = include_str!("contract_tests/cheatcodes.sol");

/// forge-std's `VM_ADDRESS`.
const CHEATCODE_ADDRESS: Address = address!("0x7109709ECfa91a80626fF3989D68f67F5b1DD12D");

/// The next call made by `pranker` comes from `sender`.
#[derive(Clone, Copy)]
struct Prank {
    pranker: Address,
    sender: Address,
}

/// Answers calls to `CHEATCODE_ADDRESS` and applies pending pranks.
#[derive(Default)]
struct Cheatcodes {
    /// The creation bytecode of each contract in the compilation, or the error
    /// from compiling it.
    creation_bytecode: BTreeMap<String, Result<Vec<u8>, String>>,
    prank: Option<Prank>,
}

impl Cheatcodes {
    /// Returns the ABI-encoded result of a cheatcode call, or its revert message.
    fn answer(&mut self, caller: Address, calldata: &[u8]) -> Result<Vec<u8>, String> {
        let (selector, arguments) = calldata
            .split_at_checked(4)
            .ok_or("cheatcode calldata is shorter than a selector")?;
        if selector == function_selector("prank(address)") {
            let sender = abi_decode_address(arguments)?;
            self.prank = Some(Prank {
                pranker: caller,
                sender,
            });
            Ok(Vec::new())
        } else if selector == function_selector("getCode(string)") {
            let artifact = abi_decode_string(arguments)?;
            // Accepts `Name`, `File.sol` and `File.sol:Name`, as forge-std does.
            let contract = artifact
                .rsplit(':')
                .next()
                .unwrap_or_default()
                .trim_end_matches(".sol");
            match self.creation_bytecode.get(contract) {
                Some(Ok(code)) => Ok(abi_encode_bytes(code)),
                Some(Err(error)) => Err(format!("{contract} does not compile: {error}")),
                None => Err(format!("no contract named {contract}")),
            }
        } else {
            Err(format!("unsupported cheatcode {}", hex(selector)))
        }
    }
}

impl<CTX: ContextTr> Inspector<CTX> for Cheatcodes {
    fn call(&mut self, context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.target_address == CHEATCODE_ADDRESS {
            let calldata = inputs.input.bytes(context);
            let (result, output) = match self.answer(inputs.caller, &calldata) {
                Ok(output) => (InstructionResult::Return, output),
                Err(message) => (InstructionResult::Revert, message.into_bytes()),
            };
            return Some(CallOutcome::new(
                InterpreterResult::new(result, output.into(), Gas::new(inputs.gas_limit)),
                inputs.return_memory_offset.clone(),
            ));
        }
        if let Some(prank) = self.prank.filter(|prank| prank.pranker == inputs.caller) {
            inputs.caller = prank.sender;
            self.prank = None;
        }
        None
    }
}

/// An in-memory chain that keeps every deployment, with the playground's
/// funded accounts.
struct TestChain {
    db: InMemoryDB,
    cheatcodes: Cheatcodes,
}

impl TestChain {
    fn new(creation_bytecode: BTreeMap<String, Result<Vec<u8>, String>>) -> Self {
        let mut db = InMemoryDB::default();
        for account in ACCOUNTS {
            db.insert_account_info(
                account,
                AccountInfo::default().with_balance(STARTING_BALANCE),
            );
        }
        Self {
            db,
            cheatcodes: Cheatcodes {
                creation_bytecode,
                prank: None,
            },
        }
    }

    fn transact(
        &mut self,
        caller: Address,
        kind: TxKind,
        data: Bytes,
        commit: bool,
    ) -> ExecutionResult {
        let nonce = self
            .db
            .cache
            .accounts
            .get(&caller)
            .map_or(0, |account| account.info.nonce);
        let mut evm = Context::mainnet()
            .with_db(&mut self.db)
            .modify_cfg_chained(|cfg| {
                cfg.set_spec_and_mainnet_gas_params(SpecId::OSAKA);
                cfg.memory_limit = MEMORY_LIMIT;
            })
            .modify_block_chained(|block| {
                block.basefee = 0;
                block.gas_limit = DEPLOYMENT_GAS_LIMIT;
                block.number = U256::from(1);
                block.timestamp = U256::from(1);
            })
            .build_mainnet_with_inspector(std::mem::take(&mut self.cheatcodes));
        let tx = TxEnv::builder()
            .caller(caller)
            .kind(kind)
            .data(data)
            .nonce(nonce)
            .gas_limit(transaction_gas_limit(kind))
            .gas_price(0)
            .build_fill();
        let result = if commit {
            evm.inspect_tx_commit(tx)
        } else {
            evm.inspect_tx(tx).map(|output| output.result)
        };
        self.cheatcodes = evm.inspector;
        // A prank does not outlive the transaction that set it.
        self.cheatcodes.prank = None;
        result.expect("transaction is valid")
    }

    /// Deploys `init_bytecode` followed by the ABI-encoded constructor arguments.
    fn deploy(
        &mut self,
        caller: Address,
        init_bytecode: &[u8],
        constructor_arguments: &[u8],
    ) -> Address {
        let code = [init_bytecode, constructor_arguments].concat();
        match self.transact(caller, TxKind::Create, code.into(), true) {
            ExecutionResult::Success {
                output: Output::Create(_, Some(address)),
                ..
            } => address,
            other => panic!("deployment failed: {other:?}"),
        }
    }

    /// Calls `to`, and keeps the state changes only when `commit` is set.
    fn call(
        &mut self,
        caller: Address,
        to: Address,
        calldata: Vec<u8>,
        commit: bool,
    ) -> Result<(), String> {
        call_outcome(&self.transact(caller, TxKind::Call(to), calldata.into(), commit))
    }
}

fn workspace_with_files(files: &[(&str, &str)], entry: &str) -> Workspace {
    let mut workspace = Workspace::new();
    workspace.apply_file_changes(
        files
            .iter()
            .map(|(path, contents)| WorkspaceFileChange::Set {
                path: (*path).to_owned(),
                contents: (*contents).to_owned(),
            }),
    );
    workspace.set_entry(entry);
    workspace
}

fn checked_hull_program(workspace: &Workspace) -> hull::Program<'_> {
    let db = workspace.db();
    let module = workspace.entry_module().expect("entry module");
    let file = db.module_file(module).expect("entry file");
    compiler::build_checked_hull(db, file, Default::default())
        .unwrap_or_else(|diagnostics| panic!("{} compiler diagnostics", diagnostics.len()))
        .program
}

/// Compiles the deploy object of `contract` to init bytecode.
fn init_bytecode(
    backend: Backend,
    db: &AnalysisHost,
    program: &hull::Program<'_>,
    contract: &str,
) -> Result<Vec<u8>, String> {
    let mut program = program.clone();
    program
        .objects
        .retain(|object| object.name.as_str() == format!("{contract}Deploy"));
    program.functions.clear();
    program.entry_points.clear();
    backend::compile_program(backend, db, &program)?
        .pop()
        .map(|object| object.init)
        .filter(|init| !init.is_empty())
        .ok_or_else(|| "missing init bytecode".to_owned())
}

/// Compiles every contract of the entry module to init bytecode.
fn creation_bytecode_by_contract(
    backend: Backend,
    workspace: &Workspace,
    program: &hull::Program<'_>,
) -> BTreeMap<String, Result<Vec<u8>, String>> {
    sandbox::discover(workspace)
        .into_iter()
        .map(|contract| {
            let code = init_bytecode(backend, workspace.db(), program, &contract.name);
            (contract.name, code)
        })
        .collect()
}

/// Copies the deploy object of `created` into the runtime object of `creator`,
/// so code in `creator` can refer to it with `datasize` and `dataoffset`.
fn embed_deploy_object(program: &mut hull::Program<'_>, creator: &str, created: &str) {
    let created_object = program
        .objects
        .iter()
        .find(|object| object.name.as_str() == format!("{created}Deploy"))
        .cloned()
        .expect("deploy object of the created contract");
    let creator_object = program
        .objects
        .iter_mut()
        .find(|object| object.name.as_str() == format!("{creator}Deploy"))
        .expect("deploy object of the creating contract");
    creator_object.inners[0].inners.push(created_object);
}

fn function_selector(signature: &str) -> Vec<u8> {
    keccak256(signature.as_bytes())[..4].to_vec()
}

fn abi_encode_address(address: Address) -> Vec<u8> {
    [&[0; 12], address.as_slice()].concat()
}

fn abi_encode_u256(value: u64) -> Vec<u8> {
    U256::from(value).to_be_bytes::<32>().to_vec()
}

/// Encodes `bytes` as the only return value: offset, length and padded data.
fn abi_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    let padding = (32 - bytes.len() % 32) % 32;
    [
        abi_encode_u256(32),
        abi_encode_u256(bytes.len() as u64),
        bytes.to_vec(),
        vec![0; padding],
    ]
    .concat()
}

fn abi_word(arguments: &[u8], offset: usize) -> Result<&[u8], String> {
    arguments
        .get(offset..offset + 32)
        .ok_or_else(|| format!("ABI data has no word at offset {offset}"))
}

fn abi_word_as_usize(word: &[u8]) -> Result<usize, String> {
    let (high, low) = word.split_at(24);
    if high.iter().any(|byte| *byte != 0) {
        return Err("ABI offset or length does not fit in usize".to_owned());
    }
    Ok(u64::from_be_bytes(low.try_into().expect("8 bytes")) as usize)
}

fn abi_decode_address(arguments: &[u8]) -> Result<Address, String> {
    let word = abi_word(arguments, 0)?;
    Ok(Address::from_slice(&word[12..]))
}

/// Decodes a `string` that is the only argument.
fn abi_decode_string(arguments: &[u8]) -> Result<String, String> {
    let offset = abi_word_as_usize(abi_word(arguments, 0)?)?;
    let length = abi_word_as_usize(abi_word(arguments, offset)?)?;
    let bytes = arguments
        .get(offset + 32..offset + 32 + length)
        .ok_or("ABI string is shorter than its length")?;
    String::from_utf8(bytes.to_vec()).map_err(|error| error.to_string())
}

fn hex(bytes: &[u8]) -> String {
    format!(
        "0x{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn call_outcome(result: &ExecutionResult) -> Result<(), String> {
    match result {
        ExecutionResult::Success { .. } => Ok(()),
        ExecutionResult::Revert { output, .. } => Err(revert_message(output)),
        ExecutionResult::Halt { reason, .. } => Err(format!("halted: {reason:?}")),
    }
}

/// Reads a revert payload as text when it is printable, and as hex otherwise.
fn revert_message(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(text) if !text.is_empty() && !text.chars().any(char::is_control) => text.to_owned(),
        _ => hex(data),
    }
}

/// The signatures of the contract's public functions whose names start with `test`.
fn test_function_signatures(workspace: &Workspace, contract: &str) -> Vec<String> {
    sandbox::discover(workspace)
        .into_iter()
        .find(|candidate| candidate.name == contract)
        .expect("test contract")
        .methods
        .into_iter()
        .map(|method| method.signature)
        .filter(|signature| signature.starts_with("test"))
        .collect()
}

/// Calls each test function without committing, so every test starts from the
/// same state.
fn run_test_functions(
    chain: &mut TestChain,
    test_contract: Address,
    signatures: &[String],
) -> Vec<(String, Result<(), String>)> {
    signatures
        .iter()
        .map(|signature| {
            let result = chain.call(CALLER, test_contract, function_selector(signature), false);
            println!("{signature}: {result:?}");
            (signature.clone(), result)
        })
        .collect()
}

/// Deploys `test_contract`, calls its `setUp()`, and runs its test functions
/// from the state `setUp()` leaves.
fn run_test_contract(
    backend: Backend,
    workspace: &Workspace,
    test_contract: &str,
) -> Vec<(String, Result<(), String>)> {
    let program = checked_hull_program(workspace);
    let creation_bytecode = creation_bytecode_by_contract(backend, workspace, &program);
    let init = creation_bytecode[test_contract]
        .clone()
        .unwrap_or_else(|error| panic!("{test_contract} does not compile: {error}"));
    let mut chain = TestChain::new(creation_bytecode);
    let address = chain.deploy(CALLER, &init, &[]);
    chain
        .call(CALLER, address, function_selector("setUp()"), true)
        .unwrap_or_else(|message| panic!("{test_contract}.setUp() reverted: {message}"));
    run_test_functions(
        &mut chain,
        address,
        &test_function_signatures(workspace, test_contract),
    )
}

/// The revert message of each failing test, by signature.
fn failures(results: &[(String, Result<(), String>)]) -> Vec<(&str, &str)> {
    results
        .iter()
        .filter_map(|(signature, result)| {
            result
                .as_ref()
                .err()
                .map(|message| (signature.as_str(), message.as_str()))
        })
        .collect()
}

fn module_files() -> [(&'static str, &'static str); 3] {
    [
        ("token_client.sol", TOKEN_CLIENT_SOURCE),
        ("external_calls.sol", EXTERNAL_CALLS_SOURCE),
        ("cheatcodes.sol", CHEATCODES_SOURCE),
    ]
}

fn token_test_workspace() -> Workspace {
    let files = [
        [("token_test.sol", TOKEN_TEST_SOURCE)].as_slice(),
        &module_files(),
    ]
    .concat();
    workspace_with_files(&files, "token_test.sol")
}

fn guarded_test_workspace() -> Workspace {
    let files = [
        [("guarded_test.sol", GUARDED_TEST_SOURCE)].as_slice(),
        &module_files(),
    ]
    .concat();
    workspace_with_files(&files, "guarded_test.sol")
}

#[test]
fn test_contract_calls_a_token_deployed_by_the_runner() {
    let backend = test_backend();
    let workspace = token_test_workspace();
    let db = workspace.db();
    let program = checked_hull_program(&workspace);
    let mut chain = TestChain::new(BTreeMap::new());

    let token = chain.deploy(
        CALLER,
        &init_bytecode(backend, db, &program, "Token").unwrap(),
        &[],
    );
    let test_contract = chain.deploy(
        CALLER,
        &init_bytecode(backend, db, &program, "TokenTest").unwrap(),
        &abi_encode_address(token),
    );
    let transfer_calldata = [
        function_selector("transfer(address,uint256)"),
        abi_encode_address(test_contract),
        abi_encode_u256(100),
    ]
    .concat();
    chain.call(CALLER, token, transfer_calldata, true).unwrap();

    let results = run_test_functions(
        &mut chain,
        test_contract,
        &test_function_signatures(&workspace, "TokenTest"),
    );
    assert_eq!(results.len(), 4);
    assert_eq!(
        failures(&results),
        [("testFailsOnPurpose()", "deliberately failing check")]
    );
}

#[test]
fn creating_another_contract_fails_without_its_deploy_object() {
    let backend = test_backend();
    let workspace = token_test_workspace();
    let program = checked_hull_program(&workspace);
    let error = init_bytecode(backend, workspace.db(), &program, "TokenSetUpTest").unwrap_err();
    println!("{backend:?}: {error}");
}

#[test]
fn set_up_deploys_the_token_from_an_embedded_deploy_object() {
    let backend = test_backend();
    let workspace = token_test_workspace();
    let db = workspace.db();
    let mut program = checked_hull_program(&workspace);
    embed_deploy_object(&mut program, "TokenSetUpTest", "Token");
    let mut chain = TestChain::new(BTreeMap::new());

    let test_contract = chain.deploy(
        CALLER,
        &init_bytecode(backend, db, &program, "TokenSetUpTest").unwrap(),
        &[],
    );
    chain
        .call(CALLER, test_contract, function_selector("setUp()"), true)
        .unwrap();

    let results = run_test_functions(
        &mut chain,
        test_contract,
        &test_function_signatures(&workspace, "TokenSetUpTest"),
    );
    assert_eq!(results.len(), 2);
    assert!(failures(&results).is_empty(), "{results:?}");
}

#[test]
fn cheatcodes_deploy_the_token_and_change_the_sender() {
    let results = run_test_contract(
        test_backend(),
        &token_test_workspace(),
        "TokenCheatcodeTest",
    );
    assert_eq!(results.len(), 5);
    assert_eq!(
        failures(&results),
        [(
            "testMisspelledContractNameFailsWhenTheTestRuns()",
            "getCode failed"
        )]
    );
}

#[test]
fn prank_makes_the_guarded_function_reject_the_call() {
    let results = run_test_contract(test_backend(), &guarded_test_workspace(), "GuardedTest");
    assert_eq!(results.len(), 2);
    assert!(failures(&results).is_empty(), "{results:?}");
}

#[test]
fn cheatcode_selectors_match_forge_std() {
    assert_eq!(hex(&function_selector("prank(address)")), "0xca669fa7");
    assert_eq!(hex(&function_selector("getCode(string)")), "0x8d1cc925");
}
