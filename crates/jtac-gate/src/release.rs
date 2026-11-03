//! The release verdict (DESIGN.md section 16.3): every commit in the range covered or
//! declared off by a root whose signer the tag's tree trusts, else refused naming each one.
//! An unknown signer's root is absence here: a release vouches for what a trusted key
//! judged. With `require_ci`, a first-parent commit counts only under the CI gate (section
//! 9.5, two gates), so a tag on a pull request commit that CI never judged is named.

use jtac_core::{GitOid, Sha256Digest, State};

/// One commit of the range as the gatherer established it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseFact {
    /// The commit.
    pub commit: GitOid,
    /// On the first-parent line from the tag back to `from`.
    pub first_parent: bool,
    /// Its state; `None` is absent.
    pub state: Option<State>,
    /// The trust file names the signer of the root that gave the state.
    pub trusted: bool,
    /// That root is CI's (it has `from`), or `ci_waived`.
    pub ci: bool,
    /// The commit's own root carries a CI waiver naming it: declared off under the CI gate
    /// while the local gate covered it (DESIGN.md section 9.5).
    pub ci_waived: bool,
    /// That root's id.
    pub root: Option<Sha256Digest>,
}

/// What the attestation may say, or why there is none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseVerdict {
    /// Nothing below is listed.
    pub pass: bool,
    /// No root at all, in range order.
    pub absent: Vec<GitOid>,
    /// A root whose signer the trust file does not name.
    pub untrusted: Vec<GitOid>,
    /// A first-parent commit held to the CI gate and not under it.
    pub not_ci: Vec<GitOid>,
    /// Declared off, in range order.
    pub waived: Vec<GitOid>,
    /// Every root that gave a state, in range order, each once.
    pub roots: Vec<Sha256Digest>,
}

/// Judge. A commit lands in one list at most: absent before untrusted before not CI.
pub fn release(facts: &[ReleaseFact], require_ci: bool) -> ReleaseVerdict {
    let mut v = ReleaseVerdict::default();
    for f in facts {
        match f.state {
            None => v.absent.push(f.commit),
            Some(_) if !f.trusted => v.untrusted.push(f.commit),
            Some(_) if require_ci && f.first_parent && !f.ci => v.not_ci.push(f.commit),
            Some(state) => {
                if state == State::DeclaredOff || f.ci_waived {
                    v.waived.push(f.commit);
                }
                if let Some(root) = f.root
                    && !v.roots.contains(&root)
                {
                    v.roots.push(root);
                }
            }
        }
    }
    v.pass = v.absent.is_empty() && v.untrusted.is_empty() && v.not_ci.is_empty();
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
        state: Option<State>,
        trusted: bool,
        ci: bool,
    ) -> ReleaseFact {
        ReleaseFact {
            commit: oid(n),
            first_parent,
            state,
            trusted,
            ci,
            ci_waived: false,
            root: state.map(|_| digest(n / 2)),
        }
    }

    #[test]
    fn a_range_of_trusted_roots_passes_and_names_each_root_once() {
        let facts = [
            fact(2, true, Some(State::Covered), true, true),
            fact(3, false, Some(State::DeclaredOff), true, false),
        ];
        let v = release(&facts, false);
        assert!(v.pass, "{v:?}");
        assert_eq!(v.roots, vec![digest(1)]);
        assert_eq!(v.waived, vec![oid(3)]);
    }

    #[test]
    fn an_absent_or_untrusted_commit_is_named_and_refuses_whatever_the_rest_say() {
        let facts = [
            fact(2, true, Some(State::Covered), true, true),
            fact(4, false, None, false, false),
            fact(6, false, Some(State::Covered), false, false),
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
            fact(2, true, Some(State::Covered), true, false),
            fact(4, false, Some(State::Covered), true, false),
            fact(6, true, Some(State::DeclaredOff), true, true),
        ];
        assert!(release(&facts, false).pass);
        let v = release(&facts, true);
        assert!(!v.pass);
        assert_eq!(v.not_ci, vec![oid(2)]);
        assert_eq!(v.waived, vec![oid(6)]);
    }

    #[test]
    fn a_ci_waiver_counts_as_ci_and_lists_the_commit_as_waived() {
        let mut f = fact(2, true, Some(State::Covered), true, true);
        f.ci_waived = true;
        let v = release(&[f], true);
        assert!(v.pass);
        assert_eq!(v.waived, vec![oid(2)]);
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
