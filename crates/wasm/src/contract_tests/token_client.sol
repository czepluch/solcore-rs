pragma solcore;

import * from std;
import { mload } from std.opcodes;
import { selector } from external_calls;

export { callBalanceOf, tryTransfer, callTransfer };

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
