//! `JT-ADD-001`: lines added past `max_added_lines`. The count is reported either way.

use jtac_core::Severity;
use jtac_policy::Integrity;

use super::report::Finding;
use super::rule::Rule;
use super::view::View;

/// Lines the change added across every file.
pub(crate) fn count(views: &[View]) -> u64 {
    views
        .iter()
        .map(|v| u64::try_from(v.edit.added.len()).unwrap_or(u64::MAX))
        .sum()
}

/// One minor finding, with no path, when the budget is set and exceeded.
pub(crate) fn check(added: u64, integrity: &Integrity) -> Vec<Finding> {
    integrity
        .max_added_lines
        .filter(|&max| added > max)
        .map(|max| {
            Finding::new(
                Rule::Add001,
                Severity::Minor,
                "",
                0,
                format!("{added} lines added, budget {max}"),
            )
        })
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_only_judged_when_set_and_exact_is_not_over() {
        let mut integrity = Integrity::default();
        assert!(check(5, &integrity).is_empty());
        integrity.max_added_lines = Some(5);
        assert!(check(5, &integrity).is_empty());
        let got = check(6, &integrity);
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].severity, got[0].path.as_str(), got[0].line),
            (Severity::Minor, "", 0)
        );
        assert_eq!(got[0].message, "6 lines added, budget 5");
    }
}
