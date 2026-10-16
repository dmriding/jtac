//! `git …` as the text says it: the directory, the hook bypass in any spelling, and for
//! `push` the refspecs and the force form. Global options are read as git reads them, so a
//! value never passes for the subcommand; a long option may be abbreviated, as git allows.

use super::program::{base_name, is_assignment};
use super::{Action, Bypass, Dir, Force, Push, Unread};
use crate::shell::{Simple, Word};

/// What a `KEY=value` word does to every git command that sees it in its environment.
enum Env {
    Bypass(Bypass),
    /// `alias.*`: the command that runs is not in the text.
    Alias,
    /// `GIT_DIR` or `GIT_WORK_TREE` set to something: the repository is no longer here.
    Retarget,
}

fn env_effect(text: &str) -> Option<Env> {
    let (k, v) = text.split_once('=')?;
    let lower = v.to_ascii_lowercase();
    if k.starts_with("GIT_CONFIG_KEY_") {
        return match lower.as_str() {
            "core.hookspath" => Some(Env::Bypass(Bypass::ConfigEnvVar)),
            a if a.starts_with("alias.") => Some(Env::Alias),
            _ => None,
        };
    }
    if k == "GIT_CONFIG_PARAMETERS" {
        return if lower.contains("core.hookspath") {
            Some(Env::Bypass(Bypass::ConfigParameters))
        } else if lower.contains("alias.") {
            Some(Env::Alias)
        } else {
            None
        };
    }
    (matches!(k, "GIT_DIR" | "GIT_WORK_TREE") && !v.is_empty()).then_some(Env::Retarget)
}

/// A bare assignment or an `export` that every git command after it in this shell sees: a
/// bypass variable, an alias, or a repository that is no longer here.
pub(crate) fn exported(cmd: &Simple) -> Option<Action> {
    let bare = cmd.words.iter().all(|w| is_assignment(&w.text));
    let decl = matches!(base_name(cmd.at(0)), "export" | "declare" | "typeset");
    if !(bare || decl) {
        return None;
    }
    let raw = cmd.raw.clone();
    cmd.words
        .iter()
        .find_map(|w| env_effect(&w.text))
        .map(|e| match e {
            Env::Bypass(spelling) => Action::HookBypass { spelling, raw },
            Env::Alias => Action::Unreadable {
                why: Unread::Alias,
                raw,
            },
            Env::Retarget => Action::ChangeDir { to: None, raw },
        })
}

/// Does `t` spell `full`, in full or as the abbreviation git accepts? Five characters is the
/// shortest prefix this reader honours; an ambiguous one git refuses anyway.
fn long(t: &str, full: &str) -> bool {
    let name = t.split_once('=').map_or(t, |(n, _)| n);
    name.len() >= 5 && full.starts_with(name)
}

