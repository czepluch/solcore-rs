pragma solcore;

import * from std;
import { mload, create } from std.opcodes;

export { prank, getCode, deployCode };

// --- Test cheatcodes ---
// Functions that `solcore test` answers at forge-std's VM_ADDRESS, called with
// the signatures of forge-std's Vm interface.

function cheatcodeAddress() returns (address) {
    return address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
}

// The calldata of a cheatcode call: the 4-byte selector of `sig`, computed at
// compile time, followed by `arguments`.
function cheatcodeCalldata<a>(comptime sig: string, arguments: a) returns (memory<bytes>) where a: MemorySize, a: MemoryEncode {
    return concat(truncate(to_bytes(bytes32(keccakLit(sig))), 4), arguments);
}

// Makes the next call from this contract come from `sender`.
function prank(sender: address) {
    let payload = cheatcodeCalldata("prank(address)", abi_encode(sender));
    match (raw_call(cheatcodeAddress(), uint256(0), payload)) {
        case (ok, _) {
            require(ok, "prank failed");
        }
    }
}

// Returns the creation bytecode of the contract named `name`.
function getCode(name: memory<string>) returns (memory<bytes>) {
    let payload = cheatcodeCalldata("getCode(string)", abi_encode(name));
    match (raw_call(cheatcodeAddress(), uint256(0), payload)) {
        case (ok, ret) {
            require(ok, "getCode failed");
            // The return data starts after the length word of `ret`. Its first
            // word is the offset of the bytes, which are stored as length and data.
            let returnData = Typedef.rep(ret) + 32;
            return memory(returnData + mload(returnData));
        }
    }
}

// Deploys the contract named `name` and returns its address.
// TODO: replace with a compile-time checked way to deploy a contract once the language has one.
function deployCode(name: memory<string>) returns (address) {
    let code = Typedef.rep(getCode(name));
    let deployed = create(0, code + 32, mload(code));
    require(deployed != 0, "deployCode failed");
    return address(deployed);
}
