//! Arguments. clap owns `--help` and `--version`; every other refusal is the CLI's own
//! (`refuse`), so a command this release does not ship is refused in three parts.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// The `jtac` command line.
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(
    name = "jtac",
    version,
    about = "Provenance and verification for AI-written code",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// The command; none prints the version.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Every command the parser accepts; `init`, `check`, `status`, `why`, `verify`, `replay`,
/// `audit`, `show`, `waive`, and `hook` run in this release.
#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Command {
    /// Set this repository up: signing key, local store, policy, the pre-push hook, the
    /// fetch refspec for evidence
    Init(InitArgs),
    /// Run the gate on HEAD: diff-integrity rules, the policy's checks, then the verdict;
    /// a pass writes a signed root
    Check(CheckArgs),
    /// Whether a push would pass: HEAD's root, the unpushed commits, the last verdict
    Status(JsonArg),
    /// The last verdict's reasons, each with its rule, why it exists, and the next command
    Why(JsonArg),
    /// Signatures and chain of the roots over a range; exit 1 when a root is invalid
    Verify(VerifyArgs),
    /// Recompute one root's evidence, counts and verdict from the local nodes; exit 1 on a
    /// mismatch
    Replay(ReplayArgs),
    /// Coverage per commit over a range: covered, declared off, absent, with the totals
    Audit(RangeArgs),
    /// The root refs a push of the range would send, byte for byte
    Show(ShowArgs),
    /// The signed off-switch: waive one commit, or one rule under one path for up to 30 days
    Waive(WaiveArgs),
    /// The public proof: attack every control through its real path in a scratch
    /// repository; exit 1 when one does not hold
    Bench(JsonArg),
    /// A git hook entry point; git runs it, `jtac init` installs it
    #[command(subcommand)]
    Hook(Hook),
    /// A command a later release ships.
    #[command(external_subcommand)]
    Other(Vec<String>),
}

/// `jtac init` flags.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct InitArgs {
    /// Print everything init would write and write nothing
    #[arg(long)]
    pub dry_run: bool,
    /// The remote that gets the evidence refspec, when there is more than one and none is
    /// `origin`
    #[arg(long, value_name = "NAME")]
    pub remote: Option<String>,
    /// Keep a pre-push hook jtac did not write, unread: you added the exec line to it
    #[arg(long)]
    pub keep_hook: bool,
    /// Register the advisory tool-call hook with this agent (`claude-code`), in its
    /// per-clone settings file
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,
    /// Print the structure the text is rendered from, as JSON
    #[arg(long)]
    pub json: bool,
}

/// The hooks git can call. Stdout is the protocol wire; diagnostics go to stderr.
#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Hook {
    /// The push gate: git writes one `<local ref> <local oid> <remote ref> <remote oid>`
    /// line per ref to stdin
    PrePush {
        /// The remote's name, or its URL when it has none
        remote: String,
        /// The remote's URL
        url: String,
    },
    /// The advisory on an agent's proposed command: the agent's hook JSON on stdin, a
    /// note on stdout, never a block
    PreTool {
        /// Which agent's wire to speak: `claude-code`
        agent: String,
    },
}

/// `jtac check` flags.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct CheckArgs {
    /// Print the verdict with its reasons and write nothing; the only mode that runs on a
    /// dirty tree
    #[arg(long)]
    pub preview: bool,
    /// Diff from the merge base of REV and HEAD instead of the nearest covered ancestor,
    /// the upstream, or the default branch
    #[arg(long, value_name = "REV")]
    pub base: Option<String>,
    /// Print the structure the text is rendered from, as JSON
    #[arg(long)]
    pub json: bool,
}

/// `jtac waive` flags.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct WaiveArgs {
    /// The commit to turn the gate off for; its root reads `declared_off`
    #[arg(value_name = "REV")]
    pub rev: Option<String>,
    /// The rule to waive, with `--path`; the root stays `covered`
    #[arg(long, value_name = "RULE")]
    pub rule: Option<String>,
    /// The path prefix the rule is waived under, whole components
    #[arg(long, value_name = "PREFIX")]
    pub path: Option<String>,
    /// How long a rule waiver runs from now: `14d`, at most `30d`
    #[arg(long, value_name = "DAYS")]
    pub expires: Option<String>,
    /// Why, verbatim, for every reader of the root
    #[arg(long, value_name = "TEXT")]
    pub reason: String,
    /// Print the structure the text is rendered from, as JSON
    #[arg(long)]
    pub json: bool,
}

/// `jtac verify` flags.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct VerifyArgs {
    /// What `git rev-list` reads: `A..B`, `A...B`, a rev for its whole history; HEAD when
    /// none
    #[arg(value_name = "RANGE")]
    pub range: Vec<String>,
    /// An allowed-signers file instead of `.jtac/signers` at HEAD
    #[arg(long, value_name = "FILE")]
    pub signers: Option<PathBuf>,
    /// Exit 1 on an absent commit as well as on an invalid root
    #[arg(long)]
    pub require_coverage: bool,
    /// Print the structure the text is rendered from, as JSON
    #[arg(long)]
    pub json: bool,
}

