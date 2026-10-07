//! Lowercase hex encoding and decoding, shared by the MCP list cursor, the
//! plugin script digest and the managed-store module hash.

use std::fmt::Write;

/// Lowercase hex of `bytes`, two digits per byte.
pub fn encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
            // Writing to a String cannot fail.
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

/// The bytes `s` spells, or `None` if it is empty, odd-length or not hex.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_byte() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)), Some(all));
    }

    #[test]
    fn encodes_lowercase_two_digits() {
        assert_eq!(encode(&[0, 15, 255]), "000fff");
    }

    #[test]
    fn rejects_empty_odd_and_non_hex() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("abc"), None);
        assert_eq!(decode("zz"), None);
        assert_eq!(decode("é"), None);
    }
}
