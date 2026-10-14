//! Function spans and brace depths with no parser: braces over scanned code, indentation for
//! Python (DESIGN.md section 10.3). A brace in a string or comment is already blanked, so what
//! defeats this is a signature or a call split across lines; `functions_per_language` rows the
//! shapes that hold.

use regex::Regex;
use std::sync::LazyLock;

use super::lang::Lang;
use super::scan::{Kind, Line};

/// Brace depth around one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Depth {
    /// Before the line.
    pub before: u32,
    /// Lowest point inside it.
    pub min: u32,
    /// After it.
    pub after: u32,
    /// Whether it has a `{` at all: a one-line item opens and closes.
    pub opens: bool,
}

impl Depth {
    /// Whether the line opens a block, closed on the same line or not.
    pub fn opens(self) -> bool {
        self.opens
    }

    /// Whether the line leaves open a block it did not enter with.
    pub fn leaves_open(self) -> bool {
        self.after > self.min
    }

    /// Whether the line closes a block it entered with.
    pub fn closes(self) -> bool {
        self.min < self.before
    }
}

/// A function or test body, by line index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// The function's name, or a test's title.
    pub name: String,
    /// Index of the line that names it.
    pub start: usize,
    /// Index of its last line, inclusive.
    pub end: usize,
    /// Whether the language marks it as a test.
    pub test: bool,
}

impl Span {
    /// Lines from the name to the close, inclusive.
    pub fn len(&self) -> u64 {
        u64::try_from(self.end - self.start + 1).unwrap_or(u64::MAX)
    }
}

/// Depth per line, from a stray `}` never going below zero.
pub fn depths(lines: &[Line]) -> Vec<Depth> {
    let mut depth = 0u32;
    lines
        .iter()
        .map(|line| {
            let before = depth;
            let mut min = depth;
            let mut opens = false;
            for c in line.code.chars() {
                match c {
                    '{' => {
                        depth += 1;
                        opens = true;
                    }
                    '}' => depth = depth.saturating_sub(1),
                    _ => continue,
                }
                min = min.min(depth);
            }
            Depth {
                before,
                min,
                after: depth,
                opens,
            }
        })
        .collect()
}

static RUST_FN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+(?:"[^"]*"\s+)?)?fn\s+([A-Za-z_][A-Za-z0-9_]*)"#,
    )
    .expect("literal")
});
static RUST_TEST_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"#\[\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*(?:[A-Za-z_]*test|test_case|quickcheck|proptest)\b",
    )
    .expect("literal")
});
static GO_FN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^func\s+(?:\([^)]*\)\s*)?([A-Za-z_][A-Za-z0-9_]*)").expect("literal")
});
static PY_DEF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\s*)(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)").expect("literal")
});
static JS_FN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][A-Za-z0-9_$]*)")
        .expect("literal")
});
static JS_ARROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?::[^=]*)?=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][A-Za-z0-9_$]*)\s*(?::[^=]*)?=>",
    )
    .expect("literal")
});
static JS_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(?:(?:public|private|protected|static|async|override|readonly|get|set)\s+)*\*?\s*([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]*>)?\s*\([^)]*\)\s*(?::\s*[^{=]+)?\{\s*$",
    )
    .expect("literal")
});
/// A test call up to its title; `TITLE` reads the title from the line as written, `BLANK`
/// proves the call is code on the scanned line, so a test quoted in a template is none.
const JS_TEST: &str =
    r"^\s*(?:it|test)(?:\.(?:only|skip|todo|failing|concurrent|sequential|each\([^)]*\)))*\s*\(\s*";
