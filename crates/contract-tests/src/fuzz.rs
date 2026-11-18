//! Arguments for fuzz tests: test functions with parameters.

use fastrand::Rng;
use revm::primitives::{Address, I256, U256};

use crate::abi::to_hex;

/// The number of runs of a fuzz test that passes.
pub const FUZZ_RUNS: u32 = 256;

/// A seed for [`crate::run_test_contracts`] from the system's randomness.
pub fn random_fuzz_seed() -> u64 {
    fastrand::u64(..)
}

/// A parameter type that the runner generates values for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParameterType {
    /// `uintN`, with the width in bits.
    Uint(usize),
    /// `intN`, with the width in bits.
    Int(usize),
    Address,
    Bool,
    /// `bytesN`, with the length in bytes.
    FixedBytes(usize),
}

/// The parameter types of `signature`, such as `testTransfer(address,uint256)`.
pub(crate) fn parameter_types(signature: &str) -> Result<Vec<ParameterType>, String> {
    let parameters = signature
        .split_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .ok_or_else(|| format!("`{signature}` is not a function signature"))?;
    if parameters.is_empty() {
        return Ok(Vec::new());
    }
    if parameters.contains(['(', '[']) {
        return Err("fuzzing does not support tuple, array or enum parameters".to_owned());
    }
    parameters.split(',').map(parameter_type).collect()
}

fn parameter_type(name: &str) -> Result<ParameterType, String> {
    let unsupported = || format!("fuzzing does not support parameters of type `{name}`");
    let width = |digits: &str, valid: fn(usize) -> bool| {
        digits
            .parse::<usize>()
            .ok()
            .filter(|width| valid(*width))
            .ok_or_else(unsupported)
    };
    let integer_bits = |bits: usize| bits.is_multiple_of(8) && (8..=256).contains(&bits);
    match name {
        "address" => Ok(ParameterType::Address),
        "bool" => Ok(ParameterType::Bool),
        _ => {
            if let Some(digits) = name.strip_prefix("uint") {
                width(digits, integer_bits).map(ParameterType::Uint)
            } else if let Some(digits) = name.strip_prefix("int") {
                width(digits, integer_bits).map(ParameterType::Int)
            } else if let Some(digits) = name.strip_prefix("bytes") {
                width(digits, |length| (1..=32).contains(&length)).map(ParameterType::FixedBytes)
            } else {
                Err(unsupported())
            }
        }
    }
}

impl ParameterType {
    /// A random ABI-encoded value. One value in 4 is an edge case: 0, 1 or
    /// the maximum for unsigned integers, 0, 1, -1, the minimum or the
    /// maximum for signed integers, the zero address, and all zero or all
    /// one bits for fixed-size bytes.
    pub(crate) fn generate(self, rng: &mut Rng) -> U256 {
        let edge_case = rng.usize(..4) == 0;
        match self {
            Self::Uint(bits) => {
                let max = U256::MAX >> (256 - bits);
                if edge_case {
                    pick(rng, &[U256::ZERO, U256::ONE, max])
                } else {
                    random_word(rng) & max
                }
            }
            Self::Int(bits) => {
                let max = U256::MAX >> (257 - bits);
                let min = U256::MAX << (bits - 1);
                if edge_case {
                    pick(rng, &[U256::ZERO, U256::ONE, U256::MAX, min, max])
                } else {
                    sign_extend(random_word(rng), bits)
                }
            }
            Self::Address => {
                let mask = U256::MAX >> 96;
                if edge_case {
                    U256::ZERO
                } else {
                    random_word(rng) & mask
                }
            }
            Self::Bool => U256::from(rng.bool()),
            Self::FixedBytes(length) => {
                let mask = U256::MAX << (256 - 8 * length);
                if edge_case {
                    pick(rng, &[U256::ZERO, mask])
                } else {
                    random_word(rng) & mask
                }
            }
        }
    }

    /// `value` written the way Solidity source writes it.
    pub(crate) fn format(self, value: U256) -> String {
        match self {
            Self::Uint(_) => value.to_string(),
            Self::Int(_) => I256::from_raw(value).to_string(),
            Self::Address => Address::from_word(value.to_be_bytes::<32>().into()).to_string(),
            Self::Bool => (value == U256::ONE).to_string(),
            Self::FixedBytes(length) => to_hex(&value.to_be_bytes::<32>()[..length]),
        }
    }
}

fn random_word(rng: &mut Rng) -> U256 {
    let mut bytes = [0; 32];
    rng.fill(&mut bytes);
    U256::from_be_bytes(bytes)
}

fn pick(rng: &mut Rng, values: &[U256]) -> U256 {
    values[rng.usize(..values.len())]
}

/// Keeps the low `bits` of `value` and copies its sign bit into the bits above.
fn sign_extend(value: U256, bits: usize) -> U256 {
    if bits == 256 {
        return value;
    }
    let low = value & (U256::MAX >> (256 - bits));
    if low.bit(bits - 1) {
        low | (U256::MAX << bits)
    } else {
        low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_types_are_read_from_the_signature() {
        assert_eq!(parameter_types("testUnit()"), Ok(Vec::new()));
        assert_eq!(
            parameter_types("testMany(uint8,int256,address,bool,bytes4)"),
            Ok(vec![
                ParameterType::Uint(8),
                ParameterType::Int(256),
                ParameterType::Address,
                ParameterType::Bool,
                ParameterType::FixedBytes(4),
            ])
        );
        assert_eq!(
            parameter_types("testText(string)"),
            Err("fuzzing does not support parameters of type `string`".to_owned())
        );
        assert_eq!(
            parameter_types("testOdd(uint7)"),
            Err("fuzzing does not support parameters of type `uint7`".to_owned())
        );
        assert_eq!(
            parameter_types("testPair((uint256,bool))"),
            Err("fuzzing does not support tuple, array or enum parameters".to_owned())
        );
        assert_eq!(
            parameter_types("testChange(sum(uint256,sum(uint256,)))"),
            Err("fuzzing does not support tuple, array or enum parameters".to_owned())
        );
    }

    #[test]
    fn generated_values_fit_their_type() {
        let mut rng = Rng::with_seed(1);
        for _ in 0..1000 {
            assert!(ParameterType::Uint(8).generate(&mut rng) <= U256::from(255));
            assert!(ParameterType::Address.generate(&mut rng) >> 160 == U256::ZERO);
            assert!(ParameterType::Bool.generate(&mut rng) <= U256::ONE);
            let bytes4 = ParameterType::FixedBytes(4).generate(&mut rng);
            assert_eq!(bytes4 << 32, U256::ZERO);
            let int8 = I256::from_raw(ParameterType::Int(8).generate(&mut rng));
            assert!(I256::try_from(-128).unwrap() <= int8 && int8 <= I256::try_from(127).unwrap());
        }
    }

    #[test]
    fn values_are_formatted_as_in_solidity() {
        assert_eq!(ParameterType::Int(8).format(U256::MAX), "-1");
        assert_eq!(ParameterType::Uint(256).format(U256::from(42)), "42");
        assert_eq!(ParameterType::Bool.format(U256::ONE), "true");
        assert_eq!(
            ParameterType::FixedBytes(2).format(U256::MAX << 240),
            "0xffff"
        );
    }
}
