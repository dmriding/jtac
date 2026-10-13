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
            other => Err(serde::de::Error::custom(UnsupportedVersion(other))),
        }
    }
}

/// The reader saw a version it has no reader for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub struct UnsupportedVersion(pub u64);

impl fmt::Display for UnsupportedVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "schema version {} is not supported; this build reads version 1",
            self.0
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
}
