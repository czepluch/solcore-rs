//! Runs tests written as Solcore contracts.
//!
//! A test contract is a contract with public functions whose names start with
//! `test`. The runner deploys it on a fresh in-memory chain, calls its
//! `setUp()` when it has one, and calls each test function from the state
//! `setUp()` leaves. Calls to forge-std's `VM_ADDRESS` are answered as
//! cheatcodes; `std.test` provides the Solcore functions that make them.

pub mod abi;
mod chain;
mod cheatcodes;
mod discovery;
mod runner;

pub use chain::{DEPLOYER, Execution, TestChain};
pub use cheatcodes::CHEATCODE_ADDRESS;
pub use discovery::{TestContract, defines_contracts, test_contracts};
pub use runner::{
    ContractReport, CreationBytecode, SetUpFailure, SetUpStep, TestReport, creation_bytecode,
    merge_creation_bytecode, run_test_contracts,
};
