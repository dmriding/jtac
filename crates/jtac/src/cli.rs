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

/// Every command the parser accepts; only `check --preview` runs in this release.
#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Command {
    /// Run the gate on HEAD: diff-integrity rules, the policy's checks, then the verdict
    Check(CheckArgs),
    /// A command a later release ships.
    #[command(external_subcommand)]
    Other(Vec<String>),
}

/// `jtac check` flags.
#[derive(Args, Debug, PartialEq, Eq)]
pub struct CheckArgs {
    /// Print the verdict with its reasons and write nothing; the only mode this release runs
    #[arg(long)]
    pub preview: bool,
    /// Diff from the merge base of REV and HEAD instead of the upstream or default branch
    #[arg(long, value_name = "REV")]
    pub base: Option<String>,
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
        assert_eq!(
            cli.command,
            Some(Command::Other(vec!["status".into(), "--json".into()]))
        );
        assert_eq!(Cli::try_parse_from(["jtac"]).unwrap().command, None);
        assert!(Cli::try_parse_from(["jtac", "check", "--nope"]).is_err());
    }
}
