//! Which program a command runs once the wrappers are stepped over, and where a wrapper or
//! a `cd` puts it. Names are matched by file name without regard to case: `/usr/bin/git`,
//! `GIT.EXE` and `\git` are all git.

use super::Action;
use crate::shell::{Simple, Word};

/// What a command runs, as far as this reader goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Program {
    Git,
    /// A shell, `eval` or `trap`: the real command is inside a word.
    Indirect,
    ChangeDir,
    Other,
}

/// Programs that hand their tail through unchanged. A name belongs here only when the program
/// runs its tail as an argv in this same directory; one that also takes a directory of its own
/// is answered in `wrapper_chdir` too (`env -C`, `sudo -D`).
const WRAPPERS: [&str; 15] = [
    "env", "command", "builtin", "exec", "sudo", "nice", "time", "nohup", "xargs", "doas",
    "timeout", "setsid", "stdbuf", "ionice", "flock",
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
const RUNS_A_STRING: [&str; 35] = [
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

/// The program and the index of its first argument.
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
            // Search forward rather than skip flags: `nice -n 10 git push` has a value word.
            Some(Named::Wrapper) => {
                let start = if base_name(t).eq_ignore_ascii_case("exec") {
                    after_exec_options(cmd, i + 1)
                } else {
                    i + 1
                };
                match next_program(
                    cmd,
                    start,
                    |t| matches!(named(t), Some(p) if p != Named::Wrapper),
                ) {
                    Some(n) => i = n,
                    None => return (Program::Other, i),
                }
            }
            None => return (Program::Other, i),
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Named {
    Git,
    Shell,
    Cd,
    Wrapper,
}

fn named(token: &str) -> Option<Named> {
    let n = base_name(token);
    if n.eq_ignore_ascii_case("git") {
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
    if WRAPPERS.iter().any(|s| n.eq_ignore_ascii_case(s)) {
        return Some(Named::Wrapper);
    }
    None
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

/// The first word at or after `from` for which `is_program` holds.
fn next_program(cmd: &Simple, from: usize, is_program: impl Fn(&str) -> bool) -> Option<usize> {
    cmd.words
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, w)| is_program(&w.text))
        .map(|(i, _)| i)
}

// `exec -a NAME` takes NAME as the child's argv[0], and NAME may be `git`.
fn after_exec_options(cmd: &Simple, mut i: usize) -> usize {
    while let Some(w) = cmd.words.get(i) {
        let t = w.text.as_str();
        if t == "--" {
            return i + 1;
        }
        if !t.starts_with('-') {
            return i;
        }
        i += if t == "-a" { 2 } else { 1 };
    }
    i
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
        let n = base_name(t);
        if n.eq_ignore_ascii_case("env") {
            return env_chdir(cmd, i + 1);
        }
        if n.eq_ignore_ascii_case("sudo") || n.eq_ignore_ascii_case("doas") {
            return sudo_chdir(cmd, i + 1);
        }
        match named(t) {
            Some(Named::Wrapper) => i = next_program(cmd, i + 1, |t| named(t).is_some())?,
            _ => return None,
        }
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

// Read left to right as getopt does: `-uC` unsets a variable named C, `-iC ../x` moves.
fn env_chdir(cmd: &Simple, from: usize) -> Option<Chdir> {
    let mut i = from;
    'word: while let Some(w) = cmd.words.get(i) {
        let t = w.text.as_str();
        if t == "--" {
            return None;
        }
        if let Some(long) = t.strip_prefix("--") {
            let (name, value) = long
                .split_once('=')
                .map_or((long, None), |(n, v)| (n, Some(v)));
            match name {
                "split-string" => return Some(Chdir::Split),
                "chdir" => {
                    return Some(match value {
                        Some(v) => chdir_to(&Word {
                            text: v.to_string(),
                            unresolved: w.unresolved,
                        }),
                        None => cmd.words.get(i + 1).map_or(Chdir::Unnamed, chdir_to),
                    });
                }
                "unset" | "block-signal" | "default-signal" | "ignore-signal"
                    if value.is_none() =>
                {
                    i += 2
                }
                _ => i += 1,
            }
            continue;
        }
        if let Some(short) = t.strip_prefix('-').filter(|s| !s.is_empty()) {
            for (k, c) in short.char_indices() {
                match c {
                    'S' => return Some(Chdir::Split),
                    'C' => return Some(attached_or_next(cmd, i, &short[k + 1..], w.unresolved)),
                    'u' => {
                        i += if k + 1 == short.len() { 2 } else { 1 };
                        continue 'word;
                    }
                    _ => {}
                }
            }
            i += 1;
            continue;
        }
        if !is_assignment(t) {
            return None;
        }
        i += 1;
    }
    None
}

// `sudo -D DIR` is `env -C`; `-R`/`--chroot` goes somewhere this reader will not follow.
// `doas -C` names a config file, so `C` is a value option here.
fn sudo_chdir(cmd: &Simple, from: usize) -> Option<Chdir> {
    const TAKES_A_VALUE: [char; 9] = ['u', 'g', 'h', 'p', 'C', 'T', 'U', 'r', 't'];
    let mut i = from;
    'word: while let Some(w) = cmd.words.get(i) {
        let t = w.text.as_str();
        if t == "--" {
            return None;
        }
        if let Some(long) = t.strip_prefix("--") {
            let (name, value) = long
                .split_once('=')
                .map_or((long, None), |(n, v)| (n, Some(v)));
            match name {
                "chroot" => return Some(Chdir::Unnamed),
                "chdir" => {
                    return Some(match value {
                        Some(v) => chdir_to(&Word {
                            text: v.to_string(),
                            unresolved: w.unresolved,
                        }),
                        None => cmd.words.get(i + 1).map_or(Chdir::Unnamed, chdir_to),
                    });
                }
                "user" | "group" | "host" | "prompt" | "close-from" | "type" | "other-user"
                | "role" | "command-timeout"
                    if value.is_none() =>
                {
                    i += 2;
                }
                _ => i += 1,
            }
            continue;
        }
        if let Some(short) = t.strip_prefix('-').filter(|s| !s.is_empty()) {
            for (k, c) in short.char_indices() {
                match c {
                    'R' => return Some(Chdir::Unnamed),
                    'D' => return Some(attached_or_next(cmd, i, &short[k + 1..], w.unresolved)),
                    c if TAKES_A_VALUE.contains(&c) => {
                        i += if k + 1 == short.len() { 2 } else { 1 };
                        continue 'word;
                    }
                    _ => {}
                }
            }
            i += 1;
            continue;
        }
        if !is_assignment(t) {
            return None;
        }
        i += 1;
    }
    None
}

/// `cd <dir>`: `None` when the text does not name where it lands. `~` and `~/x` do name it;
/// `~user/x` does not.
pub(crate) fn change_dir(cmd: &Simple, first: usize) -> Action {
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
                    && !w.text.starts_with('-')
                    && (!w.text.starts_with('~') || w.text == "~" || w.text.starts_with("~/"))
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
    if b.is_empty() || is_absolute(b) {
        return b.to_string();
    }
    if a.is_empty() {
        return b.to_string();
    }
    format!("{}/{b}", a.trim_end_matches('/'))
}

/// Rooted on any platform: `/x`, `\x`, `C:`, `~`.
pub(crate) fn is_absolute(p: &str) -> bool {
    p.starts_with(['/', '\\', '~'])
        || (p.len() >= 2 && p.as_bytes()[1] == b':' && p.as_bytes()[0].is_ascii_alphabetic())
}
