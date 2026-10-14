//! `JT-CI-001`: in a CI definition, fewer step lines after than before, or an added line that
//! lets a step fail. A step line is a step opener or a command beyond the first under it, so
//! the order of a step's keys and the spelling of its commands change no count. No YAML is
//! parsed, so a step replaced by another of the same shape is not seen; every CI path is
//! protected by default, so `JT-PROT-001` names the file either way. `ci_spellings` rows the
//! table.

use jtac_core::Severity;
use regex::Regex;
use std::sync::LazyLock;

use super::change::split_lines;
use super::report::Finding;
use super::rule::Rule;
use super::sup::shown;
use super::view::View;

const STEP: &[&str] = &[
    r"^\s*-\s*(?:run|uses|name|id|if|env|with|shell|working-directory|timeout-minutes|continue-on-error|command):",
    r"^\s*(?:before_|after_)?script:",
    r"^\s*sh\s+['\x22]",
    r"^\s*stage\s*\(",
];

const UNBLOCK: &[&str] = &[
    r"continue-on-error:\s*(?:true\b|\$\{\{)",
    r"allow_failure:\s*(?:true\b|\$\{\{|$)",
    r"soft_fail:\s*true\b",
    r"\|\|\s*(?:true\b|exit\s+0\b|:)",
    r"\bset\s+\+e\b",
    r"^\s*(?:-\s*)?if:\s*(?:false\b|\$\{\{\s*false\s*\}\})",
    r"^\s*(?:-\s*)?when:\s*manual\b",
    r"\b(?:catchError|warnError|unstable)\s*\(",
];

static STEPS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    STEP.iter()
        .map(|r| Regex::new(r).expect("literal"))
        .collect()
});
static UNBLOCKS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    UNBLOCK
        .iter()
        .map(|r| Regex::new(r).expect("literal"))
        .collect()
});
/// A GitLab script key opening a list.
static LIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:before_|after_)?script:\s*$").expect("literal"));
/// A `"` or `'` opening a scalar value after a key or a dash, past an anchor or tag; one at a
/// line start continues a plain scalar and opens nothing, unless the line above was `BARE`.
static QUOTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:-\s+)*(?:(?:"[^"]*"|'[^']*'|[^\s:#"']+):\s+|-\s+)(?:[&!]\S+\s+)?(["'])"#)
        .expect("literal")
});
/// A key, anchor or tag with its value on the next line.
static BARE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:-\s*)*(?:(?:"[^"]*"|'[^']*'|[^\s:#"']+):\s*)?(?:[&!]\S+)?\s*(?:#.*)?$"#)
        .expect("literal")
});
/// A `"` or `'` opening the value of the key above.
static VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*(?:[&!]\S+\s+)?(["'])"#).expect("literal"));
/// A command key opening a block scalar.
static BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(?:-\s*|(?:-\s*)?(?:run|script|before_script|after_script|command|bash|pwsh|powershell):\s*)[|>]([-+0-9]*)\s*(?:#.*)?$",
    )
    .expect("literal")
});

/// Whether `path` is a CI definition the rule reads.
pub(crate) fn is_ci_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let yaml = name.ends_with(".yml") || name.ends_with(".yaml");
    (yaml && (path.starts_with(".github/workflows/") || path.starts_with(".buildkite/")))
        || matches!(
            path,
            ".gitlab-ci.yml"
                | ".circleci/config.yml"
                | ".travis.yml"
                | ".drone.yml"
                | ".woodpecker.yml"
                | "azure-pipelines.yml"
                | "bitbucket-pipelines.yml"
                | "Jenkinsfile"
        )
}

