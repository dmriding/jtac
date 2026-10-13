//! Line counts for the size and comment rules, as `scripts/loc-cap.sh` counts them: production
//! is every line outside a Rust `#[cfg(test)] mod` tail, and a test file has none
//! (DESIGN.md section 10.3). Doc comments count on neither side of the comment ratio. The
//! markers are read from scanned code, so a string quoting them opens no tail, and the tail
//! ends at the module's closing brace by depth or at the script's column-0 line outside a
//! string, whichever is first, so a brace the scanner misses holds nothing open.

use regex::Regex;
use std::sync::LazyLock;

use super::lang::Lang;
use super::scan::{Kind, Line};

/// What a file weighs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Lines outside test code, blanks and comments included.
    pub production: u64,
    /// Plain comment lines among them.
    pub comment: u64,
    /// Non-blank, non-comment lines among them.
    pub code: u64,
}

static TEST_MOD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(pub(\([a-z]+\))? )?mod [a-z0-9_]+ \{$").expect("literal"));

/// Counts over `lines`; all zero for a test file or a Rust file opening with `#![cfg(test)]`.
pub fn counts(lang: Lang, lines: &[Line], test_file: bool) -> Counts {
    let rust = lang == Lang::Rust;
    if test_file || (rust && lines.first().is_some_and(|l| l.code == "#![cfg(test)]")) {
        return Counts::default();
    }
    let mut counts = Counts::default();
    let (mut pending, mut tail) = (false, 0u32);
    for line in lines {
        if rust {
            if std::mem::take(&mut pending) {
                if TEST_MOD.is_match(&line.code) {
                    tail = 1;
                    continue;
                }
                counts.production += 1;
                counts.code += 1;
            } else if line.code == "#[cfg(test)]" {
                pending = true;
                continue;
            }
            if tail > 0
                && !line.continued
                && line
                    .text
                    .starts_with(|c: char| !c.is_whitespace() && c != '}')
            {
                tail = 0;
            } else if tail > 0 {
                tail = line
                    .code
                    .chars()
                    .try_fold(tail, |depth, c| match c {
                        '{' => Some(depth + 1),
                        '}' if depth == 1 => None,
                        '}' => Some(depth - 1),
                        _ => Some(depth),
                    })
                    .unwrap_or(0);
                continue;
            }
        }
        counts.production += 1;
        match line.kind {
            Kind::Comment => counts.comment += 1,
            Kind::Code => counts.code += 1,
            Kind::Blank | Kind::Doc => {}
        }
    }
    counts
}

/// Whether line 1, or line 2 after a shebang, carries `jtac-loc-exempt: <reason>`.
pub fn exempt(lang: Lang, lines: &[Line]) -> bool {
    let marker = format!("{} jtac-loc-exempt: ", lang.line_comment());
    let first = lines.first().map_or("", |l| l.text);
    let line = if first.starts_with("#!") && !first.starts_with("#![") {
        lines.get(1).map_or("", |l| l.text)
    } else {
        first
    };
    line.strip_prefix(&marker)
        .is_some_and(|reason| !reason.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::scan::scan;

    fn fns(n: usize) -> String {
        (0..n).map(|i| format!("pub fn f{i}() {{}}\n")).collect()
    }

    #[test]
    fn production_excludes_only_a_cfg_test_mod_tail() {
        let tail = "#[cfg(test)]\npub(crate) mod tests {\n    fn t() {}\n}\n";
        let rows: &[(&str, bool, (u64, u64, u64))] = &[
            (&format!("{}{tail}", fns(3)), false, (3, 0, 3)),
            (&format!("{}{tail}{}", fns(3), fns(2)), false, (5, 0, 5)),
            ("#[cfg(test)]\nuse std::fmt;\n", false, (2, 0, 2)),
            ("#[cfg(test)]\nmod tests;\n", false, (2, 0, 2)),
            ("#![cfg(test)]\nfn t() {}\n", false, (0, 0, 0)),
            (
                "#[cfg(test)]\nmod tests {\n}\n fn a() {}\n fn b() {}\n",
                false,
                (2, 0, 2),
            ),
            (
                &format!("{}{tail}    mod x {{\n    }}\n", fns(1)),
                false,
                (3, 0, 3),
            ),
            (
                "#[cfg(test)]\nmod tests {\n    fn t() {\n    }\n    mod inner {\n    }\n}\nfn a() {}\n",
                false,
                (1, 0, 1),
            ),
            (
                "#[cfg(test)]\nmod tests {\n    /* /* */ { */\n}\npub fn a() {}\npub fn b() {}\n",
                false,
                (2, 0, 2),
            ),
            (
                "#[cfg(test)]\nmod tests {\n} mod real {\n    pub fn a() {}\n}\n",
                false,
                (2, 0, 2),
            ),
            (
                "pub fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() {\n        let s = r#\"\nhello\n\"#;\n    }\n}\n",
                false,
                (1, 0, 1),
            ),
            (&fns(2), true, (0, 0, 0)),
            (
                "fn a() {}\n// why\n\n/// doc\n//! inner\n//// four\n",
                false,
                (6, 2, 1),
            ),
            ("/* a\n b */\nfn a() {}\n", false, (3, 2, 1)),
        ];
        for (text, test_file, (production, comment, code)) in rows {
            let got = counts(Lang::Rust, &scan(Lang::Rust, text), *test_file);
            assert_eq!(
                (got.production, got.comment, got.code),
                (*production, *comment, *code),
                "{text:?}"
            );
        }
        let go = counts(
            Lang::Go,
            &scan(Lang::Go, "// Doc.\nfunc f() {}\n// why\n"),
            false,
        );
        assert_eq!((go.production, go.comment, go.code), (3, 1, 1));
    }

    #[test]
    fn exemption_marker_needs_the_syntax_the_position_and_a_reason() {
        let rows = [
            (Lang::Rust, "// jtac-loc-exempt: one table\nfn a() {}", true),
            (Lang::Rust, "// jtac-loc-exempt:  \nfn a() {}", false),
            (Lang::Rust, "//jtac-loc-exempt: x\n", false),
            (Lang::Rust, "//! doc\n// jtac-loc-exempt: x\n", false),
            (Lang::Rust, "#![allow(x)]\n// jtac-loc-exempt: x\n", false),
            (
                Lang::Rust,
                "#!/usr/bin/env rust-script\n// jtac-loc-exempt: x\n",
                true,
            ),
            (Lang::Rust, "# jtac-loc-exempt: x\n", false),
            (Lang::Python, "# jtac-loc-exempt: x\n", true),
            (
                Lang::Python,
                "#!/usr/bin/env python3\n# jtac-loc-exempt: x\n",
                true,
            ),
            (
                Lang::Python,
                "#!/usr/bin/env python3\n\n# jtac-loc-exempt: x\n",
                false,
            ),
            (Lang::Js, "// jtac-loc-exempt: x", true),
            (Lang::Go, "", false),
        ];
        for (lang, text, want) in rows {
            assert_eq!(exempt(lang, &scan(lang, text)), want, "{text:?}");
        }
    }

    #[test]
    fn test_module_markers_inside_a_string_open_no_tail() {
        let text = "const S: &str = r#\"\n#[cfg(test)]\nmod tests {\n    x\n}\n\"#;\n";
        let lines = scan(Lang::Rust, text);
        assert_eq!(counts(Lang::Rust, &lines, false).production, 6);
        let real = scan(
            Lang::Rust,
            "fn a() {}\n#[cfg(test)]\nmod tests {\n    x\n}\n",
        );
        assert_eq!(counts(Lang::Rust, &real, false).production, 1);
    }
}
