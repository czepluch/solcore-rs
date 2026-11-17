//! Runs tests written as Solcore contracts: deploys several contracts on one
//! in-memory chain and calls each public `test*()` function of a test contract.

use nameres::Db as _;
use revm::{
    Context, ExecuteCommitEvm, ExecuteEvm, MainBuilder, MainContext,
    context::TxEnv,
    context_interface::result::{ExecutionResult, Output},
    database::InMemoryDB,
    primitives::{Address, Bytes, TxKind, U256, hardfork::SpecId, keccak256},
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
const TOKEN_CLIENT_SOURCE: &str = include_str!("contract_tests/token_client.sol");

/// An in-memory chain that keeps every deployment, with the playground's
/// funded accounts.
struct TestChain {
    db: InMemoryDB,
}

impl TestChain {
    fn new() -> Self {
        let mut db = InMemoryDB::default();
        for account in ACCOUNTS {
            db.insert_account_info(
                account,
                AccountInfo::default().with_balance(STARTING_BALANCE),
            );
        }
        Self { db }
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
            .build_mainnet();
        let tx = TxEnv::builder()
            .caller(caller)
            .kind(kind)
            .data(data)
            .nonce(nonce)
            .gas_limit(transaction_gas_limit(kind))
            .gas_price(0)
            .build_fill();
        if commit {
            evm.transact_commit(tx).expect("transaction is valid")
        } else {
            evm.transact(tx).expect("transaction is valid").result
        }
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
        _ => format!(
            "0x{}",
            data.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
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

fn token_test_workspace() -> Workspace {
    workspace_with_files(
        &[
            ("token_test.sol", TOKEN_TEST_SOURCE),
            ("token_client.sol", TOKEN_CLIENT_SOURCE),
        ],
        "token_test.sol",
    )
}

#[test]
fn test_contract_calls_a_token_deployed_by_the_runner() {
    let backend = test_backend();
    let workspace = token_test_workspace();
    let db = workspace.db();
    let program = checked_hull_program(&workspace);
    let mut chain = TestChain::new();

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
    let failures = results
        .iter()
        .filter_map(|(signature, result)| {
            result
                .as_ref()
                .err()
                .map(|message| (signature.as_str(), message.as_str()))
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 4);
    assert_eq!(
        failures,
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
    let mut chain = TestChain::new();

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
    assert!(
        results.iter().all(|(_, result)| result.is_ok()),
        "{results:?}"
    );
}
