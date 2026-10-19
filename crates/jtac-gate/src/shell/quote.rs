//! Quote removal for text bash reads a second time: the shell's own decoding of `$'…'`
//! escapes and backslashes, so a `$(` that quoting split or hid shows up joined.

/// `\x24`, `\u0024`, `\U00000024`, `\044` decode to their character; `\\`, `\'` and `\"`
/// drop the backslash; anything else is kept as written.
pub(crate) fn ansi_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('\\') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let (radix, digits, max) = match tail.as_bytes().first() {
            Some(b'x') => (16, &tail[1..], 2),
            Some(b'u') => (16, &tail[1..], 4),
            Some(b'U') => (16, &tail[1..], 8),
            Some(b'0'..=b'7') => (8, tail, 3),
            Some(b'\\' | b'\'' | b'"') => {
                out.push_str(&tail[..1]);
                rest = &tail[1..];
                continue;
            }
            _ => {
                out.push('\\');
                rest = tail;
                continue;
            }
        };
        let run: String = digits
            .chars()
            .take(max)
            .take_while(|c| c.is_digit(radix))
            .collect();
        match u32::from_str_radix(&run, radix)
            .ok()
            .and_then(char::from_u32)
        {
            Some(c) => out.push(c),
            None => out.push('\\'),
        }
        rest = &digits[run.len()..];
    }
    out.push_str(rest);
    out
}

/// A backslash quotes the character after it.
pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            chars.next().unwrap_or('\\')
        } else {
            c
        });
    }
    out
}

/// Does decoded text spell a substitution bash would run on a second read?
pub(crate) fn spells_a_substitution(decoded: &str) -> bool {
    decoded.contains("$(") || decoded.contains('`')
}
