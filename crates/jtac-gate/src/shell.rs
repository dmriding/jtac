//! A shell script as the classifier reads it: simple commands in run order, words with the
//! quoting removed and flagged where the shell will build them, subshells kept as their own
//! list because a `cd` inside one dies with it (DESIGN.md section 11.4). `adapt` is the one
//! module that knows the parser's syntax tree.

mod adapt;
mod scan;

pub(crate) use adapt::read;

/// One word of one command, quoting removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Word {
    /// The text as written, quotes removed; for an unresolved word, the syntax as written.
    pub text: String,
    /// The shell will build this word from a variable or a substitution, so `text` is not
    /// what arrives.
    pub unresolved: bool,
}

/// One simple command: leading assignments, the program, its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Simple {
    /// Assignments as `KEY=value`, then the program, then its arguments.
    pub words: Vec<Word>,
    /// The command's source text, for quoting back.
    pub raw: String,
    /// Inside an `if`, a loop, a `case`, a function body, or after `||`: the shell may never
    /// run it, so a `cd` here names no directory.
    pub conditional: bool,
    /// The heredoc or here-string on its stdin, if any.
    pub heredoc: Option<Word>,
    /// Its stdin is another command's output, so a shell here reads a script this text does
    /// not hold.
    pub piped: bool,
}

/// A command, or a subshell whose commands run in a copy of the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// Runs in this shell.
    Command(Simple),
    /// `( … )`, `$( … )`, `` ` … ` ``, `<( … )`, `coproc`: its `cd` cannot move the parent.
    Subshell(Vec<Item>),
}

impl Simple {
    /// The `n`th word's text, or `""` past the end.
    pub fn at(&self, n: usize) -> &str {
        self.words.get(n).map_or("", |w| w.text.as_str())
    }
}
