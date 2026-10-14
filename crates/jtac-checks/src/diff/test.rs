//! `JT-TEST-001`, `-002`, `-003`: a test removed, newly skipped or focused, or asserting less
//! than it did. Identity is the test's name in a function span (`span::functions`), so a
//! renamed test is a removal plus an addition and needs a waiver. `skip_spellings` and
//! `assertion_spellings` row the tables.

use jtac_core::Severity;
use jtac_policy::Integrity;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use super::attr::probes;
use super::lang::Lang;
use super::report::Finding;
use super::rule::Rule;
use super::scan::{Kind, Line};
use super::span::{Span, functions};
use super::sup::shown;
use super::view::View;

/// Skip spellings: language, pattern, whether it focuses one test and so skips the others.
const SKIP: &[(Lang, &str, bool)] = &[
    (Lang::Rust, r"#\[\s*ignore\b", false),
    (Lang::Rust, r"#\[\s*cfg_attr\(.*,\s*ignore\b", false),
    (
        Lang::Rust,
        r"#\[\s*cfg\(\s*(?:any\(\s*\)|false|FALSE)\s*\)",
        false,
    ),
    (Lang::Python, r"\bmark\.(?:skip|skipif|xfail)\b", false),
    (
        Lang::Python,
        r"@\s*(?:unittest\.)?skip(?:If|Unless)?\b",
        false,
    ),
    (Lang::Python, r"\braise\s+(?:unittest\.)?SkipTest\b", false),
    (Lang::Python, r"\bpytest\.(?:skip|xfail)\(", false),
    (Lang::Python, r"\bself\.skipTest\(", false),
    (Lang::Go, r"\bt\.Skip(?:f|Now)?\(", false),
    (Lang::Go, r"^//\s*(?:go:build|\+build)\s+ignore\b", false),
    (
        Lang::Js,
        r#"\b(?:it|test|describe|suite|context)\b(?:[\s.\[\]'"`A-Za-z]|\?\.)*\b(?:skip|todo|fixme|skipIf|runIf)\b"#,
        false,
    ),
    (
        Lang::Js,
        r#"\b(?:it|test|describe|suite|context)\s*\(\s*(?:"[^"]*"|'[^']*'|`[^`]*`)\s*,\s*\{[^}]*\b(?:skip|todo)\s*:\s*(?:true\b|['"])"#,
        false,
    ),
    (Lang::Js, r"\b(?:xit|xtest|xdescribe|xcontext)\s*\(", false),
    (Lang::Js, r"\b(?:this|ctx|t|context)\.skip\(", false),
    (Lang::Js, r"(?:^|[^.\w])skip\(\s*\)", false),
    (
        Lang::Js,
        r#"\b(?:it|test|describe|suite|context)\b(?:[\s.\[\]'"`A-Za-z]|\?\.)*\bonly\b"#,
        true,
    ),
    (
        Lang::Js,
        r#"\b(?:it|test|describe|suite|context)\s*\(\s*(?:"[^"]*"|'[^']*'|`[^`]*`)\s*,\s*\{[^}]*\bonly\s*:\s*true\b"#,
        true,
    ),
    (Lang::Js, r"\b(?:fit|ftest|fdescribe|fcontext)\s*\(", true),
];

const ASSERT: &[(Lang, &str)] = &[
    (Lang::Rust, r"\b[A-Za-z_]*assert[A-Za-z_]*!\s*\("),
    (Lang::Python, r"\bassert[A-Za-z_]*\b"),
    (Lang::Python, r"\braises\s*\("),
    (
        Lang::Go,
        r"\bt\.(?:Error|Errorf|Fatal|Fatalf|Fail|FailNow)\s*\(",
    ),
    (Lang::Go, r"\b(?:assert|require)\.[A-Za-z]+\s*\("),
    (Lang::Js, r"\bexpect(?:TypeOf)?\s*\("),
    (Lang::Js, r"\bassert\s*[.(]"),
    (Lang::Js, r"\.should\b"),
];

