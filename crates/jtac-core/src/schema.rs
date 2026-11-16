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

/// A node's `v`: 2 only on a waiver whose commit scope names a gate, 3 only on a check read
/// from CI (DESIGN.md section 9.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeVersion {
    /// The v0.1 shape.
    V1,
    /// Version 1 plus `scope.gate`.
    V2,
    /// A check whose body holds `observed` in place of a run's fields.
    V3,
}

impl Serialize for NodeVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(match self {
            Self::V1 => 1,
            Self::V2 => 2,
            Self::V3 => 3,
        })
    }
}

impl<'de> Deserialize<'de> for NodeVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match u64::deserialize(d)? {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            3 => Ok(Self::V3),
            other => Err(serde::de::Error::custom(UnsupportedVersion {
                got: other,
                reads: "versions 1 to 3",
            })),
        }
    }
}

/// The `"v"` field of a root: 1, 2 with `base` and `from` (DESIGN.md sections 9.4 and 15),
/// or 3 with `observed`, `unread` and the state `failed` (section 15.2). Any other number is
/// refused at the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootVersion {
    /// Names neither `base` nor `from`.
    V1,
    /// Names `base`, and `from` on a merge root.
    V2,
    /// Version 2 under a policy with a `ci` check, or a failure record.
    V3,
}

impl Serialize for RootVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(match self {
            Self::V1 => 1,
            Self::V2 => 2,
            Self::V3 => 3,
        })
    }
}

impl<'de> Deserialize<'de> for RootVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match u64::deserialize(d)? {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            3 => Ok(Self::V3),
            other => Err(serde::de::Error::custom(UnsupportedVersion {
                got: other,
                reads: "root versions 1 to 3",
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
    fn a_node_version_is_one_to_three_and_nothing_else() {
        for (v, text) in [
            (NodeVersion::V1, "1"),
            (NodeVersion::V2, "2"),
            (NodeVersion::V3, "3"),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), text);
            assert_eq!(serde_json::from_str::<NodeVersion>(text).unwrap(), v);
        }
        for bad in ["0", "4"] {
            let err = serde_json::from_str::<NodeVersion>(bad)
                .unwrap_err()
                .to_string();
            assert!(err.contains("versions 1 to 3"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_root_version_is_one_to_three_and_nothing_else() {
        for (v, text) in [
            (RootVersion::V1, "1"),
            (RootVersion::V2, "2"),
            (RootVersion::V3, "3"),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), text);
            assert_eq!(serde_json::from_str::<RootVersion>(text).unwrap(), v);
        }
        for bad in ["0", "4", "\"2\""] {
            let err = serde_json::from_str::<RootVersion>(bad)
                .unwrap_err()
                .to_string();
            assert!(
                bad == "\"2\"" || err.contains("root versions 1 to 3"),
                "{bad}: {err}"
            );
        }
    }
}
