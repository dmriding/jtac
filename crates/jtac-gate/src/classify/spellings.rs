#![cfg(test)]
//! The one table (DESIGN.md section 11.4): every spelling the classifier must read, bypasses
//! first. A new bypass is a new row, with the fix.

use super::{Action, Dir, Force, Unread, classify};

fn show(actions: &[Action]) -> String {
    let mut lines = Vec::new();
    for a in actions {
        lines.push(match a {
            Action::Push(p) => {
                let mut s = format!("push {}", p.remote.as_deref().unwrap_or("?"));
                for (src, dst) in p.sources.iter().zip(&p.destinations) {
                    s.push_str(&format!(" {src}>{dst}"));
                }
                for dst in p.destinations.iter().skip(p.sources.len()) {
                    s.push_str(&format!(" >{dst}"));
                }
                match p.force {
                    Force::No => {}
                    Force::Force => s.push_str(" force"),
                    Force::WithLease => s.push_str(" lease"),
                }
                for (on, word) in [
                    (p.delete, "delete"),
                    (p.all_refs, "all"),
                    (p.dry_run, "dry"),
                    (p.opaque, "opaque"),
                ] {
                    if on {
                        s.push_str(&format!(" {word}"));
                    }
                }
                if let Some(b) = p.bypass {
                    s.push_str(&format!(" bypass={b:?}"));
                }
                match &p.dir {
                    Dir::Here => {}
                    Dir::At(d) => s.push_str(&format!(" @{d}")),
                    Dir::Unknown => s.push_str(" @?"),
                }
                s
            }
            Action::HookBypass { spelling, .. } => format!("bypass {spelling:?}"),
            Action::ChangeDir { to: Some(t), .. } => format!("cd {t}"),
            Action::ChangeDir { to: None, .. } => "cd ?".to_string(),
            Action::Unreadable { why, .. } => format!(
                "unreadable {}",
                match why {
                    Unread::Syntax(_) => "Syntax".to_string(),
                    Unread::Runner(p) => format!("Runner({p})"),
                    other => format!("{other:?}"),
                }
            ),
        });
    }
    lines.join(" ; ")
}

/// Hook bypasses, every spelling.
const BYPASSES: &[(&str, &str)] = &[
    ("git push --no-verify", "push ? bypass=NoVerify"),
    (
        "git push origin main --no-verify",
        "push origin main>main bypass=NoVerify",
    ),
    ("git commit -n -m x", "bypass CommitShortN"),
    ("git commit -nm x", "bypass CommitShortN"),
    ("git commit -anm x", "bypass CommitShortN"),
    ("git commit -m -n", ""),
    ("git commit -m x -n", "bypass CommitShortN"),
    ("git commit --no-verify -m x", "bypass NoVerify"),
    ("git commit -m x -- -n", ""),
    ("git merge --no-verify topic", "bypass NoVerify"),
    ("git am --no-verify < p", "bypass NoVerify"),
    (
        "git -c core.hooksPath=/dev/null push origin main",
        "push origin main>main bypass=ConfigFlag",
    ),
    ("git -c CORE.HooksPath= push", "push ? bypass=ConfigFlag"),
    ("git -c core.hooksPath=x status", "bypass ConfigFlag"),
    (
        "git --config-env=core.hooksPath=HP push",
        "push ? bypass=ConfigEnvFlag",
    ),
    (
        "git --config-env=CORE.HOOKSPATH=HP commit -m x",
        "bypass ConfigEnvFlag",
    ),
    (
        "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/tmp git push",
        "push ? bypass=ConfigEnvVar",
    ),
    (
        "env GIT_CONFIG_KEY_0=core.hookspath git push",
        "push ? bypass=ConfigEnvVar",
    ),
    (
        "GIT_CONFIG_PARAMETERS=\"'core.hooksPath=/x'\" git push",
        "push ? bypass=ConfigParameters",
    ),
    (
        "export GIT_CONFIG_PARAMETERS=\"'core.hooksPath=/x'\"; git push",
        "bypass ConfigParameters ; push ?",
    ),
    (
        "GIT_CONFIG_KEY_0=core.hooksPath\ngit push",
        "bypass ConfigEnvVar ; push ?",
    ),
    (
        "declare -x GIT_CONFIG_KEY_1=Core.HooksPath",
        "bypass ConfigEnvVar",
    ),
    ("git config core.hooksPath /dev/null", "bypass ConfigWrite"),
    ("git config --global core.hookspath x", "bypass ConfigWrite"),
    ("git config set core.hooksPath x", "bypass ConfigWrite"),
    ("git config core.hooksPath ''", "bypass ConfigWrite"),
    ("git config --get core.hooksPath", ""),
    ("git config --unset core.hooksPath", ""),
    ("git config core.hooksPath", ""),
    ("git -c alias.p=push p origin main", "unreadable Alias"),
    ("alias g='git push'", "unreadable Alias"),
    ("alias g=git", "unreadable Alias"),
    ("git push -n", "push ? dry"),
    (
        "git push --dry-run origin main",
        "push origin main>main dry",
    ),
    ("git push -o ci.skip origin main", "push origin main>main"),
    ("git push -on origin main", "push origin main>main"),
    (
        "git push origin -- --no-verify",
        "push origin --no-verify>--no-verify",
    ),
    ("git push # --no-verify", "push ?"),
    ("git commit -m '--no-verify'", ""),
];

