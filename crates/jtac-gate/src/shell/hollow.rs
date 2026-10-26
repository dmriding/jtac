//! What the parser dropped and did not say. shuck reparses the body of a `${…}` operand, a
//! subscript, a pattern or an arithmetic expression with a cumulative depth cap of its own;
//! past it, under a clean status, a substitution comes back with an empty body, an operand or a
//! pattern comes back as plain text, and an arithmetic command comes back with no expression.
//! Each is text the shell will run and this reader did not see, so each is unreadable
//! (`a_substitution_the_parser_left_empty_is_unreadable`, `plain_text_the_parser_built_holds_no_expansion`).

/// True when a substitution body holds no statement and no comment but `text`, its own source,
/// says something once delimiters, line continuations and whitespace are stripped.
pub(crate) fn hollow(statements: usize, comments: usize, text: &str) -> bool {
    let inner = text.trim_matches(|c: char| c.is_whitespace() || "$()<>`\\".contains(c));
    statements == 0 && comments == 0 && !inner.is_empty()
}

/// True when `source`, the text of a part the parser kept as a literal, holds an expansion the
/// shell would still perform: a backtick, a `$` before what can open one in bash's grammar
/// (`{ ( [`, a name, a digit, a special parameter; `$'…'` and `$"…"` run nothing), or where
/// `quoted` is false a `<(` or `>(`, each behind an even number of backslashes. A literal the
/// parser built itself never holds one; `"^fn main$"` and `[$]x` are text to bash.
pub(crate) fn degraded(source: &str, quoted: bool) -> bool {
    let b = source.as_bytes();
    let mut slashes = 0usize;
    for (i, &c) in b.iter().enumerate() {
        let next = b.get(i + 1).copied().unwrap_or(b' ');
        let paren = next == b'(';
        let expands = next.is_ascii_alphanumeric() || b"{([_@*#?$!-".contains(&next);
        let opens =
            (c == b'$' && expands) || c == b'`' || (!quoted && paren && matches!(c, b'<' | b'>'));
        if opens && slashes.is_multiple_of(2) {
            return true;
        }
        slashes = if c == b'\\' { slashes + 1 } else { 0 };
    }
    false
}

/// True when bash reads a heredoc other than the parser did. A body line as bash reads it
/// (leading tabs stripped under `<<-`) that equals the delimiter, or inside a substitution
/// starts with it and holds a `)` after it, closed the heredoc there; the line after the body
/// not being the delimiter exactly means bash reads on. A body that runs to the end of the
/// text is read the same way by both.
pub(crate) fn heredoc_cut_short(
    source: &str,
    from: usize,
    to: usize,
    delimiter: &str,
    strip_tabs: bool,
    nested: bool,
) -> bool {
    fn strip(line: &str, tabs: bool) -> &str {
        if tabs {
            line.trim_start_matches('\t')
        } else {
            line
        }
    }
    let body = source.get(from..to).unwrap_or("");
    let closes = body.lines().map(|l| strip(l, strip_tabs)).any(|l| {
        l == delimiter || (nested && l.strip_prefix(delimiter).is_some_and(|r| r.contains(')')))
    });
    let rest = source.get(to..).unwrap_or("");
    if rest.is_empty() {
        return closes;
    }
    closes || strip(rest.split('\n').next().unwrap_or(""), strip_tabs) != delimiter
}

