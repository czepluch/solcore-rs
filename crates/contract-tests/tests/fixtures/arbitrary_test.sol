pragma solcore;

import * from std;
import * from std.dispatch;
import * from std.Generic;
import { arbitrary } from std.test;

// A change to a balance.
enum Change {
    Deposit(uint256),
    Withdraw(uint256),
    Reset
}

// A change made by an account.
enum Payment {
    Payment(address, Change)
}

// The balance after `change`. Reverts when a deposit overflows or a
// withdrawal is larger than the balance.
function applyChange(balance: uint256, change: Change) returns (uint256) {
    match (change) {
        case Change.Deposit(amount) {
            return balance + amount;
        }
        case Change.Withdraw(amount) {
            return balance - amount;
        }
        case Change.Reset {
            return uint256(0);
        }
    }
}

// 0 for Deposit, 1 for Withdraw and 2 for Reset.
function constructorNumber(change: Change) returns (uint256) {
    match (change) {
        case Change.Deposit(_) {
            return uint256(0);
        }
        case Change.Withdraw(_) {
            return uint256(1);
        }
        case Change.Reset {
            return uint256(2);
        }
    }
}

// Builds Change and Payment values from the seed the runner generates.
contract ArbitraryTest {
    // Fails on a withdrawal larger than the balance.
    function testApplyingAChangeNeverReverts(seed: uint256) public {
        let change: Change = arbitrary(seed);
        let balance = applyChange(uint256(100), change);
    }

    function testAPaymentKeepsItsChange(seed: uint256) public {
        let payment: Payment = arbitrary(seed);
        match (payment) {
            case Payment.Payment(_, change) {
                require(constructorNumber(change) <= uint256(2), "unknown change");
            }
        }
    }

    // The constructor number of the Change built from `seed`.
    function changeConstructor(seed: uint256) public returns (uint256) {
        let change: Change = arbitrary(seed);
        return constructorNumber(change);
    }
}
