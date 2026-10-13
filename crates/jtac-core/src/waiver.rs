//! Which waivers apply at an instant, and what each one covers (DESIGN.md section 11.6).
//! The verdict and `Root::validate` share this, so a hand-written lever node is judged the
//! same on every machine and no writer is trusted.

use crate::node::{Action, Body, FindingBody, LeverBody, Node, Scope};
use crate::oid::GitOid;
use crate::time::Ts;

/// Longest a waiver may run past its own `ts` (DESIGN.md decision 12).
pub const MAX_WAIVER_SECS: i64 = 30 * 86_400;

/// Why a lever node does not apply to `commit` at `now`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutOfScope {
    /// Not a lever node with action `waive`.
    NotAWaiver,
    /// A waiver with no scope.
    NoScope,
    /// A commit waiver for some other commit.
    OtherCommit,
    /// A rule waiver with no expiry.
    NoExpiry,
    /// Written after `now`.
    Future,
    /// `expires` is before `now`.
    Expired,
    /// `expires` is more than `MAX_WAIVER_SECS` past the waiver's own `ts`.
    TooLong,
    /// The waiver's tree and `commit` use different hash algorithms.
    OidAlgorithm,
}

/// The waiver in `node` when it applies to `commit` at `now`.
pub fn scope_at<'a>(node: &'a Node, commit: &GitOid, now: Ts) -> Result<&'a LeverBody, OutOfScope> {
    let Body::Lever(body) = &node.body else {
        return Err(OutOfScope::NotAWaiver);
    };
    if body.action != Action::Waive {
        return Err(OutOfScope::NotAWaiver);
    }
    if node.ts > now {
        return Err(OutOfScope::Future);
    }
    if matches!(node.tree, GitOid::Sha1(_)) != matches!(commit, GitOid::Sha1(_)) {
        return Err(OutOfScope::OidAlgorithm);
    }
    match &body.scope {
        None => return Err(OutOfScope::NoScope),
        Some(Scope::Commit { commit: waived }) if waived != commit => {
            return Err(OutOfScope::OtherCommit);
        }
        Some(Scope::Rule { .. }) if body.expires.is_none() => return Err(OutOfScope::NoExpiry),
        Some(_) => {}
    }
    if let Some(expires) = body.expires {
        if expires < now {
            return Err(OutOfScope::Expired);
        }
        // The cap runs from when the waiver was written; `now` may come weeks later.
        let written = node.ts.timestamp().as_second();
        if expires.timestamp().as_second() - written > MAX_WAIVER_SECS {
            return Err(OutOfScope::TooLong);
        }
    }
    Ok(body)
}

/// Whether a rule waiver covers `finding`: same rule, path equal to or under the prefix by
/// whole component. A commit waiver covers nothing here; it covers the verdict.
pub fn covers_finding(body: &LeverBody, finding: &FindingBody) -> bool {
    match &body.scope {
        Some(Scope::Rule { rule, path }) => rule == &finding.rule && under(&finding.path, path),
        _ => false,
    }
}

