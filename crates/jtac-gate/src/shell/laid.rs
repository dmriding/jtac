//! Where the parser laid a word's parts. shuck lays a literal it cooked at its cooked width
//! (`\"` and a continuation in `"…"`; `\$`, `\\` and `\/` in a `${…}` operand), so every span
//! after it falls left of the source, and what the reader then slices is not what runs. One
//! rule in place of modelling the shift: a part that can run a command is laid where the
//! source has it, framed by its own opener and closer and tiling its span with the siblings
//! before it, else the word is unreadable (`every_row…`, the laid rows). Inside a `${…}` the
//! parser shifts every span alike, so the parts still tile and only the frame shows it. Text
//! after the last part that runs sits wherever it sits and runs nothing. Text, a name and a
//! quoted string run nothing, so their layout does not matter (`${BRANCH//\//-}` and
//! `"${N:-\$HOME}"` read), unless a name is laid over an opener bash reads
//! (`${M:-\${$(x)}`, where the cooked reparse saw an opener bash does not), which hides it.

use shuck_ast::{
    BourneParameterExpansion, CommandSubstitutionSyntax, ParameterExpansionSyntax, WordPart,
};

/// True when the parts that run sit where the source has them: their spans, with the ones
/// before them, tile from `from` through the last part that runs.
pub(crate) fn placed(parts: impl Iterator<Item = (bool, (usize, usize))>, from: usize) -> bool {
    let parts: Vec<_> = parts.collect();
    let Some(last) = parts.iter().rposition(|p| p.0) else {
        return true;
    };
    tiles(parts[..=last].iter().map(|p| p.1), from, parts[last].1.1)
}

/// True when `spans`, in order, start at `from`, each where the previous ended, and end at
/// `to`. No parts tile an empty span only.
pub(crate) fn tiles(
    spans: impl IntoIterator<Item = (usize, usize)>,
    from: usize,
    to: usize,
) -> bool {
    let mut at = from;
    for (start, end) in spans {
        if start != at || end < start {
            return false;
        }
        at = end;
    }
    at == to
}

/// True when a part may run a command: a substitution, an arithmetic body, a `${…}` with an
/// operand, a pattern or a subscript, or a `"…"` holding one. A literal, a name, a quoted
/// string and a plain `${name}` run nothing.
pub(crate) fn runs(part: &WordPart) -> bool {
    use BourneParameterExpansion::{Access, Indices, Length, PrefixMatch, Transformation};
    match part {
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_) => false,
        WordPart::DoubleQuoted { parts, .. } => parts.iter().any(|p| runs(&p.kind)),
        WordPart::Parameter(p) => match &p.syntax {
            ParameterExpansionSyntax::Bourne(
                Access { reference }
                | Length { reference }
                | Indices { reference }
                | Transformation { reference, .. },
            ) => reference.subscript.is_some(),
            ParameterExpansionSyntax::Bourne(PrefixMatch { .. }) => false,
            _ => true,
        },
        _ => true,
    }
}

/// True when `text`, the source a part's span covers, starts with the part's opener and ends
/// with its closer. A name or a plain `${…}` runs nothing and has no frame, but hides what
/// it is laid over.
pub(crate) fn framed(text: &str, part: &WordPart) -> bool {
    use CommandSubstitutionSyntax::Backtick;
    let (open, close): (&[&str], &str) = match part {
        WordPart::Variable(_) | WordPart::Parameter(_) if !runs(part) => return !opens(text),
        _ if !runs(part) => return true,
        WordPart::CommandSubstitution { syntax, .. } if *syntax == Backtick => (&["`"], "`"),
        WordPart::CommandSubstitution { .. } => (&["$("], ")"),
        WordPart::ProcessSubstitution { .. } => (&["<(", ">("], ")"),
        WordPart::ArithmeticExpansion { .. } => (&["$((", "$["], ")"),
        WordPart::DoubleQuoted { .. } => (&["\"", "$\""], "\""),
        _ => (&["${"], "}"),
    };
    fits(text, open, close)
}

