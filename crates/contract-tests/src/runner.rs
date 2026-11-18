//! Compiles contracts to init bytecode and runs test contracts.

use std::collections::{BTreeMap, btree_map::Entry};

use fastrand::Rng;
use revm::primitives::Address;
use solcore_evm_backend::{self as backend, Backend};

use crate::{
    abi::selector,
    chain::{DEPLOYER, TestChain},
    discovery::TestContract,
    fuzz::{FUZZ_RUNS, ParameterType, parameter_types},
};

/// The init bytecode of each contract by name, or the error from compiling it.
pub type CreationBytecode = BTreeMap<String, Result<Vec<u8>, String>>;

#[derive(Clone, Debug)]
pub struct TestReport {
    pub signature: String,
    /// The revert message of a failing test, or why it could not run.
    pub result: Result<(), String>,
    pub kind: TestKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestKind {
    /// A test function without parameters, called once.
    Unit { gas_used: u64 },
    /// A test function with parameters, called with generated arguments.
    Fuzz(FuzzRuns),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FuzzRuns {
    /// The number of runs, including the failing one.
    pub runs: u32,
    pub mean_gas: u64,
    pub median_gas: u64,
    /// The arguments of the failing run.
    pub counterexample: Option<Counterexample>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counterexample {
    pub calldata: Vec<u8>,
    /// Each argument written the way Solidity source writes it.
    pub arguments: Vec<String>,
}

/// The step that stopped a test contract before its tests ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetUpStep {
    Compile,
    Deploy,
    SetUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetUpFailure {
    pub step: SetUpStep,
    /// The compiler error or revert message.
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct ContractReport {
    pub contract: String,
    /// Its tests do not run when compiling, deploying or `setUp()` failed.
    pub set_up: Result<(), SetUpFailure>,
    pub tests: Vec<TestReport>,
}

/// Compiles each contract of `program` to init bytecode.
pub fn creation_bytecode(
    backend: Backend,
    db: &dyn hir::Db,
    program: &hull::Program<'_>,
) -> CreationBytecode {
    program
        .objects
        .iter()
        .filter_map(|object| object.name.as_str().strip_suffix("Deploy"))
        .map(|contract| {
            let code = init_bytecode(backend, db, program, contract);
            (contract.to_owned(), code)
        })
        .collect()
}

/// Adds the contracts of `more` to `all`. A name that both define maps to an
/// error.
pub fn merge_creation_bytecode(all: &mut CreationBytecode, more: CreationBytecode) {
    for (contract, code) in more {
        match all.entry(contract) {
            Entry::Vacant(entry) => {
                entry.insert(code);
            }
            Entry::Occupied(mut entry) => {
                *entry.get_mut() = Err("more than one contract has this name".to_owned());
            }
        }
    }
}

/// Compiles the deploy object of `contract` to init bytecode.
fn init_bytecode(
    backend: Backend,
    db: &dyn hir::Db,
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

/// Deploys each test contract on a fresh chain, calls its `setUp()` when it
/// has one, and calls each test function from the state `setUp()` leaves.
/// A test function with parameters runs up to [`FUZZ_RUNS`] times, with
/// arguments generated from `fuzz_seed`, and stops at the first failing run.
pub fn run_test_contracts(
    creation_bytecode: &CreationBytecode,
    test_contracts: &[TestContract],
    fuzz_seed: u64,
) -> Vec<ContractReport> {
    test_contracts
        .iter()
        .map(|test_contract| run_test_contract(creation_bytecode, test_contract, fuzz_seed))
        .collect()
}

fn run_test_contract(
    creation_bytecode: &CreationBytecode,
    test_contract: &TestContract,
    fuzz_seed: u64,
) -> ContractReport {
    let failed = |step, message| ContractReport {
        contract: test_contract.name.clone(),
        set_up: Err(SetUpFailure { step, message }),
        tests: Vec::new(),
    };
    let mut chain = TestChain::new(creation_bytecode.clone());
    let init = match creation_bytecode.get(&test_contract.name) {
        Some(Ok(init)) => init,
        Some(Err(error)) => return failed(SetUpStep::Compile, error.clone()),
        None => return failed(SetUpStep::Compile, "was not compiled".to_owned()),
    };
    let address = match chain.deploy(DEPLOYER, init, &[]) {
        Ok(address) => address,
        Err(message) => return failed(SetUpStep::Deploy, message),
    };
    if test_contract.has_set_up {
        let set_up = chain.call(DEPLOYER, address, selector("setUp()"), true);
        if let Err(message) = set_up.result {
            return failed(SetUpStep::SetUp, message);
        }
    }
    let tests = test_contract
        .test_functions
        .iter()
        .map(|signature| run_test(&mut chain, address, signature, fuzz_seed))
        .collect();
    ContractReport {
        contract: test_contract.name.clone(),
        set_up: Ok(()),
        tests,
    }
}

fn run_test(
    chain: &mut TestChain,
    address: Address,
    signature: &str,
    fuzz_seed: u64,
) -> TestReport {
    let report = |result, kind| TestReport {
        signature: signature.to_owned(),
        result,
        kind,
    };
    match parameter_types(signature) {
        Ok(types) if types.is_empty() => {
            let execution = chain.call(DEPLOYER, address, selector(signature), false);
            report(
                execution.result.map(|_| ()),
                TestKind::Unit {
                    gas_used: execution.gas_used,
                },
            )
        }
        Ok(types) => {
            let (result, runs) = run_fuzz_test(chain, address, signature, &types, fuzz_seed);
            report(result, TestKind::Fuzz(runs))
        }
        Err(message) => report(Err(message), TestKind::Fuzz(FuzzRuns::default())),
    }
}

/// Calls the test function from the same state with new arguments each run,
/// and stops at the first run that reverts.
fn run_fuzz_test(
    chain: &mut TestChain,
    address: Address,
    signature: &str,
    types: &[ParameterType],
    fuzz_seed: u64,
) -> (Result<(), String>, FuzzRuns) {
    let mut rng = Rng::with_seed(fuzz_seed);
    let selector = selector(signature);
    let mut gas = Vec::new();
    let mut failure = None;
    for _ in 0..FUZZ_RUNS {
        let values = types
            .iter()
            .map(|parameter_type| parameter_type.generate(&mut rng))
            .collect::<Vec<_>>();
        let calldata = [
            selector.clone(),
            values
                .iter()
                .flat_map(|value| value.to_be_bytes::<32>())
                .collect(),
        ]
        .concat();
        let execution = chain.call(DEPLOYER, address, calldata.clone(), false);
        gas.push(execution.gas_used);
        if let Err(message) = execution.result {
            let arguments = types
                .iter()
                .zip(&values)
                .map(|(parameter_type, value)| parameter_type.format(*value))
                .collect();
            failure = Some((
                message,
                Counterexample {
                    calldata,
                    arguments,
                },
            ));
            break;
        }
    }
    let (result, counterexample) = match failure {
        Some((message, counterexample)) => (Err(message), Some(counterexample)),
        None => (Ok(()), None),
    };
    gas.sort_unstable();
    let runs = FuzzRuns {
        runs: gas.len() as u32,
        mean_gas: gas.iter().sum::<u64>() / gas.len() as u64,
        median_gas: gas[gas.len() / 2],
        counterexample,
    };
    (result, runs)
}