/// The line closing an `each` table, up to the title that follows it.
const JS_EACH_TITLE: &str = r"[)`]\s*\(\s*";
const TITLE: &str = r#"(?:"([^"]*)"|'([^']*)'|`([^`]*)`)"#;
const BLANK: &str = "(?:\"\"|''|``)";
static JS_TESTS: LazyLock<[Regex; 2]> = LazyLock::new(|| pair(JS_TEST));
static JS_EACH_TITLES: LazyLock<[Regex; 2]> = LazyLock::new(|| pair(JS_EACH_TITLE));
static JS_EACH_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:it|test)(?:\.[A-Za-z]+)*\.each\s*[(`]").expect("literal"));

fn pair(prefix: &str) -> [Regex; 2] {
    [TITLE, BLANK].map(|t| Regex::new(&format!("{prefix}{t}")).expect("literal"))
}

/// The title `pair` reads from `line`, when its scanned code carries the call.
fn titled<'a>(pair: &[Regex; 2], line: &'a Line) -> Option<regex::Captures<'a>> {
    pair[1]
        .is_match(&line.code)
        .then(|| pair[0].captures(line.text))
        .flatten()
}
const JS_KEYWORDS: &[&str] = &[
    "if", "for", "while", "switch", "catch", "function", "return", "new", "typeof", "await",
    "else", "do", "try", "with", "import", "export", "yield", "delete", "void", "throw", "in",
    "of",
];

/// Every function in `lines`. `test_file` marks what the language's own convention names a
/// test only inside a test file; a Rust `#[test]` is one anywhere. A JavaScript test with no `{`
/// on its title line is that line, so a callback passed by name is still a test.
pub fn functions(lang: Lang, lines: &[Line], test_file: bool) -> Vec<Span> {
    let depths = depths(lines);
    let mut spans = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.kind != Kind::Code {
            continue;
        }
        let found = match lang {
            Lang::Rust => rust(lines, i).map(|(n, t)| (n, t, i)),
            Lang::Go => go(line, test_file).map(|(n, t)| (n, t, i)),
            Lang::Python => python(line, test_file).map(|(n, t)| (n, t, i)),
            Lang::Js => js(lines, i, test_file),
        };
        let Some((name, test, body)) = found else {
            continue;
        };
        let end = match lang {
            Lang::Python => python_end(lines, i),
            Lang::Js if test && !lines[body].code.contains('{') => Some(body),
            _ => brace_end(lines, &depths, body),
        };
        if let Some(end) = end {
            spans.push(Span {
                name,
                start: i,
                end,
                test,
            });
        }
    }
    spans
}

/// Attributes above a Rust item may be split across lines and separated from it by blank,
/// comment, and doc lines; the run ends at the previous item's `}`, `;`, or `{` and is read
/// as one line, so `#[\n    test\n]` marks a test too.
fn rust(lines: &[Line], i: usize) -> Option<(String, bool)> {
    let name = RUST_FN.captures(&lines[i].code)?[1].to_owned();
    let attribute =
        |l: &Line| l.kind != Kind::Code || !l.code.trim_end().ends_with(['}', ';', '{']);
    let mut run: Vec<&str> = lines[..i]
        .iter()
        .rev()
        .take_while(|l| attribute(l))
        .map(|l| l.code.trim())
        .collect();
    run.reverse();
    Some((name, RUST_TEST_ATTR.is_match(&run.join(" "))))
}

fn go(line: &Line, test_file: bool) -> Option<(String, bool)> {
    let name = GO_FN.captures(&line.code)?[1].to_owned();
    let test = test_file
        && ["Test", "Benchmark", "Fuzz", "Example"]
            .iter()
            .any(|p| name.starts_with(p));
    Some((name, test))
}

fn python(line: &Line, test_file: bool) -> Option<(String, bool)> {
    let name = PY_DEF.captures(&line.code)?[2].to_owned();
    let test = test_file && name.starts_with("test");
    Some((name, test))
}

/// A test's title, or for an `each` table opened on this line the title on the line that
/// closes the table, which is also where its body starts.
fn js(lines: &[Line], i: usize, test_file: bool) -> Option<(String, bool, usize)> {
    let line = &lines[i];
    let title = |c: regex::Captures| {
        let t = c
            .get(1)
            .or(c.get(2))
            .or(c.get(3))
            .map_or("", |m| m.as_str());
        format!("test {t}")
    };
    if test_file {
        if let Some(c) = titled(&JS_TESTS, line) {
            return Some((title(c), true, i));
        }
        if JS_EACH_OPEN.is_match(&line.code) {
            return (i + 1..lines.len().min(i + 1 + SIGNATURE_LINES))
                .find_map(|j| titled(&JS_EACH_TITLES, &lines[j]).map(|c| (title(c), true, j)));
        }
    }
    let name = JS_FN
        .captures(&line.code)
        .or_else(|| JS_ARROW.captures(&line.code))
        .or_else(|| JS_METHOD.captures(&line.code))
        .map(|c| c[1].to_owned())?;
    (!JS_KEYWORDS.contains(&name.as_str())).then_some((name, false, i))
}

/// Code lines a signature may run on before its `{`, so a file of bare signatures stays linear.
const SIGNATURE_LINES: usize = 64;

/// The line closing the body opened at or after `start`; `None` when `;` comes first, except
/// for a one-line arrow function, which is its own span.
fn brace_end(lines: &[Line], depths: &[Depth], start: usize) -> Option<usize> {
    if !lines[start].code.contains('{') && lines[start].code.contains("=>") {
        return Some(start);
    }
    let mut seen = 0;
    let opener = (start..lines.len()).find_map(|j| {
        seen += usize::from(lines[j].kind == Kind::Code);
        if seen > SIGNATURE_LINES {
            return Some(None);
        }
        match lines[j].code.chars().find(|c| matches!(c, '{' | ';' | '}')) {
            Some('{') => Some(Some(j)),
            Some(_) => Some(None),
            None => None,
        }
    })??;
    let base = depths[opener].before;
    Some(
        (opener..lines.len())
            .find(|&k| depths[k].after <= base)
            .unwrap_or(lines.len() - 1),
    )
}

/// The last non-blank line before the suite's indentation ends; a bracket left open, as in a
/// signature split across lines, holds the suite open.
fn python_end(lines: &[Line], start: usize) -> Option<usize> {
    let indent = |l: &Line| l.text.len() - l.text.trim_start().len();
    let brackets = |l: &Line| {
        l.code.chars().fold(0i64, |n, c| match c {
            '(' | '[' | '{' => n + 1,
            ')' | ']' | '}' => n - 1,
            _ => n,
        })
    };
    let base = indent(&lines[start]);
    let mut open = brackets(&lines[start]).max(0);
    let mut end = start;
    for (j, line) in lines.iter().enumerate().skip(start + 1) {
        match line.kind {
            Kind::Blank => continue,
            Kind::Code if open == 0 && !line.continued && indent(line) <= base => break,
            _ => end = j,
        }
        open = (open + brackets(line)).max(0);
    }
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::scan::scan;

    fn spans(lang: Lang, text: &str, test_file: bool) -> Vec<(String, usize, usize, bool)> {
        functions(lang, &scan(lang, text), test_file)
            .into_iter()
            .map(|s| (s.name, s.start, s.end, s.test))
            .collect()
    }

    type Want<'a> = &'a [(&'a str, usize, usize, bool)];

    fn assert_rows(rows: &[(Lang, &str, bool, Want)]) {
        for (lang, text, test_file, want) in rows {
            let want: Vec<(String, usize, usize, bool)> = want
                .iter()
                .map(|(n, s, e, t)| ((*n).to_owned(), *s, *e, *t))
                .collect();
            assert_eq!(spans(*lang, text, *test_file), want, "{lang:?}: {text:?}");
        }
    }

    #[test]
    fn functions_per_language() {
        let rows: &[(Lang, &str, bool, Want)] = &[
            (
                Lang::Rust,
                "fn a() {\n    x();\n}\n\n#[test]\nfn b() {}\n",
                false,
                &[("a", 0, 2, false), ("b", 5, 5, true)],
            ),
            (
                Lang::Rust,
                "#[tokio::test(flavor = \"x\")]\n/// doc\nasync fn t() {\n}",
                false,
                &[("t", 2, 3, true)],
            ),
            (
                Lang::Rust,
                "#[rstest]\n#[case(1)]\nfn t(#[case] n: u8) {}",
                false,
                &[("t", 2, 2, true)],
            ),
            (
                Lang::Rust,
                "#[test_case(1)]\nfn t() {}\n#[cfg(test)]\nfn h() {}\n#[testing::x]\nfn u() {}",
                false,
                &[("t", 1, 1, true), ("h", 3, 3, false), ("u", 5, 5, false)],
            ),
            (
                Lang::Rust,
                "pub(crate) const unsafe fn c() -> u8\nwhere\n    T: X,\n{\n    0\n}",
                false,
                &[("c", 0, 5, false)],
            ),
            (
                Lang::Rust,
                "trait T {\n    fn d(&self);\n    fn e(&self) {\n    }\n}",
                false,
                &[("e", 2, 3, false)],
            ),
            (
                Lang::Rust,
                "fn s() { let b = \"}\"; }\nfn n() {\n    if x { } else { }\n}",
                false,
                &[("s", 0, 0, false), ("n", 1, 3, false)],
            ),
            (
                Lang::Rust,
                "fn open() {\n    x(",
                false,
                &[("open", 0, 1, false)],
            ),
            (
                Lang::Go,
                "func TestA(t *testing.T) {\n}\nfunc (s *S) Run() {\n}\nfunc helper() {}",
                true,
                &[
                    ("TestA", 0, 1, true),
                    ("Run", 2, 3, false),
                    ("helper", 4, 4, false),
                ],
            ),
            (
                Lang::Go,
                "func TestA(t *testing.T) {}",
                false,
                &[("TestA", 0, 0, false)],
            ),
            (
                Lang::Python,
                "def test_a():\n    assert 1\n\n    assert 2\n\nclass T:\n    def test_b(self):\n        pass\n    async def c(self): pass\nx = 1",
                true,
                &[
                    ("test_a", 0, 3, true),
                    ("test_b", 6, 7, true),
                    ("c", 8, 8, false),
                ],
            ),
            (
                Lang::Python,
                "def test_a():\n    pass",
                false,
                &[("test_a", 0, 1, false)],
            ),
            (
                Lang::Python,
                "def a():\n    pass\n    # trailing\n\ndef b(): pass",
                false,
                &[("a", 0, 2, false), ("b", 4, 4, false)],
            ),
            (
                Lang::Js,
                "export async function f(a) {\n  return a;\n}\nconst g = async (x: number): Promise<void> => {\n};\nlet h = y => y;\nclass C {\n  static m(a) {\n  }\n  if (x) {\n  }\n}",
                false,
                &[
                    ("f", 0, 2, false),
                    ("g", 3, 4, false),
                    ("h", 5, 5, false),
                    ("m", 7, 8, false),
                ],
            ),
            (
                Lang::Js,
                "it('adds', () => {\n  expect(1).toBe(1);\n});\ntest.only(\"b\", async function () {\n});\nit.each([1])('c %i', (n) => expect(n).toBe(n));\ndescribe('d', () => {});",
                true,
                &[
                    ("test adds", 0, 2, true),
                    ("test b", 3, 4, true),
                    ("test c %i", 5, 5, true),
                ],
            ),
            (
                Lang::Js,
                "if (x) {\n}\nwhile (y) {\n}\nfunction* gen() {\n}",
                false,
                &[("gen", 4, 5, false)],
            ),
        ];
        assert_rows(rows);
    }

    #[test]
    fn signatures_tables_and_callbacks_split_across_lines() {
        let rows: &[(Lang, &str, bool, Want)] = &[
            (
                Lang::Python,
                "def test_a(\n    fixture,\n):\n    assert fixture\n\ndef b(): pass",
                true,
                &[("test_a", 0, 3, true), ("b", 5, 5, false)],
            ),
            (
                Lang::Js,
                "it('regression', checkRegression);\nit('b', () => {\n  expect(1).toBe(1);\n});\nit('c', f)\nit('d', () => {\n});",
                true,
                &[
                    ("test regression", 0, 0, true),
                    ("test b", 1, 3, true),
                    ("test c", 4, 4, true),
                    ("test d", 5, 6, true),
                ],
            ),
            (Lang::Js, "it('adds', () => {\n});", false, &[]),
            (
                Lang::Rust,
                "#[\n    test\n]\nfn t() {}\n#[cfg(\n    test\n)]\nfn h() {}",
                false,
                &[("t", 3, 3, true), ("h", 7, 7, false)],
            ),
            (
                Lang::Js,
                "const src = `\nit('sample', () => {});\n`;\nit('real', () => {\n});\nit.each([\n  1,\n])('row', () => {\n});",
                true,
                &[("test real", 3, 4, true), ("test row", 5, 8, true)],
            ),
            (
                Lang::Js,
                "test.each([\n  [1, 2],\n  { a: 1 },\n])('adds %i', (a, b) => {\n  expect(a).toBe(b);\n});\ntest.each`\n  a | b\n  ${1} | ${2}\n`('sums $a', ({ a, b }) => {\n  expect(a).toBe(b);\n});",
                true,
                &[("test adds %i", 0, 5, true), ("test sums $a", 6, 11, true)],
            ),
            (
                Lang::Rust,
                &format!(
                    "#[test]\nfn t(\n{}) {{\n    assert!(x);\n}}",
                    "    // long\n".repeat(70)
                ),
                false,
                &[("t", 1, 74, true)],
            ),
        ];
        assert_rows(rows);
    }

    #[test]
    fn depths_open_and_close_per_line() {
        let lines = scan(
            Lang::Rust,
            "fn f() {\n    if x {\n    } else {\n    }\n}\n}",
        );
        let got: Vec<(u32, u32, u32, bool)> = depths(&lines)
            .into_iter()
            .map(|d| (d.before, d.min, d.after, d.opens()))
            .collect();
        assert_eq!(
            got,
            [
                (0, 0, 1, true),
                (1, 1, 2, true),
                (2, 1, 2, true),
                (2, 1, 1, false),
                (1, 0, 0, false),
                (0, 0, 0, false)
            ]
        );
        let d = depths(&lines);
        assert!(!d[0].closes() && d[2].closes() && d[3].closes());
        assert!(d[0].leaves_open() && d[2].leaves_open() && !d[3].leaves_open());
        assert!(depths(&scan(Lang::Rust, "fn b() {}"))[0].opens());
    }
}