/// Force forms.
const FORCE: &[(&str, &str)] = &[
    ("git push -f origin main", "push origin main>main force"),
    (
        "git push --force origin main",
        "push origin main>main force",
    ),
    (
        "git push --force-with-lease origin main",
        "push origin main>main lease",
    ),
    (
        "git push --force-with-lease=main:abc origin main",
        "push origin main>main lease",
    ),
    (
        "git push --force-with-lease --force origin main",
        "push origin main>main force",
    ),
    ("git push origin +main", "push origin main>main force"),
    ("git push -fu origin main", "push origin main>main force"),
    ("git push --mirror origin", "push origin force all"),
    ("git push --all origin", "push origin all"),
    (
        "git push origin 'refs/heads/*'",
        "push origin refs/heads/*>refs/heads/* all",
    ),
];

/// Deletions and refspecs.
const REFSPECS: &[(&str, &str)] = &[
    ("git push -d origin old", "push origin >old delete"),
    ("git push --delete origin a b", "push origin >a >b delete"),
    ("git push origin :old", "push origin >old delete"),
    ("git push -fd origin old", "push origin >old force delete"),
    (
        "git push origin main:refs/heads/x",
        "push origin main>refs/heads/x",
    ),
    (
        "git push origin HEAD:main topic",
        "push origin HEAD>main topic>topic",
    ),
    (
        "git push origin tag v1",
        "push origin refs/tags/v1>refs/tags/v1",
    ),
    ("git push --repo=origin", "push origin"),
    ("git push --repo origin main", "push main"),
    (
        "git push origin \"$BRANCH\"",
        "push origin $BRANCH>$BRANCH opaque",
    ),
    (
        "git push origin $(git branch --show-current)",
        "push origin $(git branch --show-current)>$(git branch --show-current) opaque",
    ),
];

/// Wrappers and spellings of git.
const WRAPPERS: &[(&str, &str)] = &[
    ("sudo git push", "push ?"),
    ("env -i git push", "push ?"),
    ("nice -n 10 git push", "push ?"),
    ("timeout 60 git push", "push ?"),
    ("command git push", "push ?"),
    ("exec git push", "push ?"),
    ("exec -a sh git push", "push ?"),
    ("nohup git push &", "push ?"),
    ("time git push", "push ?"),
    ("xargs git push < list", "push ?"),
    ("/usr/bin/git push", "push ?"),
    ("'C:/Program Files/Git/cmd/git.exe' push", "push ?"),
    ("GIT.EXE push", "push ?"),
    ("\\git push", "push ?"),
    (r"git pu\sh", "push ?"),
    ("\"git\" 'push'", "push ?"),
    ("git commit $'\\x2dn' -m x", "bypass CommitShortN"),
    (
        "git push $'\\x2d\\x66' origin main",
        "push origin main>main force",
    ),
    ("A=1 B=2 git push", "push ?"),
];

