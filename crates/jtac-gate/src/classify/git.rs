//! `git …` as the text says it: the directory, the hook bypass in any spelling, and for
//! `push` the refspecs and the force form. Global options are read as git reads them, so a
//! value never passes for the subcommand; a long option may be abbreviated, as git allows.

use super::program::{base_name, is_assignment, names_git};
use super::{Action, Bypass, Dir, Force, Push, Unread};
use crate::shell::{Simple, Word};

/// What a `KEY=value` word does to every git command that sees it in its environment.
#[derive(Clone)]
enum Env {
    Bypass(Bypass),
    /// `alias.*`: the command that runs is not in the text.
    Alias,
    /// `include.path`, `GIT_CONFIG_GLOBAL`: config from a file the text does not show.
    Include,
    /// `GIT_DIR` or `GIT_WORK_TREE` set to something: the repository is no longer here.
    Retarget,
    /// A variable git reads, with a value the shell builds or the text never gives.
    Opaque,
}

/// Variables git reads for its config and repository; a value this reader cannot see is
/// a bypass it cannot rule out.
fn git_reads(k: &str) -> bool {
    k.starts_with("GIT_CONFIG_") || matches!(k, "GIT_DIR" | "GIT_WORK_TREE")
}

fn env_effect(w: &Word) -> Option<Env> {
    let (k, v) = w.text.split_once('=')?;
    if !git_reads(k) {
        return None;
    }
    if w.unresolved {
        return Some(Env::Opaque);
    }
    let lower = v.to_ascii_lowercase();
    if k.starts_with("GIT_CONFIG_KEY_") {
        return match lower.as_str() {
            "core.hookspath" => Some(Env::Bypass(Bypass::ConfigEnvVar)),
            a if a.starts_with("alias.") => Some(Env::Alias),
            a if a.starts_with("include") => Some(Env::Include),
            _ => None,
        };
    }
    if k == "GIT_CONFIG_PARAMETERS" {
        return if lower.contains("core.hookspath") {
            Some(Env::Bypass(Bypass::ConfigParameters))
        } else if lower.contains("alias.") {
            Some(Env::Alias)
        } else if lower.contains("include") {
            Some(Env::Include)
        } else {
            None
        };
    }
    if matches!(k, "GIT_CONFIG_GLOBAL" | "GIT_CONFIG_SYSTEM") {
        return (!v.is_empty()).then_some(Env::Include);
    }
    (matches!(k, "GIT_DIR" | "GIT_WORK_TREE") && !v.is_empty()).then_some(Env::Retarget)
}

/// `PS4='$(cmd) '`: under `set -x`, or in a child shell that inherits it, bash expands the
/// value before every traced command. A leading assignment or a declaration arms it.
pub(crate) fn traced(cmd: &Simple) -> Option<Action> {
    let decl = cmd.words.iter().any(|w| {
        matches!(
            base_name(&w.text),
            "export" | "declare" | "typeset" | "local" | "readonly"
        )
    });
    let armed = cmd.words.iter().enumerate().any(|(i, w)| {
        let leading = cmd.words[..i].iter().all(|p| is_assignment(&p.text));
        (leading || decl)
            && w.text
                .strip_prefix("PS4=")
                .is_some_and(|v| v.contains("$(") || v.contains('`'))
    });
    armed.then(|| Action::Unreadable {
        why: Unread::Prompt,
        raw: cmd.raw.clone(),
    })
}

fn env_action(e: Env, raw: String) -> Action {
    match e {
        Env::Bypass(spelling) => Action::HookBypass { spelling, raw },
        Env::Alias => Action::Unreadable {
            why: Unread::Alias,
            raw,
        },
        Env::Include => Action::Unreadable {
            why: Unread::Include,
            raw,
        },
        Env::Retarget => Action::ChangeDir { to: None, raw },
        Env::Opaque => Action::Unreadable {
            why: Unread::Variable,
            raw,
        },
    }
}

