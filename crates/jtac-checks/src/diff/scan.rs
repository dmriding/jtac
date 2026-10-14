//! Each line classified once: blank, plain comment, doc comment, or code with its strings and
//! comments blanked so `span` can count braces and match keywords (DESIGN.md section 10.3).
//! A string's delimiters stay so `("x")` still reads as a call with one argument.

use super::change::split_lines;
use super::lang::Lang;

/// What a line is to the size and comment rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Whitespace only.
    Blank,
    /// A plain comment: counted against code by `JT-CMT-001`.
    Comment,
    /// A doc comment: `///`, `//!`, `/** */`, `/*! */`, or a Go comment block above a declaration.
    Doc,
    /// Anything else, including the lines of a multi-line string.
    Code,
}

/// One line of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line<'a> {
    /// What it is.
    pub kind: Kind,
    /// As written, without its terminator.
    pub text: &'a str,
    /// `text` with string contents and comments removed; empty unless `Code`.
    pub code: String,
    /// The comment on the line, if any, as written.
    pub comment: String,
    /// Whether the line began inside a string or block comment.
    pub continued: bool,
}

impl Line<'_> {
    /// `code` and `comment` together: the line with only its strings blanked.
    pub fn visible(&self) -> String {
        format!("{}{}", self.code, self.comment)
    }
}

/// A token still open at a line end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Clear,
    Block { doc: bool, depth: u32 },
    Str(Quote),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quote {
    Double,
    Single,
    Backtick,
    /// A JS template with the `${ }` depth open at the line end.
    Template(u32),
    Triple(char),
    Raw(usize),
    Regex,
}

/// Every line of `text`. A line-1 shebang is a comment in every language here; `#![` is not one.
pub fn scan(lang: Lang, text: &str) -> Vec<Line<'_>> {
    let mut state = State::Clear;
    let mut lines: Vec<Line> = split_lines(text)
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            if i == 0 && text.starts_with("#!") && !text.starts_with("#![") {
                return Line {
                    kind: Kind::Comment,
                    text,
                    code: String::new(),
                    comment: text.to_owned(),
                    continued: false,
                };
            }
            let mut scanner = Scanner::new(lang, text, state);
            scanner.run();
            state = scanner.state;
            scanner.line()
        })
        .collect();
    if lang == Lang::Go {
        go_doc(&mut lines);
    }
    lines
}

struct Scanner<'a> {
    lang: Lang,
    text: &'a str,
    chars: Vec<char>,
    at: usize,
    state: State,
    code: String,
    comment_text: String,
    has_code: bool,
    comment: Option<bool>,
    continued: bool,
    /// Word characters of a JavaScript string keying a member, as `it['only']`, stay in `code`
    /// so the focus is seen; a string in an array literal, or in any other language, is data.
    keep: bool,
}

impl<'a> Scanner<'a> {
    fn new(lang: Lang, text: &'a str, state: State) -> Self {
        let comment = match state {
            State::Block { doc, .. } => Some(doc),
            _ => None,
        };
        Self {
            lang,
            text,
            chars: text.chars().collect(),
            at: 0,
            state,
            code: String::with_capacity(text.len()),
            comment_text: String::new(),
            has_code: false,
            comment,
            continued: state != State::Clear,
            keep: false,
        }
    }

