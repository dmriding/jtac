//! `JT-SUP-001`: a suppression the diff adds. Every marker is a finding; all of them block when
//! the count passes `max_new_suppressions`, so a waiver by path covers what a reviewer saw.
//! `suppression_spellings` rows the table, bypasses first.

use jtac_core::Severity;
use jtac_policy::Integrity;
use regex::Regex;
use std::sync::LazyLock;

use super::attr::probes;
use super::lang::Lang;
use super::report::Finding;
use super::rule::Rule;
use super::view::View;

/// Marker spellings by language; `None` applies everywhere.
const TABLE: &[(Option<Lang>, &str)] = &[
    (None, r"(?:^|\s)(?://|#)\s*jtac-loc-exempt:"),
    (Some(Lang::Rust), r"^\s*#!?\[.*\b(?:allow|expect)\s*\("),
    (Some(Lang::Python), r"(?i)#\s*noqa\b"),
    (Some(Lang::Python), r"#\s*flake8:\s*noqa\b"),
    (Some(Lang::Python), r"#\s*type:\s*ignore\b"),
    (Some(Lang::Python), r"#\s*pyright:\s*ignore\b"),
    (Some(Lang::Python), r"#\s*pylint:\s*(?:disable|skip-file)\b"),
    (Some(Lang::Python), r"#\s*mypy:\s*ignore-errors\b"),
    (Some(Lang::Python), r"#\s*ruff:\s*noqa\b"),
    (Some(Lang::Python), r"#\s*nosec\b"),
    (Some(Lang::Python), r"#\s*fmt:\s*(?:off|skip)\b"),
    (Some(Lang::Go), r"//\s*nolint\b"),
    (Some(Lang::Go), r"//\s*lint:(?:ignore|file-ignore)\b"),
    (Some(Lang::Go), r"//\s*#nosec\b"),
    (Some(Lang::Js), r"(?://|/\*)\s*eslint-disable\b"),
    (
        Some(Lang::Js),
        r"(?://|/\*)\s*eslint\s+[\w@/-]+\s*:\s*(?:off|0)\b",
    ),
    (
        Some(Lang::Js),
        r"//\s*@ts-(?:ignore|expect-error|nocheck)\b",
    ),
    (Some(Lang::Js), r"(?://|/\*)\s*biome-ignore\b"),
    (Some(Lang::Js), r"(?://|/\*)\s*prettier-ignore\b"),
    (
        Some(Lang::Js),
        r"(?://|/\*)\s*(?:istanbul|c8|v8)\s+ignore\b",
    ),
    (Some(Lang::Js), r"(?://|/\*)\s*tslint:disable\b"),
];

static PATTERNS: LazyLock<Vec<(Option<Lang>, Regex)>> = LazyLock::new(|| {
    TABLE
        .iter()
        .map(|(lang, re)| (*lang, Regex::new(re).expect("literal")))
        .collect()
});

/// Whether an added `line` in a file of `lang` carries a marker.
pub(crate) fn marker(lang: Option<Lang>, line: &str) -> bool {
    PATTERNS
        .iter()
        .any(|(l, re)| (l.is_none() || *l == lang) && re.is_match(line))
}

pub(crate) fn check(views: &[View], integrity: &Integrity) -> Vec<Finding> {
    let mut hits = Vec::new();
    for v in views {
        for (n, text, probe) in probes(v, true) {
            if marker(v.new_lang, &probe) {
                hits.push((v.path(), n, text));
            }
        }
    }
    let over = u64::try_from(hits.len()).unwrap_or(u64::MAX) > integrity.max_new_suppressions;
    let severity = if over {
        Severity::Blocker
    } else {
        Severity::Minor
    };
    hits.into_iter()
        .map(|(path, line, text)| {
            Finding::new(
                Rule::Sup001,
                severity,
                path,
                line,
                format!("new suppression `{}`", shown(text)),
            )
        })
        .collect()
}

