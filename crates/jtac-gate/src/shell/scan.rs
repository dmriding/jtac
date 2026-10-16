//! What the text says before the parser sees it: how deep its brackets go and how long an
//! arithmetic body runs, because the parser recurses on the one and is quadratic in the other.

pub(crate) const MAX_SUBSHELLS: u32 = 16;

/// An arithmetic, subscript or `${…}` body longer than this is refused: shuck's arithmetic
/// parser is quadratic, and 60 KiB of `1+1+…` takes it half a minute in a debug build.
const MAX_ARITH: usize = 4096;

/// A bracket the pre-parse scan has open. `Paren` and `Brace` shield what they hold from the
/// body cap, which applies to the other three.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Paren,
    Arith,
    Square,
    Brace,
    Dollar,
}

/// Where the scan is in the text: a heredoc body (the delimiter, and whether quoting turned
/// expansion off) or plain script.
struct Scan {
    depth: u32,
    in_backtick: bool,
    open: Vec<(Open, usize)>,
    pending: Vec<(String, bool)>,
    heredoc: Option<(String, bool)>,
    line_start: usize,
}

// Brackets are counted before the parser runs because nested `$(` and `a[a[…]]` cost it far
// more stack per level than any other shape. Single quotes, comments and quoted heredoc bodies
// are text the parser never recurses on and are skipped; an unquoted heredoc body expands, so
// only its `$(`, `${` and backticks count. Backticks cannot nest without an escape, which
// `parse` refuses, so a pair is one level and a stray is off by one at most.
pub(crate) fn too_deep_or_long(script: &str) -> Option<String> {
    let b = script.as_bytes();
    let mut s = Scan {
        depth: 0,
        in_backtick: false,
        open: Vec::new(),
        pending: Vec::new(),
        heredoc: None,
        line_start: 0,
    };
    let mut i = 0;
    let mut prev = b' ';
    while i < b.len() {
        let c = b[i];
        let expands = matches!(c, b'(' | b')' | b'`' | b'}') || (c == b'{' && prev == b'$');
        let text = s
            .heredoc
            .as_ref()
            .is_some_and(|(_, quoted)| *quoted || !expands);
        if c == b'\n' {
            s.newline(b, i);
        } else if text {
            // a heredoc body: nothing here reaches the parser as syntax
        } else if c == b'\'' {
            i = b[i + 1..]
                .iter()
                .position(|&x| x == b'\'')
                .map_or(b.len(), |n| i + 1 + n);
        } else if c == b'#' && matches!(prev, b' ' | b'\t' | b';' | b'\n') {
            i = b[i..]
                .iter()
                .position(|&x| x == b'\n')
                .map_or(b.len(), |n| i + n)
                - 1;
        } else if c == b'<'
            && prev != b'<'
            && b.get(i + 1) == Some(&b'<')
            && b.get(i + 2) != Some(&b'<')
        {
            i = s.heredoc_delimiter(b, i + 2);
        } else {
            s.bracket(c, prev, i);
        }
        prev = b.get(i).copied().unwrap_or(b' ');
        i += 1;
        if s.depth > MAX_SUBSHELLS {
            return Some(format!(
                "it nests brackets or substitutions more than {MAX_SUBSHELLS} deep"
            ));
        }
        if s.open
            .last()
            .is_some_and(|(k, at)| !matches!(k, Open::Paren | Open::Brace) && i - at > MAX_ARITH)
        {
            return Some(format!(
                "it holds an arithmetic, subscript or parameter body longer than {MAX_ARITH} characters"
            ));
        }
    }
    None
}

impl Scan {
    fn bracket(&mut self, c: u8, prev: u8, at: usize) {
        match c {
            b'(' => {
                self.depth += 1;
                let kind = if prev == b'(' {
                    Open::Arith
                } else {
                    Open::Paren
                };
                self.open.push((kind, at));
            }
            b'[' => {
                self.depth += 1;
                self.open.push((Open::Square, at));
            }
            b'{' => {
                self.depth += 1;
                let kind = if prev == b'$' {
                    Open::Dollar
                } else {
                    Open::Brace
                };
                self.open.push((kind, at));
            }
            b')' | b']' | b'}' => {
                self.depth = self.depth.saturating_sub(1);
                self.open.pop();
            }
            b'`' => {
                self.depth = if self.in_backtick {
                    self.depth.saturating_sub(1)
                } else {
                    self.depth + 1
                };
                self.in_backtick = !self.in_backtick;
            }
            _ => {}
        }
    }

    // `<<WORD`, `<<-'WORD'`: the body starts on the next line and ends at a line that is WORD.
    fn heredoc_delimiter(&mut self, b: &[u8], mut i: usize) -> usize {
        if b.get(i) == Some(&b'-') {
            i += 1;
        }
        while matches!(b.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        let start = i;
        while b
            .get(i)
            .is_some_and(|x| !x.is_ascii_whitespace() && !b";|&<>()".contains(x))
        {
            i += 1;
        }
        let raw = &b[start..i];
        let quoted = raw.iter().any(|x| matches!(x, b'\'' | b'"' | b'\\'));
        let word: Vec<u8> = raw
            .iter()
            .copied()
            .filter(|x| !matches!(x, b'\'' | b'"' | b'\\'))
            .collect();
        if !word.is_empty() {
            self.pending
                .push((String::from_utf8_lossy(&word).into_owned(), quoted));
        }
        i.saturating_sub(1)
    }

    fn newline(&mut self, b: &[u8], at: usize) {
        let line = &b[self.line_start..at];
        let line = line.strip_prefix(b"\t").unwrap_or(line);
        let line = line
            .iter()
            .skip_while(|x| **x == b'\t')
            .copied()
            .collect::<Vec<u8>>();
        if self
            .heredoc
            .as_ref()
            .is_some_and(|(d, _)| d.as_bytes() == line.as_slice())
        {
            self.heredoc = None;
        }
        if self.heredoc.is_none() && !self.pending.is_empty() {
            self.heredoc = Some(self.pending.remove(0));
        }
        self.line_start = at + 1;
    }
}