    fn line(self) -> Line<'a> {
        let kind = match (self.has_code, self.comment) {
            (true, _) => Kind::Code,
            (false, Some(true)) => Kind::Doc,
            (false, Some(false)) => Kind::Comment,
            (false, None) => Kind::Blank,
        };
        let code = if kind == Kind::Code {
            self.code
        } else {
            String::new()
        };
        Line {
            kind,
            text: self.text,
            code,
            comment: self.comment_text,
            continued: self.continued,
        }
    }

    fn rest(&self) -> &[char] {
        &self.chars[self.at..]
    }

    fn starts(&self, s: &str) -> bool {
        let needle: Vec<char> = s.chars().collect();
        self.rest().starts_with(&needle)
    }

    fn run(&mut self) {
        while self.at < self.chars.len() {
            match self.state {
                State::Block { doc, depth } => self.in_block(doc, depth),
                State::Str(quote) => self.in_string(quote),
                State::Clear => {
                    if self.clear() {
                        break;
                    }
                }
            }
        }
        if let State::Str(Quote::Double | Quote::Single | Quote::Regex) = self.state
            && self.lang != Lang::Rust
            && !(self.lang != Lang::Go && self.continues())
        {
            self.state = State::Clear;
        }
    }

    /// Whether the line ends in an odd run of backslashes: a string continued on the next line.
    fn continues(&self) -> bool {
        self.chars.iter().rev().take_while(|&&c| c == '\\').count() % 2 == 1
    }

    fn in_block(&mut self, doc: bool, depth: u32) {
        let (end, depth) = block_end(self.rest(), depth, self.lang == Lang::Rust);
        self.state = match depth {
            0 => State::Clear,
            depth => State::Block { doc, depth },
        };
        let chunk: String = self.chars[self.at..self.at + end].iter().collect();
        self.comment_text.push_str(&chunk);
        self.at += end;
    }

    fn in_string(&mut self, quote: Quote) {
        let rest = self.rest();
        let closer: String = match quote {
            Quote::Double => "\"".into(),
            Quote::Single => "'".into(),
            Quote::Backtick | Quote::Template(_) => "`".into(),
            Quote::Triple(q) => q.to_string().repeat(3),
            Quote::Raw(hashes) => format!("\"{}", "#".repeat(hashes)),
            Quote::Regex => "/".into(),
        };
        let (close, quote) = match quote {
            Quote::Double | Quote::Single => (
                find_unescaped(rest, closer.chars().next().unwrap_or('"')),
                quote,
            ),
            Quote::Template(depth) => {
                let (close, depth) = template_close(rest, depth, 0);
                (close, Quote::Template(depth))
            }
            Quote::Regex => (regex_close(rest), quote),
            _ => (
                find(rest, &closer).map(|i| i + closer.chars().count()),
                quote,
            ),
        };
        let end = close.unwrap_or(rest.len());
        let content = end - close.map_or(0, |_| closer.chars().count());
        let has_content = rest[..content].iter().any(|c| !c.is_whitespace());
        let kept: String = if self.keep {
            rest[..content]
                .iter()
                .map(|&c| {
                    if c.is_alphanumeric() || c == '_' {
                        c
                    } else {
                        ' '
                    }
                })
                .collect()
        } else {
            String::new()
        };
        self.has_code |= has_content;
        self.code.push_str(&kept);
        self.at += end;
        self.state = State::Str(quote);
        if close.is_some() {
            self.has_code = true;
            self.code.push_str(&closer);
            self.state = State::Clear;
        }
    }

    /// One token in code. Returns true when the line is finished.
    fn clear(&mut self) -> bool {
        let c = self.chars[self.at];
        let line_comment = self.lang.line_comment();
        if self.starts(line_comment) {
            let doc = self.lang == Lang::Rust
                && (self.starts("//!") || (self.starts("///") && !self.starts("////")));
            self.note_comment(doc);
            let rest: String = self.rest().iter().collect();
            self.comment_text.push_str(&rest);
            return true;
        }
        if self.lang != Lang::Python && self.starts("/*") {
            let doc = (self.starts("/**") && !self.starts("/**/"))
                || (self.lang == Lang::Rust && self.starts("/*!"));
            self.note_comment(doc);
            self.comment_text.push_str("/*");
            self.at += 2;
            self.state = State::Block { doc, depth: 1 };
            return false;
        }
        if let Some((quote, len)) = self.string_opener() {
            self.keep = self.lang == Lang::Js
                && self
                    .code
                    .trim_end()
                    .strip_suffix('[')
                    .is_some_and(|before| indexes(before.trim_end()));
            self.code.extend(&self.chars[self.at..self.at + len]);
            self.at += len;
            self.state = State::Str(quote);
            return false;
        }
        if !c.is_whitespace() {
            self.has_code = true;
        }
        self.code.push(c);
        self.at += 1;
        false
    }

    fn note_comment(&mut self, doc: bool) {
        if self.comment.is_none() {
            self.comment = Some(doc);
        }
    }

    fn string_opener(&self) -> Option<(Quote, usize)> {
        let c = self.chars[self.at];
        let next = self.chars.get(self.at + 1).copied();
        match (self.lang, c) {
            (Lang::Python, '"' | '\'') if self.starts(&c.to_string().repeat(3)) => {
                Some((Quote::Triple(c), 3))
            }
            (_, '"') => Some((Quote::Double, 1)),
            (Lang::Python | Lang::Js | Lang::Go, '\'') => Some((Quote::Single, 1)),
            (Lang::Rust, '\'') => rust_char(self.rest()).map(|_| (Quote::Single, 1)),
            (Lang::Js, '`') => Some((Quote::Template(0), 1)),
            (Lang::Go, '`') => Some((Quote::Backtick, 1)),
            (Lang::Js, '/') if self.regex_allowed() && regex_close(&self.rest()[1..]).is_some() => {
                Some((Quote::Regex, 1))
            }
            (Lang::Rust, 'r') if !self.after_ident() => {
                let hashes = self.rest()[1..].iter().take_while(|&&h| h == '#').count();
                (next.is_some() && self.chars.get(self.at + 1 + hashes) == Some(&'"'))
                    .then_some((Quote::Raw(hashes), hashes + 2))
            }
            _ => None,
        }
    }

    /// Whether a `/` here starts a regex literal rather than a division: what precedes it is
    /// nothing, an operator, an opening bracket, or a keyword that takes an expression.
    fn regex_allowed(&self) -> bool {
        let code = self.code.trim_end();
        let Some(last) = code.chars().last() else {
            return true;
        };
        if code.ends_with("++") || code.ends_with("--") {
            return false;
        }
        if "(,=:[!&|?{};+-*%<>~^".contains(last) {
            return true;
        }
        let word: String = code
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        [
            "return", "typeof", "case", "do", "else", "in", "of", "throw", "new", "delete", "void",
            "yield", "await",
        ]
        .contains(&word.as_str())
    }

    /// Whether the char before `at` continues an identifier, allowing the `b`/`c` string prefix.
    fn after_ident(&self) -> bool {
        let ident = |c: char| c.is_alphanumeric() || c == '_';
        match self.at.checked_sub(1).map(|i| self.chars[i]) {
            None => false,
            Some('b' | 'c') => self.at.checked_sub(2).is_some_and(|i| ident(self.chars[i])),
            Some(c) => ident(c),
        }
    }
}

