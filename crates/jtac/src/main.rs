//! The `jtac` binary. Impure: gathers facts, renders verdicts, decides nothing.

use std::process::ExitCode;

/// The version line printed by `jtac --version`.
fn version() -> String {
    format!("jtac {}", env!("CARGO_PKG_VERSION"))
}

/// Anything but a version request is refused: a gate that is not built yet must not look like
/// one that passed, so a hook calling `jtac check` sees a non-zero exit, never a pass.
fn refusal(arg: &str) -> String {
    format!(
        "jtac: `{arg}` is not a command in this release.\n\
         jtac {} is pre-implementation and runs no gate. Exiting zero here would let a hook \
         treat \"not built\" as \"passed\".\n\
         Run `jtac --version` to confirm the install, and watch the repository for the first \
         release that ships `jtac check`.",
        env!("CARGO_PKG_VERSION")
    )
}

fn run(args: &[String]) -> Result<String, String> {
    match args {
        [] => Ok(version()),
        [flag] if flag == "--version" || flag == "-V" => Ok(version()),
        [first, ..] => Err(refusal(first)),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(text) => {
            eprintln!("{text}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{refusal, run, version};

    #[test]
    fn version_line_names_the_binary_and_the_workspace_version() {
        assert_eq!(version(), format!("jtac {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn no_args_and_version_flags_print_the_version() {
        for args in [
            vec![],
            vec!["--version".to_string()],
            vec!["-V".to_string()],
        ] {
            assert_eq!(run(&args), Ok(version()));
        }
    }

    #[test]
    fn any_other_argument_is_refused_with_what_why_how() {
        let err = run(&["check".to_string(), "--strict".to_string()]).unwrap_err();
        assert!(err.contains("`check` is not a command"), "what");
        assert!(err.contains("not built"), "why");
        assert!(err.contains("jtac --version"), "how");
        assert_eq!(err, refusal("check"));
    }
}
