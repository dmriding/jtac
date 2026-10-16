//! `git …` as the text says it: the directory, the hook bypass in any spelling, and for
//! `push` the refspecs and the force form. Global options are read as git reads them, so a
//! value never passes for the subcommand.

use super::program::{base_name, is_assignment};
use super::{Action, Bypass, Dir, Force, Push, Unread};
use crate::shell::{Simple, Word};

/// A `KEY=value` word that switches the hooks off through git's environment.
fn env_bypass(text: &str) -> Option<Bypass> {
    let (k, v) = text.split_once('=')?;
    if k.starts_with("GIT_CONFIG_KEY_") && v.eq_ignore_ascii_case("core.hookspath") {
        return Some(Bypass::ConfigEnvVar);
    }
    if k == "GIT_CONFIG_PARAMETERS" && v.to_ascii_lowercase().contains("core.hookspath") {
        return Some(Bypass::ConfigParameters);
    }
    None
}

/// A bare assignment or an `export` of a bypass variable: every git command after it in this
/// shell skips its hooks.
pub(crate) fn exported_bypass(cmd: &Simple) -> Option<Action> {
    let bare = cmd.words.iter().all(|w| is_assignment(&w.text));
    let decl = matches!(base_name(cmd.at(0)), "export" | "declare" | "typeset");
    if !(bare || decl) {
        return None;
    }
    cmd.words
        .iter()
        .find_map(|w| env_bypass(&w.text))
        .map(|spelling| Action::HookBypass {
            spelling,
            raw: cmd.raw.clone(),
        })
}

fn names_the_hooks_path(value: &str) -> bool {
    value.to_ascii_lowercase().starts_with("core.hookspath=")
}

/// Does this `git config` write `core.hooksPath`? A read, in any spelling, does not.
fn sets_the_hooks_path(args: &[Word]) -> bool {
    let mut key = false;
    for (n, w) in args.iter().enumerate() {
        let lower = w.text.to_ascii_lowercase();
        if lower.starts_with("--get")
            || lower.starts_with("--unset")
            || matches!(lower.as_str(), "-l" | "--list")
            || (n == 0 && matches!(lower.as_str(), "get" | "list" | "unset" | "unset-all"))
        {
            return false;
        }
        if lower == "core.hookspath" && n + 1 < args.len() {
            key = true;
        }
    }
    key
}

/// `--no-verify` before a `--`.
fn flag_bypass(args: &[Word]) -> Option<Bypass> {
    args.iter()
        .take_while(|w| w.text != "--")
        .any(|w| w.text == "--no-verify")
        .then_some(Bypass::NoVerify)
}

/// Short options of `git commit` that take a value: the rest of the cluster, else the next
/// word, so `-nm x` is `-n -m x` and `-m -n` is a message.
const COMMIT_VALUE_FLAGS: [char; 5] = ['m', 'F', 'C', 'c', 't'];

fn commit_bypass(args: &[Word]) -> Option<Bypass> {
    let mut skip = false;
    for w in args.iter().take_while(|w| w.text != "--") {
        if skip {
            skip = false;
            continue;
        }
        let t = w.text.as_str();
        if t == "--no-verify" {
            return Some(Bypass::NoVerify);
        }
        if matches!(
            t,
            "--message" | "--file" | "--reuse-message" | "--reedit-message" | "--template"
        ) {
            skip = true;
            continue;
        }
        let Some(letters) = t
            .strip_prefix('-')
            .filter(|l| !t.starts_with("--") && !l.is_empty())
        else {
            continue;
        };
        for (k, c) in letters.char_indices() {
            if c == 'n' {
                return Some(Bypass::CommitShortN);
            }
            if COMMIT_VALUE_FLAGS.contains(&c) {
                skip = k + 1 == letters.len();
                break;
            }
        }
    }
    None
}

/// The action of one git command, if it has one this reader reports.
pub(crate) fn git(cmd: &Simple, first: usize) -> Option<Action> {
    let env = &cmd.words[..first.saturating_sub(1)];
    let mut retargeted = env.iter().any(|w| {
        matches!(w.text.split_once('='), Some((k, v)) if !v.is_empty() && matches!(k, "GIT_DIR" | "GIT_WORK_TREE"))
    });
    let mut bypass = env.iter().find_map(|w| env_bypass(&w.text));
    let mut dir = Dir::Here;
    let mut i = first;
    while let Some(w) = cmd.words.get(i) {
        let t = w.text.as_str();
        match t {
            // Each `-C` is relative to the one before it.
            "-C" => {
                dir = match cmd.words.get(i + 1) {
                    Some(d) if !d.unresolved => dir.join(&Dir::At(d.text.clone())),
                    _ => Dir::Unknown,
                };
                i += 2;
            }
            "--git-dir" | "--work-tree" => {
                retargeted = true;
                i += 2;
            }
            "-c" if cmd.at(i + 1).starts_with("alias.") => {
                return Some(Action::Unreadable {
                    why: Unread::Alias,
                    raw: cmd.raw.clone(),
                });
            }
            "-c" if names_the_hooks_path(cmd.at(i + 1)) => {
                bypass.get_or_insert(Bypass::ConfigFlag);
                i += 2;
            }
            _ if t
                .to_ascii_lowercase()
                .starts_with("--config-env=core.hookspath=") =>
            {
                bypass.get_or_insert(Bypass::ConfigEnvFlag);
                i += 1;
            }
            _ if t.starts_with("--work-tree=") || t.starts_with("--git-dir=") => {
                retargeted = true;
                i += 1;
            }
            "-c" | "--namespace" | "--exec-path" | "--attr-source" | "--super-prefix" => i += 2,
            _ if t.starts_with('-') => i += 1,
            _ => break,
        }
    }
    // A work tree does not say which repository the refs come from.
    if retargeted {
        dir = Dir::Unknown;
    }
    let sub = cmd.at(i);
    let args = &cmd.words[(i + 1).min(cmd.words.len())..];
    let hook_bypass = |spelling| Action::HookBypass {
        spelling,
        raw: cmd.raw.clone(),
    };
    match sub {
        "push" => Some(Action::Push(push(
            args,
            flag_bypass(args).or(bypass),
            dir,
            &cmd.raw,
        ))),
        "commit" => commit_bypass(args).or(bypass).map(hook_bypass),
        "config" if sets_the_hooks_path(args) => Some(hook_bypass(Bypass::ConfigWrite)),
        _ => flag_bypass(args).or(bypass).map(hook_bypass),
    }
}

