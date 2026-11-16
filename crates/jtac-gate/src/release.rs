//! The release verdict (DESIGN.md section 16.3): every commit in the range covered or
//! declared off by a root whose signer the tag's tree trusts, else refused naming each one.
//! An unknown signer's root is absence here: a release vouches for what a trusted key
//! judged. With `require_ci`, a first-parent commit counts only under the CI gate (section
//! 9.5, two gates), so a tag on a pull request commit that CI never judged is named.

use jtac_core::{GitOid, Sha256Digest};

use crate::audit::Reading;

/// One commit of the range as the gatherer established it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseFact {
    /// The commit.
    pub commit: GitOid,
    /// On the first-parent line from the tag back to `from`.
    pub first_parent: bool,
    /// Its reading; `None` is absent. The gatherer refuses a `NotHeld` before it asks.
    pub state: Option<Reading>,
    /// The trust file names the signer of the root that gave the state, and of every
    /// waiver it carries.
    pub trusted: bool,
    /// The CI gate reads the commit covered or declared off (DESIGN.md section 9.5).
    pub ci: bool,
    /// The CI gate reads it declared off, by any document.
    pub ci_off: bool,
    /// By a CI waiver in the commit's own root: declared off under the CI gate while the
    /// local gate covered it.
    pub ci_waived: bool,
    /// The CI gate reads it `failed`, which a refusal names in place of absence.
    pub failed: bool,
    /// The root that gave the state.
    pub root: Option<Sha256Digest>,
    /// The root the CI gate read it through, when it read one.
    pub ci_root: Option<Sha256Digest>,
}

/// What the attestation may say, or why there is none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseVerdict {
    /// Nothing refused is listed.
    pub pass: bool,
    /// No root at all, in range order.
    pub absent: Vec<GitOid>,
    /// A root whose signer the trust file does not name.
    pub untrusted: Vec<GitOid>,
    /// The local gate's root alone, a `ci` check left unread (section 15.2).
    pub partial: Vec<GitOid>,
    /// A first-parent commit held to the CI gate and not under it.
    pub not_ci: Vec<GitOid>,
    /// Refused as absent, partial or not CI's, where the CI gate's verdict failed.
    pub failed: Vec<GitOid>,
    /// Declared off, in range order.
    pub waived: Vec<GitOid>,
    /// Every root that gave a state, in range order, each once; with `require_ci`, the
    /// root the CI gate read a first-parent commit through as well.
    pub roots: Vec<Sha256Digest>,
}