static SKIPS: LazyLock<Vec<(Lang, Regex, bool)>> = LazyLock::new(|| {
    SKIP.iter()
        .map(|(l, re, focus)| (*l, Regex::new(re).expect("literal"), *focus))
        .collect()
});
static ASSERTS: LazyLock<Vec<(Lang, Regex)>> = LazyLock::new(|| {
    ASSERT
        .iter()
        .map(|(l, re)| (*l, Regex::new(re).expect("literal")))
        .collect()
});

/// Whether `line` skips a test, and whether by focusing another.
pub(crate) fn skip(lang: Lang, line: &str) -> Option<bool> {
    SKIPS
        .iter()
        .find(|(l, re, _)| *l == lang && re.is_match(line))
        .map(|(_, _, focus)| *focus)
}

/// Assertions in `lines[start..=end]`, over code with strings blanked.
pub(crate) fn assertions(lang: Lang, lines: &[Line], span: &Span) -> u64 {
    lines[span.start..=span.end]
        .iter()
        .filter(|l| l.kind == Kind::Code)
        .map(|l| {
            ASSERTS
                .iter()
                .filter(|(l2, _)| *l2 == lang)
                .map(|(_, re)| re.find_iter(&l.code).count())
                .sum::<usize>()
        })
        .sum::<usize>() as u64
}

/// A copy of a test name: assertions and the index of the line naming it.
type Copy = (u64, usize);
/// A copy gone from a file: name, assertions, and the removal to report if nothing arrives.
type Gone = (String, u64, Finding);
/// A copy a file gained: name, assertions, line.
type Arrived = (String, u64, u64);

pub(crate) fn check(views: &[View], integrity: &Integrity) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut gone = Vec::new();
    let mut arrived: BTreeMap<String, Vec<(u64, &str, u64)>> = BTreeMap::new();
    for v in views {
        let (left, came) = removed(v, integrity, &mut findings);
        gone.extend(left);
        for (name, now, line) in came {
            arrived.entry(name).or_default().push((now, v.path(), line));
        }
        if let Some(lang) = v.new_lang {
            findings.extend(skipped(v, lang, integrity));
        }
    }
    gone.sort_by_key(|a| std::cmp::Reverse(a.1));
    for copies in arrived.values_mut() {
        copies.sort_unstable();
    }
    for (name, was, removal) in gone {
        match arrived.get_mut(&name).and_then(Vec::pop) {
            Some((now, path, line)) if now < was => {
                findings.push(weaker(&name, now, was, path, line));
            }
            Some(_) => {}
            None => findings.push(removal),
        }
    }
    findings
}

/// `JT-TEST-001` candidates and `JT-TEST-003` within one file. Copies of a name pair strongest
/// first, so the copies gone are the weakest; `check` pairs what is gone with what arrived in
/// another file.
fn removed(
    v: &View,
    integrity: &Integrity,
    findings: &mut Vec<Finding>,
) -> (Vec<Gone>, Vec<Arrived>) {
    let severity = if integrity.allow_removed_tests {
        Severity::Minor
    } else {
        Severity::Blocker
    };
    let finding = |message: String| Finding::new(Rule::Test001, severity, v.path(), 0, message);
    let old = v
        .old_lang
        .map(|l| tests(l, &v.old, v.old_test))
        .unwrap_or_default();
    let new = v
        .new_lang
        .map(|l| tests(l, &v.new, v.new_test))
        .unwrap_or_default();
    let (mut gone, mut arrived) = (Vec::new(), Vec::new());
    for (name, old_copies) in &old {
        let new_copies = new.get(name).map_or(&[][..], Vec::as_slice);
        for (i, (was, _)) in old_copies.iter().enumerate() {
            match new_copies.get(i) {
                Some((now, start)) if now < was => {
                    findings.push(weaker(name, *now, *was, v.path(), line(*start)));
                }
                Some(_) => {}
                None => gone.push((
                    name.clone(),
                    *was,
                    finding(format!("test `{name}` removed")),
                )),
            }
        }
    }
    for (name, new_copies) in &new {
        let kept = old.get(name).map_or(0, Vec::len);
        arrived.extend(
            new_copies
                .iter()
                .skip(kept)
                .map(|(now, start)| (name.clone(), *now, line(*start))),
        );
    }
    if v.change.new.is_none() && v.old_test && old.is_empty() {
        gone.push((String::new(), 0, finding("test file removed".to_owned())));
    }
    (gone, arrived)
}

