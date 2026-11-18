//! Runs the test contracts in `tests/fixtures`.
//!
//! `SOLCORE_CONTRACT_TESTS_BACKEND=solc` compiles them with solc (see `SOLC`);
//! anything else compiles them with Sonatina.

use hir::input::SourceFile;
use solcore_contract_tests::{
    ContractReport, CreationBytecode, DEPLOYER, SetUpFailure, SetUpStep, TestChain, abi,
    creation_bytecode, merge_creation_bytecode, run_test_contracts, test_contracts,
};
use solcore_evm_backend::Backend;
use vfs::{AnalysisHost, Workspace, WorkspaceFileChange};

const FIXTURES: &[(&str, &str)] = &[
    ("token_test.sol", include_str!("fixtures/token_test.sol")),
    (
        "guarded_test.sol",
        include_str!("fixtures/guarded_test.sol"),
    ),
    ("counter.sol", include_str!("fixtures/counter.sol")),
    (
        "counter_test.sol",
        include_str!("fixtures/counter_test.sol"),
    ),
    (
        "token_client.sol",
        include_str!("fixtures/token_client.sol"),
    ),
    (
        "external_calls.sol",
        include_str!("fixtures/external_calls.sol"),
    ),
];

fn backend() -> Backend {
    match std::env::var("SOLCORE_CONTRACT_TESTS_BACKEND").as_deref() {
        Ok("solc") => Backend::Solc,
        _ => Backend::Sonatina,
    }
}

fn fixture_workspace(entry: &str) -> Workspace {
    let mut workspace = Workspace::new();
    workspace.apply_file_changes(FIXTURES.iter().map(|(path, contents)| {
        WorkspaceFileChange::Set {
            path: (*path).to_owned(),
            contents: (*contents).to_owned(),
        }
    }));
    workspace.set_entry(entry);
    workspace
}

fn fixture_file(workspace: &Workspace, name: &str) -> SourceFile {
    workspace
        .db()
        .source_file(format!("/main/{name}"))
        .unwrap_or_else(|| panic!("fixture {name}"))
}

fn checked_hull_program(db: &AnalysisHost, file: SourceFile) -> hull::Program<'_> {
    compiler::build_checked_hull(db, file, Default::default())
        .unwrap_or_else(|diagnostics| panic!("{} compiler diagnostics", diagnostics.len()))
        .program
}

/// Compiles the contracts of each of `files`.
fn fixture_creation_bytecode(workspace: &Workspace, files: &[&str]) -> CreationBytecode {
    let mut all = CreationBytecode::new();
    for file in files {
        let program = checked_hull_program(workspace.db(), fixture_file(workspace, file));
        merge_creation_bytecode(
            &mut all,
            creation_bytecode(backend(), workspace.db(), &program),
        );
    }
    all
}

/// Runs `test_contract` from `entry`, with the contracts of `files` available
/// to `getCode`.
fn run_fixture(
    workspace: &Workspace,
    entry: &str,
    files: &[&str],
    test_contract: &str,
) -> ContractReport {
    let creation_bytecode = fixture_creation_bytecode(workspace, files);
    let contracts = test_contracts(workspace.db(), fixture_file(workspace, entry))
        .into_iter()
        .filter(|contract| contract.name == test_contract)
        .collect::<Vec<_>>();
    assert_eq!(contracts.len(), 1, "test contract {test_contract}");
    let report = run_test_contracts(&creation_bytecode, &contracts)
        .pop()
        .expect("one report");
    println!("{report:#?}");
    report
}

