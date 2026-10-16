//! Which program a command runs once the wrappers are stepped over, and where a wrapper or
//! a `cd` puts it. Names are matched by file name without regard to case: `/usr/bin/git`,
//! `GIT.EXE`, `\git` and `git-push` are all git. A wrapper's options are read as the wrapper
//! reads them, so `sudo -u git git push` finds the second word.

use super::Action;
use crate::shell::{Simple, Word};

/// What a command runs, as far as this reader goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Program {
    Git,
    /// A shell, `eval`, `trap`, or a wrapper's `-c` string: the real command is inside a word.
    Indirect,
    ChangeDir,
    Other,
}

/// A program that hands its tail through. `short` are the option letters that take a value,
/// `long` the long options that take one as a separate word, `positionals` the words of its
/// own before the command, `script` the letter that makes the next word a shell string.
struct Wrapper {
    name: &'static str,
    short: &'static str,
    long: &'static [&'static str],
    positionals: u8,
    script: Option<char>,
}

const WRAPPERS: [Wrapper; 14] = [
    Wrapper {
        name: "env",
        short: "uCS",
        long: &[
            "unset",
            "chdir",
            "block-signal",
            "default-signal",
            "ignore-signal",
            "split-string",
        ],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "sudo",
        short: "ughpCTUrtDR",
        long: &[
            "user",
            "group",
            "host",
            "prompt",
            "close-from",
            "type",
            "other-user",
            "role",
            "command-timeout",
            "chdir",
            "chroot",
        ],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "doas",
        short: "uC",
        long: &[],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "nice",
        short: "n",
        long: &["adjustment"],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "timeout",
        short: "sk",
        long: &["signal", "kill-after"],
        positionals: 1,
        script: None,
    },
    Wrapper {
        name: "stdbuf",
        short: "ioe",
        long: &["input", "output", "error"],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "ionice",
        short: "cnp",
        long: &["class", "classdata", "pid"],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "flock",
        short: "wEs",
        long: &["wait", "timeout", "conflict-exit-code"],
        positionals: 1,
        script: Some('c'),
    },
    Wrapper {
        name: "exec",
        short: "a",
        long: &[],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "command",
        short: "",
        long: &[],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "builtin",
        short: "",
        long: &[],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "time",
        short: "",
        long: &[],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "nohup",
        short: "",
        long: &[],
        positionals: 0,
        script: None,
    },
    Wrapper {
        name: "setsid",
        short: "",
        long: &[],
        positionals: 0,
        script: None,
    },
];

/// Shells this reader opens with `-c`, plus the two builtins that run a word as a script.
const PARSED_SHELLS: [&str; 9] = [
    "sh", "bash", "zsh", "dash", "ksh", "ash", "busybox", "eval", "trap",
];

/// Every name that is a shell, parsed or not. An unparsed one is a runner.
const SHELL_NAMES: [&str; 20] = [
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "ash",
    "csh",
    "tcsh",
    "fish",
    "busybox",
    "pwsh",
    "powershell",
    "cmd",
    "eval",
    "trap",
    "nu",
    "xonsh",
    "elvish",
    "mksh",
    "yash",
];

/// Programs that take a program as a string, or run one somewhere this text does not name.
/// A program that merely takes text does not belong here.
const RUNS_A_STRING: [&str; 36] = [
    "ssh",
    "plink",
    "mosh",
    "rsh",
    "docker",
    "podman",
    "nerdctl",
    "kubectl",
    "nsenter",
    "chroot",
    "systemd-run",
    "machinectl",
    "script",
    "screen",
    "tmux",
    "byobu",
    "parallel",
    "xargs",
    "watch",
    "entr",
    "watchexec",
    "hyperfine",
    "uv",
    "uvx",
    "poetry",
    "pipx",
    "npx",
    "bunx",
    "su",
    "sudo",
    "doas",
    "runuser",
    "pkexec",
    "wsl",
    "vagrant",
    "ansible",
];

/// Language interpreters: each takes a program as a string.
const INTERPRETERS: [&str; 21] = [
    "python",
    "python2",
    "python3",
    "py",
    "pypy",
    "perl",
    "ruby",
    "node",
    "nodejs",
    "deno",
    "bun",
    "php",
    "lua",
    "luajit",
    "tclsh",
    "expect",
    "awk",
    "gawk",
    "mawk",
    "nawk",
    "osascript",
];

/// After these the shell is somewhere the text does not say: `popd` names a stack slot, and
/// a sourced file can `cd` anywhere.
const LOSES_THE_DIRECTORY: [&str; 3] = ["popd", "source", "."];