/// A bare assignment or an `export` that every git command after it in this shell sees: a
/// bypass variable, an alias, or a repository that is no longer here. `export NAME` without
/// a value exports whatever an earlier line put there; a nameref (`declare -n`) writes through
/// to a name a later line may choose.
pub(crate) fn exported(cmd: &Simple) -> Option<Action> {
    let mut i = 0;
    while cmd.words.get(i).is_some_and(|w| is_assignment(&w.text)) {
        i += 1;
    }
    let bare = i == cmd.words.len() && i > 0;
    while cmd
        .words
        .get(i)
        .is_some_and(|w| matches!(base_name(&w.text), "builtin" | "command"))
    {
        i += 1;
        while cmd.words.get(i).is_some_and(|w| w.text.starts_with('-')) {
            i += 1;
        }
    }
    let decl = matches!(
        base_name(cmd.at(i)),
        "export" | "declare" | "typeset" | "local" | "readonly"
    );
    if !(bare || decl) {
        return None;
    }
    let words: Vec<&Word> = cmd
        .words
        .iter()
        .skip(if bare { 0 } else { i + 1 })
        .collect();
    let nameref = matches!(base_name(cmd.at(i)), "declare" | "typeset" | "local")
        && words
            .iter()
            .any(|w| w.text.starts_with('-') && w.text.contains('n'));
    let hidden = |w: &&Word| {
        let (key, value) = w.text.split_once('=').unwrap_or((&w.text, ""));
        (value.is_empty() && git_reads(key)) || (w.unresolved && !is_assignment(&w.text))
    };
    let effects: Vec<Env> = words.iter().filter_map(|w| env_effect(w)).collect();
    let pick = |f: fn(&&Env) -> bool| effects.iter().find(f);
    let effect = pick(|e| matches!(e, Env::Bypass(_)))
        .or_else(|| pick(|e| matches!(e, Env::Alias | Env::Include)))
        .or_else(|| pick(|e| matches!(e, Env::Opaque)))
        .or_else(|| (nameref || words.iter().any(hidden)).then_some(&Env::Opaque))
        .or_else(|| pick(|e| matches!(e, Env::Retarget)));
    effect.map(|e| env_action(e.clone(), cmd.raw.clone()))
}

/// Does `t` spell `full`, in full or as the abbreviation git accepts? Five characters is the
/// shortest prefix this reader honours; an ambiguous one git refuses anyway.
fn long(t: &str, full: &str) -> bool {
    let name = t.split_once('=').map_or(t, |(n, _)| n);
    name.len() >= 5 && full.starts_with(name)
}

/// The option `t` spells among `table`, in full or as the unambiguous prefix git accepts.
fn abbrev<'a>(t: &str, table: &[&'a str]) -> Option<&'a str> {
    let name = t.split_once('=').map_or(t, |(n, _)| n);
    if name.len() < 3 {
        return None;
    }
    if let Some(exact) = table.iter().find(|o| **o == name) {
        return Some(exact);
    }
    let mut hits = table.iter().filter(|o| o.starts_with(name));
    match (hits.next(), hits.next()) {
        (Some(o), None) => Some(o),
        _ => None,
    }
}

/// `key=…` for `core.hooksPath`, an alias, or an include, in a config value.
fn config_value(value: &str) -> Option<Env> {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("core.hookspath=") {
        Some(Env::Bypass(Bypass::ConfigFlag))
    } else if lower.starts_with("alias.") {
        Some(Env::Alias)
    } else if lower.starts_with("include") {
        Some(Env::Include)
    } else {
        None
    }
}

enum ConfigWrite {
    Read,
    HooksPath,
    /// A key the shell builds, with a value after it.
    Opaque,
    Other,
}

