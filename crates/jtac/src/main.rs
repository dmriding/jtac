//! The `jtac` binary. Impure: gathers facts, renders verdicts, decides nothing. Exit 0 is
//! `CLEARED HOT`, 1 is `ABORT`, 2 is a refusal: the gate could not answer.

mod agent;
mod agentcfg;
mod audit;
mod bench;
mod chain;
mod check;
mod ci;
mod cli;
mod coverage;
mod detect;
mod headof;
mod hook;
mod hookfile;
mod identity;
mod init;
mod initout;
mod judge;
mod keydir;
mod ledger;
mod lever;
mod merge;
mod output;
mod policy;
mod pretool;
mod range;
mod reason;
mod refspec;
mod refuse;
mod render;
mod replay;
mod repo;
mod root;
mod rootpush;
mod show;
mod state;
mod status;
mod survey;
mod trust;
mod update;
mod verify;
mod waive;
mod why;
mod wire;
mod write;

use std::path::Path;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, Hook};
use crate::refuse::Refusal;

/// A verdict a command rendered, with a warning that does not change the exit code: the
/// verdict stands and its root is written, only `state.json` could not be.
pub struct Printed {
    /// The text for stdout.
    pub stdout: String,
    /// Exit 0 when true, 1 when false.
    pub pass: bool,
    /// For stderr, when something after the verdict failed.
    pub warn: Option<Refusal>,
    /// For stderr, a fact about the verdict a person must not miss: nothing failed.
    pub note: Option<String>,
}

impl Printed {
    fn verdict(stdout: String, pass: bool) -> Self {
        Self {
            stdout,
            pass,
            warn: None,
            note: None,
        }
    }
}

/// What one invocation printed and how it exited.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    stdout: String,
    stderr: String,
    code: u8,
}

impl Run {
    fn out(stdout: String, code: u8) -> Self {
        Self {
            stdout,
            stderr: String::new(),
            code,
        }
    }

    fn refused(r: &Refusal, json: bool) -> Self {
        Self {
            stdout: String::new(),
            stderr: if json { r.json() } else { r.text() },
            code: 2,
        }
    }

    fn verdict(ran: Result<Printed, Refusal>, json: bool) -> Self {
        match ran {
            Ok(p) => Self {
                stdout: p.stdout,
                stderr: p.note.unwrap_or_default()
                    + &p.warn
                        .map_or_else(String::new, |w| if json { w.json() } else { w.text() }),
                code: u8::from(!p.pass),
            },
            Err(r) => Self::refused(&r, json),
        }
    }
}

fn version() -> String {
    format!("jtac {}", env!("CARGO_PKG_VERSION"))
}

fn run(args: &[String], cwd: &Path, stdin: &Result<String, String>) -> Run {
    let json = args.iter().any(|a| a == "--json");
    let cli =
        match Cli::try_parse_from(std::iter::once("jtac").chain(args.iter().map(String::as_str))) {
            Ok(cli) => cli,
            Err(e) if e.use_stderr() => return Run::refused(&Refusal::arguments(&e), json),
            Err(e) => return Run::out(e.to_string(), 0),
        };
    match cli.command {
        None => Run::out(format!("{}\n", version()), 0),
        Some(Command::Other(words)) => {
            let first = words.first().map_or("", String::as_str);
            Run::refused(&Refusal::not_built(first), json)
        }
        Some(Command::Init(a)) => Run::verdict(init::init(&a, cwd), a.json),
        Some(Command::Check(a)) => Run::verdict(check::check(&a, cwd), a.json),
        Some(Command::Ci(a)) => Run::verdict(ci::ci(&a, cwd), a.json),
        Some(Command::Hook(Hook::PrePush { remote, url })) => {
            let (stderr, code) = hook::pre_push(cwd, &remote, &url, stdin);
            Run {
                stdout: String::new(),
                stderr,
                code,
            }
        }
        Some(Command::Hook(Hook::PreTool { agent })) => {
            let (stdout, stderr) = pretool::pre_tool(&agent, cwd, Box::new(stdin_text));
            Run {
                stdout,
                stderr,
                code: 0,
            }
        }
        Some(Command::Status(a)) => Run::verdict(status::status(cwd, a.json), a.json),
        Some(Command::Why(a)) => Run::verdict(why::why(cwd, a.json), a.json),
        Some(Command::Verify(a)) => Run::verdict(verify::verify(&a, cwd), a.json),
        Some(Command::Replay(a)) => Run::verdict(replay::replay(&a, cwd), a.json),
        Some(Command::Audit(a)) => Run::verdict(audit::audit(&a, cwd), a.json),
        Some(Command::Show(a)) => Run::verdict(show::show(&a, cwd), a.json),
        Some(Command::Waive(a)) => Run::verdict(waive::waive(&a, cwd), a.json),
        Some(Command::Bench(a)) => Run::verdict(bench::bench(a.json), a.json),
    }
}

/// All of stdin; `pre-push` reads it before judging, `pre-tool` on its worker under the cap.
fn stdin_text() -> Result<String, String> {
    let mut bytes = Vec::new();
    match std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes) {
        Ok(_) => String::from_utf8(bytes).map_err(|_| "it is not UTF-8".to_owned()),
        Err(e) => Err(e.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let hook = args.first().is_some_and(|a| a == "hook");
    let advisory = args.get(1).is_some_and(|a| a == "pre-tool");
    let stdin = if hook && !advisory {
        stdin_text()
    } else {
        Ok(String::new())
    };
    let ran = run(&args, &cwd, &stdin);
    print!("{}", ran.stdout);
    eprint!("{}", ran.stderr);
    ExitCode::from(ran.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn no_args_and_version_flags_print_the_version() {
        for args in [&[][..], &["--version"], &["-V"]] {
            let ran = run(&words(args), Path::new("."), &Ok(String::new()));
            assert_eq!(ran, Run::out(format!("{}\n", version()), 0), "{args:?}");
        }
    }

    #[test]
    fn help_goes_to_stdout_with_exit_zero_and_names_every_command() {
        let rows: [(&[&str], &str); 10] = [
            (&["--help"], "check"),
            (&["ci", "--help"], "--require-local-root"),
            (&["--help"], "bench"),
            (&["--help"], "audit"),
            (&["verify", "--help"], "--require-coverage"),
            (&["--help"], "status"),
            (&["-h"], "why"),
            (&["--help"], "init"),
            (&["hook", "--help"], "pre-push"),
            (&["check", "--help"], "--preview"),
        ];
        for (args, want) in rows {
            let ran = run(&words(args), Path::new("."), &Ok(String::new()));
            assert_eq!((ran.code, ran.stderr.as_str()), (0, ""), "{args:?}");
            assert!(ran.stdout.contains(want), "{args:?}: {}", ran.stdout);
        }
    }

    #[test]
    fn unbuilt_commands_and_bad_flags_are_refused_with_exit_two() {
        let rows: [(&[&str], &str); 3] = [
            (&["policy"], "`policy` is not a command"),
            (&["check", "--nope"], "--nope"),
            (&["check", "--nope", "--json"], "\"error\""),
        ];
        for (args, want) in rows {
            let ran = run(&words(args), Path::new("."), &Ok(String::new()));
            assert_eq!((ran.code, ran.stdout.as_str()), (2, ""), "{args:?}");
            assert!(ran.stderr.contains(want), "{args:?}: {}", ran.stderr);
        }
    }
}
