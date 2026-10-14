//! `JT-PROT-001`: a change to a protected path, and the tiers a change lands in.

use std::collections::BTreeSet;

use jtac_core::Severity;
use jtac_policy::Matchers;

use super::report::Finding;
use super::rule::Rule;
use super::view::View;

/// A finding per protected path touched, by either name of a rename.
pub(crate) fn check(views: &[View], matchers: &Matchers) -> Vec<Finding> {
    views
        .iter()
        .flat_map(|v| {
            [Some(v.path()), v.change.from.as_deref()]
                .into_iter()
                .flatten()
                .filter(|p| matchers.protected.is_match(p))
                .map(move |p| {
                    Finding::new(
                        Rule::Prot001,
                        Severity::Major,
                        v.path(),
                        0,
                        format!("protected path `{p}` changed"),
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
        let got: Vec<(String, String)> = check(&views, &matchers)
            .into_iter()
            .map(|f| (f.path, f.message))
            .collect();
        assert_eq!(
            got,
            [
                (
                    ".jtac/policy.toml".to_owned(),
                    "protected path `.jtac/policy.toml` changed".to_owned()
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
}
