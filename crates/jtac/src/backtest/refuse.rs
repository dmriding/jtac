//! What `jtac backtest` refuses (DESIGN.md sections 11.10 and 12), each with exit 2: no
//! policy, a policy that does not read, a rows file that exists or cannot be written, and
//! history this clone cannot read or diff. Every one names `jtac backtest` with the range
//! filled in and the policy it actually read, never the gate's file, commit or command.

use std::path::Path;

use jtac_checks::diff::git::{GitError, quoted};
use jtac_core::GitOid;
use jtac_policy::{POLICY_PATH, PolicyError};

use super::policy::Source;
use crate::init::ask;
use crate::refuse::Refusal;
use crate::repo::RepoError;

const OWN_POLICY: &str = "../backtest-policy.toml";

fn shown(range: &[String]) -> String {
    let words: Vec<String> = range.iter().map(|w| quoted(w, "RANGE")).collect();
    words.join(" ")
}

fn file(path: &Path) -> String {
    quoted(&path.to_string_lossy(), "FILE")
}

fn named(source: &Source) -> String {
    match source {
        Source::File(path) => format!("the `--policy` file `{}`", file(path)),
        Source::Newest(commit) => format!(
            "`{POLICY_PATH}` in the range's newest commit `{}`",
            commit.hex()
        ),
    }
}

impl Refusal {
    /// git could not answer about the repository.
    pub(crate) fn backtest_repository(err: &RepoError, range: &[String]) -> Self {
        let range = shown(range);
        let first = match err {
            RepoError::Git(GitError::Spawn { .. }) => "Install git and put it on PATH",
            _ => "Run `git status` here; once git answers",
        };
        ask(
            err.to_string(),
            "A report over history git cannot read would count what it could not see as no \
             finding, so backtest refuses rather than print a rate.",
            format!("{first}, then run `jtac backtest {range}` again."),
        )
    }

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
    pub(crate) fn backtest_policy_unread(source: &Source, detail: &str, range: &[String]) -> Self {
        let range = shown(range);
        ask(
            format!("{} could not be read: {detail}.", named(source)),
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
    pub(crate) fn backtest_policy(source: &Source, err: &PolicyError, range: &[String]) -> Self {
        let range = shown(range);
        match source {
            Source::File(path) => {
                let file = file(path);
                let [what, why, fix] = err.parts_in(&file);
                ask(
                    format!("the `--policy` file was refused: {what}"),
                    why,
                    format!("{fix}, then run `jtac backtest --policy {file} {range}` again."),
                )
            }
            Source::Newest(commit) => {
                let hex = commit.hex();
                let [what, why, _] = err.parts_in(POLICY_PATH);
                let [_, _, fix] = err.parts_in(OWN_POLICY);
                ask(
                    format!(
                        "the range's newest commit `{hex}` holds a policy this build refuses: \
                         {what}"
                    ),
                    why,
                    format!(
                        "That commit's policy cannot be edited and the work tree's is not \
                         read: write the output of `git show {hex}:{POLICY_PATH}` to \
                         `{OWN_POLICY}`, {fix}, then run `jtac backtest --policy {OWN_POLICY} \
                         {range}`."
                    ),
                )
            }
        }
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
        let own = Source::File("/workspace/own.toml".into());
        let newest = Source::Newest(oid);
        let spawn = RepoError::Git(GitError::Spawn {
            argv: vec!["git".into()],
            source: std::io::ErrorKind::NotFound.into(),
        });
        let failed = RepoError::Git(GitError::Failed {
            argv: vec!["git".into(), "rev-parse".into()],
            status: Some(128),
            stderr: "fatal: not a git repository".into(),
        });
        let rows = [
            Refusal::backtest_no_policy(&range),
            Refusal::backtest_policy_unread(&own, "not UTF-8", &range),
            Refusal::backtest_policy_unread(&newest, "it is a link", &range),
            Refusal::backtest_policy(&own, &parse, &range),
            Refusal::backtest_policy(&newest, &parse, &range),
            Refusal::backtest_repository(&spawn, &range),
            Refusal::backtest_repository(&failed, &range),
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
            assert!(
                !r.text().contains("jtac check"),
                "{r:?}: the gate's command"
            );
            assert!(!r.text().contains("HEAD is"), "{r:?}: the gate's commit");
        }
        assert!(rows[0].how.contains("jtac init --dry-run"));
    }

    #[test]
    fn a_refused_policy_names_the_file_it_was_read_from_and_the_edit_that_fixes_it() {
        let range = ["main".to_owned()];
        let oid: GitOid = "sha1:0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let parse = jtac_policy::parse("version = 1\nunknown = 1\n").unwrap_err();
        let own = Refusal::backtest_policy(&Source::File("own.toml".into()), &parse, &range);
        assert!(
            own.what
                .starts_with("the `--policy` file was refused: own.toml line 2, column 1: "),
            "{}",
            own.what
        );
        assert_eq!(
            own.how,
            "edit own.toml line 2, column 1, then run `jtac backtest --policy own.toml main` \
             again."
        );
        assert!(!own.text().contains(POLICY_PATH), "{own:?}");
        let newest = Refusal::backtest_policy(&Source::Newest(oid), &parse, &range);
        let hex = oid.hex();
        assert!(
            newest.what.starts_with(&format!(
                "the range's newest commit `{hex}` holds a policy this build refuses: \
                 {POLICY_PATH} line 2, column 1: "
            )),
            "{}",
            newest.what
        );
        assert!(
            newest.how.contains(&format!(
                "`git show {hex}:{POLICY_PATH}` to `{OWN_POLICY}`, edit {OWN_POLICY} line 2, \
                 column 1, then run `jtac backtest --policy {OWN_POLICY} main`."
            )),
            "{}",
            newest.how
        );
    }
}
