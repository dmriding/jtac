//! The command inside a word: `bash -c '…'`, `eval`, `trap`, a heredoc handed to a shell, and
//! the programs that run a string this reader will not open.

use super::program::{base_name, dashed_git, names_a_shell, names_git, runs_a_string};
use super::{Action, MAX_NESTING, Unread, classify_at, sealed};
use crate::shell::{Simple, Word};

fn unreadable(cmd: &Simple, why: Unread) -> Vec<Action> {
    vec![Action::Unreadable {
        why,
        raw: cmd.raw.clone(),
    }]
}

// A body that may not have run names no directory.
fn unplaced(actions: Vec<Action>) -> Vec<Action> {
    actions
        .into_iter()
        .map(|a| match a {
            Action::ChangeDir { raw, .. } => Action::ChangeDir { to: None, raw },
            other => other,
        })
        .collect()
}

/// The script a shell, `eval` or `trap` runs, classified as the script it is. `eval` runs in
/// this shell, so its `cd` stands unless the `eval` itself may not run; a child shell's dies
/// with it; a trap body runs at exit, after everything here.
pub(crate) fn indirect(cmd: &Simple, first: usize, depth: u8) -> Vec<Action> {
    if depth >= MAX_NESTING {
        return unreadable(cmd, Unread::Nesting);
    }
    let program = base_name(cmd.at(first.saturating_sub(1)));
    if program.eq_ignore_ascii_case("trap") {
        return match trap_script(cmd, first) {
            None => Vec::new(),
            Some(w) if w.unresolved => unreadable(cmd, Unread::ShellString),
            Some(w) => sealed(classify_at(&w.text, depth + 1)),
        };
    }
    if program.eq_ignore_ascii_case("eval") {
        let rest = &cmd.words[first.min(cmd.words.len())..];
        if rest.iter().any(|w| w.unresolved) {
            return unreadable(cmd, Unread::Eval);
        }
        let joined = rest
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let acts = classify_at(&joined, depth + 1);
        return if cmd.conditional {
            unplaced(acts)
        } else {
            acts
        };
    }
    let script = match script_word_at(cmd, first) {
        Some(i) => &cmd.words[i],
        None => match &cmd.heredoc {
            Some(h) => h,
            None if cmd.piped => return unreadable(cmd, Unread::ShellString),
            None if cmd
                .words
                .iter()
                .skip(first)
                .any(|w| quotes_a_command(&w.text)) =>
            {
                return unreadable(cmd, Unread::Runner(program.to_string()));
            }
            None => return Vec::new(),
        },
    };
    if script.unresolved {
        return unreadable(cmd, Unread::ShellString);
    }
    sealed(classify_at(&script.text, depth + 1))
}

// `trap <script> SIG…`: the first non-option word, when a signal follows it; `trap -p` and
// `trap - EXIT` arm nothing.
fn trap_script(cmd: &Simple, first: usize) -> Option<&Word> {
    let args: Vec<&Word> = cmd
        .words
        .iter()
        .skip(first)
        .skip_while(|w| w.text.starts_with('-') && w.text != "-")
        .collect();
    match args.as_slice() {
        [script, _, ..] if script.text != "-" => Some(script),
        _ => None,
    }
}

// The word after a `-c`, bundled or not (`-c`, `-lc`, `-xec`), or flock's `--command`.
fn script_word_at(cmd: &Simple, first: usize) -> Option<usize> {
    let mut i = first;
    let mut saw_c = false;
    while let Some(w) = cmd.words.get(i) {
        let t = w.text.as_str();
        match t.strip_prefix('-') {
            Some(short) if !t.starts_with("--") && !short.is_empty() => {
                saw_c |= short.contains('c')
            }
            Some(_) => saw_c |= t == "--command",
            None => break,
        }
        i += 1;
    }
    (saw_c && i < cmd.words.len()).then_some(i)
}

/// A program this reader does not know. It is fine until a word of it is a `git` with a
/// subcommand after it, or, for a program that runs strings, a word that quotes one or names
/// a shell. `alias` is the other way to hide a command: the name it binds runs later.
pub(crate) fn runner(cmd: &Simple, idx: usize) -> Option<Action> {
    let prog = base_name(cmd.at(idx));
    let rest = || cmd.words.iter().skip(idx + 1);
    let plain = rest()
        .enumerate()
        .any(|(n, w)| dashed_git(&w.text) || names_git(&w.text) && n + 1 < rest().count());
    let quoted = runs_a_string(prog) && rest().any(|w| quotes_a_command(&w.text));
    let why = if prog.eq_ignore_ascii_case("alias")
        && rest().any(|w| quotes_a_command(&w.text) || plain_git(&w.text))
    {
        Unread::Alias
    } else if plain || quoted {
        Unread::Runner(prog.to_string())
    } else {
        return None;
    };
    Some(Action::Unreadable {
        why,
        raw: cmd.raw.clone(),
    })
}

fn plain_git(text: &str) -> bool {
    text.split('=').next_back().is_some_and(names_git)
}

/// Does this text spell a git command with a subcommand, or name a shell, anywhere a shell
/// would start a command?
pub(crate) fn quotes_a_command(text: &str) -> bool {
    let tokens: Vec<&str> = text
        .split(|c: char| {
            c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '`' | '\'' | '"' | '=')
        })
        .filter(|t| !t.is_empty())
        .collect();
    tokens.iter().any(|t| names_a_shell(t) || dashed_git(t))
        || tokens
            .windows(2)
            .any(|w| names_git(w[0]) && !w[1].starts_with('-'))
}