/// Whether `'` opens a char literal rather than a lifetime: `'x'`, `'\n'`, `'\u{1F600}'`.
fn rust_char(rest: &[char]) -> Option<usize> {
    match rest.get(1)? {
        '\\' => find_unescaped(&rest[1..], '\'').map(|i| i + 1),
        _ => (rest.get(2) == Some(&'\'')).then_some(3),
    }
}

/// Templates a line may nest before a backtick reads as text, so a crafted line cannot
/// overflow the stack.
const NEST: u32 = 32;

/// Index just past the backtick closing a JS template, and the `${ }` depth left open. In the
/// expression braces count and a string, comment, regex (a `/` after an operator or opener
/// that closes on the line) or template on the same line is skipped whole; one left open at
/// the line end still skews the depth.
fn template_close(hay: &[char], mut depth: u32, nested: u32) -> (Option<usize>, u32) {
    let mut i = 0;
    let mut last = ' ';
    while i < hay.len() {
        let rest = hay.len() - i - 1;
        match (hay[i], depth) {
            ('\\', 0) => i += 1,
            ('`', 0) => return (Some(i + 1), 0),
            ('$', 0) if hay.get(i + 1) == Some(&'{') => {
                depth = 1;
                i += 1;
            }
            ('{', 1..) => depth += 1,
            ('}', 1..) => depth -= 1,
            (q @ ('"' | '\''), 1..) => i += find_unescaped(&hay[i + 1..], q).unwrap_or(rest),
            ('`', 1..) if nested < NEST => {
                i += template_close(&hay[i + 1..], 0, nested + 1)
                    .0
                    .unwrap_or(rest);
            }
            ('/', 1..) if hay.get(i + 1) == Some(&'/') => return (None, depth),
            ('/', 1..) if hay.get(i + 1) == Some(&'*') => {
                i += find(&hay[i + 2..], "*/").map_or(rest, |j| j + 3);
            }
            ('/', 1..) if "(,=:[!&|?{};+-*%<>~^".contains(last) => {
                i += regex_close(&hay[i + 1..]).unwrap_or(0);
            }
            _ => {}
        }
        if let Some(&c) = hay.get(i).filter(|c| !c.is_whitespace()) {
            last = c;
        }
        i += 1;
    }
    (None, depth)
}

