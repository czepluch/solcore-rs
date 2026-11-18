//! Answers calls to forge-std's `VM_ADDRESS` during test transactions.

use revm::{
    Inspector,
    context_interface::ContextTr,
    interpreter::{CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult},
    primitives::{Address, address},
};

use crate::{
    abi::{decode_address, decode_string, encode_bytes, selector, to_hex},
    runner::CreationBytecode,
};

/// forge-std's `VM_ADDRESS`.
pub const CHEATCODE_ADDRESS: Address = address!("0x7109709ECfa91a80626fF3989D68f67F5b1DD12D");

/// The next call made by `pranker` comes from `sender`.
#[derive(Clone, Copy)]
struct Prank {
    pranker: Address,
    sender: Address,
}

/// Answers `prank(address)` and `getCode(string)`, and applies a pending prank
/// to the next call of the contract that set it.
#[derive(Default)]
pub(crate) struct Cheatcodes {
    creation_bytecode: CreationBytecode,
    prank: Option<Prank>,
}

impl Cheatcodes {
    pub(crate) fn new(creation_bytecode: CreationBytecode) -> Self {
        Self {
            creation_bytecode,
            prank: None,
        }
    }

    pub(crate) fn clear_prank(&mut self) {
        self.prank = None;
    }

    /// Returns the ABI-encoded result of a cheatcode call, or its revert message.
    fn answer(&mut self, caller: Address, calldata: &[u8]) -> Result<Vec<u8>, String> {
        let (called, arguments) = calldata
            .split_at_checked(4)
            .ok_or("cheatcode calldata is shorter than a selector")?;
        if called == selector("prank(address)") {
            let sender = decode_address(arguments)?;
            self.prank = Some(Prank {
                pranker: caller,
                sender,
            });
            Ok(Vec::new())
        } else if called == selector("getCode(string)") {
            let artifact = decode_string(arguments)?;
            // Accepts `Name`, `File.sol` and `File.sol:Name`, as forge-std does.
            let contract = artifact
                .rsplit(':')
                .next()
                .unwrap_or_default()
                .trim_end_matches(".sol");
            match self.creation_bytecode.get(contract) {
                Some(Ok(code)) => Ok(encode_bytes(code)),
                Some(Err(error)) => Err(format!("{contract} does not compile: {error}")),
                None => Err(format!("no contract named {contract}")),
            }
        } else {
            Err(format!("unsupported cheatcode {}", to_hex(called)))
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

#[cfg(test)]
mod tests {
    use crate::abi::{selector, to_hex};

    #[test]
    fn selectors_match_forge_std() {
        assert_eq!(to_hex(&selector("prank(address)")), "0xca669fa7");
        assert_eq!(to_hex(&selector("getCode(string)")), "0x8d1cc925");
    }
}