/// `key=…` for `core.hooksPath`, or an alias, in a config value.
fn config_value(value: &str) -> Option<Env> {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("core.hookspath=") {
        Some(Env::Bypass(Bypass::ConfigFlag))
    } else if lower.starts_with("alias.") {
        Some(Env::Alias)
    } else {
        None
    }
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

/// `--no-verify`, abbreviated or not, before a `--`.
fn flag_bypass(args: &[Word]) -> Option<Bypass> {
    args.iter()
        .take_while(|w| w.text != "--")
        .any(|w| w.text.len() >= 6 && long(&w.text, "--no-verify"))
        .then_some(Bypass::NoVerify)
}

/// Short options of `git commit` that take a value: the rest of the cluster, else the next
/// word, so `-nm x` is `-n -m x` and `-m -n` is a message.
const COMMIT_VALUE_FLAGS: [char; 5] = ['m', 'F', 'C', 'c', 't'];
const COMMIT_VALUE_LONGS: [&str; 9] = [
    "--message",
    "--file",
    "--reuse-message",
    "--reedit-message",
    "--template",
    "--fixup",
    "--squash",
    "--author",
    "--date",
];

fn commit_bypass(args: &[Word]) -> Option<Bypass> {
    let mut skip = false;
    for w in args.iter().take_while(|w| w.text != "--") {
        if skip {
            skip = false;
            continue;
        }
        let t = w.text.as_str();
        if t.len() >= 6 && long(t, "--no-verify") {
            return Some(Bypass::NoVerify);
        }
        if !t.contains('=') && COMMIT_VALUE_LONGS.iter().any(|l| long(t, l)) {
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

/// Subcommands that run a string or an argv of their own: `submodule foreach`, `rebase -x`,
/// `bisect run`, `filter-branch`, `difftool --extcmd`.
const RUNS_A_STRING: [&str; 5] = ["submodule", "rebase", "bisect", "filter-branch", "difftool"];

/// The action of one git command, if it has one this reader reports.
pub(crate) fn git(cmd: &Simple, first: usize) -> Option<Action> {
    let raw = cmd.raw.clone();
    let unreadable = |why| {
        Some(Action::Unreadable {
            why,
            raw: raw.clone(),
        })
    };
    let hook_bypass = |spelling| {
        Some(Action::HookBypass {
            spelling,
            raw: raw.clone(),
        })
    };
    let env = &cmd.words[..first.saturating_sub(1)];
    let mut retargeted = false;
    let mut bypass = None;
    for e in env.iter().filter_map(|w| env_effect(&w.text)) {
        match e {
            Env::Bypass(b) => bypass = bypass.or(Some(b)),
            Env::Alias => return unreadable(Unread::Alias),
            Env::Retarget => retargeted = true,
        }
    }
    let mut dir = Dir::Here;
    let mut i = first;
    // `git-push …` is `git push …` with no room for global options.
    let dashed = base_name(cmd.at(first.saturating_sub(1)));
    let sub = if let Some(sub) = dashed
        .get(4..)
        .filter(|_| dashed[..4].eq_ignore_ascii_case("git-"))
    {
        i = first.saturating_sub(1);
        sub.to_string()
    } else {
        while let Some(w) = cmd.words.get(i) {
            if w.unresolved {
                return unreadable(Unread::Variable);
            }
            let t = w.text.as_str();
            let value = cmd.at(i + 1);
            match t {
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
                _ if t.starts_with("--work-tree=") || t.starts_with("--git-dir=") => {
                    retargeted = true;
                    i += 1;
                }
                "-c" | "--config-env" | "--namespace" | "--exec-path" | "--attr-source"
                | "--super-prefix" => {
                    match (t, config_value(value)) {
                        ("-c" | "--config-env", Some(Env::Alias)) => {
                            return unreadable(Unread::Alias);
                        }
                        ("-c", Some(Env::Bypass(b))) => bypass = bypass.or(Some(b)),
                        ("--config-env", Some(Env::Bypass(_))) => {
                            bypass = bypass.or(Some(Bypass::ConfigEnvFlag))
                        }
                        _ => {}
                    }
                    i += 2;
                }
                _ if t.starts_with("--config-env=") => {
                    match config_value(&t["--config-env=".len()..]) {
                        Some(Env::Alias) => return unreadable(Unread::Alias),
                        Some(Env::Bypass(_)) => bypass = bypass.or(Some(Bypass::ConfigEnvFlag)),
                        _ => {}
                    }
                    i += 1;
                }
                _ if t.starts_with('-') => i += 1,
                _ => break,
            }
        }
        cmd.at(i).to_string()
    };
    // A work tree does not say which repository the refs come from.
    if retargeted {
        dir = Dir::Unknown;
    }
    let args = &cmd.words[(i + 1).min(cmd.words.len())..];
    match sub.as_str() {
        "push" => Some(Action::Push(push(
            args,
            flag_bypass(args).or(bypass),
            dir,
            &raw,
        ))),
        "commit" => commit_bypass(args).or(bypass).and_then(hook_bypass),
        "config" if sets_the_hooks_path(args) => hook_bypass(Bypass::ConfigWrite),
        s if RUNS_A_STRING.contains(&s)
            && args.iter().any(|w| {
                super::indirect::quotes_a_command(&w.text)
                    || base_name(&w.text).eq_ignore_ascii_case("git")
            }) =>
        {
            unreadable(Unread::Runner(format!("git {s}")))
        }
        _ => flag_bypass(args).or(bypass).and_then(hook_bypass),
    }
}

/// Long options of `git push` whose value is a separate word.
const PUSH_VALUE_LONGS: [&str; 5] = [
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
            .filter(|s| !t.starts_with("--") && s.chars().all(|c| c.is_ascii_alphanumeric()))
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
        if t == "--" {
            flags_done = true;
        } else if long(t, "--mirror") {
            p.all_refs = true;
            p.force = Force::Force;
        } else if long(t, "--all") || long(t, "--branches") {
            p.all_refs = true;
        } else if long(t, "--dry-run") {
            p.dry_run = true;
        } else if long(t, "--force-with-lease") && t.len() > "--force".len() {
            if p.force == Force::No {
                p.force = Force::WithLease;
            }
        } else if long(t, "--force") {
            p.force = Force::Force;
        } else if long(t, "--delete") {
            p.delete = true;
            delete_flag = true;
        } else if long(t, "--repo") && t.contains('=') {
            repo_flag = t.split_once('=').map(|(_, v)| v.to_string());
        } else if !t.contains('=') && PUSH_VALUE_LONGS.iter().any(|l| long(t, l)) {
            skip_next = true;
            take_repo = long(t, "--repo");
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