/// Flags of `git push` whose value is a separate word.
const PUSH_VALUE_FLAGS: [&str; 6] = [
    "-o",
    "--push-option",
    "--receive-pack",
    "--exec",
    "--recurse-submodules",
    "--repo",
];

// A short cluster is read left to right and `-o` takes the rest of it, or the next word;
// `origin tag v1` is git's spelling of `refs/tags/v1:refs/tags/v1`.
fn push(args: &[Word], bypass: Option<Bypass>, dir: Dir, raw: &str) -> Push {
    let mut p = Push {
        remote: None,
        sources: Vec::new(),
        destinations: Vec::new(),
        force: Force::No,
        delete: false,
        all_refs: false,
        dry_run: false,
        bypass,
        opaque: args.iter().any(|w| w.unresolved),
        dir,
        raw: raw.to_string(),
    };
    let mut delete_flag = false;
    let mut repo_flag = None;
    let mut positional: Vec<&str> = Vec::new();
    let mut flags_done = false;
    let mut skip_next = false;
    let mut take_repo = false;
    for w in args {
        let t = w.text.as_str();
        if skip_next {
            skip_next = false;
            if take_repo {
                take_repo = false;
                repo_flag = Some(t.to_string());
            }
            continue;
        }
        if flags_done || !t.starts_with('-') || t == "-" {
            positional.push(t);
            continue;
        }
        if let Some(short) = t
            .strip_prefix('-')
            .filter(|s| !t.starts_with("--") && s.chars().all(|c| c.is_ascii_alphabetic()))
        {
            for (k, c) in short.char_indices() {
                match c {
                    'f' => p.force = Force::Force,
                    'd' => {
                        p.delete = true;
                        delete_flag = true;
                    }
                    'n' => p.dry_run = true,
                    'o' => {
                        skip_next = k + 1 == short.len();
                        break;
                    }
                    _ => {}
                }
            }
            continue;
        }
        match t {
            "--" => flags_done = true,
            "--mirror" => {
                p.all_refs = true;
                p.force = Force::Force;
            }
            "--all" | "--branches" => p.all_refs = true,
            "--dry-run" => p.dry_run = true,
            "--force" => p.force = Force::Force,
            _ if t == "--force-with-lease" || t.starts_with("--force-with-lease=") => {
                if p.force == Force::No {
                    p.force = Force::WithLease;
                }
            }
            "--delete" => {
                p.delete = true;
                delete_flag = true;
            }
            _ if t.starts_with("--repo=") => {
                repo_flag = t.strip_prefix("--repo=").map(str::to_string)
            }
            _ if PUSH_VALUE_FLAGS.contains(&t) => {
                skip_next = true;
                take_repo = t == "--repo";
            }
            _ => {}
        }
    }
    p.remote = positional.first().map(|s| (*s).to_string()).or(repo_flag);
    let mut specs: Vec<String> = positional
        .iter()
        .skip(1)
        .map(|s| (*s).to_string())
        .collect();
    let mut i = 0;
    while i + 1 < specs.len() {
        if specs[i] == "tag" {
            specs[i] = format!("refs/tags/{}", specs[i + 1]);
            specs.remove(i + 1);
        }
        i += 1;
    }
    for spec in &specs {
        let spec = match spec.strip_prefix('+') {
            Some(rest) => {
                p.force = Force::Force;
                rest
            }
            None => spec,
        };
        match spec.split_once(':') {
            Some(("", dst)) => {
                p.delete = true;
                p.destinations.push(dst.to_string());
            }
            Some((src, dst)) => {
                p.sources.push(src.to_string());
                p.destinations.push(dst.to_string());
            }
            None => {
                p.sources.push(spec.to_string());
                p.destinations.push(spec.to_string());
            }
        }
    }
    if delete_flag {
        p.sources.clear();
    }
    if p.destinations.iter().any(|d| d.contains('*')) {
        p.all_refs = true;
    }
    p
}