/// The program and the index of its first argument (for `Other`, of the program word).
pub(crate) fn program_of(cmd: &Simple) -> (Program, usize) {
    let mut i = 0;
    loop {
        let Some(w) = cmd.words.get(i) else {
            return (Program::Other, i);
        };
        let t = w.text.as_str();
        if is_assignment(t) {
            i += 1;
            continue;
        }
        match named(t) {
            Some(Named::Git) => return (Program::Git, i + 1),
            Some(Named::Shell) => return (Program::Indirect, i + 1),
            Some(Named::Cd) => return (Program::ChangeDir, i + 1),
            Some(Named::Wrapper(spec)) => match after_wrapper(cmd, i + 1, spec) {
                After::Command(n) => i = n,
                After::Script(n) => return (Program::Indirect, n),
                After::Nothing => return (Program::Other, i),
            },
            None => return (Program::Other, i),
        }
    }
}

enum After {
    /// The wrapped command starts here.
    Command(usize),
    /// A `-c` makes the word after this index a shell string.
    Script(usize),
    Nothing,
}

// Read as getopt reads: an option's value is the rest of its cluster, else the next word.
fn after_wrapper(cmd: &Simple, from: usize, spec: &Wrapper) -> After {
    let mut i = from;
    let mut positionals = spec.positionals;
    while let Some(w) = cmd.words.get(i) {
        let t = w.text.as_str();
        if t == "--" {
            return cmd
                .words
                .get(i + 1)
                .map_or(After::Nothing, |_| After::Command(i + 1));
        }
        if let Some(long) = t.strip_prefix("--") {
            i += if !long.contains('=') && spec.long.contains(&long) {
                2
            } else {
                1
            };
            continue;
        }
        if let Some(short) = t.strip_prefix('-').filter(|s| !s.is_empty()) {
            let mut step = 1;
            for (k, c) in short.char_indices() {
                if spec.script == Some(c) {
                    return After::Script(i);
                }
                if spec.short.contains(c) {
                    step = if k + 1 == short.len() { 2 } else { 1 };
                    break;
                }
            }
            i += step;
            continue;
        }
        if spec.name == "env" && is_assignment(t) {
            i += 1;
            continue;
        }
        if positionals > 0 {
            positionals -= 1;
            i += 1;
            continue;
        }
        return After::Command(i);
    }
    After::Nothing
}

