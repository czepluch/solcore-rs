//! The ABI encoding the runner needs for calldata and cheatcode answers.

use revm::primitives::{Address, U256, keccak256};

/// The 4-byte selector of a function signature such as `transfer(address,uint256)`.
pub fn selector(signature: &str) -> Vec<u8> {
    keccak256(signature.as_bytes())[..4].to_vec()
}

pub fn encode_address(address: Address) -> Vec<u8> {
    [&[0; 12], address.as_slice()].concat()
}

pub fn encode_u256(value: u64) -> Vec<u8> {
    U256::from(value).to_be_bytes::<32>().to_vec()
}

/// Encodes `bytes` as the only value: offset, length and data padded to a word.
pub fn encode_bytes(bytes: &[u8]) -> Vec<u8> {
    let padding = (32 - bytes.len() % 32) % 32;
    [
        encode_u256(32),
        encode_u256(bytes.len() as u64),
        bytes.to_vec(),
        vec![0; padding],
    ]
    .concat()
}

pub(crate) fn decode_address(arguments: &[u8]) -> Result<Address, String> {
    let word = word_at(arguments, 0)?;
    Ok(Address::from_slice(&word[12..]))
}

/// Decodes a `string` that is the only argument.
pub(crate) fn decode_string(arguments: &[u8]) -> Result<String, String> {
    let offset = word_as_usize(word_at(arguments, 0)?)?;
    let length = word_as_usize(word_at(arguments, offset)?)?;
    let bytes = arguments
        .get(offset + 32..offset + 32 + length)
        .ok_or("ABI string is shorter than its length")?;
    String::from_utf8(bytes.to_vec()).map_err(|error| error.to_string())
}

fn word_at(data: &[u8], offset: usize) -> Result<&[u8], String> {
    data.get(offset..offset + 32)
        .ok_or_else(|| format!("ABI data has no word at offset {offset}"))
}

fn word_as_usize(word: &[u8]) -> Result<usize, String> {
    let (high, low) = word.split_at(24);
    if high.iter().any(|byte| *byte != 0) {
        return Err("ABI offset or length does not fit in usize".to_owned());
    }
    Ok(u64::from_be_bytes(low.try_into().expect("8 bytes")) as usize)
}

pub fn to_hex(bytes: &[u8]) -> String {
    format!(
        "0x{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
