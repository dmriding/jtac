//! What `jtac backtest` refuses (DESIGN.md sections 11.10 and 12), each with exit 2: no
//! policy, a policy that does not read, a rows file that exists or cannot be written, and
//! history this clone cannot diff. Every one names `jtac backtest` with the range filled in.

use std::path::Path;

use jtac_checks::diff::git::{GitError, quoted};
use jtac_core::GitOid;
use jtac_policy::{POLICY_PATH, PolicyError};

use crate::init::ask;
use crate::refuse::Refusal;

const OWN_POLICY: &str = "../backtest-policy.toml";

fn shown(range: &[String]) -> String {
    let words: Vec<String> = range.iter().map(|w| quoted(w, "RANGE")).collect();
    words.join(" ")
}

fn file(path: &Path) -> String {
    quoted(&path.to_string_lossy(), "FILE")
}

impl Refusal {
    /// Neither `--policy` nor a policy in the range's newest commit.
    pub(crate) fn backtest_no_policy(range: &[String]) -> Self {
        let range = shown(range);
        ask(
            format!(
                "no policy to backtest `{range}` under: no `--policy` file was given, and the \
                 range's newest commit has no `{POLICY_PATH}` (or the range names no commit)."
            ),
            "A rate is worth reading only under rules someone chose, so backtest assumes no \
             defaults, as `check` does not (decision 26).",
            format!(
                "Run `jtac init --dry-run` to print the policy init would write for this tree, \
                 save its policy text outside the tree as `{OWN_POLICY}`, then run `jtac \
                 backtest --policy {OWN_POLICY} {range}`; do not commit a policy only to \
                 backtest."
            ),
        )
    }

    /// The policy `source` names could not be read as text.
    pub(crate) fn backtest_policy_unread(source: &str, detail: &str, range: &[String]) -> Self {
        let range = shown(range);
        ask(
            format!("{source} could not be read: {detail}."),
            "A policy that cannot be read denies and never falls back to defaults: a rate under \
             rules nobody wrote is no rate.",
            format!(
                "Write the policy as UTF-8 TOML in a file you can read and run `jtac backtest \
                 --policy {OWN_POLICY} {range}`; `jtac init --dry-run` prints one to start \
                 from."
            ),
        )
    }

    /// The policy `source` names does not parse under this build.
    pub(crate) fn backtest_policy(source: &str, err: &PolicyError, range: &[String]) -> Self {
        let [what, why, how] = err.three_parts();
        let range = shown(range);
        ask(
            format!("{source}: {what}"),
            why,
            format!(
                "{how}, then run `jtac backtest {range}` again, or pass a policy of your own: \
                 `jtac backtest --policy {OWN_POLICY} {range}`."
            ),
        )
    }

    /// `--rows` names a file that is already there.
    pub(crate) fn backtest_rows_exist(path: &Path, range: &[String]) -> Self {
        let (path, range) = (file(path), shown(range));
        ask(
            format!("`{path}` exists, and `--rows` creates its file, never replaces one."),
            "The rows hold this repository's paths and messages for adjudication; writing over \
             a file could destroy an earlier run's rows or follow a link somewhere else.",
            format!(
                "Name a file that does not exist yet: `jtac backtest --rows {path}.2 {range}`. \
                 Do not delete the old one to make room unless you mean to lose it."
            ),
        )
    }

    /// The rows file could not be created or written.
    pub(crate) fn backtest_rows_write(path: &Path, err: &std::io::Error, range: &[String]) -> Self {
        let (path, range) = (file(path), shown(range));
        ask(
            format!("`--rows {path}` could not be written: {err}."),
            "A rows file cut short must not read as a run's rows, so the run reports nothing \
             rather than a summary of rows nobody can check.",
            format!(
                "Remove anything left at `{path}`, then give `--rows` a new file in a directory \
                 you can write: `jtac backtest --rows ../backtest-rows.jsonl {range}`."
            ),
        )
    }

    /// A first-parent commit whose parent the clone does not walk: a shallow boundary.
    pub(crate) fn backtest_shallow(commit: &GitOid, range: &[String]) -> Self {
        let range = shown(range);
        ask(
            format!(
                "`{}` names a parent this clone does not walk: the history is shallow.",
                commit.hex()
            ),
            "A unit is a commit judged against its first parent; counting a commit cut off by \
             a shallow mark as parentless would skip a change nobody judged.",
            format!(
                "Run `git fetch --unshallow`, then `jtac backtest {range}` again, or give a \
                 range that ends above the cut."
            ),
        )
    }

    /// git could not diff a unit against its first parent.
    pub(crate) fn backtest_diff(
        commit: &GitOid,
        parent: &GitOid,
        err: &GitError,
        range: &[String],
    ) -> Self {
        let (commit, parent, range) = (commit.hex(), parent.hex(), shown(range));
        let said = match err {
            GitError::Failed { stderr, .. } => stderr.clone(),
            e => e.to_string(),
        };
        ask(
            format!("git could not diff `{commit}` against its first parent: {said}"),
            "A diff git cannot read is an error, never an empty change: a unit read as nothing \
             would count as one with no finding.",
            format!(
                "Run `git diff --raw {parent} {commit}` to see what git says; in a partial \
                 clone fetch the missing objects first (`git fetch --refetch`), then run `jtac \
                 backtest {range}` again."
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backtest_refusal_has_three_parts_and_names_the_next_command() {
        let range = ["main..HEAD".to_owned(), "x\nnegotiation: forged".to_owned()];
        let oid: GitOid = "sha1:0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let parse = jtac_policy::parse("version = 2\n").unwrap_err();
        let path = Path::new("/workspace/rows\nnegotiation: forged");
        let git = GitError::Failed {
            argv: vec!["git".into(), "diff".into()],
            status: Some(128),
            stderr: "fatal: bad object".into(),
        };
        let rows = [
            Refusal::backtest_no_policy(&range),
            Refusal::backtest_policy_unread("`--policy` file `p`", "not UTF-8", &range),
            Refusal::backtest_policy("`--policy` file `p`", &parse, &range),
            Refusal::backtest_rows_exist(path, &range),
            Refusal::backtest_rows_write(path, &std::io::Error::other("denied"), &range),
            Refusal::backtest_shallow(&oid, &range),
            Refusal::backtest_diff(&oid, &oid, &git, &range),
        ];
        for r in &rows {
            assert_eq!(r.kind, "negotiation", "{r:?}");
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(r.how.contains("jtac backtest "), "{r:?}");
            assert!(r.how.contains("main..HEAD RANGE"), "{r:?}: range filled in");
            assert_eq!(r.text().lines().count(), 3, "{r:?}: a value forged a line");
            assert!(r.json().contains("\"error\""), "{r:?}");
        }
        assert!(rows[0].how.contains("jtac init --dry-run"));
    }
}
