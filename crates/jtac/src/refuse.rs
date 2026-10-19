//! What the CLI refuses and how it says so: what, why, how (DESIGN.md section 12), on stderr
//! with exit 2. A refusal is never a verdict: the gate could not answer. Every refusal here
//! is a negotiation; the hooks build the fences with `fence`.

use jtac_checks::diff::git::{GitError, quoted};
use jtac_policy::{POLICY_PATH, PolicyError};
use jtac_sign::TRUST_CAP;
use serde::Serialize;

use crate::repo::RepoError;
use crate::trust::TRUST_PATH;

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

    /// A fence: no command clears it from here. `how` says what would, elsewhere.
    pub fn fence(what: impl Into<String>, why: impl Into<String>, how: impl Into<String>) -> Self {
        Self {
            kind: "fence",
            ..Self::new(what, why, how)
        }
    }

    /// A command a later release ships.
    pub fn not_built(command: &str) -> Self {
        Self::new(
            format!("`{command}` is not a command in this release."),
            format!(
                "jtac {} ships `init`, `check`, `status`, `why`, `verify`, `replay`, `audit`, `show`, \
                 `waive`, `bench`, and `hook`; exiting zero on anything else would let a hook treat \"not \
                 built\" as \"passed\".",
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
                "Run `jtac init` to write `{POLICY_PATH}` from what the tree holds, commit it, \
                 and run `jtac check` again."
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

    /// The repository has a grafts file.
    pub fn grafted(path: &std::path::Path) -> Self {
        let path = path.to_string_lossy().replace('\\', "/");
        Self::new(
            format!("the repository has a grafts file, `{path}`."),
            "A graft changes the parents git reports, so the base, the diff, and `covers` \
             would describe a history the root does not name; replace refs are ignored for \
             the same reason, but a grafts file cannot be.",
            format!(
                "Remove `{path}` (`git replace --convert-graft-file` keeps it as replace refs, \
                 which jtac ignores; unset `GIT_GRAFT_FILE` if that named it), then run the \
                 command again."
            ),
        )
    }

    /// A writing `check` in a shallow repository.
    pub fn shallow() -> Self {
        Self::new(
            "the history is shallow: `.git/shallow` marks commits whose parents this clone \
             does not walk.",
            "A root's base and `covers` come from walking history, and a shallow mark cuts \
             the walk where git cannot tell a depth-limited clone from a file written by hand; \
             a root here could vouch for commits it never judged.",
            "Run `git fetch --unshallow` (remove `.git/shallow` if it was written by hand), \
             then `jtac check` again; `jtac check --preview` answers here and writes nothing.",
        )
    }

    /// HEAD or the work tree changed between the preflight and the write.
    pub fn moved(dirty: Option<&str>) -> Self {
        let what = match dirty {
            Some(why) => format!("the work tree changed while the checks ran: {why}."),
            None => "HEAD changed while the checks ran.".to_owned(),
        };
        Self::new(
            what,
            "The checks read the work tree as it was during the run; a root and a cache entry \
             name HEAD's tree only when the tree stayed HEAD's from start to end. A check that \
             writes into the tree is the usual cause.",
            "Add what a check writes to `.gitignore` and commit, or leave the tree alone while \
             `jtac check` runs; then run it again.",
        )
    }

    /// No signing key, or no principal, in git config.
    pub fn no_identity(err: &jtac_sign::ResolveError) -> Self {
        let how = match err {
            jtac_sign::ResolveError::NoKey => {
                "Run `jtac init`, which keeps an SSH `user.signingkey` or generates a dedicated \
                 key and names it in `jtac.signingkey`, then `jtac check`."
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

    /// A range git could not list.
    pub fn range(command: &str, specs: &[String], err: &GitError) -> Self {
        let spec: Vec<String> = specs.iter().map(|w| quoted(w, "RANGE")).collect();
        let spec = spec.join(" ");
        let said = match err {
            GitError::Failed { stderr, .. } => stderr.clone(),
            e => e.to_string(),
        };
        Self::new(
            format!("`{spec}` is not a range git can list: {said}"),
            "`verify`, `audit`, and `show` walk exactly the commits git names, so a range git \
             cannot resolve is refused rather than read as empty.",
            format!(
                "Run `git rev-list {spec}` to see what git says, then `jtac {command}` with a \
                 range it lists."
            ),
        )
    }

    /// A rev that does not name a commit. The rev is rendered as a shell word, never raw
    /// (`every_refusal_has_three_non_empty_parts_and_names_the_next_command` forges a
    /// line with one).
    pub fn rev(command: &str, rev: &str) -> Self {
        let rev = quoted(rev, "REV");
        Self::new(
            format!("`{rev}` does not name a commit."),
            format!(
                "`jtac {command}` acts on one commit; a rev git cannot resolve names none, and \
                 guessing one would act on a commit nobody named."
            ),
            format!(
                "Run `git rev-parse {rev}` to see what git says, then `jtac {command}` with a \
                 commit it names."
            ),
        )
    }

    /// `jtac waive` was given neither a commit nor a rule with a path.
    pub fn waive_scope() -> Self {
        Self::new(
            "`jtac waive` needs a scope: a commit, or `--rule` with `--path`.",
            "A waiver with no scope would turn the gate off for everything; every waiver names \
             exactly what it covers, and that is what every reader of the root sees.",
            "Run `jtac waive HEAD --reason <text>` for this commit, or `jtac waive --rule \
             <JT-RULE> --path <prefix> --expires 14d --reason <text>` for one rule under one \
             path.",
        )
    }

    /// `--expires` is missing on a rule waiver, or is not `Nd` with N in 1..=30.
    pub fn waive_expires(rule: &str, path: &str, reason: &str, given: Option<&str>) -> Self {
        let what = match given {
            Some(g) => format!(
                "`--expires {}` is not a number of days from 1 to 30.",
                quoted(g, "DAYS")
            ),
            None => "a rule waiver needs `--expires`.".to_owned(),
        };
        Self::new(
            what,
            "A rule waiver with an unstated life is the silent kind: DESIGN.md decision 12 caps \
             every waiver at 30 days from when it was written, and the verdict refuses a longer \
             one on every machine.",
            format!(
                "Run `jtac waive --rule {} --path {} --expires 14d --reason {}`, with a number \
                 of days up to 30.",
                quoted(rule, "RULE"),
                quoted(path, "PREFIX"),
                quoted(reason, "TEXT")
            ),
        )
    }

    /// `--expires` on a commit waiver.
    pub fn commit_expires(rev: &str, reason: &str) -> Self {
        Self::new(
            "a commit waiver takes no `--expires`.",
            "A commit waiver covers one commit and nothing after it, so it has no life to cap; \
             an expiry on it would read as a promise the verdict does not keep.",
            format!(
                "Run `jtac waive {} --reason {}` without `--expires`, or waive a rule under a \
                 path with `jtac waive --rule <JT-RULE> --path <prefix> --expires 14d --reason {1}`.",
                quoted(rev, "REV"),
                quoted(reason, "TEXT")
            ),
        )
    }

    /// `--reason`, `--rule` or `--path` is blank or holds a control character, which a
    /// terminal would act on.
    pub fn waive_text(flag: &str) -> Self {
        Self::new(
            format!("`{flag}` is blank or holds a control character."),
            "Every word of a waiver travels in the root verbatim and is printed by every \
             reader; a blank one names nothing, and a carriage return or escape in one could \
             overwrite the line that flags the waiver.",
            format!("Run `jtac waive` again with `{flag}` as plain text on one line."),
        )
    }

    /// `--path` has an empty, `.`, or `..` component or a backslash.
    pub fn waive_path(rule: &str, path: &str, reason: &str) -> Self {
        let path = quoted(path, "PREFIX");
        Self::new(
            format!("`--path {path}` is not a clean repository-relative prefix."),
            "A prefix with `..`, `.`, an empty component, or a backslash covers nothing in the \
             verdict (DESIGN.md section 11.2), so the waiver would be recorded and never apply.",
            format!(
                "Run `jtac waive --rule {} --path <dir/under/repo> --expires 14d --reason {}` \
                 with forward slashes and whole components.",
                quoted(rule, "RULE"),
                quoted(reason, "TEXT")
            ),
        )
    }

    /// `--signers` names a file that could not be read as a trust file.
    pub fn signers_file(command: &str, path: &std::path::Path, err: &jtac_sign::SignError) -> Self {
        let path = quoted(&path.to_string_lossy(), "FILE");
        Self::new(
            format!("`--signers` could not be read: {err}."),
            "A trust file that cannot be read must not read as empty trust: a root whose \
             signer nobody could look up would otherwise pass as unknown.",
            format!(
                "Give `--signers` a plain allowed-signers file, or drop it to read `{TRUST_PATH}` \
                 at HEAD: `jtac {command} --signers {path}` named this one."
            ),
        )
    }

    /// `show` with no range and no upstream.
    pub fn no_upstream() -> Self {
        Self::new(
            "HEAD has no upstream, so what the next push carries is not known.",
            "`jtac show` prints the roots a push would send; without an upstream there is no \
             push to describe.",
            "Run `git push -u origin HEAD` once, or `jtac show main..HEAD` with the range to \
             describe.",
        )
    }

    /// `.jtac/signers` at HEAD is not a readable trust file.
    pub fn trust(what: &str) -> Self {
        Self::new(
            format!("`{TRUST_PATH}` at HEAD is {what}."),
            "Verification reads the trust file from HEAD's tree, never the work tree, so a link \
             or an edit there cannot change who is trusted; a file that cannot be read must \
             not read as empty trust.",
            format!(
                "Commit a plain `{TRUST_PATH}` under {TRUST_CAP} bytes, or run `jtac verify \
                 --signers FILE` with a file of your own."
            ),
        )
    }

    /// `replay` on a commit with no root ref.
    pub fn no_root(commit: &str) -> Self {
        Self::new(
            format!("{commit} has no root in this clone."),
            "`jtac replay` recomputes a root from its nodes; a commit without a root has \
             nothing to replay, and a commit covered by a later root is replayed through that \
             root's tip.",
            format!(
                "Run `jtac audit {commit}` to see which root covers it, or `git fetch origin \
                 +refs/jtac/*:refs/jtac/*` if the root was written elsewhere."
            ),
        )
    }

    /// `replay` on a root whose nodes this clone never wrote.
    pub fn not_local(commit: &str, what: &str) -> Self {
        Self::new(
            format!("the root for {commit} verifies, but {what}."),
            "Only roots and signatures travel; the nodes stay on the machine that wrote the \
             root, so replay there is the only full recomputation.",
            format!(
                "Run `jtac verify {commit}` here, or `jtac replay {commit}` on the machine that \
                 ran `jtac check`."
            ),
        )
    }

    /// `replay` could not rebuild the facts: the policy the root names is not here.
    pub fn policy_differs(commit: &str, hash: &str) -> Self {
        Self::new(
            format!(
                "the root for {commit} names policy {hash}, which this clone cannot rebuild: \
                 the policy file at its tree resolves to another hash under this build."
            ),
            "The verdict is a function of the resolved policy; recomputing it under a policy \
             that differs would compare two different questions.",
            format!(
                "Run `jtac replay {commit}` on the machine that wrote the root, where the \
                 resolved policy is stored, or with the jtac release that wrote it."
            ),
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
        let git_failed = || GitError::Failed {
            argv: vec!["git".into(), "rev-list".into()],
            status: Some(128),
            stderr: "fatal: bad revision".into(),
        };
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
            Refusal::grafted(std::path::Path::new("/workspace/.git/info/grafts")),
            Refusal::shallow(),
            Refusal::moved(None),
            Refusal::moved(Some("y")),
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
            Refusal::range("audit", &["main..HEAD".into(), "a b".into()], &git_failed()),
            Refusal::rev("replay", "no such\nnegotiation: forged"),
            Refusal::waive_scope(),
            Refusal::waive_expires("JT-SUP-001", "src", "x", None),
            Refusal::waive_expires("JT-SUP-001", "src", "x", Some("31d\nnegotiation: forged")),
            Refusal::commit_expires("HEAD~1", "base only\nnegotiation: forged"),
            Refusal::waive_text("--reason"),
            Refusal::waive_path("JT-SUP-001", "../x", "x"),
            Refusal::signers_file(
                "verify",
                std::path::Path::new("/workspace/s"),
                &jtac_sign::SignError::Trust {
                    path: "/workspace/s".into(),
                    what: "not a plain file",
                },
            ),
            Refusal::no_upstream(),
            Refusal::trust("not a regular file"),
            Refusal::no_root("abc"),
            Refusal::not_local("abc", "its nodes are not in this clone's store"),
            Refusal::policy_differs("abc", "sha256:0"),
        ];
        for r in &rows {
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(r.how.contains("jtac ") || r.how.contains("git "), "{r:?}");
            assert!(r.text().starts_with("jtac: "), "{r:?}");
            assert!(r.text().contains("\nnegotiation: "), "{r:?}");
            assert_eq!(r.text().lines().count(), 3, "{r:?}: a value forged a line");
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
