pragma solcore;

import * from std;
import * from std.dispatch;

export { Counter };

contract Counter {
    count: uint256;

    function increment() public {
        count = count + uint256(1);
    }

    function current() public returns (uint256) {
        return count;
    }
}
