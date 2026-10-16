//! What the text says before the parser sees it: how deep and how many its brackets are,
//! and how long an arithmetic body runs. The parser recurses on the first two and is
//! quadratic in the third. The scan reads no quotes, comments or heredocs on purpose: a
//! lexer of its own would drift from the parser's, and every drift is a hole. Counted blind,
//! a quoted bracket can only make a script unreadable, never hide one.

/// Frames deeper than this are not read: the parser recurses once per level.
pub(crate) const MAX_SUBSHELLS: usize = 16;
/// Openers in all, whatever the closers say: a quoted `)` cannot lower the count, and the
/// parser thread's stack holds this many nested `$(` in a debug build.
const MAX_OPENERS: usize = 4096;
/// An arithmetic or `${…}` body longer than this is refused: shuck's arithmetic parser is
/// quadratic, and 60 KiB of `1+1+…` takes it half a minute in a debug build.
const MAX_ARITH: usize = 4096;

/// An open bracket and where it opened. `Arith` is a `(` right after `(`, `Dollar` a `{`
/// right after `$`; a `)` closes only a paren frame, a `}` only a brace frame, a `]` only a
/// square one, so a stray or quoted closer pops nothing it does not match.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Paren,
    Arith,
    Square,
    Brace,
    Dollar,
}

fn closes(c: u8, kind: Open) -> bool {
    match c {
        b')' => matches!(kind, Open::Paren | Open::Arith),
        b'}' => matches!(kind, Open::Brace | Open::Dollar),
        b']' => kind == Open::Square,
        _ => false,
    }
}

/// Why the parser must not see this script, if there is a reason.
pub(crate) fn too_deep_or_long(script: &str) -> Option<String> {
    let mut open: Vec<(Open, usize)> = Vec::new();
    let mut openers = 0usize;
    let mut in_backtick = false;
    let mut prev = b' ';
    for (i, &c) in script.as_bytes().iter().enumerate() {
        match c {
            b'(' => open.push((
                if prev == b'(' {
                    Open::Arith
                } else {
                    Open::Paren
                },
                i,
            )),
            b'{' => open.push((
                if prev == b'$' {
                    Open::Dollar
                } else {
                    Open::Brace
                },
                i,
            )),
            b'[' => open.push((Open::Square, i)),
            b')' | b'}' | b']' => {
                if open.last().is_some_and(|(k, _)| closes(c, *k)) {
                    open.pop();
                }
            }
            b'`' => in_backtick = !in_backtick,
            _ => {}
        }
        if matches!(c, b'(' | b'{' | b'[') || (c == b'`' && in_backtick) {
            openers += 1;
        }
        prev = c;
        if open.len() + usize::from(in_backtick) > MAX_SUBSHELLS {
            return Some(format!(
                "it nests brackets or substitutions more than {MAX_SUBSHELLS} deep"
            ));
        }
        if openers > MAX_OPENERS {
            return Some(format!("it opens more than {MAX_OPENERS} brackets"));
        }
        if body_too_long(&open, i) {
            return Some(format!(
                "it holds an arithmetic or parameter body longer than {MAX_ARITH} characters"
            ));
        }
    }
    None
}

// The innermost arithmetic or `${…}` frame is measured unless a `$(…)` sits inside it: a
// command substitution is a script of its own, not arithmetic text.
fn body_too_long(open: &[(Open, usize)], at: usize) -> bool {
    open.iter().rev().find_map(|(k, from)| match k {
        Open::Paren => Some(false),
        Open::Arith | Open::Dollar => Some(at - from > MAX_ARITH),
        Open::Square | Open::Brace => None,
    }) == Some(true)
}
