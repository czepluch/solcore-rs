# solcore-contract-tests

A research prototype for tests written in Solcore and run with `solcore test`.
It is not meant to be merged in its current form.

## Running tests

```sh
cargo build -p solcore-driver
target/debug/solcore test --std-root std crates/contract-tests/tests/fixtures/counter_test.sol
```

The output follows Forge's layout: one `[PASS]` or `[FAIL: <reason>]` line per test with its gas, a result per test contract, and a total.
The exit code is 0 when every test passes, 1 when a test fails, the code does not compile or there are no test contracts, and 2 for a usage error.

`--backend sonatina|solc` picks the backend.
Sonatina is the default and needs no other tools; `solc` runs the executable named by `SOLC`, or `solc` on `PATH`.
`--fuzz-seed N` repeats the arguments of an earlier fuzz run.

## Writing tests

A test contract is a contract with public functions whose names start with `test`.
For each test contract, the runner deploys it on a fresh in-memory revm chain, calls its `setUp()` when it has one, and calls each test function from the state `setUp()` leaves.
A test fails when it reverts.

```solidity
import { prank, deployCode } from std.test;

contract GuardedTest {
    guarded: address;

    function setUp() public {
        guarded = deployCode("Guarded");
    }

    function testOtherAccountIsRejected() public {
        prank(address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8));
        require(not(tryOnlyAdmin(guarded)), "call from another account succeeded");
    }
}
```

`tryOnlyAdmin` calls `Guarded.onlyAdmin()` with `raw_call` (see `tests/fixtures/guarded_test.sol`).

`std.test` provides:

- `prank(sender)`: the next call from the test contract comes from `sender`.
- `getCode(name)` and `deployCode(name)`: the creation bytecode of a contract from the compiled project, and a deployment of it.
- `arbitrary(seed)`: a value of any type with a `Generic` instance, such as an enum, built from a `uint256` seed. The module that defines the type must import `std.Generic`.

The cheatcodes use forge-std's `Vm` signatures at forge-std's cheatcode address, and the runner answers them.

A test function with parameters is a fuzz test.
It runs up to 256 times with generated arguments and stops at the first failing run, whose arguments the output shows.
Supported parameter types are `uintN`, `intN`, `address`, `bool` and `bytesN`.
For other types, such as enums, take a `uint256` seed and call `arbitrary(seed)`.

## Layout

- `src/`: the chain, the cheatcodes, test discovery, fuzz arguments and the runner.
- `tests/fixtures/`: example test contracts, run by `tests/run_test_contracts.rs`.
- `crates/driver/src/test_command.rs`: the `solcore test` command.
- `crates/evm-backend/`: bytecode generation, shared with the playground.
- `std/test.sol`: the cheatcodes and `Arbitrary`.

## Known gaps

- Solcore cannot deploy a contract from its source, so `deployCode` takes the contract name as a string, which is checked only when the test runs.
- Solcore has no typed external calls, so tests call contracts through hand-written `raw_call` wrappers.
- Test contracts are deployed without constructor arguments.
- Fuzzing has no `assume` or `bound`, does not shrink failing inputs, and does not generate dynamic types.
- `arbitrary` picks the first constructor of an enum about half the time, and gives numbers over their full range.
- Revert data is printed as text when it is printable and as hex otherwise; panics and std's errors are not decoded.

## Running under Foundry

The same compiled test contracts also ran under Forge 1.8.5, through generated Solidity wrappers (not in this branch) that deploy each test contract and forward its tests.
`prank` and both kinds of fuzz tests work there.
Under Forge, `deployCode` needs an artifact path such as `build/Counter.json`, which this runner does not accept, and test names that start with `testFail` are rejected.
