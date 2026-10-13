//! Policy text echoed into an error, made safe for a terminal or a model to read: hidden
//! characters dropped and length capped, so a hostile file cannot reorder, hide, or flood the
//! message. Category Cf is Unicode 16.0, checked against the UCD when written.

use std::ops::RangeInclusive;

/// Longest echoed value, in chars, before the `…` marker.
pub(crate) const MAX_ECHO_CHARS: usize = 64;

/// Unicode general category Cf (format), which includes the bidi overrides and isolates.
const FORMAT: &[RangeInclusive<char>] = &[
    '\u{ad}'..='\u{ad}',
    '\u{600}'..='\u{605}',
    '\u{61c}'..='\u{61c}',
    '\u{6dd}'..='\u{6dd}',
    '\u{70f}'..='\u{70f}',
    '\u{890}'..='\u{891}',
    '\u{8e2}'..='\u{8e2}',
    '\u{180e}'..='\u{180e}',
    '\u{200b}'..='\u{200f}',
    '\u{202a}'..='\u{202e}',
    '\u{2060}'..='\u{2064}',
    '\u{2066}'..='\u{206f}',
    '\u{feff}'..='\u{feff}',
    '\u{fff9}'..='\u{fffb}',
    '\u{110bd}'..='\u{110bd}',
    '\u{110cd}'..='\u{110cd}',
    '\u{13430}'..='\u{1343f}',
    '\u{1bca0}'..='\u{1bca3}',
    '\u{1d173}'..='\u{1d17a}',
    '\u{e0001}'..='\u{e0001}',
    '\u{e0020}'..='\u{e007f}',
];

/// Whether `c` is invisible or breaks the line: Cc, Cf, and the line and paragraph separators.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}')
        || FORMAT.iter().any(|range| range.contains(&c))
}

/// `raw` without hidden characters, at most `MAX_ECHO_CHARS` of them, `…` marking a cut.
pub(crate) fn shown(raw: &str) -> String {
    let mut kept = raw.chars().filter(|c| !hidden(*c));
    let mut out: String = kept.by_ref().take(MAX_ECHO_CHARS).collect();
    if kept.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_char_table() {
        let table = [
            ('\u{202e}', true),
            ('\u{202a}', true),
            ('\u{2066}', true),
            ('\u{2069}', true),
            ('\u{200b}', true),
            ('\u{200f}', true),
            ('\u{feff}', true),
            ('\u{ad}', true),
            ('\u{e0041}', true),
            ('\u{85}', true),
            ('\u{2028}', true),
            ('\u{2029}', true),
            ('\u{1b}', true),
            ('\n', true),
            ('\r', true),
            ('\t', true),
            ('\0', true),
            ('a', false),
            ('é', false),
            (' ', false),
            ('…', false),
            ('\u{2065}', false),
        ];
        for (c, want) in table {
            assert_eq!(hidden(c), want, "U+{:04X}", u32::from(c));
        }
    }

    #[test]
    fn shown_caps_at_the_limit_and_marks_the_cut() {
        let at_limit = "x".repeat(MAX_ECHO_CHARS);
        assert_eq!(shown(&at_limit), at_limit);
        let over = "é".repeat(MAX_ECHO_CHARS + 1);
        assert_eq!(shown(&over), format!("{}…", "é".repeat(MAX_ECHO_CHARS)));
        assert_eq!(shown("a\u{202e}b\nc"), "abc");
    }
}