fn fits(text: &str, open: &[&str], close: &str) -> bool {
    open.iter().any(|o| text.starts_with(o)) && text.ends_with(close) && text.len() >= 2
}

// `$(`, `${`, `$[`, a backtick, `<(` or `>(` behind an even run of backslashes, past the
// part's own `$`.
fn opens(text: &str) -> bool {
    let b = text.strip_prefix('$').unwrap_or(text).as_bytes();
    let mut slashes = 0usize;
    for (i, &c) in b.iter().enumerate() {
        let next = b.get(i + 1).copied().unwrap_or(b' ');
        let opens = (c == b'$' && b"({[".contains(&next))
            || c == b'`'
            || (matches!(c, b'<' | b'>') && next == b'(');
        if opens && slashes.is_multiple_of(2) {
            return true;
        }
        slashes = if c == b'\\' { slashes + 1 } else { 0 };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{fits, opens, placed, tiles};

    #[test]
    fn parts_tile_their_span_or_the_layout_is_not_the_source() {
        type Row<'a> = (&'a [(usize, usize)], usize, usize, bool);
        let rows: &[Row] = &[
            (&[(5, 8), (8, 12)], 5, 12, true),
            (&[(5, 5)], 5, 5, true),
            (&[], 6, 6, true),
            (&[], 5, 6, false),
            (&[(5, 7), (8, 12)], 5, 12, false),
            (&[(5, 7), (7, 11)], 5, 12, false),
            (&[(6, 8), (8, 12)], 5, 12, false),
            (&[(5, 8), (7, 12)], 5, 12, false),
            (&[(5, 3)], 5, 3, false),
        ];
        for (spans, from, to, want) in rows {
            assert_eq!(tiles(spans.iter().copied(), *from, *to), *want, "{spans:?}");
        }
    }

    #[test]
    fn tiling_holds_through_the_last_part_that_runs() {
        type Row<'a> = (&'a [(bool, (usize, usize))], usize, bool);
        let rows: &[Row] = &[
            (&[(true, (5, 8)), (false, (9, 12))], 5, true),
            (&[(false, (5, 7)), (true, (8, 12))], 5, false),
            (&[(true, (6, 8))], 5, false),
            (&[(false, (5, 7)), (false, (9, 12))], 5, true),
            (&[], 5, true),
        ];
        for (parts, from, want) in rows {
            assert_eq!(placed(parts.iter().copied(), *from), *want, "{parts:?}");
        }
    }

    #[test]
    fn a_name_laid_over_an_opener_hides_it() {
        let rows: &[(&str, bool)] = &[
            ("$HOME", false),
            ("${HOME}", false),
            ("\\$HOM", false),
            ("\\${HOME", false),
            ("\\${$(git push", true),
            ("\\${#$(git push", true),
            ("${x`git push`", true),
            ("$x<(git push", true),
            ("$x$[1", true),
            ("$x\\$(y", false),
            ("", false),
        ];
        for (text, want) in rows {
            assert_eq!(opens(text), *want, "{text:?}");
        }
    }

    #[test]
    fn a_part_is_framed_by_its_own_opener_and_closer() {
        let rows: &[(&str, &[&str], &str, bool)] = &[
            ("$(x)", &["$("], ")", true),
            ("/$(git push", &["$("], ")", false),
            ("x$(git push", &["$("], ")", false),
            ("$(", &["$("], ")", false),
            ("`x`", &["`"], "`", true),
            ("`x", &["`"], "`", false),
            ("<(x)", &["<(", ">("], ")", true),
            (">(x)", &["<(", ">("], ")", true),
            ("${x:-y", &["${"], "}", false),
        ];
        for (text, open, close, want) in rows {
            assert_eq!(fits(text, open, close), *want, "{text:?}");
        }
    }
}
