//! `JT-SIZE-001` and `-002`: a changed file past the production-line cap with no exemption
//! marker, and a changed non-test function past the line cap. A function changed when an
//! added line falls inside its span. Each finding says whether the before side already broke
//! the same cap: the file, or the function of the same name when one has it on each side.

use jtac_core::Severity;
use jtac_policy::Integrity;

use super::lang::Lang;
use super::lines::{counts, exempt};
use super::report::Finding;
use super::rule::Rule;
use super::scan::Line;
use super::span::{Span, functions};
use super::view::View;

pub(crate) fn check(views: &[View], integrity: &Integrity) -> Vec<Finding> {
    let mut findings = Vec::new();
    for v in views {
        let Some(lang) = v.new_lang.filter(|_| !v.new.is_empty()) else {
            continue;
        };
        let before = v.old_lang.filter(|_| !v.old.is_empty());
        if let Some(production) = over_file_cap(lang, &v.new, v.new_test, integrity) {
            let was =
                before.is_some_and(|l| over_file_cap(l, &v.old, v.old_test, integrity).is_some());
            findings.push(
                Finding::new(
                    Rule::Size001,
                    Severity::Major,
                    v.path(),
                    0,
                    format!(
                        "{production} production lines, cap {}",
                        integrity.max_file_lines
                    ),
                )
                .with_preexisting(was),
            );
        }
        let added: Vec<usize> = v
            .edit
            .added
            .iter()
            .map(|(n, _)| usize::try_from(*n).unwrap_or(usize::MAX) - 1)
            .collect();
        let now = functions(lang, &v.new, v.new_test);
        let changed = now
            .iter()
            .filter(|s| over_function_cap(s, v.new_test, integrity))
            .filter(|s| added.iter().any(|&i| (s.start..=s.end).contains(&i)));
        let then = before.map_or_else(Vec::new, |l| functions(l, &v.old, v.old_test));
        for span in changed {
            let was = paired(&now, span, &then)
                .is_some_and(|b| over_function_cap(b, v.old_test, integrity));
            findings.push(
                Finding::new(
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
                )
                .with_preexisting(was),
            );
        }
    }
    findings
}

/// Production lines when past the file cap with no exemption marker.
fn over_file_cap(lang: Lang, lines: &[Line], test: bool, integrity: &Integrity) -> Option<u64> {
    let production = counts(lang, lines, test).production;
    (production > integrity.max_file_lines && !exempt(lang, lines)).then_some(production)
}

/// A non-test function past the function cap.
fn over_function_cap(s: &Span, test: bool, integrity: &Integrity) -> bool {
    !s.test && !test && s.len() > integrity.max_function_lines
}