/// Does this `git config` write `core.hooksPath`? A read, in any spelling, does not; a key
/// the shell builds may. `-f`/`--file` values are skipped so a variable file is not a key.
fn config_write(args: &[Word]) -> ConfigWrite {
    let mut opaque = false;
    let mut file_value = false;
    for (n, w) in args.iter().enumerate() {
        let lower = w.text.to_ascii_lowercase();
        if std::mem::take(&mut file_value) {
            continue;
        }
        file_value = matches!(
            lower.as_str(),
            "-f" | "--file" | "--blob" | "--comment" | "--default" | "-t" | "--type" | "--value"
        );
        if lower.starts_with("--get")
            || lower.starts_with("--unset")
            || matches!(lower.as_str(), "-l" | "--list")
            || (n == 0 && matches!(lower.as_str(), "get" | "list" | "unset" | "unset-all"))
        {
            return ConfigWrite::Read;
        }
        if lower == "core.hookspath" && n + 1 < args.len() {
            return ConfigWrite::HooksPath;
        }
        opaque |= w.unresolved && !lower.starts_with('-') && n + 1 < args.len();
    }
    if opaque {
        ConfigWrite::Opaque
    } else {
        ConfigWrite::Other
    }
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
    for w in args {
        if skip {
            skip = false;
            continue;
        }
        let t = w.text.as_str();
        if t == "--" {
            break;
        }
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

/// The action of one git command, if it has one this reader reports. `git-push …` is
/// `git push …` with no room for global options.
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
    let mut opaque = false;
    for e in env.iter().filter_map(env_effect) {
        match e {
            Env::Bypass(b) => bypass = bypass.or(Some(b)),
            Env::Alias => return unreadable(Unread::Alias),
            Env::Include => return unreadable(Unread::Include),
            Env::Retarget => retargeted = true,
            Env::Opaque => opaque = true,
        }
    }
    let mut dir = Dir::Here;
    let mut i = first;
    let dashed = base_name(cmd.at(first.saturating_sub(1)));
    let sub = if let Some(sub) = dashed
        .get(4..)
        .filter(|_| dashed[..4].eq_ignore_ascii_case("git-"))
    {
        i = first.saturating_sub(1);
        sub.to_ascii_lowercase()
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
                    if cmd.words.get(i + 1).is_some_and(|v| v.unresolved) {
                        return unreadable(Unread::Variable);
                    }
                    match (t, config_value(value)) {
                        ("-c" | "--config-env", Some(Env::Alias)) => {
                            return unreadable(Unread::Alias);
                        }
                        ("-c" | "--config-env", Some(Env::Include)) => {
                            return unreadable(Unread::Include);
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
                        Some(Env::Include) => return unreadable(Unread::Include),
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
    if opaque && bypass.is_none() {
        return unreadable(Unread::Variable);
    }
    let args = &cmd.words[(i + 1).min(cmd.words.len())..];
    match sub.as_str() {
        "push" => Some(Action::Push(push(args, bypass, dir, &raw))),
        "commit" => commit_bypass(args).or(bypass).and_then(hook_bypass),
        "config" => match config_write(args) {
            ConfigWrite::HooksPath => hook_bypass(Bypass::ConfigWrite),
            ConfigWrite::Opaque => unreadable(Unread::Variable),
            ConfigWrite::Read | ConfigWrite::Other => bypass.and_then(hook_bypass),
        },
        s if RUNS_A_STRING.contains(&s)
            && args
                .iter()
                .any(|w| super::indirect::quotes_a_command(&w.text) || names_git(&w.text)) =>
        {
            unreadable(Unread::Runner(format!("git {s}")))
        }
        _ => flag_bypass(args).or(bypass).and_then(hook_bypass),
    }
}

/// Every long option of `git push` and its `--no-` form, so an abbreviation resolves as git
/// resolves it and a later `--no-dry-run` undoes an earlier `-n`.
const PUSH_LONGS: [&str; 54] = [
    "--all",
    "--atomic",
    "--branches",
    "--delete",
    "--dry-run",
    "--exec",
    "--follow-tags",
    "--force",
    "--force-if-includes",
    "--force-with-lease",
    "--ipv4",
    "--ipv6",
    "--mirror",
    "--porcelain",
    "--progress",
    "--prune",
    "--push-option",
    "--quiet",
    "--receive-pack",
    "--recurse-submodules",
    "--repo",
    "--set-upstream",
    "--signed",
    "--tags",
    "--thin",
    "--verbose",
    "--verify",
    "--no-all",
    "--no-atomic",
    "--no-branches",
    "--no-delete",
    "--no-dry-run",
    "--no-exec",
    "--no-follow-tags",
    "--no-force",
    "--no-force-if-includes",
    "--no-force-with-lease",
    "--no-ipv4",
    "--no-ipv6",
    "--no-mirror",
    "--no-porcelain",
    "--no-progress",
    "--no-prune",
    "--no-push-option",
    "--no-quiet",
    "--no-receive-pack",
    "--no-recurse-submodules",
    "--no-repo",
    "--no-set-upstream",
    "--no-signed",
    "--no-tags",
    "--no-thin",
    "--no-verbose",
    "--no-verify",
];

/// Long options of `git push` whose value is a separate word.
const PUSH_VALUE_LONGS: [&str; 5] = [
    "--push-option",
    "--receive-pack",
    "--exec",
    "--recurse-submodules",
    "--repo",
];

// `--mirror` sets git's force bit with its own; `--no-force` and `--no-mirror` both clear it,
// and a `--force-with-lease` given alongside is still armed.
fn unforced(lease: bool) -> Force {
    if lease { Force::WithLease } else { Force::No }
}

// `-fofoo=bar`: `-o` takes the rest of the cluster, or the next word when it ends one.
fn cluster(p: &mut Push, short: &str) -> bool {
    for (k, c) in short.char_indices() {
        match c {
            'f' => p.force = Force::Force,
            'd' => p.delete = true,
            'n' => p.dry_run = true,
            'o' => return k + 1 == short.len(),
            _ => {}
        }
    }
    false
}

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
    let mut lease = false;
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
        if let Some(short) = t.strip_prefix('-').filter(|_| !t.starts_with("--")) {
            skip_next = cluster(&mut p, short);
            delete_flag |= p.delete;
            continue;
        }
        if t == "--" {
            flags_done = true;
            continue;
        }
        match abbrev(t, &PUSH_LONGS) {
            Some("--mirror") => {
                p.all_refs = true;
                p.force = Force::Force;
            }
            Some("--all" | "--branches") => p.all_refs = true,
            Some("--no-all" | "--no-branches") => p.all_refs = false,
            Some("--no-mirror") => {
                p.all_refs = false;
                if p.force == Force::Force {
                    p.force = unforced(lease);
                }
            }
            Some("--dry-run") => p.dry_run = true,
            Some("--no-dry-run") => p.dry_run = false,
            Some("--force-with-lease") => {
                lease = true;
                if p.force == Force::No {
                    p.force = Force::WithLease;
                }
            }
            Some("--force") => p.force = Force::Force,
            Some("--no-force") if p.force == Force::Force => p.force = unforced(lease),
            Some("--no-force-with-lease") => {
                lease = false;
                if p.force == Force::WithLease {
                    p.force = Force::No;
                }
            }
            Some("--delete") => {
                p.delete = true;
                delete_flag = true;
            }
            Some("--no-delete") => {
                p.delete = false;
                delete_flag = false;
            }
            Some("--no-verify") => p.bypass = Some(Bypass::NoVerify),
            Some("--verify") => {
                if p.bypass == Some(Bypass::NoVerify) {
                    p.bypass = bypass;
                }
            }
            Some("--repo") if t.contains('=') => {
                repo_flag = t.split_once('=').map(|(_, v)| v.to_string());
            }
            Some(o) if !t.contains('=') && PUSH_VALUE_LONGS.contains(&o) => {
                skip_next = true;
                take_repo = o == "--repo";
            }
            _ => {}
        }
    }
    p.remote = positional.first().map(|s| (*s).to_string()).or(repo_flag);
    refspecs(&mut p, &positional[positional.len().min(1)..], delete_flag);
    p
}

// `tag v1` is one refspec; `+` forces; `:dst` deletes; a `*` destination is every ref.
fn refspecs(p: &mut Push, positional: &[&str], delete_flag: bool) {
    let mut specs: Vec<String> = positional.iter().map(|s| (*s).to_string()).collect();
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
}
