pragma solcore;

import * from std;
import { address as addressOpcode } from std.opcodes;

export { thisAddress, selector };

// The address of the contract running this code.
function thisAddress() returns (address) {
    return address(addressOpcode());
}

// The 4-byte selector of a function signature, computed at compile time.
function selector(comptime sig: string) returns (memory_ref) {
    return truncate(to_bytes(bytes32(keccakLit(sig))), 4);
}
