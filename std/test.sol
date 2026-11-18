pragma solcore;

import * from std;
import * from std.Generic;
import { mload, create, shr } from std.opcodes;

export { prank, getCode, deployCode, Arbitrary, arbitrary };

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

// --- Random values for fuzz tests ---

// Builds values of type `a` from a seed. `generate` returns the value and the
// seed for the next value.
trait Arbitrary<a> {
    function generate(p: Proxy<a>, seed: word) returns ((a, word));
}

// A value of type `a` built from `seed`, such as a fuzz test's argument.
function arbitrary<a>(seed: uint256) returns (a) where a: Arbitrary {
    let p: Proxy<a>;
    match (Arbitrary.generate(p, Typedef.rep(seed))) {
        case (value, _) {
            return value;
        }
    }
}

impl Arbitrary<uint256> {
    function generate(p: Proxy<uint256>, seed: word) returns ((uint256, word)) {
        let next = hash1(seed);
        let value: uint256 = Typedef.abs(next);
        return (value, next);
    }
}

impl Arbitrary<address> {
    function generate(p: Proxy<address>, seed: word) returns ((address, word)) {
        let next = hash1(seed);
        let value: address = Typedef.abs(shr(96, next));
        return (value, next);
    }
}

impl Arbitrary<bool> {
    function generate(p: Proxy<bool>, seed: word) returns ((bool, word)) {
        let next = hash1(seed);
        return (tobool(shr(255, next)), next);
    }
}

// The cases below build any type with a Generic instance from its
// representation: units, pairs and sums.

impl Arbitrary<()> {
    function generate(p: Proxy<()>, seed: word) returns (((), word)) {
        return ((), seed);
    }
}

impl<f, g> Arbitrary<(f, g)> where f: Arbitrary, g: Arbitrary {
    function generate(p: Proxy<(f, g)>, seed: word) returns (((f, g), word)) {
        let pf: Proxy<f>;
        let pg: Proxy<g>;
        match (Arbitrary.generate(pf, seed)) {
            case (first, afterFirst) {
                match (Arbitrary.generate(pg, afterFirst)) {
                    case (second, afterSecond) {
                        return ((first, second), afterSecond);
                    }
                }
            }
        }
    }
}

// Picks either side with equal chance, so in a nested sum the later
// constructors are picked less often than the first.
impl<f, g> Arbitrary<sum<f, g>> where f: Arbitrary, g: Arbitrary {
    function generate(p: Proxy<sum<f, g>>, seed: word) returns ((sum<f, g>, word)) {
        let next = hash1(seed);
        match (shr(255, next)) {
            case 0 {
                let pf: Proxy<f>;
                match (Arbitrary.generate(pf, next)) {
                    case (left, afterLeft) {
                        return (inl(left), afterLeft);
                    }
                }
            }
            default {
                let pg: Proxy<g>;
                match (Arbitrary.generate(pg, next)) {
                    case (right, afterRight) {
                        return (inr(right), afterRight);
                    }
                }
            }
        }
    }
}

default impl<a, rep> Arbitrary<a> where a: Generic<rep>, rep: Arbitrary {
    function generate(p: Proxy<a>, seed: word) returns ((a, word)) {
        let prep: Proxy<rep>;
        match (Arbitrary.generate(prep, seed)) {
            case (representation, next) {
                return (Generic.to(representation), next);
            }
        }
    }
}
