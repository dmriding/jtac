//! `JT-CMT-001`: a changed file with at least as many plain comment lines as code lines, and an
//! added plain comment inside a block. A comment sits at a block's head, at its tail, or
//! directly above the line that opens one; anything else is inside.

use jtac_core::Severity;

use super::lang::Lang;
use super::lines::counts;
use super::report::Finding;
use super::rule::Rule;
use super::scan::{Kind, Line};
use super::span::depths;
use super::view::View;

pub(crate) fn check(views: &[View]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for v in views {
        let Some(lang) = v.new_lang.filter(|_| !v.new.is_empty()) else {
            continue;
        };
        let finding =
            |line, message| Finding::new(Rule::Cmt001, Severity::Minor, v.path(), line, message);
        let c = counts(lang, &v.new, v.new_test);
        if c.comment > 0 && c.comment >= c.code {
            findings.push(finding(
                0,
                format!("{} comment lines, {} code lines", c.comment, c.code),
            ));
        }
        let inside = inside_lines(lang, &v.new);
        for (n, _) in &v.edit.added {
            let i = usize::try_from(*n).unwrap_or(usize::MAX) - 1;
            if inside[i] {
                findings.push(finding(*n, "comment inside a block".to_owned()));
            }
        }
    }
    findings
}

/// Per line, whether it is a plain comment inside a block.
pub(crate) fn inside_lines(lang: Lang, lines: &[Line]) -> Vec<bool> {
    let depth = depths(lines);
    let code = |i: usize| lines[i].kind == Kind::Code;
    let indent = |i: usize| lines[i].text.len() - lines[i].text.trim_start().len();
    let python_opens = |i: usize| {
        let t = lines[i].code.trim_end();
        t.ends_with(':') || t.trim_start().starts_with('@')
    };
    let opens = |i: usize| match lang {
        Lang::Python => python_opens(i),
        _ => depth[i].opens(),
    };
    let leaves_open = |i: usize| match lang {
        Lang::Python => python_opens(i),
        _ => depth[i].leaves_open(),
    };
    let mut prev_code = Vec::with_capacity(lines.len());
    let mut last = None;
    for i in 0..lines.len() {
        prev_code.push(last);
        if code(i) {
            last = Some(i);
        }
    }
    let mut next_code = vec![None; lines.len()];
    let mut following = None;
    for (j, slot) in next_code.iter_mut().enumerate().rev() {
        *slot = following;
        if code(j) {
            following = Some(j);
        }
    }
    (0..lines.len())
        .map(|i| {
            if lines[i].kind != Kind::Comment {
                return false;
            }
            let (prev, next) = (prev_code[i], next_code[i]);
            let head = prev.is_none_or(leaves_open) || next.is_some_and(opens);
            let tail = next.is_none_or(|j| match lang {
                Lang::Python => indent(j) < indent(i),
                _ => depth[j].closes(),
            });
            !head && !tail
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::change::{Blob, Change};
    use crate::diff::scan::scan;

    fn inside(lang: Lang, text: &str) -> Vec<usize> {
        inside_lines(lang, &scan(lang, text))
            .into_iter()
            .enumerate()
            .filter_map(|(i, b)| b.then_some(i + 1))
            .collect()
    }

    #[test]
    fn a_comment_is_at_a_head_a_tail_above_an_opener_or_inside() {
        let rows: &[(Lang, &str, &[usize])] = &[
            (
                Lang::Rust,
                "// file head\nfn a() {\n    // head\n    x();\n    // inside\n    y();\n    // tail\n}\n// file tail",
                &[5],
            ),
            (
                Lang::Rust,
                "fn a() {\n    x();\n    // above the loop\n    for i in v {\n    }\n    // above else? no: inside\n    z();\n}",
                &[6],
            ),
            (
                Lang::Rust,
                "fn a() {\n    if x {\n        y();\n    // tail of if\n    } else {\n        z();\n    }\n}",
                &[],
            ),
            (
                Lang::Rust,
                "fn a() {}\n// between items, above the next\nfn b() {}",
                &[],
            ),
            (
                Lang::Rust,
                "fn a() {}\n// between items, nothing opens next\nconst X: u8 = 1;",
                &[2],
            ),
            (
                Lang::Rust,
                "fn a() {\n    x();\n    /* inside\n       block */\n    y();\n}",
                &[3, 4],
            ),
            (
                Lang::Rust,
                "fn a() {\n    x();\n    /// doc inside is not plain\n    y();\n}",
                &[],
            ),
            (
                Lang::Python,
                "# head\ndef a():\n    # head\n    x()\n    # inside\n    y()\n    # tail\n\n# above a decorator\n@dec\ndef b():\n    pass\n# file tail",
                &[5],
            ),
            (
                Lang::Python,
                "def a():\n    x()\n    # above an if\n    if x:\n        y()\n    # inside\n    z()",
                &[6],
            ),
            (Lang::Go, "func a() {\n\tx()\n\t// inside\n\ty()\n}", &[3]),
            (
                Lang::Js,
                "function a() {\n  x();\n  // inside\n  y();\n}\n// tail",
                &[3],
            ),
        ];
        for (lang, text, want) in rows {
            assert_eq!(inside(*lang, text), *want, "{lang:?}: {text:?}");
        }
    }

    #[test]
    fn ratio_is_over_the_file_and_position_over_added_lines() {
        let old = "fn a() {\n    x();\n    // old inside\n    y();\n}\n";
        let new = "fn a() {\n    x();\n    // old inside\n    // new inside\n    y();\n}\n";
        let change = Change {
            path: "src/a.rs".into(),
            from: None,
            old: Some(Blob::Text(old.into())),
            new: Some(Blob::Text(new.into())),
        };
        let got: Vec<(u64, String)> = check(&[View::of(&change)])
            .into_iter()
            .map(|f| (f.line, f.message))
            .collect();
        assert_eq!(got, [(4, "comment inside a block".to_owned())]);
        let heavy = Change {
            path: "src/b.rs".into(),
            from: None,
            old: None,
            new: Some(Blob::Text("// a\n// b\nfn a() {}\n/// doc\n".into())),
        };
        let got: Vec<(u64, String)> = check(&[View::of(&heavy)])
            .into_iter()
            .map(|f| (f.line, f.message))
            .collect();
        assert_eq!(got, [(0, "2 comment lines, 1 code lines".to_owned())]);
        let test_file = Change {
            path: "tests/b.rs".into(),
            ..heavy
        };
        assert!(check(&[View::of(&test_file)]).is_empty());
    }
}
