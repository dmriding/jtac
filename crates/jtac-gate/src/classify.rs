//! What a shell script does to a repository: pushes, hook bypasses, directory changes, and
//! the parts this reader cannot see (DESIGN.md section 11.4). Pure. A directory on an action
//! is already resolved against the `cd`s before it in the same shell; `ChangeDir` is reported
//! so the advisory can say the shell moved.

mod git;
mod indirect;
mod program;
#[cfg(test)]
mod spellings;

use crate::shell::{self, Item, Simple};

/// Scripts longer than this are not read: nobody types one, and the parser's fuel is finite.
pub const MAX_BYTES: usize = 64 * 1024;
/// Layers of `bash -c` this will open before calling the script unreadable.
pub const MAX_NESTING: u8 = 3;

/// How hard a push lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Force {
    /// A fast-forward, or the remote refuses.
    No,
    /// `--force`, `-f`, `+refspec`, `--mirror`.
    Force,
    /// `--force-with-lease`, alone.
    WithLease,
}

/// Where a command runs, relative to where the script started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dir {
    /// Where the script started.
    Here,
    /// A directory the text names.
    At(String),
    /// Somewhere the text does not name: a variable, `cd -`, `--git-dir`.
    Unknown,
}

impl Dir {
    /// `child` resolved against `self`; an absolute child wins, an unknown side stays unknown.
    fn join(&self, child: &Dir) -> Dir {
        match (self, child) {
            (_, Dir::Here) => self.clone(),
            (Dir::Here, c) => c.clone(),
            (_, Dir::Unknown) => Dir::Unknown,
            (Dir::At(a), Dir::At(b)) => Dir::At(program::join_dir(a, b)),
            (Dir::Unknown, Dir::At(b)) if program::is_absolute(b) => Dir::At(b.clone()),
            (Dir::Unknown, Dir::At(_)) => Dir::Unknown,
        }
    }

    fn then(&self, to: Option<&str>) -> Dir {
        match to {
            Some(t) => self.join(&Dir::At(t.to_string())),
            None => Dir::Unknown,
        }
    }
}

/// How a refspec names its destination, which decides whether git asks the remote about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spec {
    /// `main`: the source's own ref, mapped through `remote.<r>.push` or the upstream.
    Bare,
    /// `+main`: the source's own ref, unmapped, as git reads a forced bare name.
    Forced,
    /// `src:main`, `:main`, `--delete main`: a name the remote is asked about.
    Spelled,
}

/// How a hook is skipped. Each is a row of the spellings table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bypass {
    /// `--no-verify` on any git command.
    NoVerify,
    /// `-n` on `git commit`; on push the same letter is `--dry-run`.
    CommitShortN,
    /// `git -c core.hooksPath=…`.
    ConfigFlag,
    /// `git --config-env=core.hooksPath=…`.
    ConfigEnvFlag,
    /// `GIT_CONFIG_KEY_n=core.hooksPath` in the environment.
    ConfigEnvVar,
    /// `GIT_CONFIG_PARAMETERS` naming `core.hooksPath`.
    ConfigParameters,
    /// `git config core.hooksPath …`: lasts until somebody unsets it.
    ConfigWrite,
}

/// Why a command cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unread {
    /// The parser did not read the script clean; the parser's words.
    Syntax(String),
    /// `bash -c "$X"`, a heredoc the shell expands: the script does not exist yet.
    ShellString,
    /// `eval` on words the shell builds.
    Eval,
    /// This program takes a program as a string, or runs one elsewhere, and a word of it
    /// names git or a shell.
    Runner(String),
    /// `env -S`: env splits one string into a command line this reader does not split.
    SplitString,
    /// The program word itself is built by the shell (`$GIT push`, `"${cmd[@]}"`).
    Variable,
    /// `git -c alias.…` or `alias …`: the command that runs is not in the text.
    Alias,
    /// `include.path` or `GIT_CONFIG_GLOBAL`: config read from a file the text does not show.
    Include,
    /// `PS4` holds a substitution: bash runs it before every traced command.
    Prompt,
    /// Shells nested past [`MAX_NESTING`].
    Nesting,
    /// A script over [`MAX_BYTES`].
    Size,
}

/// A `git push`, as far as the text says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Push {
    /// The repository argument, or `--repo`.
    pub remote: Option<String>,
    /// Local sides of the refspecs, one per destination: an empty string for a `:dst`
    /// deletion, none at all under `--delete`.
    pub sources: Vec<String>,
    /// Remote sides of the refspecs, `tag X` spelled out.
    pub destinations: Vec<String>,
    /// Per destination, how it was named.
    pub specs: Vec<Spec>,
    /// How hard it lands.
    pub force: Force,
    /// `--delete`, `-d`, or a `:dst` refspec.
    pub delete: bool,
    /// `--all`, `--branches`, `--mirror`, or a wildcard destination: no branch is named.
    pub all_refs: bool,
    /// `--tags`: every tag, beside any refspec.
    pub tags: bool,
    /// `--dry-run` or `-n`.
    pub dry_run: bool,
    /// How the pre-push hook is skipped, if it is.
    pub bypass: Option<Bypass>,
    /// A word the shell builds is among the arguments, so the fields above may be incomplete.
    pub opaque: bool,
    /// Where it runs.
    pub dir: Dir,
    /// The command's text.
    pub raw: String,
}