/// `KEY=value`, not a path with an `=` in it.
pub(crate) fn is_assignment(t: &str) -> bool {
    match t.split_once('=') {
        Some((k, _)) => {
            !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

enum Named {
    Git,
    Shell,
    Cd,
    Wrapper(&'static Wrapper),
}

fn named(token: &str) -> Option<Named> {
    let n = base_name(token);
    if n.eq_ignore_ascii_case("git") || n.len() > 4 && n[..4].eq_ignore_ascii_case("git-") {
        return Some(Named::Git);
    }
    if PARSED_SHELLS.iter().any(|s| n.eq_ignore_ascii_case(s)) {
        return Some(Named::Shell);
    }
    if ["cd", "pushd"]
        .iter()
        .chain(LOSES_THE_DIRECTORY.iter())
        .any(|c| n.eq_ignore_ascii_case(c))
    {
        return Some(Named::Cd);
    }
    WRAPPERS
        .iter()
        .find(|w| n.eq_ignore_ascii_case(w.name))
        .map(Named::Wrapper)
}

/// The file name of a program word: path and `.exe` stripped, the alias-skipping `\` too.
pub(crate) fn base_name(token: &str) -> &str {
    let t = token.strip_prefix('\\').unwrap_or(token);
    let t = t.rsplit(['/', '\\']).next().unwrap_or(t);
    match t.len().checked_sub(4) {
        Some(cut) if t[cut..].eq_ignore_ascii_case(".exe") => &t[..cut],
        _ => t,
    }
}

/// Is this program one that takes a program as a string? Matched with a trailing version
/// stripped, so `python3.12` is `python3`; only digits and dots come off.
pub(crate) fn runs_a_string(prog: &str) -> bool {
    let n = base_name(prog);
    let bare = n.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    [&RUNS_A_STRING[..], &INTERPRETERS[..], &SHELL_NAMES[..]]
        .concat()
        .iter()
        .any(|s| n.eq_ignore_ascii_case(s) || (!bare.is_empty() && bare.eq_ignore_ascii_case(s)))
}

/// Does this word name a shell, parsed or not?
pub(crate) fn names_a_shell(text: &str) -> bool {
    let n = base_name(text);
    SHELL_NAMES.iter().any(|s| n.eq_ignore_ascii_case(s))
}

/// Where a wrapper puts the command it wraps.
pub(crate) enum Chdir {
    /// A directory the text names.
    To(String),
    /// It moved somewhere the text does not name.
    Unnamed,
    /// `env -S`: one string env splits into a command line.
    Split,
}

/// The chdir a wrapper performs before running what it wraps, walking the wrapper chain.
pub(crate) fn wrapper_chdir(cmd: &Simple) -> Option<Chdir> {
    let mut i = 0;
    while let Some(w) = cmd.words.get(i) {
        let t = w.text.as_str();
        if is_assignment(t) {
            i += 1;
            continue;
        }
        let Some(Named::Wrapper(spec)) = named(t) else {
            return None;
        };
        if let Some(found) = chdir_options(cmd, i + 1, spec) {
            return Some(found);
        }
        match after_wrapper(cmd, i + 1, spec) {
            After::Command(n) => i = n,
            After::Script(_) | After::Nothing => return None,
        }
    }
    None
}

// `env -C`/`--chdir`, `env -S`, `sudo -D`/`--chdir`, `sudo -R`/`--chroot`; `doas -C` is a
// config file. Read left to right so an option's value is never read as another option.
fn chdir_options(cmd: &Simple, from: usize, spec: &Wrapper) -> Option<Chdir> {
    if !matches!(spec.name, "env" | "sudo") {
        return None;
    }
    let (dir_letter, lost_letter) = if spec.name == "env" {
        ('C', 'S')
    } else {
        ('D', 'R')
    };
    let mut i = from;
    while let Some(w) = cmd.words.get(i) {
        let t = w.text.as_str();
        if t == "--" || !(t.starts_with('-') || is_assignment(t)) {
            return None;
        }
        if let Some(long) = t.strip_prefix("--") {
            let (name, value) = long
                .split_once('=')
                .map_or((long, None), |(n, v)| (n, Some(v)));
            match name {
                "split-string" | "chroot" => {
                    return Some(if name == "chroot" {
                        Chdir::Unnamed
                    } else {
                        Chdir::Split
                    });
                }
                "chdir" => {
                    return Some(match value {
                        Some(v) => chdir_to(&Word {
                            text: v.to_string(),
                            unresolved: w.unresolved,
                        }),
                        None => cmd.words.get(i + 1).map_or(Chdir::Unnamed, chdir_to),
                    });
                }
                _ => {
                    i += if value.is_none() && spec.long.contains(&name) {
                        2
                    } else {
                        1
                    }
                }
            }
            continue;
        }
        if let Some(short) = t.strip_prefix('-').filter(|s| !s.is_empty()) {
            let mut step = 1;
            for (k, c) in short.char_indices() {
                if c == lost_letter {
                    return Some(if spec.name == "env" {
                        Chdir::Split
                    } else {
                        Chdir::Unnamed
                    });
                }
                if c == dir_letter {
                    return Some(attached_or_next(cmd, i, &short[k + 1..], w.unresolved));
                }
                if spec.short.contains(c) {
                    step = if k + 1 == short.len() { 2 } else { 1 };
                    break;
                }
            }
            i += step;
            continue;
        }
        i += 1;
    }
    None
}

fn chdir_to(w: &Word) -> Chdir {
    if w.unresolved || w.text.is_empty() || w.text.starts_with('~') {
        Chdir::Unnamed
    } else {
        Chdir::To(w.text.clone())
    }
}

/// A short option's value: attached (`-C../x`, `-C=../x`) or the next word.
fn attached_or_next(cmd: &Simple, i: usize, rest: &str, unresolved: bool) -> Chdir {
    let rest = rest.strip_prefix('=').unwrap_or(rest);
    if rest.is_empty() {
        cmd.words.get(i + 1).map_or(Chdir::Unnamed, chdir_to)
    } else {
        chdir_to(&Word {
            text: rest.to_string(),
            unresolved,
        })
    }
}

/// `cd <dir>`: `None` when the text does not name where it lands: a word the shell builds,
/// `cd -`, `pushd +1`, `~user/x`, a body that may not run, or a bare name while `CDPATH` is
/// set in this script, since `cd` then searches for it.
pub(crate) fn change_dir(cmd: &Simple, first: usize, cdpath: bool) -> Action {
    let verb = base_name(cmd.at(first.saturating_sub(1)));
    let lost = cmd.conditional
        || LOSES_THE_DIRECTORY
            .iter()
            .any(|c| verb.eq_ignore_ascii_case(c));
    let to = if lost {
        None
    } else {
        cmd.words
            .iter()
            .skip(first)
            .find(|w| !matches!(w.text.as_str(), "-P" | "-L" | "-e" | "-@" | "--"))
            .filter(|w| {
                !w.unresolved
                    && !w.text.starts_with(['-', '+'])
                    && (!w.text.starts_with('~') || w.text == "~" || w.text.starts_with("~/"))
                    && !(cdpath
                        && !w.text.starts_with(['.', '/', '\\', '~'])
                        && !is_absolute(&w.text))
            })
            .map(|w| w.text.clone())
    };
    Action::ChangeDir {
        to,
        raw: cmd.raw.clone(),
    }
}

/// `a` then `b`; an absolute `b` wins.
pub(crate) fn join_dir(a: &str, b: &str) -> String {
    if b.is_empty() || is_absolute(b) || a.is_empty() {
        return b.to_string();
    }
    format!("{}/{b}", a.trim_end_matches('/'))
}

/// Rooted on any platform: `/x`, `\x`, `C:`, `~`.
pub(crate) fn is_absolute(p: &str) -> bool {
    p.starts_with(['/', '\\', '~'])
        || (p.len() >= 2 && p.as_bytes()[1] == b':' && p.as_bytes()[0].is_ascii_alphabetic())
}
