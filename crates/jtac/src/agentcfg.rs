//! `jtac init --agent <name>` (DESIGN.md section 11.7 step 5): the agent's hook entry into
//! its per-clone settings file. Opt-in, because the file is the agent's and not jtac's;
//! without the flag init only says where the entry goes, and only to a clone that shows
//! the agent.

use std::path::{Path, PathBuf};

use crate::agent;
use crate::init::ask;
use crate::init::slashes;
use crate::initout::InitOutput;
use crate::refuse::Refusal;
use crate::wire::claude::{self, Merge};

/// The write the flag asked for, judged before anything is written.
pub struct Planned {
    path: PathBuf,
    merge: Merge,
}

/// Read the agent's file and decide the write; `None` without the flag. The only name is
/// Claude Code's; another refuses and says so.
pub fn plan(repo: &Path, exe: &str, name: Option<&str>) -> Result<Option<Planned>, Refusal> {
    let Some(name) = name else {
        return Ok(None);
    };
    if name != claude::NAME {
        return Err(Refusal::init_no_agent(name));
    }
    let path = repo.join(claude::SETTINGS);
    for at in [path.parent().unwrap_or(&path), &path] {
        if at.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
            return Err(Refusal::init_agent_symlink(at));
        }
    }
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Refusal::init_io("the agent settings", &path, &e)),
    };
    let merge = claude::merge(existing.as_deref(), exe)
        .map_err(|why| Refusal::init_agent_file(&path, &why))?;
    Ok(Some(Planned { path, merge }))
}

impl Planned {
    /// Whether [`apply`] writes.
    pub fn writes(&self) -> bool {
        matches!(self.merge, Merge::Written(_))
    }
}

/// The step: write the file, or record that it is in place; without the flag, a note when
/// the clone shows the agent (a `.claude` directory, or the session's own marker).
pub fn apply(
    out: &mut InitOutput,
    planned: Option<&Planned>,
    repo: &Path,
    dry: bool,
) -> Result<(), Refusal> {
    let Some(p) = planned else {
        let seen = repo.join(".claude").is_dir() || agent::here() == Some("CLAUDECODE");
        if seen {
            out.notes.push(format!(
                "Claude Code: `jtac init --agent {}` registers the advisory tool-call hook in \
                 {}; the hook tells the agent about a fence before it pushes and never blocks",
                claude::NAME,
                claude::SETTINGS
            ));
        }
        return Ok(());
    };
    let shown = slashes(&p.path);
    match &p.merge {
        Merge::Present => out.kept("agent", format!("tool-call hook {shown}")),
        Merge::Written(text) => {
            if !dry {
                if let Some(dir) = p.path.parent() {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| Refusal::init_io("the agent settings", dir, &e))?;
                }
                std::fs::write(&p.path, text)
                    .map_err(|e| Refusal::init_io("the agent settings", &p.path, &e))?;
                exclude(repo)?;
            }
            out.wrote("agent", format!("tool-call hook {shown}"));
        }
    }
    Ok(())
}

/// The per-clone file in git's per-clone exclude list, once: it names this machine's
/// binary, so it is never committed, and left untracked it would make the work tree dirty
/// for `jtac check` (`the_agent_hook_is_opt_in_written_once_and_named_for_other_clones`).
fn exclude(repo: &Path) -> Result<(), Refusal> {
    let line = format!("/{}", claude::SETTINGS);
    let dir = repo.join(".git").join("info");
    let path = dir.join("exclude");
    let had = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(Refusal::init_io("the exclude list", &path, &e)),
    };
    if had.lines().any(|l| l.trim() == line) {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| Refusal::init_io("the exclude list", &dir, &e))?;
    let sep = if had.is_empty() || had.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    std::fs::write(&path, format!("{had}{sep}{line}\n"))
        .map_err(|e| Refusal::init_io("the exclude list", &path, &e))
}

impl Refusal {
    /// `--agent` names an agent with no adapter.
    pub fn init_no_agent(name: &str) -> Self {
        ask(
            format!("`--agent {name}` names an agent jtac has no adapter for."),
            "Each agent's tool-call hook speaks its own wire; an adapter is written per agent \
             and v0.1 ships one.",
            format!("Run `jtac init --agent {}`.", claude::NAME),
        )
    }

    /// `.claude` or the settings file is a link.
    pub fn init_agent_symlink(path: &Path) -> Self {
        let path = slashes(path);
        ask(
            format!("`{path}` is a symlink."),
            "jtac writes the agent's settings only as a file of this clone, never through a \
             link that can point anywhere on the machine.",
            format!(
                "Replace the link with a directory or file of this clone, then run `jtac init \
                 --agent {}` again.",
                claude::NAME
            ),
        )
    }

    /// The agent's settings file cannot take the entry.
    pub fn init_agent_file(path: &Path, why: &str) -> Self {
        let path = slashes(path);
        ask(
            format!("`{path}` cannot take the hook entry: {why}."),
            "jtac adds one entry to the agent's own file and rewrites nothing else in it; a \
             file it cannot read as that shape is left alone. Do not delete it to get past this.",
            format!(
                "Fix the file so it is a JSON object whose `hooks.PreToolUse` is an array, or \
                 add this entry to it by hand, then run `jtac init --agent {}` again:\n{}",
                claude::NAME,
                claude::fragment("<absolute path to jtac>")
            ),
        )
    }
}
