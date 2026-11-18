pragma solcore;

import * from std;
import * from std.dispatch;
import { mload } from std.opcodes;
import { deployCode } from std.test;
import { selector } from external_calls;
import { Counter } from counter;

// Calls counter.current().
function callCurrent(counter: address) returns (uint256) {
    match (raw_call(counter, uint256(0), selector("current()"))) {
        case (ok, ret) {
            require(ok, "current() call failed");
            return uint256(mload(Typedef.rep(ret) + 32));
        }
    }
}

// Calls counter.increment().
function callIncrement(counter: address) {
    match (raw_call(counter, uint256(0), selector("increment()"))) {
        case (ok, _) {
            require(ok, "increment() call failed");
        }
    }
}

// Deploys Counter, which counter.sol defines, in setUp.
contract CounterTest {
    counter: address;

    function setUp() public {
        counter = deployCode("Counter");
    }

    function testStartsAtZero() public {
        require(callCurrent(counter) == uint256(0), "expected 0");
    }

    function testIncrementAddsOne() public {
        callIncrement(counter);
        callIncrement(counter);
        require(callCurrent(counter) == uint256(2), "expected 2");
    }

    function testFailsOnPurpose() public {
        require(callCurrent(counter) == uint256(1), "deliberately failing check");
    }
}
