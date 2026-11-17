pragma solcore;

import * from std;
import * from std.dispatch;
import { thisAddress } from external_calls;
import { callBalanceOf, tryTransfer, callTransfer } from token_client;
import { prank, deployCode } from cheatcodes;

contract Token {
    balances: mapping(address => uint256);

    constructor() {
        balances[msgSender()] = uint256(1000);
    }

    function transfer(to: address, amount: uint256) public returns (bool) {
        let from = msgSender();
        require(balances[from] >= amount, "insufficient balance");
        balances[from] = balances[from] - amount;
        balances[to] = balances[to] + amount;
        return true;
    }

    function balanceOf(who: address) public returns (uint256) {
        return balances[who];
    }
}

// The runner deploys Token, deploys TokenTest with the token's address, and
// transfers 100 tokens to TokenTest before it calls each test function.
contract TokenTest {
    token: address;

    constructor(token_: address) {
        token = token_;
    }

    function testHoldsTransferredTokens() public {
        require(callBalanceOf(token, thisAddress()) == uint256(100), "expected 100 tokens");
    }

    function testTransferMovesBalance() public {
        let other = address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);
        callTransfer(token, other, uint256(40));
        require(callBalanceOf(token, thisAddress()) == uint256(60), "sender balance");
        require(callBalanceOf(token, other) == uint256(40), "receiver balance");
    }

    function testTransferAboveBalanceReverts() public {
        let other = address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);
        require(not(tryTransfer(token, other, uint256(101))), "transfer above balance succeeded");
        require(callBalanceOf(token, thisAddress()) == uint256(100), "balance changed");
    }

    function testFailsOnPurpose() public {
        require(callBalanceOf(token, thisAddress()) == uint256(1), "deliberately failing check");
    }
}

// Deploys its own Token in setUp, so this contract holds the whole supply.
// The compiler does not put the TokenDeploy object inside this contract's
// runtime object, so this contract compiles only after the runner adds it with
// `embed_deploy_object`.
contract TokenSetUpTest {
    token: address;

    function setUp() public {
        let deployed: word;
        assembly {
            let size := datasize("TokenDeploy")
            let ptr := mload(64)
            datacopy(ptr, dataoffset("TokenDeploy"), size)
            deployed := create(0, ptr, size)
        }
        require(deployed != 0, "token deployment failed");
        token = address(deployed);
    }

    function testDeployerHoldsSupply() public {
        require(callBalanceOf(token, thisAddress()) == uint256(1000), "expected 1000 tokens");
    }

    function testTransferMovesBalance() public {
        let other = address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);
        callTransfer(token, other, uint256(250));
        require(callBalanceOf(token, thisAddress()) == uint256(750), "sender balance");
        require(callBalanceOf(token, other) == uint256(250), "receiver balance");
    }
}

// Deploys its own Token in setUp with the deployCode cheatcode, so this
// contract holds the whole supply, and acts as other accounts with prank.
contract TokenCheatcodeTest {
    token: address;

    function setUp() public {
        token = deployCode("Token");
    }

    function testDeployerHoldsSupply() public {
        require(callBalanceOf(token, thisAddress()) == uint256(1000), "expected 1000 tokens");
    }

    function testTransferFromAccountWithoutTokensReverts() public {
        prank(address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8));
        require(not(tryTransfer(token, thisAddress(), uint256(1))), "transfer from an empty account succeeded");
    }

    function testPrankedTransferMovesTheSendersBalance() public {
        let other = address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);
        callTransfer(token, other, uint256(300));
        prank(other);
        callTransfer(token, thisAddress(), uint256(100));
        require(callBalanceOf(token, other) == uint256(200), "other balance");
        require(callBalanceOf(token, thisAddress()) == uint256(800), "test contract balance");
    }

    function testPrankAppliesToTheNextCallOnly() public {
        let other = address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8);
        prank(other);
        require(callBalanceOf(token, other) == uint256(0), "other balance");
        callTransfer(token, other, uint256(5));
        require(callBalanceOf(token, other) == uint256(5), "transfer after the prank");
    }

    function testMisspelledContractNameFailsWhenTheTestRuns() public {
        let misspelled = deployCode("Tokn");
    }
}
