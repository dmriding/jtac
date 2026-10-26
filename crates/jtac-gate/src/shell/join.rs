//! Line continuations joined once, before the parser sees the text. bash joins `\<newline>`
//! everywhere but inside `'…'`, where it is text, a comment, where the next line runs
//! anyway, and a quoted heredoc body, where it stays; the walk refuses the last two where the
//! parser places them (`cut_within`). Joining first means the parser never sees a split `$`,
//! `{`, keyword, operator or delimiter line, which it read other than bash does.

/// `script` with every `\<newline>` behind an even run of backslashes removed, and the
/// offsets in the result where one was removed, ascending.
pub(crate) fn join_continuations(script: &str) -> (String, Vec<usize>) {
    let b = script.as_bytes();
    let mut out = String::with_capacity(b.len());
    let mut cuts = Vec::new();
    let (mut slashes, mut last, mut i) = (0usize, 0, 0);
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1) == Some(&b'\n') && slashes.is_multiple_of(2) {
            out.push_str(&script[last..i]);
            cuts.push(out.len());
            last = i + 2;
            i += 2;
            continue;
        }
        slashes = if b[i] == b'\\' { slashes + 1 } else { 0 };
        i += 1;
    }
    out.push_str(&script[last..]);
    (out, cuts)
}

/// The original offsets of a span of the joined text: each cut at or before the start, and
/// each strictly before the end, stood two bytes earlier. A `'…'` keeps its continuation
/// in bash, so a word holding one is read from the original (`every_row…`, the `bash -c`
/// rows).
pub(crate) fn original_span(cuts: &[usize], from: usize, to: usize) -> (usize, usize) {
    let before = |limit: usize| 2 * cuts.partition_point(|&c| c < limit);
    (from + before(from + 1), to + before(to))
}

/// True when a continuation was removed between `from` and `to`, ends included.
pub(crate) fn cut_within(cuts: &[usize], from: usize, to: usize) -> bool {
    let first = cuts.partition_point(|&c| c < from);
    cuts.get(first).is_some_and(|&c| c <= to)
}

#[cfg(test)]
mod tests {
    use super::{cut_within, join_continuations, original_span};

    #[test]
    fn a_continuation_behind_an_even_run_of_backslashes_is_joined_and_its_place_kept() {
        let rows: &[(&str, &str, &[usize])] = &[
            ("gi\\\nt push", "git push", &[2]),
            ("a\\\n\\\nb", "ab", &[1, 1]),
            ("a\\\\\nb", "a\\\\\nb", &[]),
            ("a\\\\\\\nb", "a\\\\b", &[3]),
            ("\\\n", "", &[0]),
            ("a\\", "a\\", &[]),
            ("a\\\r\nb", "a\\\r\nb", &[]),
            ("é\\\né", "éé", &[2]),
            ("", "", &[]),
        ];
        for (script, want, cuts) in rows {
            assert_eq!(
                join_continuations(script),
                (want.to_string(), cuts.to_vec()),
                "{script:?}"
            );
        }
    }

    #[test]
    fn a_joined_span_maps_back_to_the_original_text() {
        let script = "a\\\nb 'c\\\nd' \\\ne";
        let (joined, cuts) = join_continuations(script);
        assert_eq!(joined, "ab 'cd' e");
        let back = |from: usize, to: usize| {
            let (a, b) = original_span(&cuts, from, to);
            &script[a..b]
        };
        assert_eq!(back(0, 2), "a\\\nb");
        assert_eq!(back(3, 7), "'c\\\nd'");
        assert_eq!(back(4, 6), "c\\\nd");
        assert_eq!(back(8, 9), "e");
        assert_eq!(back(7, 8), " ");
        assert_eq!(back(0, 0), "");
    }

    #[test]
    fn a_cut_is_found_between_two_offsets_ends_included() {
        let cuts = [2, 7, 7, 9];
        assert!(cut_within(&cuts, 0, 2));
        assert!(cut_within(&cuts, 2, 2));
        assert!(cut_within(&cuts, 3, 7));
        assert!(cut_within(&cuts, 8, 20));
        assert!(!cut_within(&cuts, 3, 6));
        assert!(!cut_within(&cuts, 10, 20));
        assert!(!cut_within(&[], 0, 20));
    }
}
