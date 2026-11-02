//! Schema version markers. A shipped format is never edited; a change is a new version
//! with a reader for the old one (AGENTS.md, architecture rules).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The `"v": 1` field. Any other number is refused at the reader, so an unknown future
/// format can never be half-read as this one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V1;

impl Serialize for V1 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(1)
    }
}

impl<'de> Deserialize<'de> for V1 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match u64::deserialize(d)? {
            1 => Ok(V1),
            other => Err(serde::de::Error::custom(UnsupportedVersion {
                got: other,
                reads: "version 1",
            })),
        }
    }
}

/// A node's `v`: 2 only on a waiver whose commit scope names a gate (DESIGN.md section 9.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeVersion {
    /// The v0.1 shape.
    V1,
    /// Version 1 plus `scope.gate`.
    V2,
}

impl Serialize for NodeVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(match self {
            Self::V1 => 1,
            Self::V2 => 2,
        })
    }
}

impl<'de> Deserialize<'de> for NodeVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match u64::deserialize(d)? {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            other => Err(serde::de::Error::custom(UnsupportedVersion {
                got: other,
                reads: "versions 1 and 2",
            })),
        }
    }
}

/// The `"v"` field of a root: 1, or 2 with `base` and `from` (DESIGN.md sections 9.4 and
/// 15). Any other number is refused at the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootVersion {
    /// Names neither `base` nor `from`.
    V1,
    /// Names `base`, and `from` on a merge root.
    V2,
}

impl Serialize for RootVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(match self {
            Self::V1 => 1,
            Self::V2 => 2,
        })
    }
}

impl<'de> Deserialize<'de> for RootVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match u64::deserialize(d)? {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            other => Err(serde::de::Error::custom(UnsupportedVersion {
                got: other,
                reads: "versions 1 and 2",
            })),
        }
    }
}

/// The reader saw a version it has no reader for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub struct UnsupportedVersion {
    /// The version in the document.
    pub got: u64,
    /// What this build reads, for the message.
    pub reads: &'static str,
}

impl fmt::Display for UnsupportedVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "schema version {} is not supported; this build reads {}",
            self.got, self.reads
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_is_the_integer_one() {
        assert_eq!(serde_json::to_string(&V1).unwrap(), "1");
        assert_eq!(serde_json::from_str::<V1>("1").unwrap(), V1);
    }

    #[test]
    fn any_other_version_is_refused() {
        for bad in ["0", "2", "\"1\"", "1.0", "-1"] {
            let err = serde_json::from_str::<V1>(bad).unwrap_err().to_string();
            assert!(
                bad == "\"1\"" || bad == "1.0" || bad == "-1" || err.contains("not supported"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn a_node_version_is_one_or_two_and_nothing_else() {
        assert_eq!(serde_json::to_string(&NodeVersion::V1).unwrap(), "1");
        assert_eq!(serde_json::to_string(&NodeVersion::V2).unwrap(), "2");
        assert_eq!(
            serde_json::from_str::<NodeVersion>("2").unwrap(),
            NodeVersion::V2
        );
        let err = serde_json::from_str::<NodeVersion>("3")
            .unwrap_err()
            .to_string();
        assert!(err.contains("versions 1 and 2"), "{err}");
    }

    #[test]
    fn a_root_version_is_one_or_two_and_nothing_else() {
        assert_eq!(serde_json::to_string(&RootVersion::V2).unwrap(), "2");
        assert_eq!(
            serde_json::from_str::<RootVersion>("1").unwrap(),
            RootVersion::V1
        );
        assert_eq!(
            serde_json::from_str::<RootVersion>("2").unwrap(),
            RootVersion::V2
        );
        for bad in ["0", "3", "\"2\""] {
            let err = serde_json::from_str::<RootVersion>(bad)
                .unwrap_err()
                .to_string();
            assert!(
                bad == "\"2\"" || err.contains("versions 1 and 2"),
                "{bad}: {err}"
            );
        }
    }
}
