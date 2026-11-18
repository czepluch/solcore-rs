//! Compiles contracts to init bytecode and runs test contracts.

use std::collections::{BTreeMap, btree_map::Entry};

use solcore_evm_backend::{self as backend, Backend};

use crate::{
    abi::selector,
    chain::{DEPLOYER, TestChain},
    discovery::TestContract,
};

/// The init bytecode of each contract by name, or the error from compiling it.
pub type CreationBytecode = BTreeMap<String, Result<Vec<u8>, String>>;

#[derive(Clone, Debug)]
pub struct TestReport {
    pub signature: String,
    /// The revert message of a failing test.
    pub result: Result<(), String>,
    pub gas_used: u64,
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
pub fn run_test_contracts(
    creation_bytecode: &CreationBytecode,
    test_contracts: &[TestContract],
) -> Vec<ContractReport> {
    test_contracts
        .iter()
        .map(|test_contract| run_test_contract(creation_bytecode, test_contract))
        .collect()
}

fn run_test_contract(
    creation_bytecode: &CreationBytecode,
    test_contract: &TestContract,
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
        .map(|signature| {
            let execution = chain.call(DEPLOYER, address, selector(signature), false);
            TestReport {
                signature: signature.clone(),
                result: execution.result.map(|_| ()),
                gas_used: execution.gas_used,
            }
        })
        .collect();
    ContractReport {
        contract: test_contract.name.clone(),
        set_up: Ok(()),
        tests,
    }
}
