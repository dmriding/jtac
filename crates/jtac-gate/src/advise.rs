//! The advisory an agent's tool-call hook returns (DESIGN.md section 11.3): one line per
//! action that would meet the gate, from the classifier's actions and the decision the push
//! gatherer reached for each push. Pure: the caller judged; this only says it.

use crate::classify::{Action, Bypass, Push, Unread};
use crate::push::{Decision, Refuse};

/// The note for a script, `None` when nothing in it would meet the gate. `decided` is the
/// push gatherer's answer for one push: `None` when where it runs is unnamed or not a
/// repository this process can read. A pass
/// and a dry run are silent; an unreadable command is noted only when its text says push,
/// so a `bash -c` or an `xargs git show` that cannot push stays quiet. A bypass on a
/// command that is not a push is silent unless it lasts past the command: jtac's gate is
/// the pre-push hook, and `commit-msg` ships empty.
pub fn note(actions: &[Action], decided: &dyn Fn(&Push) -> Option<Decision>) -> Option<String> {
    let lines: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            Action::Push(p) if p.dry_run => None,
            Action::Push(p) => match p.bypass {
                Some(b) => Some(bypass(b, &p.raw)),
                None => push(p, decided(p).as_ref()),
            },
            Action::HookBypass { spelling, raw } if lasts(*spelling) => {
                Some(bypass(*spelling, raw))
            }
            Action::HookBypass { .. } => None,
            Action::Unreadable { why, raw } if raw.contains("push") => Some(unread(why, raw)),
            Action::ChangeDir { .. } | Action::Unreadable { .. } => None,
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn push(p: &Push, decided: Option<&Decision>) -> Option<String> {
    let cmd = short(&p.raw);
    let line = match decided {
        None => format!(
            "jtac: `{cmd}` could not be judged: where it runs or what it pushes is not named \
             here. Run `jtac status` there before pushing."
        ),
        Some(Decision::Pass(_)) => return None,
        Some(Decision::Refuse(Refuse::Absent)) => format!(
            "jtac: `{cmd}` pushes a tip with no root, and the pre-push hook refuses that. Run \
             `jtac check` first, then push."
        ),
        Some(Decision::Refuse(Refuse::Rewind)) => format!(
            "jtac: `{cmd}` rewinds a protected branch; the policy denies that and the hook \
             refuses it. This is a fence with no clearing command: push to a new branch instead."
        ),
        Some(Decision::Refuse(Refuse::Delete)) => format!(
            "jtac: `{cmd}` deletes a protected branch; the policy denies that and the hook \
             refuses it. This is a fence with no clearing command."
        ),
        Some(Decision::Refuse(Refuse::Unknown)) => format!(
            "jtac: `{cmd}` updates a branch whose remote tip this clone does not hold, so the \
             hook will refuse it. Run `git fetch` first, then push."
        ),
    };
    Some(line)
}

/// Decided: states what lands and what shows it, names no command, since a bypass may be
/// the operator's own fence.
fn bypass(spelling: Bypass, raw: &str) -> String {
    let cmd = short(raw);
    match spelling {
        Bypass::ConfigWrite => format!(
            "jtac: `{cmd}` moves git's hooks for every later command in this repository, so \
             no push from here is judged until it is unset; `jtac verify` shows each one as \
             uncovered."
        ),
        Bypass::ConfigParameters | Bypass::ConfigEnvVar => format!(
            "jtac: `{cmd}` moves git's hooks for the rest of this script, so a push after it \
             lands with no root and `jtac verify` shows it as uncovered."
        ),
        _ => format!(
            "jtac: `{cmd}` skips the pre-push hook; the push lands with no root and \
             `jtac verify` shows it as uncovered."
        ),
    }
}

/// A bypass that outlives its own command: a config write, or an exported variable.
fn lasts(spelling: Bypass) -> bool {
    matches!(
        spelling,
        Bypass::ConfigWrite | Bypass::ConfigParameters | Bypass::ConfigEnvVar
    )
}

fn unread(why: &Unread, raw: &str) -> String {
    let cmd = short(raw);
    let what = match why {
        Unread::Syntax(_) => "did not parse",
        Unread::ShellString | Unread::Eval | Unread::SplitString => {
            "builds a command line the reader cannot see"
        }
        Unread::Runner(_) => "hands a program to another runner",
        Unread::Variable | Unread::Alias | Unread::Include => {
            "names its program or its arguments through a variable"
        }
        Unread::Prompt => "runs a prompt expansion",
        Unread::Nesting | Unread::Size => "is larger than the reader follows",
    };
    format!("jtac: `{cmd}` {what}, so nothing in it was judged. Run `jtac status` before pushing.")
}

/// The first line of the command, cut so a long script keeps the note readable.
fn short(raw: &str) -> String {
    const MAX: usize = 120;
    let line = raw.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(MAX) {
        Some((at, _)) => format!("{}...", &line[..at]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify;
    use crate::push::Pass;

    fn refuse(r: Refuse) -> impl Fn(&Push) -> Option<Decision> {
        move |_| Some(Decision::Refuse(r.clone()))
    }

    #[test]
    fn a_push_without_a_root_names_jtac_check_and_a_fence_names_no_command() {
        let script = "git push origin main";
        let absent = note(&classify(script), &refuse(Refuse::Absent)).unwrap();
        assert!(absent.contains("Run `jtac check` first"), "{absent}");
        let rewind = note(&classify(script), &refuse(Refuse::Rewind)).unwrap();
        assert!(
            rewind.contains("fence with no clearing command"),
            "{rewind}"
        );
        let unknown = note(&classify(script), &refuse(Refuse::Unknown)).unwrap();
        assert!(unknown.contains("Run `git fetch` first"), "{unknown}");
    }

    #[test]
    fn a_pass_a_dry_run_and_a_cd_are_silent() {
        let pass = |_: &Push| Some(Decision::Pass(Pass::Root(jtac_core::State::Covered)));
        assert_eq!(note(&classify("git push origin main"), &pass), None);
        let script = "cd app && git push --dry-run origin main";
        assert_eq!(note(&classify(script), &refuse(Refuse::Absent)), None);
        assert_eq!(note(&classify("cargo test"), &refuse(Refuse::Absent)), None);
    }

    #[test]
    fn an_unjudged_directory_says_so_and_names_status() {
        let n = note(&classify("cd \"$D\" && git push"), &|_| None).unwrap();
        assert!(
            n.contains("could not be judged") && n.contains("`jtac status`"),
            "{n}"
        );
    }

    #[test]
    fn a_bypass_states_what_lands_and_names_no_command() {
        let n = note(&classify("git push --no-verify origin main"), &|_| None).unwrap();
        assert!(n.contains("skips the pre-push hook"), "{n}");
        assert!(n.contains("`jtac verify` shows it as uncovered"), "{n}");
        assert!(!n.contains("Run `"), "{n}");
        let w = note(&classify("git config core.hooksPath /dev/null"), &|_| None).unwrap();
        assert!(w.contains("every later command"), "{w}");
        let e = "export GIT_CONFIG_PARAMETERS=\"'core.hooksPath=/dev/null'\"; git push";
        let w = note(&classify(e), &|_| None).unwrap();
        assert!(w.contains("rest of this script"), "{w}");
        for quiet in [
            "git commit --no-verify -m x",
            "git commit -n -m x",
            "git merge --no-verify topic",
            "git -c core.hooksPath=/dev/null log",
        ] {
            assert_eq!(note(&classify(quiet), &|_| None), None, "{quiet}");
        }
    }

    #[test]
    fn an_unreadable_command_is_noted_only_when_its_text_names_git() {
        let n = note(&classify("bash -c \"$X\""), &|_| None);
        assert_eq!(n, None);
        let n = note(&classify("eval \"git $ARGS\""), &|_| None);
        assert_eq!(n, None, "a reader cannot tell a push from a log");
        let n = note(&classify("git log -n3 | xargs -n1 git show"), &|_| None);
        assert_eq!(n, None);
        let n = note(&classify("eval \"git push $ARGS\""), &|_| None).unwrap();
        assert!(n.contains("nothing in it was judged"), "{n}");
    }

    #[test]
    fn every_line_starts_with_jtac_and_a_long_script_is_cut_to_its_first_line() {
        let long = format!("git push origin {}\ngit push origin b", "x".repeat(200));
        let n = note(&classify(&long), &refuse(Refuse::Absent)).unwrap();
        assert_eq!(n.lines().count(), 2);
        assert!(n.lines().all(|l| l.starts_with("jtac: `git push origin")));
        assert!(n.lines().next().unwrap().contains("...`"), "{n}");
    }
}
