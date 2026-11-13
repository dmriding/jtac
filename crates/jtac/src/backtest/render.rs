//! The `jtac backtest` summary as text: the counts, then what they do not say. It renders the
//! summary alone, which holds no path, message, commit id or author.

use std::fmt::Write;

use super::summary::{Alone, RuleCount, Summary};

/// Render `s`.
pub fn text(s: &Summary) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "BACKTEST: {} judged, {} skipped (no parent); block_at {}",
        units(s.units),
        units(s.skipped),
        s.block_at
    );
    let _ = writeln!(
        out,
        "  would block  {} ({})",
        units(s.blocking),
        share(s.blocking_per_mille)
    );
    let _ = writeln!(
        out,
        "  no finding   {} ({})",
        units(s.silent),
        share(s.silent_per_mille)
    );
    if !s.findings.is_empty() {
        let _ = writeln!(out, "  findings by rule and severity");
    }
    for c in &s.findings {
        let _ = writeln!(out, "    {}", count(c));
    }
    if !s.blocked_alone.is_empty() {
        let _ = writeln!(out, "  units blocked by one rule alone");
    }
    for a in &s.blocked_alone {
        let _ = writeln!(out, "    {}", alone(a));
    }
    if !s.languages.is_empty() {
        let _ = writeln!(out, "  by language");
    }
    for l in &s.languages {
        for c in &l.findings {
            let _ = writeln!(out, "    {:<7} {}", l.language, count(c));
        }
        for a in &l.blocked_alone {
            let _ = writeln!(out, "    {:<7} alone {}", l.language, alone(a));
        }
    }
    if let Some(n) = s.rows_written {
        let _ = writeln!(
            out,
            "  rows  {n} written to the --rows file, which holds paths and messages: share this \
             summary, not that file"
        );
    }
    let _ = writeln!(out, "  what this does not say");
    for line in s.does_not_say {
        let _ = writeln!(out, "    {line}");
    }
    out
}

fn count(c: &RuleCount) -> String {
    let preexisting = c
        .preexisting
        .map_or_else(String::new, |n| format!(", {n} preexisting"));
    format!(
        "{:<12} {:<8} {}{preexisting}",
        c.rule.id(),
        c.severity.to_string(),
        c.findings
    )
}

fn alone(a: &Alone) -> String {
    format!("{:<12} {}", a.rule.id(), units(a.units))
}

fn units(n: u64) -> String {
    format!("{n} unit{}", if n == 1 { "" } else { "s" })
}

/// Per mille as a percentage with one decimal, from integers alone.
fn share(per_mille: u64) -> String {
    format!("{}.{}%", per_mille / 10, per_mille % 10)
}

#[cfg(test)]
mod tests {
    use jtac_checks::diff::Rule;
    use jtac_core::Severity;

    use super::*;
    use crate::backtest::summary::{ByLanguage, DOES_NOT_SAY};

    fn summary(rows_written: Option<u64>) -> Summary {
        let size = RuleCount {
            rule: Rule::Size001,
            severity: Severity::Major,
            findings: 3,
            preexisting: Some(2),
        };
        let alone = Alone {
            rule: Rule::Test001,
            units: 1,
        };
        Summary {
            v: 1,
            command: "backtest",
            block_at: Severity::Major,
            units: 8,
            skipped: 1,
            blocking: 3,
            blocking_per_mille: 375,
            silent: 1,
            silent_per_mille: 125,
            findings: vec![size.clone()],
            blocked_alone: vec![alone.clone()],
            languages: vec![ByLanguage {
                language: "rust",
                findings: vec![size],
                blocked_alone: vec![alone],
            }],
            rows_written,
            does_not_say: DOES_NOT_SAY,
        }
    }

    #[test]
    fn text_gives_shares_from_integers_and_states_what_it_does_not_say() {
        let text = text(&summary(None));
        for want in [
            "BACKTEST: 8 units judged, 1 unit skipped (no parent); block_at major",
            "would block  3 units (37.5%)",
            "no finding   1 unit (12.5%)",
            "JT-SIZE-001  major    3, 2 preexisting",
            "rust    JT-SIZE-001",
            "rust    alone JT-TEST-001  1 unit",
            "what this does not say",
            DOES_NOT_SAY[0],
            DOES_NOT_SAY[1],
        ] {
            assert!(text.contains(want), "{want:?} in\n{text}");
        }
        assert!(!text.contains("--rows"), "{text}");
    }

    #[test]
    fn the_rows_note_says_the_file_holds_paths_and_messages() {
        let text = text(&summary(Some(8)));
        assert!(
            text.contains("8 written to the --rows file, which holds paths and messages"),
            "{text}"
        );
    }
}
