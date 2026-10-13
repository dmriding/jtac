//! The policy hash and the TOML `init` writes (DESIGN.md section 10.1, decision 25).

use jtac_core::{CanonicalError, Sha256Digest, canonical_digest};

use crate::schema::Policy;

/// SHA-256 of the canonical JSON of the resolved policy. A changed default moves it.
pub fn policy_hash(p: &Policy) -> Result<Sha256Digest, CanonicalError> {
    canonical_digest(p)
}

/// The policy as TOML, every default written out. `parse` reads it back to the same hash.
///
/// # Panics
///
/// On an integer past `i64::MAX`, which TOML cannot hold. `validate` refuses those first.
pub fn to_toml(p: &Policy) -> String {
    toml::to_string_pretty(p).expect("a validated policy has no value TOML cannot write")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::parse::parse;

    /// The first TOML block of DESIGN.md section 10.2, so the design and the code cannot drift.
    pub(crate) fn design_example() -> &'static str {
        let design = include_str!("../../../DESIGN.md");
        let section = &design[design.find("### 10.2 Shape").expect("10.2 exists")..];
        let fence = section.find("```toml").expect("10.2 has a toml block");
        let body = &section[fence..][section[fence..].find('\n').expect("fence line") + 1..];
        &body[..body.find("```").expect("toml block closes")]
    }

    fn hash(text: &str) -> String {
        policy_hash(&parse(text).unwrap()).unwrap().to_string()
    }

    #[test]
    fn design_example_parses_validates_and_hashes() {
        assert_eq!(
            hash(design_example()),
            "sha256:ff343c242c646d2850b9ef075340d988655a2038afb4b51af63746cb896178f3"
        );
    }

    #[test]
    fn required_default_and_explicit_true_hash_the_same() {
        let run = "[checks.fmt]\nrun = [\"cargo\", \"fmt\"]\n";
        assert_eq!(
            hash(&format!("version = 1\n{run}")),
            hash(&format!(
                "version = 1\n{run}required = true\ntimeout_secs = 600\n"
            ))
        );
    }

    #[test]
    fn omitted_and_spelled_out_defaults_hash_the_same() {
        let spelled = "version = 1\n[integrity]\nblock_at = \"major\"\n\
                       max_new_suppressions = 0\nmax_file_lines = 800\n\
                       [fences]\nno_verify = \"deny\"\n[scope]\nbranches = [\"*\"]\n";
        assert_eq!(hash("version = 1"), hash(spelled));
    }

    #[test]
    fn comments_and_key_order_do_not_move_the_hash() {
        let a =
            "version = 1\n[checks.b]\nrun = [\"b\"]\ntimeout_secs = 5\n[checks.a]\nrun = [\"a\"]\n";
        let b = "# policy\nversion = 1 # trailing\n[checks.a] # first\nrun = [ \"a\" ]\n\n\
                 [checks.b]\ntimeout_secs = 5\nrun = [\"b\"]\n";
        assert_eq!(hash(a), hash(b));
    }

    #[test]
    fn a_changed_value_moves_the_hash() {
        let base = hash("version = 1");
        for text in [
            "version = 1\n[integrity]\nmax_file_lines = 801",
            "version = 1\n[integrity]\nmax_added_lines = 800",
            "version = 1\n[integrity]\nblock_at = \"blocker\"",
            "version = 1\n[fences]\nno_verify = \"allow\"",
            "version = 1\n[protected]\npaths = [\".jtac/**\"]",
        ] {
            assert_ne!(hash(text), base, "{text}");
        }
    }

    #[test]
    fn absent_max_added_lines_is_omitted_not_null() {
        let bytes = jtac_core::canonical_bytes(&parse("version = 1").unwrap()).unwrap();
        assert!(
            !String::from_utf8(bytes)
                .unwrap()
                .contains("max_added_lines")
        );
    }

    #[test]
    fn to_toml_round_trips_to_the_same_hash() {
        let texts = [
            design_example(),
            "version = 1",
            "version = 1\n[checks.\"x\"]\nrun = [\"a b\", \"\\\"q\\\"\"]\nrequired = false\n\
             [integrity]\nmax_added_lines = 9007199254740991\nblock_at = \"minor\"\n\
             [fences]\nforce_push_protected = \"allow\"\n[scope]\npaths = []\n",
        ];
        for text in texts {
            let policy = parse(text).unwrap();
            let again = parse(&to_toml(&policy)).unwrap();
            assert_eq!(again, policy, "{text}");
            assert_eq!(policy_hash(&again).unwrap(), policy_hash(&policy).unwrap());
        }
    }
}
