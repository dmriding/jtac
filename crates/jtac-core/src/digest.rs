//! `sha256:<hex>` digests: node ids, policy hashes, Merkle roots (DESIGN.md section 9.1).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

/// A SHA-256 digest, written as `sha256:` followed by 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// The algorithm prefix in the text form.
    pub const PREFIX: &'static str = "sha256:";

    /// Digest of `bytes`.
    pub fn of(bytes: impl AsRef<[u8]>) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Wrap raw digest bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw digest bytes. Merkle leaves hash these, never the text form.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", Self::PREFIX, hex::encode(self.0))
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Sha256Digest {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, ParseError> {
        let hex = s.strip_prefix(Self::PREFIX).ok_or(ParseError::Prefix {
            expected: Self::PREFIX,
        })?;
        lowercase_hex(hex).map(Self)
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Why a digest or git object id text form was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The algorithm prefix is missing or not one this type accepts.
    #[error("expected the prefix {expected:?}")]
    Prefix {
        /// The prefix, or list of prefixes, that would have been accepted.
        expected: &'static str,
    },
    /// Wrong number of hex digits for the algorithm.
    #[error("expected {expected} hex digits, got {got}")]
    Length {
        /// Digits the algorithm needs.
        expected: usize,
        /// Digits present.
        got: usize,
    },
    /// A character outside `0-9a-f`. Uppercase is refused so one digest has one spelling.
    #[error("expected lowercase hex digits only")]
    NotLowercaseHex,
}

/// Decode exactly `N` bytes of lowercase hex.
pub(crate) fn lowercase_hex<const N: usize>(hex: &str) -> Result<[u8; N], ParseError> {
    if hex.len() != N * 2 {
        return Err(ParseError::Length {
            expected: N * 2,
            got: hex.len(),
        });
    }
    if !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(ParseError::NotLowercaseHex);
    }
    let mut out = [0u8; N];
    hex::decode_to_slice(hex, &mut out).map_err(|_| ParseError::NotLowercaseHex)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn digest_of_empty_input_is_the_known_sha256() {
        assert_eq!(Sha256Digest::of(b"").to_string(), EMPTY);
    }

    #[test]
    fn text_form_round_trips_through_parse_and_serde() {
        let d: Sha256Digest = EMPTY.parse().unwrap();
        assert_eq!(d, Sha256Digest::of(b""));
        assert_eq!(serde_json::to_string(&d).unwrap(), format!("{EMPTY:?}"));
        let back: Sha256Digest = serde_json::from_str(&format!("{EMPTY:?}")).unwrap();
        assert_eq!(back, d);
        assert_eq!(format!("{d:?}"), EMPTY);
    }

    #[test]
    fn every_misspelling_is_refused() {
        let hex = &EMPTY[7..];
        let table = [
            (
                hex.to_string(),
                ParseError::Prefix {
                    expected: "sha256:",
                },
            ),
            (
                format!("sha1:{hex}"),
                ParseError::Prefix {
                    expected: "sha256:",
                },
            ),
            (
                format!("SHA256:{hex}"),
                ParseError::Prefix {
                    expected: "sha256:",
                },
            ),
            (
                format!("sha256:{}", &hex[1..]),
                ParseError::Length {
                    expected: 64,
                    got: 63,
                },
            ),
            (
                format!("sha256:{hex}0"),
                ParseError::Length {
                    expected: 64,
                    got: 65,
                },
            ),
            (
                format!("sha256:{}", hex.to_uppercase()),
                ParseError::NotLowercaseHex,
            ),
            (
                format!("sha256:{}g", &hex[1..]),
                ParseError::NotLowercaseHex,
            ),
            (
                format!("sha256:{}é", &hex[2..]),
                ParseError::NotLowercaseHex,
            ),
        ];
        for (input, want) in table {
            assert_eq!(input.parse::<Sha256Digest>().unwrap_err(), want, "{input}");
        }
        assert!(serde_json::from_str::<Sha256Digest>("\"sha256:zz\"").is_err());
    }

    #[test]
    fn ordering_is_by_raw_bytes() {
        let lo = Sha256Digest::from_bytes([0; 32]);
        let hi = Sha256Digest::from_bytes([0xff; 32]);
        assert!(lo < hi);
        assert_eq!(*hi.as_bytes(), [0xff; 32]);
    }
}
