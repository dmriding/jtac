//! What the CLI refuses and how it says so: what, why, how (DESIGN.md section 12), on stderr
//! with exit 2. A refusal is never a verdict: the gate could not answer. Every refusal here
//! is a negotiation; fences arrive with the hooks.

use jtac_checks::diff::git::GitError;
use jtac_policy::{POLICY_PATH, PolicyError};
use serde::Serialize;

use crate::repo::RepoError;

/// The three parts, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Refusal {
    /// `negotiation`: there is a path to yes, and `how` is it.
    pub kind: &'static str,
    /// What happened or was refused.
    pub what: String,
    /// Why the rule exists.
    pub why: String,
    /// The exact next command.
    pub how: String,
}

const FAIL_CLOSED: &str = "A gate that cannot read the repository refuses rather than passes: \
                           a check that did not run must never look like one that passed.";

impl Refusal {
    fn new(what: impl Into<String>, why: impl Into<String>, how: impl Into<String>) -> Self {
        Self {
            kind: "negotiation",
            what: what.into(),
            why: why.into(),
            how: how.into(),
        }
    }

    /// A command a later release ships.
    pub fn not_built(command: &str) -> Self {
        Self::new(
            format!("`{command}` is not a command in this release."),
            format!(
                "jtac {} ships `check`, `status`, and `why`; exiting zero on anything else \
                 would let a hook treat \"not built\" as \"passed\".",
                env!("CARGO_PKG_VERSION")
            ),
            "Run `jtac check` for a verdict, `jtac check --preview` for one that writes \
             nothing, or `jtac --help`.",
        )
    }

    /// The directory is not inside a git work tree, or git could not answer.
    pub fn repository(err: &RepoError) -> Self {
        let how = match err {
            RepoError::Git(GitError::Spawn { .. }) => {
                "Install git and put it on PATH, then run `jtac check` again.".to_owned()
            }
            RepoError::NotAFile { path } => {
                format!("Make `{path}` a regular file, commit, and run `jtac check` again.")
            }
            RepoError::Shallow { .. } => {
                "Run `git fetch --unshallow` (or `git fetch --deepen=1` for one more commit), \
                 then `jtac check` again; do not pass `--base HEAD` to skip the \
                 history, that judges nothing."
                    .to_owned()
            }
            _ => "Run `git status` here; once git answers, run `jtac check` again.".to_owned(),
        };
        Self::new(err.to_string(), FAIL_CLOSED, how)
    }

    /// HEAD could not be read: no commit yet, or not a commit.
    pub fn no_head(err: &RepoError) -> Self {
        Self::new(
            err.to_string(),
            "The gate judges a commit and its tree; before the first commit there is nothing \
             to judge, and preview never invents a tree for uncommitted files.",
            "Make a commit on this branch, then run `jtac check` again.",
        )
    }

    /// `--base` named something git could not resolve or share history with HEAD.
    pub fn base(rev: &str, err: &RepoError) -> Self {
        Self::new(
            format!("`--base {rev}`: {err}"),
            "The diff-integrity rules read the commits between the base and HEAD, so the base \
             must be a commit that shares history with HEAD.",
            "Run `jtac check --base HEAD~1` to judge the last commit, or name a \
             branch this one forked from; `--base HEAD` judges the last commit too, never \
             nothing.",
        )
    }

    /// `jtac check` is running inside a policy check.
    pub fn nested() -> Self {
        Self::new(
            format!(
                "`jtac check` is running inside a jtac check: `{}=1` is set.",
                jtac_checks::NESTED_VAR
            ),
            "A policy check that runs the gate would run the gate's checks, which run it again, \
             until the timeout kills the tree; the runner marks every check's environment so \
             the inner run stops here.",
            format!(
                "Remove the check that runs `jtac` from `{POLICY_PATH}` and commit, or unset \
                 `{}` in the shell that exported it, then run `jtac check`.",
                jtac_checks::NESTED_VAR
            ),
        )
    }

