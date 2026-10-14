//! Arguments. clap owns `--help` and `--version`; every other refusal is the CLI's own
//! (`refuse`), so a command this release does not ship is refused in three parts.

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

/// Every command the parser accepts; `init`, `check`, `status`, `why`, and `hook` run in
/// this release.
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

/// The one flag `status` and `why` take.
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
                json: false,
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
        let cli = Cli::try_parse_from(["jtac", "verify", "--range", "x"]).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::Other(vec![
                "verify".into(),
                "--range".into(),
                "x".into()
            ]))
        );
        assert!(Cli::try_parse_from(["jtac", "check", "--nope"]).is_err());
    }
}
