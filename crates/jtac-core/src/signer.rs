//! Who signed: the principal the allowed-signers file keys on, plus the key fingerprint
//! for people and audit rows (DESIGN.md sections 9.4 and 11.8).

use serde::{Deserialize, Serialize};

/// A signing identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signer {
    /// The identity looked up in `.jtac/signers`, usually an email address.
    pub principal: String,
    /// The SSH key fingerprint, `SHA256:` followed by base64.
    pub fingerprint: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signer_round_trips_and_refuses_extra_fields() {
        let s = Signer {
            principal: "dev@example.com".into(),
            fingerprint: "SHA256:AAAA".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Signer>(&json).unwrap(), s);
        assert!(
            serde_json::from_str::<Signer>(r#"{"principal":"a","fingerprint":"b","key":"c"}"#)
                .is_err()
        );
    }
}