    /// No policy file in the tree under test.
    pub fn no_policy() -> Self {
        Self::new(
            format!("HEAD has no `{POLICY_PATH}`."),
            "Built-in defaults are what `jtac init` writes, never what `check` assumes: a gate \
             that judged by rules nobody chose would be a gate nobody can answer for. The tree \
             under test carries its own rules, so an uncommitted file does not count.",
            format!(
                "Write `{POLICY_PATH}` in the shape DESIGN.md section 10.2 shows, commit it, and \
                 run `jtac check` again. `jtac init` writes one from detection in a \
                 later release."
            ),
        )
    }

    /// The policy file is not one this build runs.
    pub fn policy(err: &PolicyError) -> Self {
        let [what, why, how] = err.three_parts();
        Self::new(
            format!("{POLICY_PATH} at HEAD: {what}"),
            why,
            format!("{how}, commit it, and run `jtac check` again."),
        )
    }

    /// The policy blob is not UTF-8.
    pub fn policy_bytes() -> Self {
        Self::new(
            format!("`{POLICY_PATH}` at HEAD is not UTF-8."),
            "A policy that does not parse denies and never falls back to defaults.",
            format!("Rewrite `{POLICY_PATH}` as UTF-8 TOML, commit, and run `jtac check`."),
        )
    }

    /// `check` on a work tree that differs from HEAD.
    pub fn dirty(why: &str) -> Self {
        Self::new(
            format!("the work tree is not HEAD's tree: {why}."),
            "A root vouches for HEAD's tree, and the checks run on the work tree; with the two \
             apart, a pass would be evidence about a tree nobody committed.",
            "Commit or stash the changes (`git stash -u`), clear the bit (`git update-index \
             --no-skip-worktree --no-assume-unchanged <path>`), or move the pattern from the \
             local exclude into `.gitignore` and commit; then run `jtac check` again, or `jtac \
             check --preview` for a verdict that writes nothing.",
        )
    }

    /// No signing key, or no principal, in git config.
    pub fn no_identity(err: &jtac_sign::ResolveError) -> Self {
        let how = match err {
            jtac_sign::ResolveError::NoKey => {
                "Run `git config jtac.signingkey <path to an SSH private key>` (or sign commits \
                 with SSH: `git config gpg.format ssh` and `user.signingkey`), then `jtac check`; \
                 `jtac init` generates a key in a later release."
            }
            jtac_sign::ResolveError::NoPrincipal => {
                "Run `git config user.email <you@example.com>`, then `jtac check`."
            }
        };
        Self::new(
            format!("cannot sign: {err}."),
            "A root is signed so a reader can tell whose gate ran; without a key there is \
             nothing to write, and the checks are not run for a root that cannot exist.",
            how,
        )
    }

    /// ssh-keygen could not run, read the key, or signed with another key.
    pub fn sign(err: &jtac_sign::SignError) -> Self {
        Self::new(
            format!("cannot sign: {err}."),
            "jtac never prompts for a passphrase and never writes a root it could not sign; a \
             key that needs one must be in the agent and named by its `.pub` file.",
            "Fix the key git config names (`git config jtac.signingkey`), or add it to the \
             agent (`ssh-add <key>`), then run `jtac check` again.",
        )
    }

    /// The local store could not be read or written.
    pub fn store(err: impl std::fmt::Display) -> Self {
        Self::new(
            format!("the local store could not answer: {err}."),
            "Evidence that cannot be written is a check that did not happen, and a store that \
             cannot be read may hold a root this run would contradict.",
            "Check free space and permissions under `.git/jtac/`, then run `jtac check` again.",
        )
    }

    /// This binary could not be read to digest it.
    pub fn exe(err: &std::io::Error) -> Self {
        Self::new(
            format!("cannot read the jtac binary to name it in the root: {err}."),
            "A root names the gate build that wrote it by digest, so a reader can tell which \
             rules ran.",
            "Run `jtac check` from an installed binary on a readable path.",
        )
    }

    /// `why` before any check.
    pub fn no_state() -> Self {
        Self::new(
            "no verdict has been recorded in this clone.",
            "`jtac why` explains the last `jtac check`; nothing has run here yet.",
            "Run `jtac check`, then `jtac why`.",
        )
    }

