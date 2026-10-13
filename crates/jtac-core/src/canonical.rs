//! RFC 8785 bytes of a value (DESIGN.md section 9.1). Floats and integers past 2^53 are
//! refused: the canonicalizer formats every number as an IEEE double, so anything else
//! would hash one way here and another way in a different implementation.

use serde::Serialize;
use serde_json::Value;

use crate::digest::Sha256Digest;

/// Largest integer every RFC 8785 implementation formats exactly.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Canonical bytes of `value`, the input to every id and signature.
pub fn canonical_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    let value = serde_json::to_value(value)?;
    check_numbers(&value, "")?;
    Ok(serde_json_canonicalizer::to_vec(&value)?)
}

/// `sha256:` of the canonical bytes.
pub fn canonical_digest<T: Serialize + ?Sized>(value: &T) -> Result<Sha256Digest, CanonicalError> {
    canonical_bytes(value).map(Sha256Digest::of)
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

/// Why a value has no canonical form.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    /// A float at `path`.
    #[error("{path}: floats are not allowed in evidence or policy")]
    Float {
        /// Dotted path from the document root, array indexes in brackets.
        path: String,
    },
    /// An integer at `path` beyond what a double represents exactly.
    #[error("{path}: integers beyond 2^53-1 are not allowed in evidence or policy")]
    UnsafeInteger {
        /// Dotted path from the document root, array indexes in brackets.
        path: String,
    },
    /// The value could not be serialized at all.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
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
    fn strings_keep_unicode_and_escape_only_what_json_requires() {
        let v = json!({"s": "é\n\"/\u{1}"});
        assert_eq!(canon(&v), "{\"s\":\"é\\n\\\"/\\u0001\"}");
    }

    #[test]
    fn safe_integers_pass_and_unsafe_ones_name_their_path() {
        assert_eq!(
            canon(&json!([MAX_SAFE_INTEGER, -(MAX_SAFE_INTEGER as i64)])),
            "[9007199254740991,-9007199254740991]"
        );
        let err = canonical_bytes(&json!({"a": [0, {"n": MAX_SAFE_INTEGER + 1}]})).unwrap_err();
        assert!(
            matches!(err, CanonicalError::UnsafeInteger { ref path } if path == ".a[1].n"),
            "{err}"
        );
        let err = canonical_bytes(&json!({"n": i64::MIN})).unwrap_err();
        assert!(matches!(err, CanonicalError::UnsafeInteger { .. }), "{err}");
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
        let b = Sha256Digest::of(br#"{"a":2,"b":1}"#);
        assert_eq!(a, b);
    }
}