/// The line, trimmed and capped.
pub(crate) fn shown(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(60) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::change::{Blob, Change};

    #[test]
    fn suppression_spellings() {
        let rows: &[(Lang, &str, bool)] = &[
            (Lang::Rust, "#[allow(dead_code)]", true),
            (Lang::Rust, "#![allow(clippy::all)]", true),
            (Lang::Rust, "    #[expect(unused)]", true),
            (Lang::Rust, "#[cfg_attr(test, allow(unused))]", true),
            (Lang::Rust, "// jtac-loc-exempt: one table", true),
            (Lang::Rust, "#[deny(warnings)]", false),
            (Lang::Rust, "let allow = f(allow(x));", false),
            (Lang::Rust, "// allow(dead_code) was here", false),
            (Lang::Python, "x = 1  # noqa: E501", true),
            (Lang::Python, "x = 1  #noqa", true),
            (
                Lang::Python,
                "y: int = f()  # type: ignore[attr-defined]",
                true,
            ),
            (Lang::Python, "# pyright: ignore", true),
            (Lang::Python, "# pylint: disable=missing-docstring", true),
            (Lang::Python, "# mypy: ignore-errors", true),
            (Lang::Python, "# ruff: noqa", true),
            (
                Lang::Python,
                "subprocess.call(x, shell=True)  # nosec",
                true,
            ),
            (Lang::Python, "# fmt: off", true),
            (Lang::Python, "# fmt: on", false),
            (Lang::Python, "x = 1  # NOQA", true),
            (Lang::Python, "# flake8: noqa", true),
            (Lang::Python, "# pylint: skip-file", true),
            (Lang::Python, "# jtac-loc-exempt: x", true),
            (Lang::Python, "# noqualified", false),
            (Lang::Python, "# type: ignored_field", false),
            (Lang::Python, "x = 1  # not a noqa", false),
            (Lang::Go, "x := f() //nolint:errcheck", true),
            (Lang::Go, "// nolint", true),
            (Lang::Go, "//lint:ignore SA1019 reason", true),
            (Lang::Go, "// #nosec G204", true),
            (Lang::Go, "//lint:file-ignore SA1019 reason", true),
            (Lang::Go, "// nolinted is a word", false),
            (Lang::Js, "// eslint-disable-next-line no-unused-vars", true),
            (Lang::Js, "/* eslint-disable */", true),
            (Lang::Js, "x(); // eslint-disable-line", true),
            (Lang::Js, "// @ts-ignore", true),
            (Lang::Js, "// @ts-expect-error: later", true),
            (Lang::Js, "// @ts-nocheck", true),
            (Lang::Js, "// biome-ignore lint: reason", true),
            (Lang::Js, "// prettier-ignore", true),
            (Lang::Js, "/* istanbul ignore next */", true),
            (Lang::Js, "/* c8 ignore start */", true),
            (Lang::Js, "// tslint:disable-next-line", true),
            (Lang::Js, "/* eslint no-unused-vars: off */", true),
            (Lang::Js, "/* eslint quotes: 0 */", true),
            (Lang::Js, "// eslint no-unused-vars: error", false),
            (Lang::Js, "// @ts-check", false),
            (Lang::Js, "// eslint-enable", false),
            (Lang::Js, "const eslintDisable = 1;", false),
            (Lang::Js, "#[allow(dead_code)]", false),
        ];
        for (lang, line, want) in rows {
            assert_eq!(marker(Some(*lang), line), *want, "{lang:?}: {line:?}");
        }
        assert!(marker(None, "// jtac-loc-exempt: x"));
        assert!(!marker(None, "#[allow(dead_code)]"));
    }

    #[test]
    fn count_past_the_budget_makes_every_marker_a_blocker() {
        let change = Change {
            path: "src/a.rs".into(),
            from: None,
            old: Some(Blob::Text("#[allow(a)]\nfn a() {}\n".into())),
            new: Some(Blob::Text(
                "#[allow(a)]\nfn a() {}\n#[allow(b)]\nfn b() {}\n#[allow(c)]\nfn c() {}\nconst S: &str = \"#[allow(d)]\";\n".into(),
            )),
        };
        let views = [View::of(&change)];
        let mut integrity = Integrity {
            max_new_suppressions: 2,
            ..Integrity::default()
        };
        let got = check(&views, &integrity);
        assert_eq!(
            got.iter().map(|f| (f.line, f.severity)).collect::<Vec<_>>(),
            [(3, Severity::Minor), (5, Severity::Minor)]
        );
        assert_eq!(got[0].message, "new suppression `#[allow(b)]`");
        integrity.max_new_suppressions = 1;
        let got = check(&views, &integrity);
        assert!(got.iter().all(|f| f.severity == Severity::Blocker));
        assert_eq!(shown(&"x".repeat(70)), format!("{}…", "x".repeat(60)));
    }

    #[test]
    fn a_rust_attribute_split_across_lines_is_one_marker() {
        let d = Integrity::default();
        type Row<'a> = (&'a str, &'a str, &'a [(u64, &'a str)]);
        let rows: &[Row] = &[
            (
                "fn a() {}\n",
                "#[cfg_attr(\n    all(),\n    allow(dead_code)\n)]\nfn a() {}\n",
                &[(1, "#[cfg_attr(")],
            ),
            (
                "#[cfg_attr(\n    all(),\n    deny(x)\n)]\nfn a() {}\n",
                "#[cfg_attr(\n    all(),\n    deny(x),\n    allow(y)\n)]\nfn a() {}\n",
                &[(1, "#[cfg_attr(")],
            ),
            ("fn a() {\n}\n", "fn a() {\n    allow(y);\n}\n", &[]),
            (
                "#[cfg_attr(\n    all(),\n    allow(x)\n)]\nfn a() {}\n",
                "#[cfg_attr(\n    all(),\n    // why\n    allow(x)\n)]\nfn a() {}\n",
                &[],
            ),
            (
                "#[cfg_attr(\n    all(),\n    allow(x)\n)]\nfn a() {}\n#[cfg_attr(\n    any(),\n    allow(x)\n)]\nfn b() {}\n",
                "#[cfg_attr(\n    all(),\n    allow(x)\n)]\nfn a() {}\n#[cfg_attr(\n    all(),\n    allow(x)\n)]\nfn b() {}\n",
                &[(6, "#[cfg_attr(")],
            ),
            (
                "#[cfg_attr(\n    all(),\n    allow(x)\n)]\nfn a() {}\n#[cfg_attr(\n    any(),\n    allow(x)\n)]\nfn b() {}\n",
                "#[cfg_attr(\n    any(),\n    allow(x)\n)]\nfn a() {}\n#[cfg_attr(\n    all(),\n    allow(x)\n)]\nfn b() {}\n",
                &[(1, "#[cfg_attr("), (6, "#[cfg_attr(")],
            ),
            (
                "#[allow(\n    dead_code, // a\n)]\nfn f() -> u8 { 1 }\n",
                "#[allow(\n    dead_code, // b\n)]\nfn f() -> u8 { 2 }\n",
                &[],
            ),
            (
                "#[allow(\n    clippy::too_many_arguments,\n)]\nfn a(x: u8) {}\n",
                "#[allow(\n    // wire format\n    clippy::too_many_arguments,\n)]\nfn a(x: u8, y: u8) {}\n",
                &[],
            ),
            (
                "#[allow(\n    dead_code,\n)]\nfn b() {}\n#[allow(\n    clippy::too_many_lines,\n)]\nfn a() {}\n",
                "#[allow(\n    clippy::too_many_lines,\n)]\nfn b() {}\n",
                &[(1, "#[allow(")],
            ),
            (
                "",
                "#[derive(Debug)]\nstruct S;\n#[allow(x)] // jtac-loc-exempt: no\nfn b() {}\n",
                &[(3, "#[allow(x)] // jtac-loc-exempt: no")],
            ),
            (
                "#[cfg_attr(\n    feature = \"never\",\n    allow(dead_code)\n)]\nfn a() {}\n",
                "#[cfg_attr(\n    feature = \"std\",\n    allow(dead_code)\n)]\nfn a() {}\n",
                &[(1, "#[cfg_attr(")],
            ),
            (
                "#[cfg_attr(\n    all(),\n    allow(x) // a\n)]\nfn a() {}\n",
                "#[cfg_attr(\n    all(),\n    allow(x) // b\n)]\nfn a() {}\n",
                &[],
            ),
            (
                "#[cfg_attr(\n    all(),\n    /* a */ allow(x)\n)]\nfn a() {}\n",
                "#[cfg_attr(\n    all(),\n    /* b */ allow(x)\n)]\nfn a() {}\n",
                &[],
            ),
        ];
        for (old, new, want) in rows {
            let change = Change {
                path: "src/a.rs".into(),
                from: None,
                old: Some(Blob::Text((*old).into())),
                new: Some(Blob::Text((*new).into())),
            };
            let got: Vec<(u64, String)> = check(&[View::of(&change)], &d)
                .into_iter()
                .map(|f| (f.line, f.message))
                .collect();
            let want: Vec<(u64, String)> = want
                .iter()
                .map(|(l, m)| (*l, format!("new suppression `{m}`")))
                .collect();
            assert_eq!(got, want, "{new:?}");
        }
    }
}
