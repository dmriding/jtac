//! The `jtac` binary. Impure: gathers facts, renders verdicts, decides nothing. Exit 0 is
//! `CLEARED HOT`, 1 is `ABORT`, 2 is a refusal: the gate could not answer.

mod cli;
mod output;
mod policy;
mod preview;
mod reason;
mod refuse;
mod render;
mod repo;

use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use jtac_core::Ts;

use crate::cli::{CheckArgs, Cli, Command};
use crate::refuse::Refusal;

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
}

fn version() -> String {
    format!("jtac {}", env!("CARGO_PKG_VERSION"))
}

fn run(args: &[String], cwd: &Path) -> Run {
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
        Some(Command::Check(a)) if !a.preview => {
            Run::refused(&Refusal::check_without_preview(), json)
        }
        Some(Command::Check(a)) => match check(&a, cwd) {
            Ok((text, pass)) => Run::out(text, u8::from(!pass)),
            Err(r) => Run::refused(&r, json),
        },
    }
}

/// Gather, run, decide, render. The rendered text and whether it passed.
fn check(a: &CheckArgs, cwd: &Path) -> Result<(String, bool), Refusal> {
    if std::env::var_os(jtac_checks::NESTED_VAR).is_some_and(|v| v == "1") {
        return Err(Refusal::nested());
    }
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    let loaded = policy::load(&repo, &head)?;
    let base = repo::base(&repo, &head, a.base.as_deref()).map_err(|e| match &a.base {
        Some(rev) => Refusal::base(rev, &e),
        None => Refusal::repository(&e),
    })?;
    let dirty = repo::dirty(&repo).map_err(|e| Refusal::repository(&e))?;
    let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
    let p = preview::preview(&repo, &loaded, head, base, now)?;
    let mut notes = Vec::new();
    if dirty {
        notes.push(
            "the work tree has uncommitted changes: the checks ran on it, the diff-integrity \
             rules judged HEAD; commit before `jtac check`"
                .to_owned(),
        );
    }
    if loaded.differs_on_disk {
        notes.push("the policy on disk differs from HEAD's; the verdict used HEAD's".to_owned());
    }
    let out = output::output(&p, notes);
    let text = if a.json {
        let mut s = serde_json::to_string_pretty(&out).map_err(Refusal::internal)?;
        s.push('\n');
        s
    } else {
        render::text(&out)
    };
    Ok((text, out.pass))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let ran = run(&args, &cwd);
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
            let ran = run(&words(args), Path::new("."));
            assert_eq!(ran, Run::out(format!("{}\n", version()), 0), "{args:?}");
        }
    }

    #[test]
    fn help_goes_to_stdout_with_exit_zero_and_names_check() {
        let rows: [(&[&str], &str); 3] = [
            (&["--help"], "check"),
            (&["-h"], "--version"),
            (&["check", "--help"], "--preview"),
        ];
        for (args, want) in rows {
            let ran = run(&words(args), Path::new("."));
            assert_eq!((ran.code, ran.stderr.as_str()), (0, ""), "{args:?}");
            assert!(ran.stdout.contains(want), "{args:?}: {}", ran.stdout);
        }
    }

    #[test]
    fn unbuilt_commands_bare_check_and_bad_flags_are_refused_with_exit_two() {
        let rows: [(&[&str], &str); 4] = [
            (&["status"], "`status` is not a command"),
            (&["check"], "without `--preview`"),
            (&["check", "--nope"], "--nope"),
            (&["check", "--json"], "\"error\""),
        ];
        for (args, want) in rows {
            let ran = run(&words(args), Path::new("."));
            assert_eq!((ran.code, ran.stdout.as_str()), (2, ""), "{args:?}");
            assert!(ran.stderr.contains(want), "{args:?}: {}", ran.stderr);
        }
    }
}
