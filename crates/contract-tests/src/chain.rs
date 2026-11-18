//! An in-memory chain that keeps every deployment and answers cheatcodes.

use revm::{
    Context, InspectCommitEvm, InspectEvm, MainBuilder, MainContext,
    context::TxEnv,
    context_interface::result::{ExecutionResult, Output},
    database::InMemoryDB,
    primitives::{
        Address, Bytes, TxKind, U256, address, eip7825::TX_GAS_LIMIT_CAP, hardfork::SpecId, uint,
    },
    state::AccountInfo,
};

use crate::{abi::to_hex, cheatcodes::Cheatcodes, runner::CreationBytecode};

/// The account that deploys test contracts and sends test calls: the first of
/// anvil's default accounts, as in the playground.
pub const DEPLOYER: Address = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");

/// 10,000 ether.
const STARTING_BALANCE: U256 = uint!(10_000_000_000_000_000_000_000_U256);
const BLOCK_GAS_LIMIT: u64 = 30_000_000;
const MEMORY_LIMIT: u64 = 16 * 1024 * 1024;

/// The result of one transaction.
#[derive(Clone, Debug)]
pub struct Execution {
    /// The return data, or the revert message.
    pub result: Result<Vec<u8>, String>,
    pub gas_used: u64,
}

pub struct TestChain {
    db: InMemoryDB,
    cheatcodes: Cheatcodes,
}

impl TestChain {
    /// A chain whose `getCode` cheatcode answers from `creation_bytecode`.
    pub fn new(creation_bytecode: CreationBytecode) -> Self {
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            DEPLOYER,
            AccountInfo::default().with_balance(STARTING_BALANCE),
        );
        Self {
            db,
            cheatcodes: Cheatcodes::new(creation_bytecode),
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
                block.gas_limit = BLOCK_GAS_LIMIT;
                block.number = U256::from(1);
                block.timestamp = U256::from(1);
            })
            .build_mainnet_with_inspector(std::mem::take(&mut self.cheatcodes));
        let tx = TxEnv::builder()
            .caller(caller)
            .kind(kind)
            .data(data)
            .nonce(nonce)
            .gas_limit(TX_GAS_LIMIT_CAP)
            .gas_price(0)
            .build_fill();
        let result = if commit {
            evm.inspect_tx_commit(tx)
        } else {
            evm.inspect_tx(tx).map(|output| output.result)
        };
        self.cheatcodes = evm.inspector;
        // A prank does not outlive the transaction that set it.
        self.cheatcodes.clear_prank();
        result.expect("transaction is valid")
    }

    /// Deploys `init_bytecode` followed by the ABI-encoded constructor
    /// arguments, and returns the new contract's address.
    pub fn deploy(
        &mut self,
        caller: Address,
        init_bytecode: &[u8],
        constructor_arguments: &[u8],
    ) -> Result<Address, String> {
        let code = [init_bytecode, constructor_arguments].concat();
        match self.transact(caller, TxKind::Create, code.into(), true) {
            ExecutionResult::Success {
                output: Output::Create(_, Some(address)),
                ..
            } => Ok(address),
            other => Err(execution(&other)
                .result
                .err()
                .unwrap_or_else(|| "deployment returned no address".to_owned())),
        }
    }

    /// Calls `to`, and keeps the state changes only when `commit` is set.
    pub fn call(
        &mut self,
        caller: Address,
        to: Address,
        calldata: Vec<u8>,
        commit: bool,
    ) -> Execution {
        execution(&self.transact(caller, TxKind::Call(to), calldata.into(), commit))
    }
}

fn execution(result: &ExecutionResult) -> Execution {
    Execution {
        result: match result {
            ExecutionResult::Success { output, .. } => Ok(output.data().to_vec()),
            ExecutionResult::Revert { output, .. } => Err(revert_message(output)),
            ExecutionResult::Halt { reason, .. } => Err(format!("halted: {reason:?}")),
        },
        gas_used: result.tx_gas_used(),
    }
}

/// Reads a revert payload as text when it is printable, and as hex otherwise.
fn revert_message(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(text) if !text.is_empty() && !text.chars().any(char::is_control) => text.to_owned(),
        _ => to_hex(data),
    }
}