/// Whether each line lies in a block scalar's body: indented past its key and no less than the
/// body's first line, or than the key plus its indentation indicator. A `#` line there is
/// shell, not YAML: it sets the body indent like any first line and hides nothing from `check`.
fn inside(text: &str) -> Vec<bool> {
    let mut block: Option<(usize, Option<usize>)> = None;
    split_lines(text)
        .into_iter()
        .map(|line| {
            let item = line.trim_start();
            let indent = line.len() - item.len();
            let body = block.is_some_and(|(key, b)| indent > key && b.is_none_or(|b| indent >= b));
            if item.is_empty() {
                return body;
            }
            if body {
                block = block.map(|(key, b)| (key, b.or(Some(indent))));
            } else if !item.starts_with('#') {
                block = BLOCK.captures(line).map(|c| {
                    let digit = c[1].chars().find_map(|d| d.to_digit(10));
                    let key = line.len() - item.trim_start_matches('-').trim_start().len();
                    (indent, digit.map(|d| key + d as usize))
                });
            }
            body
        })
        .collect()
}

/// Whether each line is shell to `check`: a block scalar's body, or the continuation of a
/// `"` or `'` scalar opened above, where a leading `#` is text. Rows in
/// `a_quoted_scalar_continuing_onto_a_hash_line_is_read` and
/// `a_quoted_value_below_a_bare_key_or_dash_opens`.
fn shell(text: &str) -> Vec<bool> {
    let mut open: Option<char> = None;
    let mut bare = false;
    split_lines(text)
        .into_iter()
        .zip(inside(text))
        .map(|(line, body)| {
            let was = open.is_some();
            let (from, quote) = match open {
                Some(q) => (0, Some(q)),
                None if body => (0, None),
                None => QUOTE
                    .captures(line)
                    .or_else(|| bare.then(|| VALUE.captures(line)).flatten())
                    .and_then(|c| c.get(1))
                    .map_or((0, None), |m| (m.end(), m.as_str().chars().next())),
            };
            open = quote.filter(|&q| !closes(&line[from..], q));
            let plain = line.trim_start();
            if !(plain.is_empty() || plain.starts_with('#')) {
                bare = open.is_none()
                    && !body
                    && BARE.is_match(line)
                    && (bare || line.contains(':') || plain.starts_with('-'));
            }
            body || was
        })
        .collect()
}

/// Whether `s` closes an open `q` scalar: `\` escapes in `"`, a doubled quote in `'`.
fn closes(s: &str, q: char) -> bool {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && q == '"' {
            chars.next();
        } else if c == q && !(q == '\'' && chars.next_if_eq(&'\'').is_some()) {
            return true;
        }
    }
    false
}

/// Whether each Jenkinsfile line lies in a `'''` or `"""` string, where `//` is shell.
fn quoted(text: &str) -> Vec<bool> {
    let mut open = false;
    split_lines(text)
        .into_iter()
        .map(|line| {
            let was = open;
            let fences = line.matches("'''").count() + line.matches("\"\"\"").count();
            open ^= fences % 2 == 1;
            was
        })
        .collect()
}

/// Step lines in a CI definition: a step opener, then each command beyond the first under it,
/// as list items (indented or not) or block scalar lines. A `#` comment line is nothing anywhere.
pub(crate) fn steps(text: &str) -> u64 {
    let mut n = 0;
    let mut list: Option<(usize, bool)> = None;
    let mut first = false;
    for (line, body) in split_lines(text).into_iter().zip(inside(text)) {
        let item = line.trim_start();
        if item.is_empty() || item.starts_with('#') {
            continue;
        }
        if body {
            n += u64::from(!std::mem::take(&mut first));
            continue;
        }
        first = BLOCK.is_match(line);
        let indent = line.len() - item.len();
        if list.is_some_and(|(s, _)| indent < s || (indent == s && !item.starts_with("- "))) {
            list = None;
        }
        if let Some((s, opener)) = list
            && item.starts_with("- ")
        {
            n += u64::from(!opener);
            list = Some((s, false));
            continue;
        }
        n += u64::from(STEPS.iter().any(|re| re.is_match(line)));
        if LIST.is_match(line) {
            list = Some((indent, true));
        }
    }
    n
}