/// Directories.
const DIRECTORIES: &[(&str, &str)] = &[
    ("cd ../other && git push", "cd ../other ; push ? @../other"),
    (
        "cd ../other; git push; git push",
        "cd ../other ; push ? @../other ; push ? @../other",
    ),
    ("git -C ../a -C b push", "push ? @../a/b"),
    ("git -C /abs push", "push ? @/abs"),
    ("cd x && git -C /abs push", "cd x ; push ? @/abs"),
    ("cd \"$D\" && git push", "cd ? ; push ? @?"),
    ("cd x || cd y; git push", "cd x ; cd ? ; push ? @?"),
    ("cd -; git push", "cd ? ; push ? @?"),
    ("pushd x; git push", "cd x ; push ? @x"),
    ("popd; git push", "cd ? ; push ? @?"),
    ("source env.sh; git push", "cd ? ; push ? @?"),
    (". ./env.sh && git push", "cd ? ; push ? @?"),
    ("if true; then cd x; fi; git push", "cd ? ; push ? @?"),
    ("cd ~/x && git push", "cd ~/x ; push ? @~/x"),
    ("cd ~bob/x && git push", "cd ? ; push ? @?"),
    ("cd -P -- ../x && git push", "cd ../x ; push ? @../x"),
    ("env -C ../x git push", "push ? @../x"),
    ("env -iC../x git push", "push ? @../x"),
    ("env --chdir=../x git push", "push ? @../x"),
    ("env -C \"$D\" git push", "push ? @?"),
    ("env -C \"$D\" make", ""),
    ("sudo -D ../x git push", "push ? @../x"),
    ("sudo -u bob -D ../x git push", "push ? @../x"),
    ("sudo -R /jail git push", "push ? @?"),
    ("env -S 'git push origin main'", "unreadable SplitString"),
    ("( cd ../x && git push ); git push", "push ? @../x ; push ?"),
    (
        "{ cd ../x && git push; }; git push",
        "cd ../x ; push ? @../x ; push ? @../x",
    ),
    (
        "echo \"$(cd ../x && git push)\"; git push",
        "push ? @../x ; push ?",
    ),
    ("git --git-dir=/r/.git push", "push ? @?"),
    ("git --work-tree . push", "push ? @?"),
    ("GIT_DIR=/x git push", "push ? @?"),
    ("GIT_DIR= git push", "push ?"),
    ("git --git-dir=x log", ""),
];

/// Indirection.
const INDIRECTION: &[(&str, &str)] = &[
    (
        "bash -lc 'git push -f origin main'",
        "push origin main>main force",
    ),
    (
        "sh -c 'cd ../x && git push'; git push",
        "push ? @../x ; push ?",
    ),
    ("cd a && bash -c 'cd b && git push'", "cd a ; push ? @a/b"),
    ("bash -c \"git push $X\"", "unreadable ShellString"),
    ("bash -c \"$SCRIPT\"", "unreadable ShellString"),
    ("busybox sh -c 'git push'", "push ?"),
    ("eval git push origin main", "push origin main>main"),
    ("eval 'cd ../x'; git push", "cd ../x ; push ? @../x"),
    ("eval \"$CMD\"", "unreadable Eval"),
    ("trap 'git push' EXIT", "push ?"),
    ("trap - EXIT", ""),
    ("trap -p", ""),
    ("trap \"$T\" EXIT", "unreadable ShellString"),
    ("bash <<'EOF'\ngit push\nEOF", "push ?"),
    ("bash -s <<EOF\ngit push $B\nEOF", "unreadable ShellString"),
    ("bash script.sh", ""),
    (
        "python3 -c \"import os; os.system('git push')\"",
        "unreadable Runner(python3)",
    ),
    ("python3.12 -c 'print(1)'", ""),
    ("ssh host 'git push'", "unreadable Runner(ssh)"),
    ("ssh host uptime", ""),
    ("docker run img git push", "unreadable Runner(docker)"),
    (r"find . -exec git push \;", "unreadable Runner(find)"),
    ("parallel git push ::: a b", "unreadable Runner(parallel)"),
    ("fish -c 'git push'", "unreadable Runner(fish)"),
    ("pwsh -Command 'git push'", "unreadable Runner(pwsh)"),
    (
        "uv run python -c 'import os; os.system(\"git push\")'",
        "unreadable Runner(uv)",
    ),
    ("npx something 'bash -c x'", "unreadable Runner(npx)"),
];

