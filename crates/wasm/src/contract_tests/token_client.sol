pragma solcore;

import * from std;
import { mload, address as addressOpcode } from std.opcodes;

export { thisAddress, selector, callBalanceOf, tryTransfer, callTransfer };

// The address of the contract running this code.
function thisAddress() returns (address) {
    return address(addressOpcode());
}

// The 4-byte selector of a function signature, computed at compile time.
function selector(comptime sig: string) returns (memory_ref) {
    return truncate(to_bytes(bytes32(keccakLit(sig))), 4);
}

// Calls token.balanceOf(who) and returns the first word of the return data.
function callBalanceOf(token: address, who: address) returns (uint256) {
    let payload = concat(selector("balanceOf(address)"), abi_encode(who));
    match (raw_call(token, uint256(0), payload)) {
        case (ok, ret) {
            require(ok, "balanceOf call failed");
            return uint256(mload(Typedef.rep(ret) + 32));
        }
    }
}

// Calls token.transfer(to, amount) and returns whether the call succeeded.
function tryTransfer(token: address, to: address, amount: uint256) returns (bool) {
    let payload = concat(selector("transfer(address,uint256)"), abi_encode((to, amount)));
    match (raw_call(token, uint256(0), payload)) {
        case (ok, _) {
            return ok;
        }
    }
}

// Calls token.transfer(to, amount) and reverts if the call fails.
function callTransfer(token: address, to: address, amount: uint256) {
    require(tryTransfer(token, to, amount), "transfer call failed");
}