/// `jtac replay` flags.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct ReplayArgs {
    /// The commit whose root to replay
    #[arg(value_name = "COMMIT")]
    pub commit: String,
    /// An allowed-signers file instead of `.jtac/signers` at HEAD
    #[arg(long, value_name = "FILE")]
    pub signers: Option<PathBuf>,
    /// Print the structure the text is rendered from, as JSON
    #[arg(long)]
    pub json: bool,
}

/// `jtac audit` flags.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct RangeArgs {
    /// What `git rev-list` reads: `A..B`, `A...B`, a rev for its whole history; HEAD when
    /// none
    #[arg(value_name = "RANGE")]
    pub range: Vec<String>,
    /// An allowed-signers file instead of `.jtac/signers` at HEAD
    #[arg(long, value_name = "FILE")]
    pub signers: Option<PathBuf>,
    /// Print the structure the text is rendered from, as JSON
    #[arg(long)]
    pub json: bool,
}

/// `jtac show` flags.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct ShowArgs {
    /// What `git rev-list` reads; the upstream to HEAD when none: what the next push carries
    #[arg(value_name = "RANGE")]
    pub range: Vec<String>,
    /// Print the structure the text is rendered from, as JSON
    #[arg(long)]
    pub json: bool,
}

/// The one flag `status`, `why` and `bench` take.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct JsonArg {
    /// Print the structure the text is rendered from, as JSON
    #[arg(long)]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_flags_parse_and_unknown_commands_land_in_other() {
        let cli = Cli::try_parse_from(["jtac", "check", "--preview", "--base", "main", "--json"])
            .unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Check(CheckArgs {
                preview: true,
                base: Some("main".into()),
                json: true,
            }))
        );
        let cli = Cli::try_parse_from(["jtac", "status", "--json"]).unwrap();
        assert_eq!(cli.command, Some(Command::Status(JsonArg { json: true })));
        let cli = Cli::try_parse_from(["jtac", "why"]).unwrap();
        assert_eq!(cli.command, Some(Command::Why(JsonArg { json: false })));
        let cli = Cli::try_parse_from(["jtac", "init", "--dry-run", "--remote", "up"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Init(InitArgs {
                dry_run: true,
                remote: Some("up".into()),
                keep_hook: false,
                agent: None,
                json: false,
            }))
        );
        let cli = Cli::try_parse_from(["jtac", "hook", "pre-tool", "claude-code"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Hook(Hook::PreTool {
                agent: "claude-code".into()
            }))
        );
        let cli = Cli::try_parse_from(["jtac", "hook", "pre-push", "origin", "x.git"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Hook(Hook::PrePush {
                remote: "origin".into(),
                url: "x.git".into(),
            }))
        );
        assert!(Cli::try_parse_from(["jtac", "hook", "pre-push"]).is_err());
        let cli = Cli::try_parse_from(["jtac", "policy", "--reason", "x"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Other(vec![
                "policy".into(),
                "--reason".into(),
                "x".into()
            ]))
        );
        let cli = Cli::try_parse_from(["jtac", "bench", "--json"]).unwrap();
        assert_eq!(cli.command, Some(Command::Bench(JsonArg { json: true })));
        let cli =
            Cli::try_parse_from(["jtac", "waive", "HEAD", "--expires", "3d", "--reason", "x"])
                .unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Waive(WaiveArgs {
                rev: Some("HEAD".into()),
                rule: None,
                path: None,
                expires: Some("3d".into()),
                reason: "x".into(),
                json: false,
            }))
        );
        let cli = Cli::try_parse_from([
            "jtac",
            "verify",
            "--require-coverage",
            "--signers",
            "s",
            "main..HEAD",
            "^v1",
        ])
        .unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Verify(VerifyArgs {
                range: vec!["main..HEAD".into(), "^v1".into()],
                signers: Some("s".into()),
                require_coverage: true,
                json: false,
            }))
        );
        let cli = Cli::try_parse_from(["jtac", "replay", "HEAD", "--json"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Replay(ReplayArgs {
                commit: "HEAD".into(),
                signers: None,
                json: true,
            }))
        );
        assert!(Cli::try_parse_from(["jtac", "replay"]).is_err());
        assert!(Cli::try_parse_from(["jtac", "audit", "--all"]).is_err());
        let cli = Cli::try_parse_from(["jtac", "show"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Show(ShowArgs {
                range: Vec::new(),
                json: false,
            }))
        );
        assert!(Cli::try_parse_from(["jtac", "check", "--nope"]).is_err());
    }
}
