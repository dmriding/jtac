//! Git object ids as git reports them, algorithm named (DESIGN.md section 9.1).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::digest::{ParseError, lowercase_hex};

/// A git object id: `sha1:<40 hex>` or `sha256:<64 hex>`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitOid {
    /// SHA-1 repositories, the default in git today.
    Sha1([u8; 20]),
    /// SHA-256 repositories.
    Sha256([u8; 32]),
}

impl GitOid {
    /// The git empty tree, same bytes in every SHA-1 repository.
    pub const EMPTY_TREE_SHA1: &'static str = "sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    /// The raw hash bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Sha1(b) => b,
            Self::Sha256(b) => b,
        }
    }

    fn prefix(&self) -> &'static str {
        match self {
            Self::Sha1(_) => "sha1:",
            Self::Sha256(_) => "sha256:",
        }
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.prefix(), hex::encode(self.as_bytes()))
    }
}

impl fmt::Debug for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for GitOid {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, ParseError> {
        if let Some(hex) = s.strip_prefix("sha1:") {
            lowercase_hex(hex).map(Self::Sha1)
        } else if let Some(hex) = s.strip_prefix("sha256:") {
            lowercase_hex(hex).map(Self::Sha256)
        } else {
            Err(ParseError::Prefix {
                expected: "sha1: or sha256:",
            })
        }
    }
}

impl Serialize for GitOid {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for GitOid {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA256_TREE: &str =
        "sha256:6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321";

    #[test]
    fn both_algorithms_round_trip() {
        for text in [GitOid::EMPTY_TREE_SHA1, SHA256_TREE] {
            let oid: GitOid = text.parse().unwrap();
            assert_eq!(oid.to_string(), text);
            assert_eq!(format!("{oid:?}"), text);
            let json = serde_json::to_string(&oid).unwrap();
            assert_eq!(json, format!("{text:?}"));
            assert_eq!(serde_json::from_str::<GitOid>(&json).unwrap(), oid);
        }
    }

    #[test]
    fn length_is_checked_per_algorithm() {
        let sha1_hex = &GitOid::EMPTY_TREE_SHA1[5..];
        let sha256_hex = &SHA256_TREE[7..];
        let table = [
            (
                format!("sha1:{sha256_hex}"),
                ParseError::Length {
                    expected: 40,
                    got: 64,
                },
            ),
            (
                format!("sha256:{sha1_hex}"),
                ParseError::Length {
                    expected: 64,
                    got: 40,
                },
            ),
            (
                format!("md5:{sha1_hex}"),
                ParseError::Prefix {
                    expected: "sha1: or sha256:",
                },
            ),
            (
                format!("sha1:{}", sha1_hex.to_uppercase()),
                ParseError::NotLowercaseHex,
            ),
        ];
        for (input, want) in table {
            assert_eq!(input.parse::<GitOid>().unwrap_err(), want, "{input}");
        }
        assert!(serde_json::from_str::<GitOid>("\"sha1:\"").is_err());
    }

    #[test]
    fn as_bytes_has_the_algorithm_length() {
        let a: GitOid = GitOid::EMPTY_TREE_SHA1.parse().unwrap();
        let b: GitOid = SHA256_TREE.parse().unwrap();
        assert_eq!(a.as_bytes().len(), 20);
        assert_eq!(b.as_bytes().len(), 32);
    }
}