/// The before-side function `span` was, known only when one function has its name on each
/// side. A line diff cannot tell two functions of one name apart (it keeps their shared
/// braces and returns wherever they fall), so with more than one this says none.
fn paired<'a>(now: &[Span], span: &Span, then: &'a [Span]) -> Option<&'a Span> {
    let named = |spans: &[Span]| spans.iter().filter(|s| s.name == span.name).count();
    then.iter()
        .find(|b| b.name == span.name)
        .filter(|_| named(now) == 1 && named(then) == 1)
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

    #[test]
    fn preexisting_is_the_same_cap_broken_before_by_the_same_counter() {
        let integrity = Integrity {
            max_file_lines: 5,
            max_function_lines: 4,
            ..Integrity::default()
        };
        let lines = |n: usize| "fn a() {}\n".repeat(n);
        let long = |name: &str, n: usize| format!("fn {name}() {{\n{}}}\n", body(n));
        let edited = long("a", 3).replace("x2", "y2");
        type Was<'a> = &'a [(Rule, bool)];
        let rows: &[(&str, Option<&str>, &str, Was)] = &[
            (
                "src/a.rs",
                Some(&lines(6)),
                &lines(7),
                &[(Rule::Size001, true)],
            ),
            (
                "src/a.rs",
                Some(&lines(5)),
                &lines(6),
                &[(Rule::Size001, false)],
            ),
            ("src/a.rs", None, &lines(6), &[(Rule::Size001, false)]),
            (
                "src/a.rs",
                Some(&format!("// jtac-loc-exempt: table\n{}", lines(6))),
                &lines(6),
                &[(Rule::Size001, false)],
            ),
            ("tests/a.rs", Some(&lines(6)), &lines(7), &[]),
            (
                "src/a.rs",
                Some(&long("a", 3)),
                &edited,
                &[(Rule::Size002, true)],
            ),
            (
                "src/a.rs",
                Some(&long("b", 3)),
                &long("a", 3),
                &[(Rule::Size002, false)],
            ),
            (
                "src/a.rs",
                Some(&long("a", 2)),
                &long("a", 3),
                &[(Rule::Size002, false)],
            ),
            (
                "src/a.rs",
                Some(&format!("#[test]\n{}", long("a", 3))),
                &edited,
                &[(Rule::Size002, false)],
            ),
        ];
        for (path, old, new, want) in rows {
            let got: Vec<(Rule, bool)> = check(&[View::of(&change(path, *old, new))], &integrity)
                .into_iter()
                .map(|f| (f.rule, f.preexisting.expect("size findings carry it")))
                .collect();
            assert_eq!(got, *want, "{path}: {old:?} -> {new:?}");
        }
        let moved = Change {
            from: Some("tests/a.rs".into()),
            ..change("src/a.rs", Some(&long("a", 3)), &edited)
        };
        let got: Vec<bool> = check(&[View::of(&moved)], &integrity)
            .into_iter()
            .filter_map(|f| f.preexisting)
            .collect();
        assert_eq!(got, [false], "a test file's function was under no cap");
    }

    #[test]
    fn a_function_pairs_by_name_only_when_one_has_the_name_on_each_side() {
        let integrity = Integrity {
            max_function_lines: 4,
            ..Integrity::default()
        };
        let f = |text: &str, n: usize| format!("fn a() {{\n{}}}\n", text.repeat(n));
        let (long, short) = (f("    x();\n", 4), f("    x();\n", 1));
        let grown = long.replace("}", "    y();\n}");
        let other = f("    other();\n", 4);
        let init = |class: &str, n: usize| {
            format!(
                "class {class}:\n    def __init__(self):\n{}\n",
                "        self.n += 1\n".repeat(n)
            )
        };
        let nested = |n: usize| {
            format!(
                "def deco(f):\n    def wrapper():\n{}    return wrapper\n",
                "        f()\n".repeat(n)
            )
        };
        type Was<'a> = &'a [(u64, bool)];
        let rows: &[(&str, &str, String, String, Was)] = &[
            (
                "one of the name on each side, long before",
                "src/a.rs",
                long.clone(),
                grown.clone(),
                &[(1, true)],
            ),
            (
                "a function of another name above it does not stand in for it",
                "src/a.rs",
                format!("{}{long}", short.replace("fn a", "fn b")),
                format!("{}{grown}", short.replace("fn a", "fn b")),
                &[(4, true)],
            ),
            (
                "a rewrite that keeps no line is still the one of its name",
                "src/a.rs",
                long.clone(),
                other.clone(),
                &[(1, true)],
            ),
            (
                "a second of the name written above a long one",
                "src/a.rs",
                long.clone(),
                format!("{other}{grown}"),
                &[(1, false), (7, false)],
            ),
            (
                "a short one grown as the long one below it is deleted",
                "src/a.rs",
                format!("{short}{long}"),
                grown.clone(),
                &[(1, false)],
            ),
            (
                "and the same with the long one above it",
                "src/a.rs",
                format!("{long}{short}"),
                grown.clone(),
                &[(1, false)],
            ),
            (
                "two classes, each with a long __init__",
                "a.py",
                format!("{}{}", init("A", 5), init("B", 5)),
                format!("{}{}", init("A", 5), init("B", 6)),
                &[(10, false)],
            ),
            (
                "a nested function has its own name",
                "a.py",
                nested(5),
                nested(6),
                &[(1, true), (2, true)],
            ),
        ];
        for (claim, path, old, new, want) in rows {
            let got: Vec<(u64, bool)> =
                check(&[View::of(&change(path, Some(old), new))], &integrity)
                    .into_iter()
                    .filter(|f| f.rule == Rule::Size002)
                    .map(|f| (f.line, f.preexisting.expect("size findings carry it")))
                    .collect();
            assert_eq!(got, *want, "{claim}: {old:?} -> {new:?}");
        }
    }
}