/// Chars to the end of a block comment or of the line, and the depth left open; `/*` nests
/// only when `nests`, as in Rust.
fn block_end(hay: &[char], mut depth: u32, nests: bool) -> (usize, u32) {
    let mut i = 0;
    while i < hay.len() && depth > 0 {
        if hay[i..].starts_with(&['*', '/']) {
            depth -= 1;
            i += 2;
        } else if nests && hay[i..].starts_with(&['/', '*']) {
            depth += 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    (i, depth)
}

/// Index just past the `/` closing a regex literal, outside any `[...]` class.
fn regex_close(hay: &[char]) -> Option<usize> {
    let mut i = 0;
    let mut class = false;
    while i < hay.len() {
        match hay[i] {
            '\\' => i += 1,
            '[' => class = true,
            ']' => class = false,
            '/' if !class => return Some(i + 1),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Keywords an array literal may follow, as `return [` does.
const ARRAY_AFTER: &[&str] = &[
    "return",
    "yield",
    "in",
    "of",
    "typeof",
    "case",
    "await",
    "throw",
    "delete",
    "void",
    "new",
    "instanceof",
    "else",
    "do",
    "export",
    "default",
    "as",
    "satisfies",
    "keyof",
    "readonly",
    "extends",
];

/// Whether `code` ends in an identifier a `[` would index.
fn indexes(code: &str) -> bool {
    let head = code.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    let name = &code[head.len()..];
    !name.is_empty() && !ARRAY_AFTER.contains(&name)
}

fn find(hay: &[char], needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    hay.windows(needle.len())
        .position(|w| w == needle.as_slice())
}

/// Index just past the first unescaped `quote`.
fn find_unescaped(hay: &[char], quote: char) -> Option<usize> {
    let mut i = 0;
    while i < hay.len() {
        match hay[i] {
            '\\' => i += 2,
            c if c == quote => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// A `//` block at column 0 directly above a top-level declaration is Go's doc comment.
fn go_doc(lines: &mut [Line]) {
    let decl = |code: &str| {
        ["func ", "type ", "var ", "const ", "package "]
            .iter()
            .any(|k| code.starts_with(k))
    };
    let mut run = 0;
    for i in 0..lines.len() {
        let plain = lines[i].kind == Kind::Comment && lines[i].text.starts_with("//");
        if plain {
            run += 1;
        } else {
            if lines[i].kind == Kind::Code && decl(&lines[i].code) {
                for line in &mut lines[i - run..i] {
                    line.kind = Kind::Doc;
                }
            }
            run = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: Lang, text: &str) -> Vec<Kind> {
        scan(lang, text).into_iter().map(|l| l.kind).collect()
    }

    fn code(lang: Lang, text: &str) -> Vec<String> {
        scan(lang, text).into_iter().map(|l| l.code).collect()
    }

    #[test]
    fn comment_kinds_per_language() {
        use Kind::*;
        let rows: &[(Lang, &str, &[Kind])] = &[
            (
                Lang::Rust,
                "// a\n/// b\n//! c\n//// d\n\nx",
                &[Comment, Doc, Doc, Comment, Blank, Code],
            ),
            (
                Lang::Rust,
                "/* a\n b */\n/** c\n */ x\n/*! d */",
                &[Comment, Comment, Doc, Code, Doc],
            ),
            (Lang::Rust, "x /* a\n still\n */ y", &[Code, Comment, Code]),
            (Lang::Rust, "/**/ x", &[Code]),
            (Lang::Rust, "/**/", &[Comment]),
            (Lang::Rust, "/* /*\n */ a\n */ b", &[Comment, Comment, Code]),
            (Lang::Js, "/* /* */ b", &[Code]),
            (Lang::Go, "/* /* */ x", &[Code]),
            (Lang::Js, "/// a\n/** b */\n// c", &[Comment, Doc, Comment]),
            (
                Lang::Python,
                "# a\n\"\"\"doc\"\"\"\nx = 1  # b\n// c",
                &[Comment, Code, Code, Code],
            ),
            (
                Lang::Go,
                "// Doc.\n// More.\nfunc f() {}\n// a\n\nfunc g() {}\n// b\nx := 1\n  // c\ntype T int",
                &[
                    Doc, Doc, Code, Comment, Blank, Code, Comment, Code, Comment, Code,
                ],
            ),
            (Lang::Go, "/* a */\ntype T int", &[Comment, Code]),
        ];
        for (lang, text, want) in rows {
            assert_eq!(kinds(*lang, text), *want, "{lang:?}: {text:?}");
        }
    }

    #[test]
    fn strings_are_blanked_and_braces_inside_them_vanish() {
        let rows: &[(Lang, &str, &[&str])] = &[
            (Lang::Rust, "let s = \"a{b}\"; // {", &["let s = \"\"; "]),
            (
                Lang::Js,
                "it['only']('x', f); m[ \"k{\" ] = 1; s = '{'",
                &["it['only']('', f); m[ \"k \" ] = 1; s = ''"],
            ),
            (Lang::Python, "print([\"assert\"])", &["print([\"\"])"]),
            (
                Lang::Js,
                "const titles = [\"it only\"]; f(['it only']); m['k'] = 1",
                &["const titles = [\"\"]; f(['']); m['k'] = 1"],
            ),
            (
                Lang::Js,
                "it ['only']('x'); return ['it only']; x = y['k']",
                &["it ['only'](''); return ['']; x = y['k']"],
            ),
            (
                Lang::Js,
                "const s = x as ['it only']; it[`only`]('x')",
                &["const s = x as ['']; it[`only`]('')"],
            ),
            (
                Lang::Js,
                "const t = `a\\`b`; x('{')\nconst u = `\\\\`; it('{')",
                &["const t = ``; x('')", "const u = ``; it('')"],
            ),
            (Lang::Go, "s := `C:\\`\nx := 1", &["s := ``", "x := 1"]),
            (
                Lang::Rust,
                "let v = [\"assert!(x)\"];",
                &["let v = [\"\"];"],
            ),
            (
                Lang::Js,
                "x = i++ / 2; y = a / b / c; z = /[/]{/; w = r / 2",
                &["x = i++ / 2; y = a / b / c; z = //; w = r / 2"],
            ),
            (
                Lang::Rust,
                "let c = '{'; let l: &'a str = \"}\";",
                &["let c = ''; let l: &'a str = \"\";"],
            ),
            (
                Lang::Rust,
                "let e = '\\''; f(b'{')",
                &["let e = ''; f(b'')"],
            ),
            (
                Lang::Rust,
                "let r = r#\"{\n}\"#; x",
                &["let r = r#\"", "\"#; x"],
            ),
            (
                Lang::Rust,
                "let r = br\"{\"; for r in v {}",
                &["let r = br\"\"; for r in v {}"],
            ),
            (
                Lang::Rust,
                "let s = \"multi\nline\"; {",
                &["let s = \"", "\"; {"],
            ),
            (
                Lang::Js,
                "const t = `a${b}\n}`; x('{') + \"}\"",
                &["const t = `", "`; x('') + \"\""],
            ),
            (
                Lang::Js,
                "const t = `${\n`}`}`; it('{')",
                &["const t = `", "`; it('')"],
            ),
            (
                Lang::Go,
                "s := `raw\n{`\nr := '{'",
                &["s := `", "`", "r := ''"],
            ),
            (
                Lang::Python,
                "s = '''a\n{\n'''\nt = \"}\" # {",
                &["s = '''", "", "'''", "t = \"\" "],
            ),
            (
                Lang::Python,
                "x = 1 if y else \"'\"",
                &["x = 1 if y else \"\""],
            ),
        ];
        for (lang, text, want) in rows {
            assert_eq!(code(*lang, text), *want, "{lang:?}: {text:?}");
        }
    }

    #[test]
    fn a_line_inside_a_string_is_code_and_inside_a_block_comment_is_comment() {
        assert_eq!(
            kinds(Lang::Python, "s = '''\n\nend'''"),
            [Kind::Code, Kind::Blank, Kind::Code]
        );
        assert_eq!(
            kinds(Lang::Rust, "/*\n\n*/"),
            [Kind::Comment, Kind::Comment, Kind::Comment]
        );
    }

    #[test]
    fn visible_keeps_the_comment_and_blanks_the_string() {
        let rows: &[(Lang, &str, &[&str])] = &[
            (
                Lang::Rust,
                "x(\"# noqa\"); // #[allow(a)]",
                &["x(\"\"); // #[allow(a)]"],
            ),
            (
                Lang::Python,
                "x = 1  # noqa: E501",
                &["x = 1  # noqa: E501"],
            ),
            (
                Lang::Js,
                "/* eslint-disable\n */ y",
                &["/* eslint-disable", " y */"],
            ),
            (Lang::Rust, "// only", &["// only"]),
            (
                Lang::Js,
                "const s = `${x /* ` */}`;\nit.only('b', f);",
                &["const s = ``;", "it.only('', f);"],
            ),
            (Lang::Rust, "/* /* */ { */ x", &[" x/* /* */ { */"]),
            (
                Lang::Js,
                "const t = `${1 /* { */}`;\nit.only('a', f);",
                &["const t = ``;", "it.only('', f);"],
            ),
            (
                Lang::Js,
                "const t = `${'}' + \"`\" + `}`}`;\nit.only('a', f);",
                &["const t = ``;", "it.only('', f);"],
            ),
            (
                Lang::Js,
                "const t = `${x // }`\n}`; it.only('a', f)",
                &["const t = `", "`; it.only('', f)"],
            ),
            (
                Lang::Js,
                "const t = `${s.replace(/\"/g, '&quot;')}`;\nit.only('a', f);",
                &["const t = ``;", "it.only('', f);"],
            ),
            (
                Lang::Js,
                "const t = `${u.replace(/^https?:\\/\\//, \"\")}`;\nit.only('a', f);",
                &["const t = ``;", "it.only('', f);"],
            ),
            (
                Lang::Js,
                "const t = `${a / b}`;\nit.only('a', f);",
                &["const t = ``;", "it.only('', f);"],
            ),
        ];
        for (lang, text, want) in rows {
            let got: Vec<String> = scan(*lang, text).iter().map(Line::visible).collect();
            assert_eq!(got, *want, "{lang:?}: {text:?}");
        }
    }

    #[test]
    fn templates_nested_past_the_cap_read_as_text_without_recursing() {
        let deep = format!("x = `{}", "${`".repeat(20_000));
        assert_eq!(code(Lang::Js, &deep), ["x = `"]);
    }

    #[test]
    fn an_unterminated_quote_ends_with_the_line_outside_rust() {
        assert_eq!(kinds(Lang::Js, "x = '\ny = 1"), [Kind::Code, Kind::Code]);
        assert_eq!(code(Lang::Js, "x = '\ny = 1"), ["x = '", "y = 1"]);
    }
}
