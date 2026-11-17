pragma solcore;

import * from std;
import * from std.dispatch;
import { selector } from external_calls;
import { prank, deployCode } from cheatcodes;

contract Guarded {
    admin: address;

    constructor() {
        admin = msgSender();
    }

    function onlyAdmin() public returns (bool) {
        require(msgSender() == admin, "not the admin");
        return true;
    }
}

// Calls guarded.onlyAdmin() and returns whether the call succeeded.
function tryOnlyAdmin(guarded: address) returns (bool) {
    match (raw_call(guarded, uint256(0), selector("onlyAdmin()"))) {
        case (ok, _) {
            return ok;
        }
    }
}

// Deploys Guarded in setUp, so this contract is its admin.
contract GuardedTest {
    guarded: address;

    function setUp() public {
        guarded = deployCode("Guarded");
    }

    function testAdminCanCall() public {
        require(tryOnlyAdmin(guarded), "call from the admin reverted");
    }

    function testOtherAccountIsRejected() public {
        prank(address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8));
        require(not(tryOnlyAdmin(guarded)), "call from another account succeeded");
    }
}
