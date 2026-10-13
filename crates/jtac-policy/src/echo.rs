//! Policy text echoed into an error. Every char outside printable ASCII, and `\` itself, is
//! written as `\u{XXXX}`: nothing is dropped, so nothing hides or reorders the message and two
//! inputs never render the same below the cap. No Unicode table lives here, by design.

/// Longest echo, in chars, before the `…` marker. Room for toml's expected-key lists.
pub(crate) const MAX_ECHO_CHARS: usize = 256;

/// `raw` escaped to printable ASCII, cut at a whole char past `MAX_ECHO_CHARS` with `…`.
pub(crate) fn shown(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        let piece = if c == '\\' || !(' '..='~').contains(&c) {
            format!("\\u{{{:04X}}}", u32::from(c))
        } else {
            c.to_string()
        };
        if out.len() + piece.len() > MAX_ECHO_CHARS {
            out.push('…');
            break;
        }
        out.push_str(&piece);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_chars_are_escaped_not_dropped() {
        let table = [
            ("\u{202e}", r"\u{202E}"),
            ("\u{202a}", r"\u{202A}"),
            ("\u{2066}", r"\u{2066}"),
            ("\u{2069}", r"\u{2069}"),
            ("\u{200b}", r"\u{200B}"),
            ("\u{200f}", r"\u{200F}"),
            ("\u{feff}", r"\u{FEFF}"),
            ("\u{fe0f}", r"\u{FE0F}"),
            ("\u{e0100}", r"\u{E0100}"),
            ("\u{34f}", r"\u{034F}"),
            ("\u{3164}", r"\u{3164}"),
            ("\u{ad}", r"\u{00AD}"),
            ("\u{e0041}", r"\u{E0041}"),
            ("\u{85}", r"\u{0085}"),
            ("\u{2028}", r"\u{2028}"),
            ("\u{2029}", r"\u{2029}"),
            ("\u{1b}[2J", r"\u{001B}[2J"),
            ("a\nb", r"a\u{000A}b"),
            ("\r\t\0", r"\u{000D}\u{0009}\u{0000}"),
            (r"\u{202E}", r"\u{005C}u{202E}"),
            ("…", r"\u{2026}"),
            ("é", r"\u{00E9}"),
            ("a b~", "a b~"),
        ];
        for (raw, want) in table {
            assert_eq!(shown(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn shown_caps_at_the_limit_and_marks_the_cut() {
        let at_limit = "x".repeat(MAX_ECHO_CHARS);
        assert_eq!(shown(&at_limit), at_limit);
        let over = "x".repeat(MAX_ECHO_CHARS + 1);
        assert_eq!(shown(&over), format!("{at_limit}…"));
        let escape_at_edge = format!("{}é", "x".repeat(MAX_ECHO_CHARS - 6));
        assert_eq!(
            shown(&escape_at_edge),
            format!("{}…", "x".repeat(MAX_ECHO_CHARS - 6))
        );
    }
}
