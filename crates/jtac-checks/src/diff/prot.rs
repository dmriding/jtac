//! `JT-PROT-001`: a change to a protected path, and the tiers a change lands in.

use std::collections::BTreeSet;

use jtac_core::Severity;
use jtac_policy::Matchers;

use super::report::Finding;
use super::rule::Rule;
use super::view::View;

/// A finding per protected path touched, by either name of a rename. The commit that adds the
/// policy had nothing in force under `.jtac/`, so a finding there is minor (decision 38;
/// `adopting_the_policy_makes_its_own_directory_minor_and_nothing_else`). `adopting` is the
/// gatherer's word that no ancestor held one: the diff's base alone is the pusher's to pick.
pub(crate) fn check(views: &[View], matchers: &Matchers, adopting: bool) -> Vec<Finding> {
    let adopting = adopting
        && views.iter().any(|v| {
            v.path() == jtac_policy::POLICY_PATH
                && v.change.old.is_none()
                && v.change.from.is_none()
        });
    views
        .iter()
        .flat_map(|v| {
            [Some(v.path()), v.change.from.as_deref()]
                .into_iter()
                .flatten()
                .filter(|p| matchers.protected.is_match(p))
                .map(move |p| {
                    let (severity, tail) = if adopting && p.starts_with(".jtac/") {
                        (
                            Severity::Minor,
                            "; the base holds no policy, so this change brings it into force",
                        )
                    } else {
                        (Severity::Major, "")
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
        let got: Vec<(String, String)> = check(&views, &matchers, true)
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
        type Row = (&'static str, bool, Vec<Change>, Vec<(Severity, bool)>);
        let rows: [Row; 4] = [
            (
                "an ancestor held the policy: a base the pusher chose does not adopt",
                false,
                vec![change(".jtac/policy.toml", None)],
                vec![(Severity::Major, false)],
            ),
            (
                "the adoption commit: .jtac/ minor, CI major",
                true,
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
                true,
                vec![edited.clone(), change(".jtac/signers", None)],
                vec![(Severity::Major, false), (Severity::Major, false)],
            ),
            (
                "a policy renamed into place is not adoption",
                true,
                vec![change(".jtac/policy.toml", Some("policy.toml"))],
                vec![(Severity::Major, false)],
            ),
        ];
        for (claim, fact, changes, want) in rows {
            let views: Vec<View> = changes.iter().map(View::of).collect();
            let got: Vec<(Severity, bool)> = check(&views, &matchers, fact)
                .into_iter()
                .map(|f| (f.severity, f.message.contains("brings it into force")))
                .collect();
            assert_eq!(got, want, "{claim}");
        }
    }
}
