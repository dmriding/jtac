//! A continuation inside a comment, found in the text. The parser lists only some comments
//! (a compound array with a keyed element skips its own, `x=( [0]=a # c \<nl>[1]=$(x) )`),
//! so the rule reads the joined source: a cut outside every span the walk read sits in
//! whitespace or in a comment, and a `#` before it on its line, outside every read span,
//! opens the comment (`a_cut_after_an_unlisted_comment_opener_is_in_a_comment`). A `$(…)`
//! or `<(…)` inside a word is `open` again, a script of its own whose words the walk read
//! one by one; the innermost span holding a byte says which it is, open when the two are one
//! span (a word that is its substitution).

/// True when a cut in `cuts` lies outside every span in `seen` and a `#` outside every span
/// in `seen` precedes it on its line of `source`, a byte being inside when the innermost of
/// `seen` and `open` holding it is a `seen` span.
pub(crate) fn commented(
    source: &str,
    cuts: &[usize],
    seen: &[(usize, usize)],
    open: &[(usize, usize)],
) -> bool {
    let inside = |i: usize| {
        let open = open.iter().map(|&s| (s, false));
        open.chain(seen.iter().map(|&s| (s, true)))
            .filter(|&((from, to), _)| from <= i && i < to)
            .min_by_key(|&((from, to), _)| to - from)
            .is_some_and(|(_, seen)| seen)
    };
    cuts.iter().any(|&c| {
        let line = source[..c].rfind('\n').map_or(0, |n| n + 1);
        !inside(c)
            && source[line..c]
                .bytes()
                .enumerate()
                .any(|(i, b)| b == b'#' && !inside(line + i))
    })
}

#[cfg(test)]
mod tests {
    use super::commented;

    #[test]
    fn a_cut_after_an_unlisted_comment_opener_is_in_a_comment() {
        type Row<'a> = (
            &'a str,
            &'a [usize],
            &'a [(usize, usize)],
            &'a [(usize, usize)],
            bool,
        );
        let rows: &[Row] = &[
            (
                "x=( [0]=a # c [1]=b\n)",
                &[14],
                &[(0, 2), (4, 9)],
                &[],
                true,
            ),
            ("git push # c\n", &[12], &[(0, 3), (4, 8)], &[], true),
            (
                "#!/bin/sh\ngit push",
                &[10],
                &[(10, 13), (14, 18)],
                &[],
                false,
            ),
            ("#!/bin/sh git push", &[10], &[], &[], true),
            ("git push", &[4], &[(0, 3), (4, 8)], &[], false),
            ("echo a#b c", &[9], &[(0, 4), (5, 8), (9, 10)], &[], false),
            ("echo '#' c", &[9], &[(0, 4), (5, 8), (9, 10)], &[], false),
            ("`# c git push`", &[5], &[(0, 14)], &[], false),
            ("x=$( # c git push)", &[9], &[(0, 18)], &[(2, 18)], true),
            (
                "echo $(# c git push)",
                &[9],
                &[(0, 4), (5, 20)],
                &[(5, 20)],
                true,
            ),
            (
                "x=$(a#b git push)",
                &[8],
                &[(0, 17), (4, 7), (8, 11), (12, 16)],
                &[(2, 17)],
                false,
            ),
            ("git push", &[], &[], &[], false),
        ];
        for (source, cuts, seen, open, want) in rows {
            assert_eq!(commented(source, cuts, seen, open), *want, "{source:?}");
        }
    }
}
