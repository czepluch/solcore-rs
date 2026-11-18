pragma solcore;

import * from std;
import * from std.dispatch;
import * from std.signed;
import * from std.unsigned;
import { prank, deployCode } from std.test;
import { thisAddress } from external_calls;
import { Guarded, tryOnlyAdmin } from guarded_test;

// Test functions with parameters, which the runner calls with generated arguments.
contract FuzzTest {
    // Fails when x is the largest uint256, which the runner tries as an edge case.
    function testAddingOneIncreasesTheValue(x: uint256) public {
        require(x + uint256(1) > x, "x + 1 is not greater than x");
    }

    // Fails when std's decoder rejects an argument as outside its type's range.
    function testAcceptsEveryParameterType(a: uint8, b: int8, c: int256, d: address, e: bool, f: bytes4, g: bytes32) public {
    }

    // The runner does not generate strings.
    function testTakesText(text: memory<string>) public {
    }
}

// Deploys Guarded in setUp, so this contract is its admin.
contract GuardedFuzzTest {
    guarded: address;

    function setUp() public {
        guarded = deployCode("Guarded");
    }

    function testEveryOtherSenderIsRejected(sender: address) public {
        if (sender != thisAddress()) {
            prank(sender);
            require(not(tryOnlyAdmin(guarded)), "a call from another sender succeeded");
        }
    }
}
