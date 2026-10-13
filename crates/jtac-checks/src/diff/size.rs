//! `JT-SIZE-001` and `-002`: a changed file past the production-line cap with no exemption
//! marker, and a changed non-test function past the line cap. A function changed when an
//! added line falls inside its span.

use jtac_core::Severity;
use jtac_policy::Integrity;

use super::lines::{counts, exempt};
use super::report::Finding;
use super::rule::Rule;
use super::span::functions;
use super::view::View;

pub(crate) fn check(views: &[View], integrity: &Integrity) -> Vec<Finding> {
    let mut findings = Vec::new();
    for v in views {
        let Some(lang) = v.new_lang.filter(|_| !v.new.is_empty()) else {
            continue;
        };
        let production = counts(lang, &v.new, v.new_test).production;
        if production > integrity.max_file_lines && !exempt(lang, &v.new) {
            findings.push(Finding::new(
                Rule::Size001,
                Severity::Major,
                v.path(),
                0,
                format!(
                    "{production} production lines, cap {}",
                    integrity.max_file_lines
                ),
            ));
        }
        let added: Vec<usize> = v
            .edit
            .added
            .iter()
            .map(|(n, _)| usize::try_from(*n).unwrap_or(usize::MAX) - 1)
            .collect();
        for span in functions(lang, &v.new, v.new_test) {
            let changed = added.iter().any(|&i| (span.start..=span.end).contains(&i));
            if !span.test && !v.new_test && changed && span.len() > integrity.max_function_lines {
                findings.push(Finding::new(
                    Rule::Size002,
                    Severity::Major,
                    v.path(),
                    u64::try_from(span.start + 1).unwrap_or(u64::MAX),
                    format!(
                        "function `{}` is {} lines, cap {}",
                        span.name,
                        span.len(),
                        integrity.max_function_lines
                    ),
                ));
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::change::{Blob, Change};

    fn change(path: &str, old: Option<&str>, new: &str) -> Change {
        Change {
            path: path.into(),
            from: None,
            old: old.map(|s| Blob::Text(s.into())),
            new: Some(Blob::Text(new.into())),
        }
    }

    type Want<'a> = &'a [(u64, &'a str)];

    fn body(n: usize) -> String {
        (0..n).map(|i| format!("    let x{i} = {i};\n")).collect()
    }

    #[test]
    fn file_cap_counts_production_lines_and_honors_the_marker() {
        let integrity = Integrity {
            max_file_lines: 5,
            ..Integrity::default()
        };
        let six = "fn a() {}\n".repeat(6);
        let rows: &[(&str, &str, Want)] = &[
            ("src/a.rs", &six, &[(0, "6 production lines, cap 5")]),
            ("src/a.rs", &"fn a() {}\n".repeat(5), &[]),
            ("tests/a.rs", &six, &[]),
            (
                "src/a.rs",
                &format!("// jtac-loc-exempt: table\n{six}"),
                &[],
            ),
            (
                "src/a.rs",
                &format!(
                    "{}#[cfg(test)]\nmod tests {{\n{}}}\n",
                    "fn a() {}\n".repeat(4),
                    "    fn t() {}\n".repeat(9)
                ),
                &[],
            ),
            ("a.md", &six, &[]),
            (
                "a.py",
                &"x = 1\n".repeat(6),
                &[(0, "6 production lines, cap 5")],
            ),
        ];
        for (path, text, want) in rows {
            let got: Vec<(u64, String)> = check(&[View::of(&change(path, None, text))], &integrity)
                .into_iter()
                .map(|f| (f.line, f.message))
                .collect();
            let want: Vec<(u64, String)> =
                want.iter().map(|(l, m)| (*l, (*m).to_owned())).collect();
            assert_eq!(got, want, "{path}: {text:?}");
        }
    }

    #[test]
    fn function_cap_applies_to_a_changed_non_test_function() {
        let integrity = Integrity {
            max_function_lines: 4,
            ..Integrity::default()
        };
        let long = format!("fn a() {{\n{}}}\n", body(3));
        let short = "fn b() {\n}\n";
        let old = format!("{long}{short}");
        let rows: &[(&str, Option<&str>, &str, Want)] = &[
            (
                "src/a.rs",
                None,
                &old,
                &[(1, "function `a` is 5 lines, cap 4")],
            ),
            (
                "src/a.rs",
                Some(&old),
                &format!("{long}fn b() {{\n    x();\n}}\n"),
                &[],
            ),
            (
                "src/a.rs",
                Some(&old),
                &format!("fn a() {{\n{}}}\n{short}", body(4)),
                &[(1, "function `a` is 6 lines, cap 4")],
            ),
            ("src/a.rs", None, &format!("#[test]\n{long}"), &[]),
            ("tests/a.rs", None, &long, &[]),
            (
                "a.py",
                None,
                &format!("def a():\n{}", "    x = 1\n".repeat(4)),
                &[(1, "function `a` is 5 lines, cap 4")],
            ),
        ];
        for (path, old, new, want) in rows {
            let got: Vec<(u64, String)> = check(&[View::of(&change(path, *old, new))], &integrity)
                .into_iter()
                .map(|f| (f.line, f.message))
                .collect();
            let want: Vec<(u64, String)> =
                want.iter().map(|(l, m)| (*l, (*m).to_owned())).collect();
            assert_eq!(got, want, "{path}: {new:?}");
        }
    }
}