fn under(path: &str, prefix: &str) -> bool {
    let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
    clean(prefix)
        && clean(path)
        && path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

// A `..` component would let a waiver on `src/ffi` cover `src/ffi/../auth`.
fn clean(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && path
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::tests::{COMMIT_ONE, TS, check_node, commit_waiver, lever, rule_waiver};
    use crate::node::{Body, Severity};

    fn at(node: &Node, commit: &str, now: &str) -> Result<(), OutOfScope> {
        scope_at(node, &commit.parse().unwrap(), now.parse().unwrap()).map(|_| ())
    }

    #[test]
    fn scope_at_names_why_a_waiver_is_out() {
        let other = "sha1:0000000000000000000000000000000000000002";
        let rows = vec![
            (
                "commit waiver, no expiry",
                commit_waiver(),
                COMMIT_ONE,
                TS,
                Ok(()),
            ),
            (
                "rule waiver in range",
                rule_waiver(),
                COMMIT_ONE,
                TS,
                Ok(()),
            ),
            (
                "rule waiver on another commit",
                rule_waiver(),
                other,
                TS,
                Ok(()),
            ),
            (
                "check node",
                check_node(),
                COMMIT_ONE,
                TS,
                Err(OutOfScope::NotAWaiver),
            ),
            (
                "init lever",
                lever(Action::Init),
                COMMIT_ONE,
                TS,
                Err(OutOfScope::NotAWaiver),
            ),
            (
                "waive with no scope",
                lever(Action::Waive),
                COMMIT_ONE,
                TS,
                Err(OutOfScope::NoScope),
            ),
            (
                "other commit",
                commit_waiver(),
                other,
                TS,
                Err(OutOfScope::OtherCommit),
            ),
            (
                "written one second after now",
                commit_waiver(),
                COMMIT_ONE,
                "2026-10-10T18:29:59Z",
                Err(OutOfScope::Future),
            ),
            (
                "expired one second before now",
                rule_waiver(),
                COMMIT_ONE,
                "2026-10-24T18:30:01Z",
                Err(OutOfScope::Expired),
            ),
            (
                "expiring at now",
                rule_waiver(),
                COMMIT_ONE,
                "2026-10-24T18:30:00Z",
                Ok(()),
            ),
            (
                "sha1 waiver on a sha256 commit",
                commit_waiver(),
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
                TS,
                Err(OutOfScope::OidAlgorithm),
            ),
        ];
        for (name, node, commit, now, want) in rows {
            assert_eq!(at(&node, commit, now), want, "{name}");
        }
        let mut no_expiry = rule_waiver();
        let mut too_long = rule_waiver();
        for (node, expires) in [
            (&mut no_expiry, None),
            (&mut too_long, Some("2026-11-09T18:30:01Z")),
        ] {
            if let Body::Lever(b) = &mut node.body {
                b.expires = expires.map(|e| e.parse().unwrap());
            }
        }
        assert_eq!(at(&no_expiry, COMMIT_ONE, TS), Err(OutOfScope::NoExpiry));
        assert_eq!(at(&too_long, COMMIT_ONE, TS), Err(OutOfScope::TooLong));
    }

    fn finding(rule: &str, path: &str) -> FindingBody {
        FindingBody {
            check: crate::node::tests::EMPTY.parse().unwrap(),
            rule: rule.into(),
            severity: Severity::Major,
            path: path.into(),
            line: 1,
            message: "example".into(),
        }
    }

    fn rule_body(path: &str) -> LeverBody {
        let Body::Lever(mut body) = rule_waiver().body else {
            unreachable!()
        };
        body.scope = Some(Scope::Rule {
            rule: "JT-SUP-001".into(),
            path: path.into(),
        });
        body
    }

    #[test]
    fn covers_finding_matches_rule_and_path_by_component() {
        let rows = [
            ("src/ffi/", "JT-SUP-001", "src/ffi/x.rs", true),
            ("src/ffi", "JT-SUP-001", "src/ffi/x.rs", true),
            ("src/ffi", "JT-SUP-001", "src/ffi", true),
            ("src/ffi", "JT-SUP-001", "src/ffi/a/b.rs", true),
            ("src/ffi", "JT-SUP-001", "src/ffix.rs", false),
            ("src/ffi", "JT-SUP-001", "src/ff", false),
            ("src/ffi", "JT-SUP-001", "lib/src/ffi/x.rs", false),
            ("src/ffi", "JT-TEST-001", "src/ffi/x.rs", false),
            ("", "JT-SUP-001", "src/ffi/x.rs", false),
            ("/", "JT-SUP-001", "src/ffi/x.rs", false),
            ("src//", "JT-SUP-001", "src/x.rs", false),
            ("src/ffi", "JT-SUP-001", "src/ffi/../../Cargo.toml", false),
            ("src/ffi", "JT-SUP-001", "src/ffi/./x.rs", false),
            ("src/ffi", "JT-SUP-001", "src/ffi//x.rs", false),
            ("src/ffi", "JT-SUP-001", "src/ffi/..", false),
            ("src/ffi", "JT-SUP-001", "src/ffi\\x.rs", false),
            ("src/ffi", "JT-SUP-001", "src/ffi/..\\..\\auth.rs", false),
            ("src/../src/ffi", "JT-SUP-001", "src/ffi/x.rs", false),
            ("./src/ffi", "JT-SUP-001", "src/ffi/x.rs", false),
            ("src\\ffi", "JT-SUP-001", "src/ffi/x.rs", false),
        ];
        for (prefix, rule, path, want) in rows {
            assert_eq!(
                covers_finding(&rule_body(prefix), &finding(rule, path)),
                want,
                "{prefix} {rule} {path}"
            );
        }
        let Body::Lever(commit) = commit_waiver().body else {
            unreachable!()
        };
        assert!(!covers_finding(
            &commit,
            &finding("JT-SUP-001", "src/ffi/x.rs")
        ));
    }
}
