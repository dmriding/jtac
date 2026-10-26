//! Words bash reads a second time, so a quoted substitution inside one runs after all
//! (`crate::classify::spellings::INDIRECTION` holds the spellings).

use shuck_ast::{Span, Word as AstWord, WordPart};

use super::Word;
use super::quote::{ansi_decode, spells_a_substitution, unescape};

// bash reads these words again as arithmetic or as an array name, so a quoted `$(…)` in one
// runs: `let 'a[$(cmd)]++'`, `unset 'a[$(cmd)]'`, `printf -v 'a[$(cmd)]' x`, `test -v`.
pub(crate) fn evaluates(prog: &str, args: &[Word], quoted: &[bool]) -> bool {
    let any = quoted.contains(&true);
    let flag = |c: char| {
        args.iter()
            .take_while(|a| a.text.starts_with('-'))
            .any(|a| a.text.contains(c))
    };
    match prog {
        "let" | "unset" | "read" | "compgen" | "wait" => any,
        "printf" => args.iter().enumerate().any(|(k, a)| {
            a.text.strip_prefix("-v").is_some_and(|rest| {
                if rest.is_empty() {
                    quoted.get(k + 1) == Some(&true)
                } else {
                    quoted[k]
                }
            })
        }),
        "test" | "[" => any && args.iter().any(|a| matches!(a.text.as_str(), "-v" | "-R")),
        "declare" | "typeset" | "local" | "readonly" => args.iter().zip(quoted).any(|(a, q)| {
            let (name, value) = a.text.split_once('=').unwrap_or((&a.text, ""));
            let scalar = prog != "readonly" && (name.contains('[') || flag('i'));
            *q && !a.text.starts_with('-')
                && (scalar || (flag('a') || flag('A')) && value.starts_with('('))
        }),
        _ => false,
    }
}

// Text now, a substitution when bash expands the word again. Judged on the word's static
// text joined and decoded, with every expansion a placeholder, so `$''(`, `\\$\\(` and
// `$'\\x24'"("` all show the `$(` they spell.
pub(crate) fn quoted_substitution(source: &str, w: &AstWord) -> bool {
    let inner = |span: Span, dollar: bool| {
        let (from, to) = (
            span.start.offset() + 1 + usize::from(dollar),
            span.end.offset(),
        );
        source
            .get(from..to.saturating_sub(1))
            .unwrap_or("")
            .to_string()
    };
    spells_a_substitution(&static_text(source, w, &inner))
}

/// The word's static text, quotes removed and escapes decoded, every expansion a
/// placeholder; `inner` gives a `'…'` or `$'…'` part's content from its span.
pub(crate) fn static_text(
    source: &str,
    w: &AstWord,
    inner: &dyn Fn(Span, bool) -> String,
) -> String {
    let mut text = String::new();
    for (p, span) in w.parts_with_spans() {
        static_part(source, p, span, inner, &mut text);
    }
    text
}

fn static_part(
    source: &str,
    p: &WordPart,
    span: Span,
    inner: &dyn Fn(Span, bool) -> String,
    out: &mut String,
) {
    match p {
        WordPart::Literal(_) => out.push_str(&unescape(&slice(source, span))),
        WordPart::SingleQuoted { dollar, .. } => {
            let content = inner(span, *dollar);
            if *dollar {
                out.push_str(&ansi_decode(&content));
            } else {
                out.push_str(&content);
            }
        }
        WordPart::DoubleQuoted { parts, .. } => {
            for n in parts {
                static_part(source, &n.kind, n.span, inner, out);
            }
        }
        _ => out.push('\u{1}'),
    }
}

/// A span's text, trimmed of whitespace and of the terminator a command may trail.
pub(crate) fn slice(source: &str, span: Span) -> String {
    source
        .get(span.start.offset()..span.end.offset())
        .unwrap_or("")
        .trim()
        .trim_end_matches(['&', '|', ';'])
        .trim()
        .to_string()
}