fn weaker(name: &str, now: u64, was: u64, path: &str, line: u64) -> Finding {
    Finding::new(
        Rule::Test003,
        Severity::Major,
        path,
        line,
        format!("test `{name}` asserts {now} times, was {was}"),
    )
}

fn skipped(v: &View, lang: Lang, integrity: &Integrity) -> Vec<Finding> {
    let severity = if integrity.allow_newly_skipped_tests {
        Severity::Minor
    } else {
        Severity::Blocker
    };
    if lang != Lang::Rust && !v.new_test {
        return Vec::new();
    }
    probes(v, lang == Lang::Go)
        .into_iter()
        .filter_map(|(n, text, probe)| {
            skip(lang, &probe).map(|focus| {
                let what = if focus {
                    "focused test skips the others"
                } else {
                    "test skipped"
                };
                Finding::new(
                    Rule::Test002,
                    severity,
                    v.path(),
                    n,
                    format!("{what} `{}`", shown(text)),
                )
            })
        })
        .collect()
}

/// Test copies by name, strongest first.
fn tests(lang: Lang, lines: &[Line], test_file: bool) -> BTreeMap<String, Vec<Copy>> {
    let mut map: BTreeMap<String, Vec<Copy>> = BTreeMap::new();
    for span in functions(lang, lines, test_file)
        .into_iter()
        .filter(|s| s.test)
    {
        let asserts = assertions(lang, lines, &span);
        map.entry(span.name)
            .or_default()
            .push((asserts, span.start));
    }
    for copies in map.values_mut() {
        copies.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    map
}

fn line(index: usize) -> u64 {
    u64::try_from(index + 1).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::change::{Blob, Change};
    use crate::diff::scan::scan;

    #[test]
    fn skip_spellings() {
        let rows: &[(Lang, &str, Option<bool>)] = &[
            (Lang::Rust, "#[ignore]", Some(false)),
            (Lang::Rust, "    #[ignore = \"flaky\"]", Some(false)),
            (Lang::Rust, "#[cfg_attr(windows, ignore)]", Some(false)),
            (
                Lang::Rust,
                "#[cfg_attr(not(feature = \"x\"), ignore = \"why\")]",
                Some(false),
            ),
            (Lang::Rust, "// #[ignore] was here", Some(false)),
            (Lang::Rust, "#[cfg(any())]", Some(false)),
            (Lang::Rust, "#[cfg(false)]", Some(false)),
            (Lang::Rust, "#[cfg(unix)]", None),
            (Lang::Rust, "#[test]", None),
            (Lang::Rust, "let ignore = 1;", None),
            (Lang::Python, "@pytest.mark.skip(reason=\"x\")", Some(false)),
            (
                Lang::Python,
                "@pytest.mark.skipif(sys.platform == \"win32\")",
                Some(false),
            ),
            (Lang::Python, "@pytest.mark.xfail", Some(false)),
            (Lang::Python, "@unittest.skip(\"x\")", Some(false)),
            (Lang::Python, "@unittest.skipIf(x, \"y\")", Some(false)),
            (Lang::Python, "    pytest.skip(\"no db\")", Some(false)),
            (Lang::Python, "    self.skipTest(\"x\")", Some(false)),
            (
                Lang::Python,
                "pytestmark = pytest.mark.skip(reason=\"x\")",
                Some(false),
            ),
            (
                Lang::Python,
                "pytestmark = [pytest.mark.slow, mark.skipif(x)]",
                Some(false),
            ),
            (Lang::Python, "@mark.skip", Some(false)),
            (Lang::Python, "@ pytest.mark.skip", Some(false)),
            (Lang::Python, "@skip('x')", Some(false)),
            (Lang::Python, "@skipIf(x, 'y')", Some(false)),
            (
                Lang::Python,
                "    raise unittest.SkipTest(\"x\")",
                Some(false),
            ),
            (Lang::Python, "    raise SkipTest", Some(false)),
            (Lang::Python, "@pytest.mark.slow", None),
            (Lang::Python, "pytestmark = pytest.mark.slow", None),
            (Lang::Python, "@skipper", None),
            (Lang::Python, "pytest.importorskip(\"numpy\")", None),
            (
                Lang::Python,
                "pytest.param(1, marks=pytest.mark.skip)",
                Some(false),
            ),
            (
                Lang::Python,
                "    item.add_marker(pytest.mark.skip(reason='x'))",
                Some(false),
            ),
            (Lang::Python, "    pytest.xfail('x')", Some(false)),
            (Lang::Python, "bookmark.skip_count = 1", None),
            (Lang::Python, "x = mark.skipped", None),
            (Lang::Go, "\tt.Skip(\"x\")", Some(false)),
            (Lang::Go, "\tt.Skipf(\"%s\", x)", Some(false)),
            (Lang::Go, "\tt.SkipNow()", Some(false)),
            (Lang::Go, "//go:build ignore", Some(false)),
            (Lang::Go, "// +build ignore", Some(false)),
            (Lang::Go, "//go:build linux", None),
            (Lang::Go, "\tt.Run(\"x\", f)", None),
            (Lang::Js, "it.skip('x', () => {})", Some(false)),
            (Lang::Js, "test.todo('x')", Some(false)),
            (Lang::Js, "describe.skip('x', () => {})", Some(false)),
            (Lang::Js, "test.fixme('x', async () => {})", Some(false)),
            (Lang::Js, "xit('x', () => {})", Some(false)),
            (Lang::Js, "xdescribe('x', () => {})", Some(false)),
            (Lang::Js, "  this.skip();", Some(false)),
            (Lang::Js, "it.only('x', () => {})", Some(true)),
            (Lang::Js, "test.only('x', () => {})", Some(true)),
            (Lang::Js, "describe.only('x', () => {})", Some(true)),
            (Lang::Js, "fit('x', () => {})", Some(true)),
            (Lang::Js, "fdescribe('x', () => {})", Some(true)),
            (Lang::Js, "it['only']('x', () => {})", Some(true)),
            (Lang::Js, "it[`only`]('x', () => {})", Some(true)),
            (Lang::Js, "it?.only('x', () => {})", Some(true)),
            (Lang::Js, "test?.skip('x', () => {})", Some(false)),
            (Lang::Js, "test[`skip`]('x', () => {})", Some(false)),
            (Lang::Js, "const files = test ? only : all;", None),
            (Lang::Js, "test.concurrent.only('x', () => {})", Some(true)),
            (Lang::Js, "it .only('x', () => {})", Some(true)),
            (
                Lang::Js,
                "describe.skipIf(true)('x', () => {})",
                Some(false),
            ),
            (Lang::Js, "test.runIf(false)('x', () => {})", Some(false)),
            (Lang::Js, "it('x', () => {})", None),
            (Lang::Js, "it('only one', () => {})", None),
            (Lang::Js, "items.only = 1", None),
            (Lang::Js, "exit('x')", None),
            (Lang::Js, "profit('x')", None),
            (Lang::Js, "it.each([1])('x', () => {})", None),
            (Lang::Js, "test('x', { skip: true }, () => {})", Some(false)),
            (
                Lang::Js,
                "test('x', { skip: 'why' }, () => {})",
                Some(false),
            ),
            (Lang::Js, "it('x', { todo: true })", Some(false)),
            (Lang::Js, "test('x', { only: true }, () => {})", Some(true)),
            (Lang::Js, "  ctx.skip();", Some(false)),
            (Lang::Js, "  t.skip('x');", Some(false)),
            (Lang::Js, "  skip();", Some(false)),
            (
                Lang::Js,
                "test('x', { timeout: 1000, skip: true }, cb)",
                Some(false),
            ),
            (
                Lang::Js,
                "test('x', { timeout: 1000, only: true }, cb)",
                Some(true),
            ),
            (Lang::Js, "test('x', { skip: false }, () => {})", None),
            (
                Lang::Js,
                "test('x', () => { const o = { timeout: 1, skip: true }; })",
                None,
            ),
            (Lang::Js, "test('x', { timeout: 1000 }, () => {})", None),
            (Lang::Js, "  list.skip();", None),
            (Lang::Js, "  skipped();", None),
        ];
        for (lang, line, want) in rows {
            assert_eq!(skip(*lang, line), *want, "{lang:?}: {line:?}");
        }
    }

    #[test]
    fn assertion_spellings() {
        let rows: &[(Lang, &str, u64)] = &[
            (
                Lang::Rust,
                "assert!(x); assert_eq!(a, b);\nassert_ne!(a, b);\nprop_assert_eq!(a, b);\ndebug_assert!(x);",
                5,
            ),
            (
                Lang::Rust,
                "let s = \"assert!(x)\"; // assert!(y)\nassert_matches!(x, Y);",
                1,
            ),
            (
                Lang::Python,
                "assert x == 1\nself.assertEqual(a, b)\nmock.assert_called_once()\nwith pytest.raises(E):\n    f()\nnp.testing.assert_array_equal(a, b)",
                5,
            ),
            (
                Lang::Python,
                "x = \"assert\"  # assert here\nassertion = 1",
                1,
            ),
            (
                Lang::Go,
                "if got != want {\n\tt.Errorf(\"x\")\n}\nrequire.NoError(t, err)\nassert.Equal(t, a, b)\nt.Fatal(x)",
                4,
            ),
            (Lang::Go, "t.Log(x)\nt.Helper()", 0),
            (
                Lang::Js,
                "expect(a).toBe(b);\nassert.equal(a, b);\nassert(x);\na.should.equal(b);\nexpectTypeOf(x).toEqualTypeOf<Y>();",
                5,
            ),
            (Lang::Js, "const s = 'expect(1)'; // expect(2)", 0),
        ];
        for (lang, text, want) in rows {
            let lines = scan(*lang, text);
            let span = Span {
                name: String::new(),
                start: 0,
                end: lines.len() - 1,
                test: true,
            };
            assert_eq!(
                assertions(*lang, &lines, &span),
                *want,
                "{lang:?}: {text:?}"
            );
        }
    }

    fn change(path: &str, from: Option<&str>, old: Option<&str>, new: Option<&str>) -> Change {
        Change {
            path: path.into(),
            from: from.map(str::to_owned),
            old: old.map(|s| Blob::Text(s.into())),
            new: new.map(|s| Blob::Text(s.into())),
        }
    }

    type Want<'a> = &'a [(Rule, Severity, u64, &'a str)];

    fn run(change: &Change, integrity: &Integrity) -> Vec<(Rule, Severity, u64, String)> {
        check(&[View::of(change)], integrity)
            .into_iter()
            .map(|f| (f.rule, f.severity, f.line, f.message))
            .collect()
    }

    #[test]
    fn removed_renamed_and_weakened_tests_per_rule() {
        let d = Integrity::default();
        let two =
            "#[test]\nfn a() {\n    assert!(x);\n    assert_eq!(1, 1);\n}\n#[test]\nfn b() {}\n";
        let rows: &[(Change, Want)] = &[
            (
                change("tests/t.rs", None, Some(two), None),
                &[
                    (Rule::Test001, Severity::Blocker, 0, "test `a` removed"),
                    (Rule::Test001, Severity::Blocker, 0, "test `b` removed"),
                ],
            ),
            (
                change(
                    "src/t.rs",
                    None,
                    Some(two),
                    Some("#[test]\nfn a() {\n    assert!(x);\n    assert_eq!(1, 1);\n}\n"),
                ),
                &[(Rule::Test001, Severity::Blocker, 0, "test `b` removed")],
            ),
            (
                change(
                    "src/t.rs",
                    None,
                    Some(two),
                    Some("#[test]\nfn a() {\n    assert!(x);\n}\n#[test]\nfn b() {}\n"),
                ),
                &[(
                    Rule::Test003,
                    Severity::Major,
                    2,
                    "test `a` asserts 1 times, was 2",
                )],
            ),
            (
                change(
                    "src/t.rs",
                    None,
                    Some(two),
                    Some(
                        "#[test]\nfn a() {\n    assert!(x);\n    assert_eq!(1, 1);\n    assert!(y);\n}\nfn b() {}\n",
                    ),
                ),
                &[(Rule::Test001, Severity::Blocker, 0, "test `b` removed")],
            ),
            (
                change(
                    "src/t.rs",
                    None,
                    Some(two),
                    Some(
                        "#[test]\nfn a() {\n    check(x);\n    assert_eq!(1, 1);\n}\n#[test]\nfn b() {}\n",
                    ),
                ),
                &[(
                    Rule::Test003,
                    Severity::Major,
                    2,
                    "test `a` asserts 1 times, was 2",
                )],
            ),
            (
                change("src/u.rs", Some("src/t.rs"), Some(two), Some(two)),
                &[],
            ),
            (
                change(
                    "pkg/a.go",
                    Some("pkg/a_test.go"),
                    Some("func TestA(t *testing.T) {}\n"),
                    Some("func TestA(t *testing.T) {}\n"),
                ),
                &[(Rule::Test001, Severity::Blocker, 0, "test `TestA` removed")],
            ),
            (
                change(
                    "src/a.test.ts",
                    None,
                    Some("it('adds', () => {\n  expect(1).toBe(1);\n});\n"),
                    Some("it('adds numbers', () => {\n  expect(1).toBe(1);\n});\n"),
                ),
                &[(
                    Rule::Test001,
                    Severity::Blocker,
                    0,
                    "test `test adds` removed",
                )],
            ),
            (
                change(
                    "tests/test_a.py",
                    None,
                    Some("def test_a():\n    assert 1\n"),
                    Some("def test_a():\n    assert 1\n\n\ndef test_b():\n    pass\n"),
                ),
                &[],
            ),
            (
                change("src/a.rs", None, None, Some("#[test]\nfn a() {}\n")),
                &[],
            ),
        ];
        for (change, want) in rows {
            let want: Vec<(Rule, Severity, u64, String)> = want
                .iter()
                .map(|(r, s, l, m)| (*r, *s, *l, (*m).to_owned()))
                .collect();
            assert_eq!(run(change, &d), want, "{change:?}");
        }
        let allowed = Integrity {
            allow_removed_tests: true,
            ..Integrity::default()
        };
        let got = run(&change("tests/t.rs", None, Some(two), None), &allowed);
        assert_eq!(got[0].1, Severity::Minor);
    }

    #[test]
    fn removed_tests_are_found_in_any_rust_file_through_renames_and_duplicates() {
        let d = Integrity::default();
        let two =
            "#[test]\nfn a() {\n    assert!(x);\n    assert_eq!(1, 1);\n}\n#[test]\nfn b() {}\n";
        let rows: &[(Change, Want)] = &[
            (
                change("src/t.rs", None, Some(two), None),
                &[
                    (Rule::Test001, Severity::Blocker, 0, "test `a` removed"),
                    (Rule::Test001, Severity::Blocker, 0, "test `b` removed"),
                ],
            ),
            (
                change(
                    "tests/t.rs.disabled",
                    Some("tests/t.rs"),
                    Some(two),
                    Some(two),
                ),
                &[
                    (Rule::Test001, Severity::Blocker, 0, "test `a` removed"),
                    (Rule::Test001, Severity::Blocker, 0, "test `b` removed"),
                ],
            ),
            (
                change(
                    "src/t.rs",
                    None,
                    Some(
                        "mod a {\n    #[test]\n    fn works() {}\n}\nmod b {\n    #[test]\n    fn works() {}\n}\n",
                    ),
                    Some("mod a {\n    #[test]\n    fn works() {}\n}\n"),
                ),
                &[(Rule::Test001, Severity::Blocker, 0, "test `works` removed")],
            ),
            (
                change(
                    "src/t.rs",
                    None,
                    Some(
                        "#[tokio::test(\n    flavor = \"multi_thread\"\n)]\n\n// why\nasync fn t() {}\n",
                    ),
                    Some(""),
                ),
                &[(Rule::Test001, Severity::Blocker, 0, "test `t` removed")],
            ),
            (
                change(
                    "src/t.rs",
                    None,
                    Some(
                        "mod a {\n    #[test]\n    fn works() {\n        assert!(x);\n    }\n}\nmod b {\n    #[test]\n    fn works() {}\n}\n",
                    ),
                    Some(
                        "mod b {\n    #[test]\n    fn works() {}\n}\nmod a {\n    #[test]\n    fn works() {\n        assert!(x);\n    }\n}\n",
                    ),
                ),
                &[],
            ),
            (
                change(
                    "tests/a.py",
                    Some("tests/a.rs"),
                    Some("#[test]\nfn works() {}\n"),
                    Some("#[test]\nfn works() {}\n"),
                ),
                &[(Rule::Test001, Severity::Blocker, 0, "test `works` removed")],
            ),
            (
                change("tests/t.rs", None, Some("fn helper() {}\n"), None),
                &[(Rule::Test001, Severity::Blocker, 0, "test file removed")],
            ),
        ];
        for (change, want) in rows {
            let want: Vec<(Rule, Severity, u64, String)> = want
                .iter()
                .map(|(r, s, l, m)| (*r, *s, *l, (*m).to_owned()))
                .collect();
            assert_eq!(run(change, &d), want, "{change:?}");
        }
    }

    #[test]
    fn a_test_that_reappears_by_name_in_another_file_moved() {
        let d = Integrity::default();
        let strong = "#[test]\nfn works() {\n    assert!(x);\n}\n";
        let weak = "#[test]\nfn works() {}\n";
        let a = change("tests/a.rs", None, Some(strong), Some(""));
        let b = change("tests/b.rs", None, None, Some(strong));
        let b_weak = change("tests/b.rs", None, None, Some(weak));
        let c = change("tests/c.rs", None, None, Some("#[test]\nfn other() {}\n"));
        let c_gone = change("tests/c.rs", None, Some(weak), Some(""));
        type Row<'a> = (&'a [&'a Change], &'a [(Rule, &'a str, u64, &'a str)]);
        let rows: &[Row] = &[
            (&[&a, &b], &[]),
            (
                &[&a, &c],
                &[(Rule::Test001, "tests/a.rs", 0, "test `works` removed")],
            ),
            (
                &[&a, &b_weak],
                &[(
                    Rule::Test003,
                    "tests/b.rs",
                    2,
                    "test `works` asserts 0 times, was 1",
                )],
            ),
            (
                &[&a, &c_gone, &b],
                &[(Rule::Test001, "tests/c.rs", 0, "test `works` removed")],
            ),
        ];
        for (changes, want) in rows {
            let views: Vec<View> = changes.iter().map(|c| View::of(c)).collect();
            let got: Vec<(Rule, String, u64, String)> = check(&views, &d)
                .into_iter()
                .map(|f| (f.rule, f.path, f.line, f.message))
                .collect();
            let want: Vec<(Rule, String, u64, String)> = want
                .iter()
                .map(|(r, p, l, m)| (*r, (*p).to_owned(), *l, (*m).to_owned()))
                .collect();
            assert_eq!(got, want, "{changes:?}");
        }
    }

    #[test]
    fn newly_skipped_tests_need_a_test_file_outside_rust() {
        let d = Integrity::default();
        let rows: &[(Change, Want)] = &[
            (
                change(
                    "src/a.rs",
                    None,
                    Some("#[test]\nfn a() {}\n"),
                    Some("#[test]\n#[ignore]\nfn a() {}\n"),
                ),
                &[(
                    Rule::Test002,
                    Severity::Blocker,
                    2,
                    "test skipped `#[ignore]`",
                )],
            ),
            (
                change(
                    "src/a.rs",
                    None,
                    Some("#[test]\nfn a() {}\n"),
                    Some(
                        "#[cfg_attr(\n    all(),\n    ignore = \"long reason\"\n)]\n#[test]\nfn a() {}\n",
                    ),
                ),
                &[(
                    Rule::Test002,
                    Severity::Blocker,
                    1,
                    "test skipped `#[cfg_attr(`",
                )],
            ),
            (
                change(
                    "src/a.test.ts",
                    None,
                    Some("it('a', f);\n"),
                    Some("it.only('a', f);\n"),
                ),
                &[(
                    Rule::Test002,
                    Severity::Blocker,
                    1,
                    "focused test skips the others `it.only('a', f);`",
                )],
            ),
            (
                change(
                    "src/a.test.ts",
                    None,
                    Some("describe('a', f);\n"),
                    Some("describe['only']('a', f);\n"),
                ),
                &[(
                    Rule::Test002,
                    Severity::Blocker,
                    1,
                    "focused test skips the others `describe['only']('a', f);`",
                )],
            ),
            (
                change(
                    "tests/test_a.py",
                    None,
                    None,
                    Some("def test_a(x):\n    pytest.xfail('later')\n"),
                ),
                &[(
                    Rule::Test002,
                    Severity::Blocker,
                    2,
                    "test skipped `pytest.xfail('later')`",
                )],
            ),
            (
                change(
                    "src/a.ts",
                    None,
                    Some("it('a', f);\n"),
                    Some("it.only('a', f);\n"),
                ),
                &[],
            ),
            (
                change(
                    "pkg/a_test.go",
                    None,
                    None,
                    Some("func TestA(t *testing.T) {\n\tt.Skip(\"x\")\n}\n"),
                ),
                &[(
                    Rule::Test002,
                    Severity::Blocker,
                    2,
                    "test skipped `t.Skip(\"x\")`",
                )],
            ),
            (
                change(
                    "tests/test_a.py",
                    None,
                    Some("def test_a():\n    pass\n"),
                    Some("@pytest.mark.skip\ndef test_a():\n    pass\n"),
                ),
                &[(
                    Rule::Test002,
                    Severity::Blocker,
                    1,
                    "test skipped `@pytest.mark.skip`",
                )],
            ),
        ];
        for (change, want) in rows {
            let want: Vec<(Rule, Severity, u64, String)> = want
                .iter()
                .map(|(r, s, l, m)| (*r, *s, *l, (*m).to_owned()))
                .collect();
            assert_eq!(run(change, &d), want, "{change:?}");
        }
        let allowed = Integrity {
            allow_newly_skipped_tests: true,
            ..Integrity::default()
        };
        let got = run(
            &change("src/a.rs", None, None, Some("#[ignore]\n")),
            &allowed,
        );
        assert_eq!(got[0].1, Severity::Minor);
    }

    #[test]
    fn an_existing_or_quoted_marker_is_no_skip() {
        let d = Integrity::default();
        let rows: &[(Change, Want)] = &[
            (
                change(
                    "src/a.rs",
                    None,
                    Some("#[cfg_attr(\n    all(),\n    ignore = \"a\"\n)]\n#[test]\nfn a() {}\n"),
                    Some(
                        "#[cfg_attr(\n    all(),\n    // why\n    ignore = \"a\"\n)]\n#[test]\nfn a() {}\n",
                    ),
                ),
                &[],
            ),
            (
                change(
                    "src/a.rs",
                    None,
                    Some("#[ignore]\nfn a() {}\n"),
                    Some("#[ignore]\nfn a() {}\nfn b() {}\n"),
                ),
                &[],
            ),
            (
                change(
                    "src/a.rs",
                    None,
                    None,
                    Some("// #[ignore]\nconst S: &str = \"#[ignore]\";\n"),
                ),
                &[],
            ),
            (
                change(
                    "src/a.test.ts",
                    None,
                    None,
                    Some("// it.skip('a', f);\nconst s = \"it.only('a')\";\n"),
                ),
                &[],
            ),
        ];
        for (change, want) in rows {
            let want: Vec<(Rule, Severity, u64, String)> = want
                .iter()
                .map(|(r, s, l, m)| (*r, *s, *l, (*m).to_owned()))
                .collect();
            assert_eq!(run(change, &d), want, "{change:?}");
        }
    }
}