/// Data, not actions.
const DATA: &[(&str, &str)] = &[
    ("echo \"run git push later\"", ""),
    ("echo git push", "unreadable Runner(echo)"),
    ("which git", ""),
    ("ls /usr/bin/git", ""),
    ("grep -rn 'git push' docs/", ""),
    ("cargo test --workspace", ""),
    ("git status", ""),
    ("git log --oneline -3", ""),
    ("git commit -m 'adopt jtac'", ""),
    ("git switch -c topic", ""),
    ("git push\0 --no-verify", "push ?"),
    (
        "for b in a b; do git push origin $b; done",
        "push origin $b>$b opaque",
    ),
    (
        "while read b; do git push origin \"$b\"; done < list",
        "push origin $b>$b opaque",
    ),
    ("case x in *) git push;; esac", "push ?"),
    ("f() { git push; }; f", "push ?"),
    ("! git push", "push ?"),
    ("git push 2>&1 | tee log", "push ?"),
    ("git push || true", "push ?"),
    ("`git push`", "push ? ; unreadable Variable"),
    ("echo <(git push)", "push ?"),
    ("until false; do git push; done", "push ?"),
    ("select x in a b; do git push; done", "push ?"),
    ("for ((i = 0; i < 1; i++)); do git push; done", "push ?"),
    (
        "if false; then :; elif true; then git push; else git push -f; fi",
        "push ? ; push ? force",
    ),
    ("coproc git push", "push ?"),
    ("time -p git push", "push ?"),
    ("export X=$(git push)", "push ?"),
    ("declare -a arr=(a b)", ""),
    ("a=(git push); \"${a[@]}\"", "unreadable Variable"),
    ("g=git; $g push", "unreadable Variable"),
    ("$HOME/bin/tool build", "unreadable Variable"),
    ("cat <<EOF > f\n$(git push)\nEOF", "push ?"),
    ("cat 3<<'EOF'\ngit push\nEOF", ""),
    ("[[ -n x ]] && git push", "push ?"),
    ("(( 1 + 1 )); git push", "push ?"),
    ("exit 1; git push", "push ?"),
];

/// Unreadable syntax.
const SYNTAX: &[(&str, &str)] = &[
    ("git push \"unterminated", "unreadable Syntax"),
    ("git push 'x", "unreadable Syntax"),
    ("git push $(", "unreadable Syntax"),
    ("git push; fi", "unreadable Syntax"),
];

/// Every row: (script, what it does). An empty second column is a script that does nothing this
/// reader reports. Bypasses first.
const TABLES: &[&[(&str, &str)]] = &[
    BYPASSES,
    FORCE,
    REFSPECS,
    WRAPPERS,
    DIRECTORIES,
    INDIRECTION,
    DATA,
    SYNTAX,
];

#[test]
fn every_row_of_the_spellings_table_classifies_as_written() {
    let rows: Vec<&(&str, &str)> = TABLES.iter().flat_map(|t| t.iter()).collect();
    let misses: Vec<String> = rows
        .iter()
        .filter_map(|(script, want)| {
            let got = show(&classify(script));
            (got != *want).then(|| format!("{script:?}\n    want: {want}\n    got:  {got}"))
        })
        .collect();
    assert!(
        misses.is_empty(),
        "{} of {} rows miss:\n{}",
        misses.len(),
        rows.len(),
        misses.join("\n")
    );
}

#[test]
fn shells_nest_three_deep_and_the_fourth_is_unreadable() {
    let nest = |script: &str| format!("bash -c '{}'", script.replace('\'', "'\\''"));
    let three = nest(&nest(&nest("git push origin main")));
    assert_eq!(show(&classify(&three)), "push origin main>main");
    assert_eq!(show(&classify(&nest(&three))), "unreadable Nesting");
}

#[test]
fn a_script_over_the_size_cap_is_unreadable_before_it_is_parsed() {
    let big = format!("git push {}", "x ".repeat(super::MAX_BYTES / 2));
    assert_eq!(show(&classify(&big)), "unreadable Size");
    let small = format!("git push {}", "x ".repeat(100));
    assert!(show(&classify(&small)).starts_with("push x "));
}

#[test]
fn every_action_keeps_the_text_of_the_command_it_came_from() {
    let actions = classify("cd ../x && git push origin main # go");
    let raws: Vec<&str> = actions
        .iter()
        .map(|a| match a {
            Action::Push(p) => p.raw.as_str(),
            Action::HookBypass { raw, .. }
            | Action::ChangeDir { raw, .. }
            | Action::Unreadable { raw, .. } => raw,
        })
        .collect();
    assert_eq!(raws, ["cd ../x", "git push origin main"]);
}