/// Judge. A commit lands in one list at most: absent before untrusted before partial
/// before not CI, and `failed` in place of any of those but untrusted. Declared off by
/// either gate counts (section 16.3), so a partial commit the CI gate reads declared off,
/// a bypass merge under a `ci` check, passes and is listed; one it reads covered does not,
/// since no CI root covers a commit and leaves its row partial.
pub fn release(facts: &[ReleaseFact], require_ci: bool) -> ReleaseVerdict {
    let mut v = ReleaseVerdict::default();
    for f in facts {
        let held = require_ci && f.first_parent;
        let refused = match f.state {
            None | Some(Reading::NotHeld) => Some(&mut v.absent),
            Some(_) if !f.trusted => {
                v.untrusted.push(f.commit);
                continue;
            }
            Some(Reading::Partial) if !f.ci_off => Some(&mut v.partial),
            Some(_) if held && !f.ci => Some(&mut v.not_ci),
            Some(_) => None,
        };
        if let Some(list) = refused {
            match f.failed {
                true => v.failed.push(f.commit),
                false => list.push(f.commit),
            }
            continue;
        }
        let partial = f.state == Some(Reading::Partial);
        let off = f.state == Some(Reading::DeclaredOff) || f.ci_waived;
        if off || (f.ci_off && (held || partial)) {
            v.waived.push(f.commit);
        }
        let ci_root = f.ci_root.filter(|_| held);
        for root in f.root.into_iter().chain(ci_root) {
            if !v.roots.contains(&root) {
                v.roots.push(root);
            }
        }
    }
    v.pass = [&v.absent, &v.untrusted, &v.partial, &v.not_ci, &v.failed]
        .iter()
        .all(|l| l.is_empty());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u8) -> GitOid {
        format!("sha1:{n:040x}").parse().unwrap()
    }

    fn digest(n: u8) -> Sha256Digest {
        format!("sha256:{n:064x}").parse().unwrap()
    }

    fn fact(
        n: u8,
        first_parent: bool,
        state: Option<Reading>,
        trusted: bool,
        ci: bool,
    ) -> ReleaseFact {
        ReleaseFact {
            commit: oid(n),
            first_parent,
            state,
            trusted,
            ci,
            ci_off: false,
            ci_waived: false,
            failed: false,
            root: state.map(|_| digest(n / 2)),
            ci_root: None,
        }
    }

    #[test]
    fn a_range_of_trusted_roots_passes_and_names_each_root_once() {
        let facts = [
            fact(2, true, Some(Reading::Covered), true, true),
            fact(3, false, Some(Reading::DeclaredOff), true, false),
        ];
        let v = release(&facts, false);
        assert!(v.pass, "{v:?}");
        assert_eq!(v.roots, vec![digest(1)]);
        assert_eq!(v.waived, vec![oid(3)]);
    }

    #[test]
    fn an_absent_or_untrusted_commit_is_named_and_refuses_whatever_the_rest_say() {
        let facts = [
            fact(2, true, Some(Reading::Covered), true, true),
            fact(4, false, None, false, false),
            fact(6, false, Some(Reading::Covered), false, false),
        ];
        let v = release(&facts, false);
        assert!(!v.pass);
        assert_eq!(v.absent, vec![oid(4)]);
        assert_eq!(v.untrusted, vec![oid(6)]);
        assert_eq!(v.roots, vec![digest(1)]);
    }

    /// The v0.2.7 case: the tag on the pull request commit, whose only root is the
    /// developer's; CI's root sits on the merge commit outside the range.
    #[test]
    fn require_ci_names_a_first_parent_commit_the_ci_gate_never_judged() {
        let facts = [
            fact(2, true, Some(Reading::Covered), true, false),
            fact(4, false, Some(Reading::Covered), true, false),
            fact(6, true, Some(Reading::DeclaredOff), true, true),
        ];
        assert!(release(&facts, false).pass);
        let v = release(&facts, true);
        assert!(!v.pass);
        assert_eq!(v.not_ci, vec![oid(2)]);
        assert_eq!(v.waived, vec![oid(6)]);
    }

    #[test]
    fn a_ci_waiver_counts_as_ci_and_lists_the_commit_as_waived() {
        let mut f = fact(2, true, Some(Reading::Covered), true, true);
        f.ci_waived = true;
        let v = release(&[f], true);
        assert!(v.pass);
        assert_eq!(v.waived, vec![oid(2)]);
    }

    /// Section 15.2: a partial reading is not covered; a failed verdict is named as one
    /// with and without `--require-ci`, and an unknown key's root stays an unknown key's.
    #[test]
    fn a_partial_commit_is_refused_and_a_failed_one_is_named_failed_not_absent() {
        let failed = |mut f: ReleaseFact| {
            f.failed = true;
            f
        };
        let facts = [
            fact(2, true, Some(Reading::Partial), true, false),
            failed(fact(4, true, Some(Reading::Partial), true, false)),
            failed(fact(6, true, None, false, false)),
            failed(fact(8, true, Some(Reading::Covered), true, false)),
            failed(fact(10, true, Some(Reading::Covered), false, false)),
            fact(12, true, Some(Reading::Partial), false, false),
        ];
        for require_ci in [false, true] {
            let v = release(&facts, require_ci);
            assert!(!v.pass);
            assert_eq!(v.partial, vec![oid(2)]);
            assert_eq!(v.untrusted, vec![oid(10), oid(12)]);
            let also = if require_ci { vec![oid(8)] } else { vec![] };
            assert_eq!(v.failed, [vec![oid(4), oid(6)], also].concat());
            assert!(v.absent.is_empty() && v.not_ci.is_empty(), "{v:?}");
        }
    }

    #[test]
    fn a_partial_root_that_carries_a_ci_waiver_passes_and_is_listed_as_waived() {
        let mut f = fact(2, true, Some(Reading::Partial), true, true);
        (f.ci_off, f.ci_waived) = (true, true);
        let mut reached = fact(4, false, Some(Reading::Partial), true, true);
        reached.ci_off = true;
        let covered = fact(6, false, Some(Reading::Partial), true, true);
        for require_ci in [false, true] {
            let v = release(&[f.clone(), reached.clone(), covered.clone()], require_ci);
            assert_eq!(
                v.waived,
                vec![oid(2), oid(4)],
                "declared off by either document"
            );
            assert_eq!(v.partial, vec![oid(6)], "CI covered is not declared off");
            assert!(!v.pass, "{v:?}");
        }
    }

    /// Held to the CI gate, a commit is listed by what that gate read and through which
    /// root: declared off by a root that reaches it, beside its own covered root.
    #[test]
    fn a_first_parent_commit_held_to_ci_lists_the_ci_gates_root_and_its_declared_off() {
        let mut f = fact(2, true, Some(Reading::Covered), true, true);
        f.ci_off = true;
        f.ci_root = Some(digest(9));
        let plain = release(std::slice::from_ref(&f), false);
        assert_eq!((plain.waived, plain.roots), (vec![], vec![digest(1)]));
        let held = release(std::slice::from_ref(&f), true);
        assert!(held.pass);
        assert_eq!(held.waived, vec![oid(2)]);
        assert_eq!(held.roots, vec![digest(1), digest(9)]);
        f.first_parent = false;
        assert_eq!(release(&[f], true).roots, vec![digest(1)]);
    }

    #[test]
    fn an_empty_range_passes_with_nothing_to_name() {
        let v = release(&[], true);
        assert_eq!(
            v,
            ReleaseVerdict {
                pass: true,
                ..ReleaseVerdict::default()
            }
        );
    }
}