    /// `state.json` this build cannot read.
    pub fn stale_state() -> Self {
        Self::new(
            "the recorded verdict was written in a shape this build does not read.",
            "A verdict shown from a file another version wrote could name reasons this \
             build would not; the file is per clone and is replaced by the next check.",
            "Run `jtac check` again, then `jtac why`.",
        )
    }

    /// Something this binary should have been able to do, and could not.
    pub fn internal(what: impl std::fmt::Display) -> Self {
        Self::new(
            format!("jtac could not finish: {what}"),
            FAIL_CLOSED,
            "Run `jtac check` again; if it repeats, report it with this output.",
        )
    }

    /// clap refused the arguments.
    pub fn arguments(err: &clap::Error) -> Self {
        let what = err
            .to_string()
            .lines()
            .next()
            .unwrap_or_default()
            .trim_start_matches("error: ")
            .to_owned();
        Self::new(
            what,
            "jtac takes only the flags it documents, so a misspelled flag cannot silently change \
             what a verdict judged.",
            "Run `jtac check --help` for the flags this release takes.",
        )
    }

    /// The text for stderr: what, why, then the kind and the next command.
    pub fn text(&self) -> String {
        format!(
            "jtac: {}\n{}\n{}: {}\n",
            self.what, self.why, self.kind, self.how
        )
    }

    /// The same three parts for `--json`, under `error`.
    pub fn json(&self) -> String {
        #[derive(Serialize)]
        struct Wire<'a> {
            v: u32,
            error: &'a Refusal,
        }
        let mut text = serde_json::to_string_pretty(&Wire { v: 1, error: self })
            .expect("a refusal is three strings");
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refusal_has_three_non_empty_parts_and_names_the_next_command() {
        let parse = jtac_policy::parse("version = 2\n").unwrap_err();
        let spawn = RepoError::Git(GitError::Spawn {
            argv: vec!["git".into()],
            source: std::io::Error::other("nope"),
        });
        let failed = RepoError::Git(GitError::Failed {
            argv: vec!["git".into(), "status".into()],
            status: Some(128),
            stderr: "fatal: not a git repository".into(),
        });
        use clap::Parser;
        let args = crate::cli::Cli::try_parse_from(["jtac", "check", "--nope"]).unwrap_err();
        let rows = [
            Refusal::not_built("status"),
            Refusal::dirty("x"),
            Refusal::no_identity(&jtac_sign::ResolveError::NoKey),
            Refusal::no_identity(&jtac_sign::ResolveError::NoPrincipal),
            Refusal::sign(&jtac_sign::SignError::KeyMismatch {
                expected: "a".into(),
                found: "b".into(),
            }),
            Refusal::store("disk full"),
            Refusal::exe(&std::io::Error::other("gone")),
            Refusal::no_state(),
            Refusal::stale_state(),
            Refusal::repository(&spawn),
            Refusal::repository(&failed),
            Refusal::no_head(&failed),
            Refusal::base("nope", &failed),
            Refusal::nested(),
            Refusal::no_policy(),
            Refusal::policy(&parse),
            Refusal::policy_bytes(),
            Refusal::internal("hash"),
            Refusal::arguments(&args),
        ];
        for r in &rows {
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(r.how.contains("jtac ") || r.how.contains("git "), "{r:?}");
            assert!(r.text().starts_with("jtac: "), "{r:?}");
            assert!(r.text().contains("\nnegotiation: "), "{r:?}");
            assert!(!r.how.contains("<rev>"), "{r:?}: unfilled value");
            assert!(r.json().contains("\"error\""), "{r:?}");
        }
        assert!(Refusal::repository(&spawn).how.contains("PATH"));
        let link = RepoError::NotAFile { path: "x".into() };
        assert!(Refusal::repository(&link).how.contains("`x`"));
        let shallow = RepoError::Shallow { what: "x".into() };
        assert!(Refusal::repository(&shallow).how.contains("--unshallow"));
        assert!(Refusal::policy(&parse).how.contains("commit it"));
        assert!(Refusal::arguments(&args).what.contains("--nope"));
    }
}