/// The revert message of each failing test, by signature.
fn failures(report: &ContractReport) -> Vec<(&str, &str)> {
    report
        .tests
        .iter()
        .filter_map(|test| {
            test.result
                .as_ref()
                .err()
                .map(|message| (test.signature.as_str(), message.as_str()))
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

#[test]
fn test_contract_calls_a_token_deployed_by_the_runner() {
    let workspace = fixture_workspace("token_test.sol");
    let bytecode = fixture_creation_bytecode(&workspace, &["token_test.sol"]);
    let init = |contract: &str| bytecode[contract].clone().expect("compiled");
    let mut chain = TestChain::new(CreationBytecode::new());

    let token = chain.deploy(DEPLOYER, &init("Token"), &[]).unwrap();
    let test_contract = chain
        .deploy(DEPLOYER, &init("TokenTest"), &abi::encode_address(token))
        .unwrap();
    let transfer_calldata = [
        abi::selector("transfer(address,uint256)"),
        abi::encode_address(test_contract),
        abi::encode_u256(100),
    ]
    .concat();
    chain
        .call(DEPLOYER, token, transfer_calldata, true)
        .result
        .unwrap();

    let token_test = test_contracts(workspace.db(), fixture_file(&workspace, "token_test.sol"))
        .into_iter()
        .find(|contract| contract.name == "TokenTest")
        .expect("TokenTest");
    let failures = token_test
        .test_functions
        .iter()
        .filter_map(|signature| {
            let execution = chain.call(DEPLOYER, test_contract, abi::selector(signature), false);
            execution
                .result
                .err()
                .map(|message| (signature.as_str(), message))
        })
        .collect::<Vec<_>>();
    assert_eq!(token_test.test_functions.len(), 4);
    assert_eq!(
        failures,
        [(
            "testFailsOnPurpose()",
            "deliberately failing check".to_owned()
        )]
    );
}

#[test]
fn creating_another_contract_fails_without_its_deploy_object() {
    let workspace = fixture_workspace("token_test.sol");
    let bytecode = fixture_creation_bytecode(&workspace, &["token_test.sol"]);
    let error = bytecode["TokenSetUpTest"].clone().unwrap_err();
    println!("{:?}: {error}", backend());
}

#[test]
fn set_up_deploys_the_token_from_an_embedded_deploy_object() {
    let workspace = fixture_workspace("token_test.sol");
    let file = fixture_file(&workspace, "token_test.sol");
    let mut program = checked_hull_program(workspace.db(), file);
    embed_deploy_object(&mut program, "TokenSetUpTest", "Token");
    let bytecode = creation_bytecode(backend(), workspace.db(), &program);
    let contracts = test_contracts(workspace.db(), file)
        .into_iter()
        .filter(|contract| contract.name == "TokenSetUpTest")
        .collect::<Vec<_>>();

    let report = run_test_contracts(&bytecode, &contracts).pop().unwrap();
    assert_eq!(report.set_up, Ok(()));
    assert_eq!(report.tests.len(), 2);
    assert!(failures(&report).is_empty(), "{report:?}");
}

#[test]
fn cheatcodes_deploy_the_token_and_change_the_sender() {
    let workspace = fixture_workspace("token_test.sol");
    let report = run_fixture(
        &workspace,
        "token_test.sol",
        &["token_test.sol"],
        "TokenCheatcodeTest",
    );
    assert_eq!(report.set_up, Ok(()));
    assert_eq!(report.tests.len(), 5);
    assert_eq!(
        failures(&report),
        [(
            "testMisspelledContractNameFailsWhenTheTestRuns()",
            "getCode failed"
        )]
    );
}

#[test]
fn prank_makes_the_guarded_function_reject_the_call() {
    let workspace = fixture_workspace("guarded_test.sol");
    let report = run_fixture(
        &workspace,
        "guarded_test.sol",
        &["guarded_test.sol"],
        "GuardedTest",
    );
    assert_eq!(report.set_up, Ok(()));
    assert_eq!(report.tests.len(), 2);
    assert!(failures(&report).is_empty(), "{report:?}");
}

#[test]
fn an_imported_contract_is_not_in_the_entry_program() {
    let workspace = fixture_workspace("counter_test.sol");
    let report = run_fixture(
        &workspace,
        "counter_test.sol",
        &["counter_test.sol"],
        "CounterTest",
    );
    assert_eq!(
        report.set_up,
        Err(SetUpFailure {
            step: SetUpStep::SetUp,
            message: "getCode failed".to_owned(),
        })
    );
}

#[test]
fn set_up_deploys_a_contract_from_an_imported_module() {
    let workspace = fixture_workspace("counter_test.sol");
    let report = run_fixture(
        &workspace,
        "counter_test.sol",
        &["counter_test.sol", "counter.sol"],
        "CounterTest",
    );
    assert_eq!(report.set_up, Ok(()));
    assert_eq!(
        failures(&report),
        [("testFailsOnPurpose()", "deliberately failing check")]
    );
}