/// An unquoted heredoc body as the shell reading it sees it: bash drops the backslash
/// before `$`, a backtick, a backslash and a newline (joining the lines) before anything
/// parses the text, and keeps every other one.
pub(crate) fn cooked_heredoc(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek().copied()) {
            ('\\', Some(n @ ('$' | '`' | '\\'))) => {
                out.push(n);
                chars.next();
            }
            ('\\', Some('\n')) => {
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{cooked_heredoc, degraded, heredoc_cut_short, hollow};

    #[test]
    fn a_substitution_the_parser_left_empty_is_unreadable() {
        let rows: &[(usize, usize, &str, bool)] = &[
            (0, 0, "$(git push)", true),
            (0, 0, "<(git push)", true),
            (0, 0, "`git push`", true),
            (0, 0, "$( (git push) )", true),
            (0, 0, "$( )", false),
            (0, 0, "$(\n\t)", false),
            (0, 0, "$(\\\n)", false),
            (0, 0, "<( )", false),
            (0, 0, "``", false),
            (0, 1, "$( # a comment alone )", false),
            (1, 0, "$(git push)", false),
            (2, 3, "$(a; b # c\n)", false),
        ];
        for (statements, comments, text, want) in rows {
            assert_eq!(hollow(*statements, *comments, text), *want, "{text:?}");
        }
    }

    #[test]
    fn a_heredoc_bash_reads_other_than_the_parser_did_is_cut_short() {
        type Row<'a> = (&'a str, usize, usize, &'a str, bool, bool, bool);
        let rows: &[Row] = &[
            ("cat <<E\nx\nE\n", 8, 10, "E", false, false, false),
            ("cat <<E\nx\nE \n", 8, 10, "E", false, false, true),
            ("cat <<E\nx\nE\t\n", 8, 10, "E", false, false, true),
            ("cat <<E\nx\\\\\nE\n", 8, 12, "E", false, false, false),
            ("cat <<-E\n\tx\n\tE\n", 9, 12, "E", true, false, false),
            ("cat <<E\n\tx\n\tE\n", 8, 11, "E", false, false, true),
            ("cat <<E\nx\nE", 8, 10, "E", false, false, false),
            ("cat <<E\nx\n", 8, 10, "E", false, false, false),
            (
                "cat <<E\nx\nE\ngit push\nE\n",
                8,
                10,
                "E",
                false,
                false,
                false,
            ),
            (
                "cat <<E\nx\nE\ngit push\nE\n",
                8,
                21,
                "E",
                false,
                false,
                true,
            ),
            (
                "x=$(cat <<E\nx\nE)\ngit push\nE\n)",
                12,
                26,
                "E",
                false,
                true,
                true,
            ),
            (
                "x=$(cat <<E\nx\nE)\ngit push\nE\n)",
                12,
                26,
                "E",
                false,
                false,
                false,
            ),
            (
                "x=$(cat <<E\nx\nEOF)\ngit push\nE\n)",
                12,
                28,
                "E",
                false,
                true,
                true,
            ),
        ];
        for (source, from, to, delimiter, strip, nested, want) in rows {
            assert_eq!(
                heredoc_cut_short(source, *from, *to, delimiter, *strip, *nested),
                *want,
                "{source:?}"
            );
        }
    }

    #[test]
    fn an_unquoted_heredoc_body_is_cooked_as_bash_hands_it_on() {
        let rows: &[(&str, &str)] = &[
            ("r=\"\\$(x)\"", "r=\"$(x)\""),
            ("x=\\`y\\`", "x=`y`"),
            ("a\\\nb", "ab"),
            ("a\\\\b", "a\\b"),
            ("a\\\"b", "a\\\"b"),
            ("a\\xb", "a\\xb"),
            ("a\\", "a\\"),
            ("", ""),
        ];
        for (body, want) in rows {
            assert_eq!(cooked_heredoc(body), *want, "{body:?}");
        }
    }

    #[test]
    fn plain_text_the_parser_built_holds_no_expansion() {
        let rows: &[(&str, bool, bool)] = &[
            ("$(git push)", false, true),
            ("echo ${b:-$(git push)}", false, true),
            ("${y@P}", false, true),
            ("$x", true, true),
            ("$1 ", false, true),
            ("$_", false, true),
            ("$@", false, true),
            ("$[1]", false, true),
            ("$'a'", false, false),
            ("$\"a\"", true, false),
            ("$ (x)", false, false),
            ("^fn main$", true, false),
            ("a$", false, false),
            ("[$]x", false, false),
            ("*$)", false, false),
            ("<(git push)", false, true),
            (">(git push)", false, true),
            ("<(git push)", true, false),
            ("a>(b)", true, false),
            ("`git push`", false, true),
            ("`git push`", true, true),
            ("a\\\\$(git push)", false, true),
            ("\\$(x)", false, false),
            ("\\$(x)", true, false),
            ("\\`b", true, false),
            ("\\\\\\$(x)", false, false),
            ("a<b>c", false, false),
            ("handle <(x) in the reader", true, false),
            ("plain", false, false),
            ("", false, false),
        ];
        for (source, quoted, want) in rows {
            assert_eq!(
                degraded(source, *quoted),
                *want,
                "{source:?} quoted={quoted}"
            );
        }
    }
}