/// One thing the script does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A push.
    Push(Push),
    /// A git command other than push that skips its hooks, or a config write that skips
    /// every hook from now on.
    HookBypass {
        /// The spelling.
        spelling: Bypass,
        /// The command's text.
        raw: String,
    },
    /// The shell moved; `None` is a directory the text does not name.
    ChangeDir {
        /// Relative to where the script started.
        to: Option<String>,
        /// The command's text.
        raw: String,
    },
    /// A command this reader cannot see into.
    Unreadable {
        /// Why.
        why: Unread,
        /// The command's text.
        raw: String,
    },
}

/// Every action a script performs, in run order. The text is read to the first NUL, as the
/// shell reads it.
pub fn classify(script: &str) -> Vec<Action> {
    let script = script.split('\0').next().unwrap_or(script);
    if script.len() > MAX_BYTES {
        return vec![Action::Unreadable {
            why: Unread::Size,
            raw: String::new(),
        }];
    }
    classify_at(script, 0)
}

fn classify_at(script: &str, depth: u8) -> Vec<Action> {
    match shell::read(script) {
        Err(why) => vec![Action::Unreadable {
            why: Unread::Syntax(why),
            raw: script.trim().to_string(),
        }],
        Ok(items) => {
            let mut out = Vec::new();
            let cdpath = sets_cdpath(&items);
            walk(&items, depth, cdpath, &mut Dir::Here, &mut out);
            out
        }
    }
}

// `CDPATH=…` anywhere in the script makes a bare `cd name` a search, not a path.
fn sets_cdpath(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Command(c) => c.words.iter().any(|w| w.text.starts_with("CDPATH=")),
        Item::Subshell(inner) => sets_cdpath(inner),
    })
}

fn walk(items: &[Item], depth: u8, cdpath: bool, here: &mut Dir, out: &mut Vec<Action>) {
    for item in items {
        match item {
            Item::Subshell(inner) => {
                let mut child = here.clone();
                let mut acts = Vec::new();
                walk(inner, depth, cdpath, &mut child, &mut acts);
                out.extend(sealed(acts));
            }
            Item::Command(cmd) => {
                let base = here.clone();
                for a in one(cmd, depth, cdpath) {
                    if let Action::ChangeDir { to, .. } = &a {
                        *here = here.then(to.as_deref());
                    }
                    out.push(place(a, &base));
                }
            }
        }
    }
}

// The actions of one command, with directories relative to the shell's position before it.
// A wrapper's own chdir (`env -C`, `sudo -D`) moves what it wraps and nothing else.
fn one(cmd: &Simple, depth: u8, cdpath: bool) -> Vec<Action> {
    if let Some(a) = git::traced(cmd).or_else(|| git::exported(cmd)) {
        return vec![a];
    }
    let (prog, first) = program::program_of(cmd);
    if prog == program::Program::Other && cmd.words.get(first).is_some_and(|w| w.unresolved) {
        return vec![Action::Unreadable {
            why: Unread::Variable,
            raw: cmd.raw.clone(),
        }];
    }
    let acts = match prog {
        program::Program::Git => git::git(cmd, first).into_iter().collect(),
        program::Program::Indirect => indirect::indirect(cmd, first, depth),
        program::Program::ChangeDir => vec![program::change_dir(cmd, first, cdpath)],
        program::Program::Other => indirect::runner(cmd, first).into_iter().collect(),
    };
    match program::wrapper_chdir(cmd) {
        None => acts,
        Some(program::Chdir::Split) => vec![Action::Unreadable {
            why: Unread::SplitString,
            raw: cmd.raw.clone(),
        }],
        Some(program::Chdir::To(d)) => acts
            .into_iter()
            .filter(|a| !matches!(a, Action::ChangeDir { .. }))
            .map(|a| place(a, &Dir::At(d.clone())))
            .collect(),
        Some(program::Chdir::Unnamed) => acts
            .into_iter()
            .filter(|a| !matches!(a, Action::ChangeDir { .. }))
            .map(|a| place(a, &Dir::Unknown))
            .collect(),
        Some(program::Chdir::Rooted) => acts
            .into_iter()
            .filter(|a| !matches!(a, Action::ChangeDir { .. }))
            .map(|a| match a {
                Action::Push(mut p) => {
                    p.dir = Dir::Unknown;
                    Action::Push(p)
                }
                other => other,
            })
            .collect(),
    }
}

/// The same action, done from `base`.
pub(crate) fn place(a: Action, base: &Dir) -> Action {
    match a {
        Action::Push(mut p) => {
            p.dir = base.join(&p.dir);
            Action::Push(p)
        }
        other => other,
    }
}

/// A child shell's actions: its directory changes die with it, so they are dropped and the
/// rest already carry the directory they resolved to.
pub(crate) fn sealed(actions: Vec<Action>) -> Vec<Action> {
    actions
        .into_iter()
        .filter(|a| !matches!(a, Action::ChangeDir { .. }))
        .collect()
}
