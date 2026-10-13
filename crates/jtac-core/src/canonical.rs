//! RFC 8785 bytes of a value, and the reader that accepts nothing else (DESIGN.md section 9.1).
//! Floats and integers past 2^53 are refused: the canonicalizer formats every number as an
//! IEEE double, so anything else would hash one way here and another way elsewhere.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::digest::Sha256Digest;

/// Largest integer every RFC 8785 implementation formats exactly.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Canonical bytes of `value`, the input to every id and signature.
///
/// Non-finite floats become `null` in `serde_json::to_value` before the number check runs;
/// the typed schemas have no float field, which is the guard.
pub fn canonical_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    let value = serde_json::to_value(value)?;
    check_numbers(&value, "")?;
    Ok(serde_json_canonicalizer::to_vec(&value)?)
}

/// `sha256:` of the canonical bytes.
pub fn canonical_digest<T: Serialize + ?Sized>(value: &T) -> Result<Sha256Digest, CanonicalError> {
    canonical_bytes(value).map(Sha256Digest::of)
}

/// Parse `bytes` and accept the result only if it writes back as exactly `bytes`. Duplicate
/// keys, dropped keys, whitespace, and alternate spellings are all refused here, so a stored
/// document hashes to its id by construction.
pub fn from_canonical<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T, ReadError> {
    let value: T = serde_json::from_slice(bytes)?;
    if canonical_bytes(&value)? != bytes {
        return Err(ReadError::NotCanonical);
    }
    Ok(value)
}

fn check_numbers(value: &Value, path: &str) -> Result<(), CanonicalError> {
    match value {
        Value::Number(n) if n.is_f64() => Err(CanonicalError::Float {
            path: path.to_owned(),
        }),
        Value::Number(n) => {
            let safe = n
                .as_i64()
                .is_some_and(|i| i.unsigned_abs() <= MAX_SAFE_INTEGER)
                || n.as_u64().is_some_and(|u| u <= MAX_SAFE_INTEGER);
            if safe {
                Ok(())
            } else {
                Err(CanonicalError::UnsafeInteger {
                    path: path.to_owned(),
                })
            }
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .try_for_each(|(i, item)| check_numbers(item, &format!("{path}[{i}]"))),
        Value::Object(map) => map
            .iter()
            .try_for_each(|(key, item)| check_numbers(item, &format!("{path}.{key}"))),
        _ => Ok(()),
    }
}

/// Why a value has no canonical form. `path` is dotted from the document root with array
/// indexes in brackets.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    /// A float at `path`.
    #[error("{path}: floats are not allowed in evidence or policy")]
    Float {
        /// Where.
        path: String,
    },
    /// An integer at `path` beyond what a double represents exactly.
    #[error("{path}: integers beyond 2^53-1 are not allowed in evidence or policy")]
    UnsafeInteger {
        /// Where.
        path: String,
    },
    /// The value could not be serialized at all.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Why `from_canonical` refused.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// Not valid JSON, or not this type.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Parsed, but has no canonical form.
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    /// Parsed, but the bytes are not the canonical form of what they parsed to.
    #[error("not canonical: the bytes are not what this version writes for this document")]
    NotCanonical,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canon(v: &Value) -> String {
        String::from_utf8(canonical_bytes(v).unwrap()).unwrap()
    }

    #[test]
    fn keys_are_sorted_and_whitespace_dropped() {
        let v = json!({"b": [1, 2], "a": {"z": null, "y": true}});
        assert_eq!(canon(&v), r#"{"a":{"y":true,"z":null},"b":[1,2]}"#);
    }

    #[test]
    fn keys_sort_by_utf16_code_units_not_bytes() {
        // U+1F600 is a surrogate pair in UTF-16 (0xD83D...), so it sorts before U+E000 and
        // U+FB33 there and after them in UTF-8.
        let v = json!({"\u{FB33}": 1, "\u{1F600}": 2, "\u{E000}": 3, "z": 4});
        assert_eq!(
            canon(&v),
            "{\"z\":4,\"\u{1F600}\":2,\"\u{E000}\":3,\"\u{FB33}\":1}"
        );
    }

    #[test]
    fn strings_keep_unicode_and_escape_only_what_json_requires() {
        let v = json!({"s": "é\n\"/\u{1}"});
        assert_eq!(canon(&v), "{\"s\":\"é\\n\\\"/\\u0001\"}");
    }

    #[test]
    fn integers_pass_up_to_2_pow_53_and_are_refused_past_it_on_both_sides() {
        let max = MAX_SAFE_INTEGER;
        assert_eq!(
            canon(&json!([max, -(max as i64)])),
            "[9007199254740991,-9007199254740991]"
        );
        for (name, doc, path) in [
            ("2^53", json!({"a": [0, {"n": max + 1}]}), ".a[1].n"),
            ("-2^53", json!({"n": -(max as i64) - 1}), ".n"),
            ("u64::MAX", json!({"n": u64::MAX}), ".n"),
            ("i64::MIN", json!({"n": i64::MIN}), ".n"),
        ] {
            let err = canonical_bytes(&doc).unwrap_err();
            assert!(
                matches!(err, CanonicalError::UnsafeInteger { path: ref p } if p == path),
                "{name}: {err}"
            );
        }
    }

    #[test]
    fn floats_are_refused_with_their_path() {
        let err = canonical_bytes(&json!({"duration": 1.5})).unwrap_err();
        assert!(
            matches!(err, CanonicalError::Float { ref path } if path == ".duration"),
            "{err}"
        );
        let err = canonical_bytes(&json!(1.0)).unwrap_err();
        assert!(matches!(err, CanonicalError::Float { ref path } if path.is_empty()));
        assert!(err.to_string().contains("floats are not allowed"));
    }

    #[test]
    fn digest_is_over_the_canonical_bytes() {
        let a = canonical_digest(&json!({"b": 1, "a": 2})).unwrap();
        assert_eq!(a, Sha256Digest::of(br#"{"a":2,"b":1}"#));
    }

    #[test]
    fn from_canonical_accepts_only_the_bytes_it_would_write() {
        let good = br#"{"a":1,"b":[true,null]}"#;
        assert_eq!(
            from_canonical::<Value>(good).unwrap(),
            json!({"a": 1, "b": [true, null]})
        );
        for (name, bytes) in [
            ("whitespace", &br#"{"a":1, "b":[true,null]}"#[..]),
            ("key order", br#"{"b":[true,null],"a":1}"#),
            (
                "duplicate key, last wins",
                br#"{"a":0,"a":1,"b":[true,null]}"#,
            ),
            ("float", br#"{"a":1.0,"b":[true,null]}"#),
            ("integer past 2^53", br#"{"a":9007199254740992}"#),
            ("escaped solidus", br#"{"a":"\/"}"#),
            ("not json", b"{"),
        ] {
            assert!(
                from_canonical::<Value>(bytes).is_err(),
                "{name} was accepted"
            );
        }
    }
}
