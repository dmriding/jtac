//! `JT-PROT-001`: a change to a protected path, and the tiers a change lands in.

use std::collections::BTreeSet;

use jtac_core::Severity;
use jtac_policy::Matchers;

use super::report::Finding;
use super::rule::Rule;
use super::view::View;

/// Whether anything was in force to protect (decisions 38 and 45): the gatherer's fact, read
/// from history or from the base branch, never from the diff, whose base the pusher picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adoption {
    /// An ancestor held the policy, or the base branch names signers.
    None,
    /// No parent's history held a policy (decision 38): `.jtac/**` is minor when the change
    /// adds the policy file.
    Policy,
    /// No `.jtac/signers` was in force (decision 45): the base branch holds none in CI, or
    /// locally the commit brings the first policy and the first signers file together; every
    /// protected path is minor, since with no trust no waiver could have cleared anything.
    Trust,
}

/// A finding per protected path touched, by either name of a rename
/// (`adopting_the_policy_makes_its_own_directory_minor_and_nothing_else`).
pub(crate) fn check(views: &[View], matchers: &Matchers, adoption: Adoption) -> Vec<Finding> {
    let adds_policy = views.iter().any(|v| {
        v.path() == jtac_policy::POLICY_PATH && v.change.old.is_none() && v.change.from.is_none()
    });
    let minor = match adoption {
        Adoption::Trust => Some((
            "",
            "; the base holds no signers file, so nothing was in force to protect",
        )),
        Adoption::Policy if adds_policy => Some((
            ".jtac/",
            "; the base holds no policy, so this change brings it into force",
        )),
        _ => None,
    };
    views
        .iter()
        .flat_map(|v| {
            [Some(v.path()), v.change.from.as_deref()]
                .into_iter()
                .flatten()
                .filter(|p| matchers.protected.is_match(p))
                .map(move |p| {
                    let (severity, tail) = match minor {
                        Some((under, tail)) if p.starts_with(under) => (Severity::Minor, tail),
                        _ => (Severity::Major, ""),
                    };
                    Finding::new(
                        Rule::Prot001,
                        severity,
                        v.path(),
                        0,
                        format!("protected path `{p}` changed{tail}"),
                    )
                })
        })
        .collect()
}

/// Every tier a changed path matches.
pub(crate) fn tiers(views: &[View], matchers: &Matchers) -> BTreeSet<String> {
    matchers
        .tiers
        .iter()
        .filter(|(_, m)| {
            views.iter().any(|v| {
                m.is_match(v.path()) || v.change.from.as_deref().is_some_and(|f| m.is_match(f))
            })
        })
        .map(|(name, _)| name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::change::Change;

    fn change(path: &str, from: Option<&str>) -> Change {
        Change {
            path: path.into(),
            from: from.map(str::to_owned),
            old: None,
            new: None,
        }
    }

    #[test]
    fn protected_paths_and_tiers_match_either_side_of_a_rename() {
        let policy = jtac_policy::parse(
            "version = 1\n[tiers.high]\npaths = [\"**/auth/**\"]\n[tiers.low]\npaths = [\"docs/**\"]\n",
        )
        .unwrap();
        let matchers = policy.matchers().unwrap();
        let changes = [
            change(".jtac/policy.toml", None),
            change("src/lib.rs", None),
            change("src/auth/mod.rs", Some(".github/workflows/ci.yml")),
        ];
        let views: Vec<View> = changes.iter().map(View::of).collect();
        let got: Vec<(String, String)> = check(&views, &matchers, Adoption::Policy)
            .into_iter()
            .map(|f| (f.path, f.message))
            .collect();
        assert_eq!(
            got,
            [
                (
                    ".jtac/policy.toml".to_owned(),
                    "protected path `.jtac/policy.toml` changed; the base holds no policy, so \
                     this change brings it into force"
                        .to_owned()
                ),
                (
                    "src/auth/mod.rs".to_owned(),
                    "protected path `.github/workflows/ci.yml` changed".to_owned()
                ),
            ]
        );
        assert_eq!(
            tiers(&views, &matchers),
            BTreeSet::from(["high".to_owned()])
        );
    }

    #[test]
    fn adopting_the_policy_makes_its_own_directory_minor_and_nothing_else() {
        use crate::diff::change::Blob;
        let matchers = jtac_policy::parse("version = 1\n")
            .unwrap()
            .matchers()
            .unwrap();
        let edited = Change {
            path: ".jtac/policy.toml".into(),
            from: None,
            old: Some(Blob::Text("version = 1\n".into())),
            new: Some(Blob::Text("version = 1\n# edited\n".into())),
        };
        type Row = (&'static str, Adoption, Vec<Change>, Vec<(Severity, bool)>);
        let rows: [Row; 5] = [
            (
                "an ancestor held the policy: a base the pusher chose does not adopt",
                Adoption::None,
                vec![change(".jtac/policy.toml", None)],
                vec![(Severity::Major, false)],
            ),
            (
                "the adoption commit: .jtac/ minor, CI major",
                Adoption::Policy,
                vec![
                    change(".jtac/policy.toml", None),
                    change(".jtac/signers", None),
                    change(".github/workflows/ci.yml", None),
                ],
                vec![
                    (Severity::Minor, true),
                    (Severity::Minor, true),
                    (Severity::Major, false),
                ],
            ),
            (
                "an edit to the policy is not adoption",
                Adoption::Policy,
                vec![edited.clone(), change(".jtac/signers", None)],
                vec![(Severity::Major, false), (Severity::Major, false)],
            ),
            (
                "a policy renamed into place is not adoption",
                Adoption::Policy,
                vec![change(".jtac/policy.toml", Some("policy.toml"))],
                vec![(Severity::Major, false)],
            ),
            (
                "the base holds no signers file: every protected path minor, an edit too",
                Adoption::Trust,
                vec![edited, change(".github/workflows/ci.yml", None)],
                vec![(Severity::Minor, true), (Severity::Minor, true)],
            ),
        ];
        for (claim, fact, changes, want) in rows {
            let views: Vec<View> = changes.iter().map(View::of).collect();
            let got: Vec<(Severity, bool)> = check(&views, &matchers, fact)
                .into_iter()
                .map(|f| (f.severity, f.message.contains("; the base holds no ")))
                .collect();
            assert_eq!(got, want, "{claim}");
        }
    }
}