fn unblock(line: &str) -> bool {
    UNBLOCKS.iter().any(|re| re.is_match(line))
}

pub(crate) fn check(views: &[View]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for v in views {
        let from = v.change.from.as_deref().unwrap_or(v.path());
        let (old_ci, new_ci) = (is_ci_path(from), is_ci_path(v.path()));
        if !old_ci && !new_ci {
            continue;
        }
        let finding =
            |line, message| Finding::new(Rule::Ci001, Severity::Blocker, v.path(), line, message);
        let count = |ci: bool, text: Option<&str>| text.filter(|_| ci).map_or(0, steps);
        let (before, after) = (
            count(old_ci, v.change.old_text()),
            count(new_ci, v.change.new_text()),
        );
        if after < before {
            findings.push(finding(
                0,
                format!("CI steps: {before} before, {after} after"),
            ));
        }
        let groovy = v.path() == "Jenkinsfile";
        let body = v
            .change
            .new_text()
            .map(if groovy { quoted } else { shell })
            .unwrap_or_default();
        let comment = |n: &u64, t: &str| {
            let at = usize::try_from(*n)
                .ok()
                .and_then(|i| body.get(i.checked_sub(1)?));
            t.trim_start().starts_with(if groovy { "//" } else { "#" })
                && !at.copied().unwrap_or(false)
        };
        for (n, text) in v
            .edit
            .added
            .iter()
            .filter(|(n, t)| new_ci && !comment(n, t) && unblock(t))
        {
            findings.push(finding(*n, format!("CI step may fail `{}`", shown(text))));
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::change::{Blob, Change};

    #[test]
    fn ci_spellings() {
        let paths = [
            (".github/workflows/ci.yml", true),
            (".github/workflows/a/b.yaml", true),
            (".github/dependabot.yml", false),
            (".gitlab-ci.yml", true),
            ("sub/.gitlab-ci.yml", false),
            (".circleci/config.yml", true),
            ("Jenkinsfile", true),
            ("azure-pipelines.yml", true),
            ("bitbucket-pipelines.yml", true),
            (".buildkite/pipeline.yml", true),
            (".travis.yml", true),
            ("ci.yml", false),
            ("Makefile", false),
        ];
        for (path, want) in paths {
            assert_eq!(is_ci_path(path), want, "{path}");
        }
        let counts = [
            (
                "steps:\n  - run: a\n  - uses: b\n  - name: c\n    run: d\n",
                3,
            ),
            (
                "test:\n  script:\n    - cargo test\n    - cargo clippy\n  after_script:\n    - echo\nother:\n  - not a step\n",
                3,
            ),
            ("test:\n  script: cargo test\n  before_script: make\n", 2),
            ("test:\n  script:\n\n    - a\n  image: x\n    - b\n", 1),
            ("steps:\n  - command: make\n", 1),
            ("  - name: Test\n    run: |\n      cargo test\n", 1),
            ("  - run: |\n      cargo test\n    name: Test\n", 1),
            ("test:\n  script:\n    - a\n", 1),
            (
                "job:\n  script:\n  - cargo fmt\n  - cargo test\nnext:\n  - x\n",
                2,
            ),
            (
                "  - run: |\n      cargo fmt\n      # why\n      cargo test\n",
                2,
            ),
            ("  - run: |\n      # why\n      cargo test\n", 1),
            ("  - run: | # checks\n      a\n      b\n", 2),
            ("test:\n  script:\n    # why\n    - a\n    - b\n", 2),
            (
                "steps:\n  - run: |\n      cargo fmt\n      cargo test\n  - name: x\n    run: cargo clippy\n",
                3,
            ),
            (
                "test:\n  script: |\n    a\n    b\n  after_script:\n    - c\n",
                3,
            ),
            ("  - run: >-\n      a\n    name: n\n  - run: b\n", 2),
            (
                "  - id: tests\n    name: tests\n    run: cargo test\n  - if: x\n    uses: a\n",
                2,
            ),
            ("    sh 'make test'\n    stage('Test') {\n", 2),
            ("runs-on: ubuntu-latest\n# - run: old\n", 0),
            ("", 0),
        ];
        for (text, want) in counts {
            assert_eq!(steps(text), want, "{text:?}");
        }
        let unblocks = [
            ("        continue-on-error: true", true),
            ("        continue-on-error: ${{ true }}", true),
            ("  allow_failure: true", true),
            ("  allow_failure:", true),
            ("  allow_failure: ${{ x }}", true),
            ("  soft_fail: true", true),
            ("      - run: cargo test || true", true),
            ("      - run: cargo test || exit 0", true),
            ("      - run: cargo test || :", true),
            ("        run: set +e; make", true),
            ("        if: false", true),
            ("        if: ${{ false }}", true),
            ("      - if: false", true),
            ("  - when: manual", true),
            ("  when: manual", true),
            ("        catchError(buildResult: 'SUCCESS') {", true),
            ("        continue-on-error: false", false),
            ("  allow_failure: false", false),
            ("  allow_failure: { exit_codes: 137 }", false),
            ("      - run: cargo test || echo failed", false),
            ("        if: ${{ github.event_name == 'push' }}", false),
            ("  when: on_success", false),
            ("        set -e", false),
            ("        run: echo '|| true is not run'", true),
        ];
        for (line, want) in unblocks {
            assert_eq!(unblock(line), want, "{line:?}");
        }
    }

    fn change(path: &str, from: Option<&str>, old: &str, new: Option<&str>) -> Change {
        Change {
            path: path.into(),
            from: from.map(str::to_owned),
            old: Some(Blob::Text(old.into())),
            new: new.map(|s| Blob::Text(s.into())),
        }
    }

    fn run(change: &Change) -> Vec<(u64, String)> {
        check(&[View::of(change)])
            .into_iter()
            .map(|f| (f.line, f.message))
            .collect()
    }

    #[test]
    fn fewer_steps_block_and_edited_or_moved_steps_do_not() {
        let old = "steps:\n  - run: a\n  - run: b\n  - run: c\n";
        let rows: &[(Change, &[(u64, &str)])] = &[
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    old,
                    Some("steps:\n  - run: b\n  - run: a\n    continue-on-error: true\n"),
                ),
                &[
                    (0, "CI steps: 3 before, 2 after"),
                    (4, "CI step may fail `continue-on-error: true`"),
                ],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    old,
                    Some("steps:\n  - run: c\n  - run: b V=1\n  - uses: a@v4\n"),
                ),
                &[],
            ),
            (
                change(".github/workflows/ci.yml", None, old, None),
                &[(0, "CI steps: 3 before, 0 after")],
            ),
            (
                change(
                    ".gitlab-ci.yml.bak",
                    Some(".gitlab-ci.yml"),
                    "t:\n  script:\n    - cargo test\n",
                    Some("t:\n  script:\n    - cargo test\n"),
                ),
                &[(0, "CI steps: 1 before, 0 after")],
            ),
            (
                change(
                    ".gitlab-ci.yml",
                    None,
                    "t:\n  script:\n    - cargo test\n    - cargo clippy\n",
                    Some("t:\n  script:\n    - cargo test\n  when: manual\n"),
                ),
                &[
                    (0, "CI steps: 2 before, 1 after"),
                    (4, "CI step may fail `when: manual`"),
                ],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: |\n      a\n      b\n",
                    Some("steps:\n  - run: |\n      a\n"),
                ),
                &[(0, "CI steps: 2 before, 1 after")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - name: tests\n    run: cargo test\n",
                    Some("steps:\n  - id: tests\n    name: tests\n    run: cargo test\n"),
                ),
                &[],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - name: Test\n    run: |\n      cargo test\n",
                    Some("steps:\n  - run: |\n      cargo test\n    name: Test\n"),
                ),
                &[],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - if: true\n    run: cargo test\n",
                    Some("steps:\n  - if: false\n    run: cargo test\n"),
                ),
                &[(2, "CI step may fail `- if: false`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: |\n      a\n      # c\n      b\n",
                    Some("steps:\n  - run: |\n      a\n      b\n"),
                ),
                &[],
            ),
            (
                change(
                    ".gitlab-ci.yml",
                    None,
                    "job:\n  script:\n  - a\n  - b\n",
                    Some("job:\n  script:\n  - a\n"),
                ),
                &[(0, "CI steps: 2 before, 1 after")],
            ),
            (change("docs/ci.yml", None, old, Some("")), &[]),
        ];
        table(rows);
    }

    #[test]
    fn a_comment_hides_an_unblock_only_at_yaml_level() {
        let old = "steps:\n  - run: a\n  - run: b\n  - run: c\n";
        let rows: &[(Change, &[(u64, &str)])] = &[
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    old,
                    Some(
                        "steps:\n  - run: a\n  - run: b\n  - run: c\n  # never continue-on-error: true\n",
                    ),
                ),
                &[],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    old,
                    Some("steps:\n  - run: a\n  - run: b\n  - run: |\n      /opt/c.sh || true\n"),
                ),
                &[(5, "CI step may fail `/opt/c.sh || true`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: |\n      cargo test\n",
                    Some("steps:\n  - run: |\n      //usr/bin/env cargo test || true\n"),
                ),
                &[(3, "CI step may fail `//usr/bin/env cargo test || true`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: |\n      cargo test\n",
                    Some("steps:\n  - run: |\n      bash -c 'cargo test\n      #' || true\n"),
                ),
                &[(4, "CI step may fail `#' || true`")],
            ),
            (
                change(
                    "Jenkinsfile",
                    None,
                    "stage('t') {\n  sh 'cargo test'\n}\n",
                    Some("stage('t') {\n  // sh 'cargo test || true'\n  sh 'cargo test'\n}\n"),
                ),
                &[],
            ),
            (
                change(
                    "Jenkinsfile",
                    None,
                    "stage('t') {\n  sh '''\n    cargo test\n  '''\n}\n",
                    Some(
                        "stage('t') {\n  sh '''\n    cargo test\n    //usr/bin/env true || true\n  '''\n}\n",
                    ),
                ),
                &[(4, "CI step may fail `//usr/bin/env true || true`")],
            ),
            (
                change(
                    ".gitlab-ci.yml",
                    None,
                    "job:\n  script:\n    - |\n      cargo test\n",
                    Some("job:\n  script:\n    - |\n      bash -c 'cargo test\n      #' || true\n"),
                ),
                &[(5, "CI step may fail `#' || true`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: |\n     # x\n       cargo test\n",
                    Some(
                        "steps:\n  - run: |\n     # x\n       bash -c 'cargo test\n     #' || true\n",
                    ),
                ),
                &[(5, "CI step may fail `#' || true`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: |2\n        cargo test\n",
                    Some("steps:\n  - run: |2\n        bash -c 'cargo test\n      #' || true\n"),
                ),
                &[(4, "CI step may fail `#' || true`")],
            ),
            (
                change(
                    "azure-pipelines.yml",
                    None,
                    "steps:\n  - bash: |\n      cargo test\n",
                    Some("steps:\n  - bash: |\n      bash -c 'cargo test\n      #' || true\n"),
                ),
                &[(4, "CI step may fail `#' || true`")],
            ),
        ];
        table(rows);
    }

    #[test]
    fn a_quoted_scalar_continuing_onto_a_hash_line_is_read() {
        let rows: &[(Change, &[(u64, &str)])] = &[
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: cargo test\n",
                    Some("steps:\n  - run: \"bash -c 'cargo test\n      #' || true\"\n"),
                ),
                &[(3, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: cargo test\n",
                    Some("steps:\n  - run: 'bash -c \"cargo test\n      #\" || true'\n"),
                ),
                &[(3, "CI step may fail `#\" || true'`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: cargo test\n",
                    Some("steps:\n  - run: \"echo \\\"a\n      #\\\" || true\"\n"),
                ),
                &[(3, "CI step may fail `#\\\" || true\"`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: cargo test\n",
                    Some("steps:\n  - run: 'it''s\n      #'' || true'\n"),
                ),
                &[(3, "CI step may fail `#'' || true'`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: cargo test\n",
                    Some("steps:\n  - run: \"cargo test\"\n  # never || true\n"),
                ),
                &[],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: |\n      \"a\n",
                    Some("steps:\n  - run: |\n      \"a\n  # never || true\n"),
                ),
                &[],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - name: Check\n      'static lifetimes\n    run: cargo test\n",
                    Some(
                        "steps:\n  - name: Check\n      'static lifetimes\n    run: cargo test\n  # never || true\n",
                    ),
                ),
                &[],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - \"run\": cargo test\n",
                    Some("steps:\n  - \"run\": \"bash -c 'cargo test\n      #' || true\"\n"),
                ),
                &[(3, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - uses: a/b@v1\n    with:\n      message: |\n        \"Nightly build\n        is green\n  - run: cargo test\n",
                    Some(
                        "steps:\n  - uses: a/b@v1\n    with:\n      message: |\n        \"Nightly build\n        is green\n  # - run: cargo test || true\n  - run: cargo test\n",
                    ),
                ),
                &[],
            ),
        ];
        table(rows);
    }

    #[test]
    fn a_quoted_value_below_a_bare_key_or_dash_opens() {
        let rows: &[(Change, &[(u64, &str)])] = &[
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: cargo test\n",
                    Some("steps:\n  - run: &cmd \"bash -c 'exit 1\n      #' || true\"\n"),
                ),
                &[(3, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: exit 1\n",
                    Some("steps:\n  - run:\n      \"bash -c 'exit 1\n      #' || true\"\n"),
                ),
                &[(4, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: exit 1\n",
                    Some("steps:\n  - run: &c\n\n      \"bash -c 'exit 1\n      #' || true\"\n"),
                ),
                &[(5, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: exit 1\n",
                    Some(
                        "steps:\n  - run:\n      # why\n      \"bash -c 'exit 1\n      #' || true\"\n",
                    ),
                ),
                &[(5, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: exit 1\n",
                    Some(
                        "steps:\n  - run:\n      !!str\n      \"bash -c 'exit 1\n      #' || true\"\n",
                    ),
                ),
                &[(5, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".github/workflows/ci.yml",
                    None,
                    "steps:\n  - run: exit 1\n",
                    Some(
                        "steps:\n  - name: t\n    env:\n    \"run\": \"bash -c 'exit 1\n      #' || true\"\n",
                    ),
                ),
                &[(5, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".gitlab-ci.yml",
                    None,
                    "job:\n  script:\n    - exit 1\n",
                    Some(
                        "job:\n  script:\n    - exit 1\n    -\n      \"bash -c 'exit 1\n      #' || true\"\n",
                    ),
                ),
                &[(6, "CI step may fail `#' || true\"`")],
            ),
            (
                change(
                    ".gitlab-ci.yml",
                    None,
                    "job:\n  script:\n    - exit 1\n",
                    Some(
                        "job:\n  script:\n    - exit 1\n    - !!str\n      \"bash -c 'exit 1\n      #' || true\"\n",
                    ),
                ),
                &[(6, "CI step may fail `#' || true\"`")],
            ),
        ];
        table(rows);
    }

    fn table(rows: &[(Change, &[(u64, &str)])]) {
        for (change, want) in rows {
            let want: Vec<(u64, String)> =
                want.iter().map(|(l, m)| (*l, (*m).to_owned())).collect();
            assert_eq!(run(change), want, "{change:?}");
        }
    }
}
