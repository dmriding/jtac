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
    (
        "git push --no-veri origin main",
        "push origin main>main bypass=NoVerify",
    ),
    ("git commit --no-verif -m x", "bypass NoVerify"),
    ("git merge --no-v topic", "bypass NoVerify"),
    (
        "git --config-env core.hooksPath=HP push origin main",
        "push origin main>main bypass=ConfigEnvFlag",
    ),
    ("git -c alias.p=push p origin main", "unreadable Alias"),
    (
        "git -c 'Alias.p=push --force' p origin main",
        "unreadable Alias",
    ),
    (
        "git --config-env=alias.p=AV p origin main",
        "unreadable Alias",
    ),
    (
        "git --config-env alias.p=AV p origin main",
        "unreadable Alias",
    ),
    (
        "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=alias.p GIT_CONFIG_VALUE_0=push git p origin main",
        "unreadable Alias",
    ),
    (
        "export GIT_CONFIG_PARAMETERS=\"'alias.p=push'\"; git p",
        "unreadable Alias",
    ),
    ("git -c \"$X\" push origin main", "unreadable Variable"),
    (
        "git --config-env \"$X\" push origin main",
        "unreadable Variable",
    ),
    ("git -c \"$X\" commit -m x", "unreadable Variable"),
    (
        "k=core.hooksPath; GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=$k GIT_CONFIG_VALUE_0=/dev/null git push origin main",
        "unreadable Variable",
    ),
    (
        "GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=\"$D\" git push origin main",
        "push origin main>main bypass=ConfigEnvVar",
    ),
    (
        "f() { local -x GIT_CONFIG_PARAMETERS=\"'core.hooksPath=/dev/null'\"; git push origin main; }; f",
        "bypass ConfigParameters ; push origin main>main",
    ),
    (
        "read -r GIT_CONFIG_PARAMETERS <<< x; export GIT_CONFIG_PARAMETERS; git push origin main",
        "unreadable Variable ; push origin main>main",
    ),
    (
        "printf -v GIT_CONFIG_PARAMETERS %s x; builtin export GIT_CONFIG_PARAMETERS; git push",
        "unreadable Variable ; push ?",
    ),
    ("command export GIT_DIR=/x; git push", "cd ? ; push ? @?"),
    ("export GIT_DIR; git push", "unreadable Variable ; push ?"),
    ("GIT_AUTHOR_NAME=\"$N\" git commit -m x", ""),
    (
        "k=core.hooksPath; git config $k /dev/null",
        "unreadable Variable",
    ),
    ("git config user.name \"$NAME\"", ""),
    ("git config -f \"$F\" user.name x", ""),
    (
        "git push -o -- --no-verify origin main",
        "push origin main>main bypass=NoVerify",
    ),
    (
        "git push --no-verify --verify origin main",
        "push origin main>main",
    ),
    (
        "git config --comment --get core.hooksPath /tmp/x",
        "bypass ConfigWrite",
    ),
    (
        "git config set --comment --get core.hooksPath /tmp/y",
        "bypass ConfigWrite",
    ),
    (
        "command -p export GIT_CONFIG_PARAMETERS=\"'core.hookspath=/tmp/x'\"; git push origin main",
        "bypass ConfigParameters ; push origin main>main",
    ),
    ("command -- export GIT_DIR=/x; git push", "cd ? ; push ? @?"),
    (
        "declare -n x=GIT_CONFIG_PARAMETERS; export x=y",
        "unreadable Variable",
    ),
    (
        "declare -n ref; ref=GIT_CONFIG_PARAMETERS; export ref=x; git push",
        "unreadable Variable ; push ?",
    ),
    ("local -n ref=OTHER", "unreadable Variable"),
    (
        "export -n GIT_DIR; git push",
        "unreadable Variable ; push ?",
    ),
    ("declare -x FOO=1; git push", "push ?"),
    (
        "N=GIT_CONFIG_PARAMETERS; export \"$N=x\"; git push",
        "unreadable Variable ; push ?",
    ),
    ("export PATH=\"$PATH:/x\"; git push", "push ?"),
    (
        "export GIT_DIR=.git GIT_CONFIG_PARAMETERS=\"'core.hookspath=/y'\"; git push",
        "bypass ConfigParameters ; push ?",
    ),
    ("git commit -m -- --no-verify", "bypass NoVerify"),
    (
        "git -c include.path=/tmp/x push origin main",
        "unreadable Include",
    ),
    (
        "git -c includeIf.gitdir:/x.path=/y push origin main",
        "unreadable Include",
    ),
    (
        "GIT_CONFIG_GLOBAL=/tmp/x git push origin main",
        "unreadable Include",
    ),
    (
        "export GIT_CONFIG_GLOBAL=/x; git push",
        "unreadable Include ; push ?",
    ),
    (
        "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=include.path GIT_CONFIG_VALUE_0=/x git push",
        "unreadable Include",
    ),
    ("git $OPTS push --force origin main", "unreadable Variable"),
    ("git \"$SUB\" origin main", "unreadable Variable"),
    (
        "g() { git \"$@\"; }; g push --force origin main",
        "unreadable Variable",
    ),
    (
        "git submodule foreach 'git push --force origin main'",
        "unreadable Runner(git submodule)",
    ),
    (
        "git rebase -x 'git push --force origin main' main",
        "unreadable Runner(git rebase)",
    ),
    (
        "git bisect run git push --force origin main",
        "unreadable Runner(git bisect)",
    ),
    ("git rebase main", ""),
    ("git submodule update --init", ""),
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
    ("git push --mirr origin", "push origin force all"),
    ("git push --m origin", "push origin force all"),
    ("git push --de origin main", "push origin >main delete"),
    ("git push --b origin", "push origin all"),
    ("git push --al origin", "push origin all"),
    ("git push --forc origin main", "push origin main>main"),
    (
        "git push --force-if-includes origin main",
        "push origin main>main",
    ),
    (
        "git push --no-force-with-lease origin main",
        "push origin main>main",
    ),
    (
        "git push --force --no-force-with-lease origin main",
        "push origin main>main force",
    ),
    (
        "git push --force-with-lease --no-force origin main",
        "push origin main>main lease",
    ),
    (
        "git push --force --force-with-lease --no-force origin main",
        "push origin main>main lease",
    ),
    ("git push --mirror --no-force origin", "push origin all"),
    (
        "git push --no-force --mirror origin",
        "push origin force all",
    ),
    (
        "git push --mirror --no-mirror origin main",
        "push origin main>main",
    ),
    (
        "git push -f --mirror --no-mirror origin main",
        "push origin main>main",
    ),
    (
        "git push --mirror --force-with-lease --no-mirror origin main",
        "push origin main>main lease",
    ),
    (
        "git push -fofoo=bar origin main",
        "push origin main>main force",
    ),
    ("git push -o=ci.skip origin main", "push origin main>main"),
    (
        "git push -n --no-dry-run origin main",
        "push origin main>main",
    ),
    ("git push -n --no-dry origin main", "push origin main>main"),
    (
        "git push -f --no-force origin main",
        "push origin main>main",
    ),
    ("git push --mirror --no-mirror origin", "push origin"),
    (
        "git push -d --no-delete origin main",
        "push origin main>main",
    ),
    (
        "git push --force-w origin main",
        "push origin main>main lease",
    ),
    (
        "git push -4f origin HEAD~1:main",
        "push origin HEAD~1>main force",
    ),
    ("git push --del origin old", "push origin >old delete"),
    ("git push --dry origin main", "push origin main>main dry"),
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
    (
        "sudo -u git -H git push origin main",
        "push origin main>main",
    ),
    ("env -u git git push origin main", "push origin main>main"),
    ("flock /tmp/lock git push", "push ?"),
    (
        "flock /tmp/lock -c 'git push --force origin main'",
        "push origin main>main force",
    ),
    ("flock -w 5 /tmp/lock git push", "push ?"),
    (
        "flock /tmp/lock --command 'git push --force origin main'",
        "push origin main>main force",
    ),
    (
        "flock -s /tmp/lock git push origin main",
        "push origin main>main",
    ),
    (
        "env --default-signal git push origin main",
        "push origin main>main",
    ),
    (
        "env --ignore-signal=INT git push --force origin main",
        "push origin main>main force",
    ),
    (
        "env --unse git git push origin main",
        "push origin main>main",
    ),
    (
        "env --chd=/elsewhere git push origin main",
        "push origin main>main @/elsewhere",
    ),
    ("env --c /elsewhere git push", "push ? @/elsewhere"),
    (
        "sudo --us git git push origin main",
        "push origin main>main",
    ),
    ("sudo --ch /x git push", "push ? @?"),
    (
        "busybox env git push --force origin main",
        "push origin main>main force",
    ),
    (
        "busybox timeout 5 git push origin main",
        "push origin main>main",
    ),
    ("busybox $APPLET git push", "unreadable Variable"),
    ("café", ""),
    ("éabc", ""),
    ("/opt/café/git push origin main", "push origin main>main"),
    ("Git-Push origin main", "push origin main>main"),
    (
        "taskset 1 /usr/lib/git-core/git-push origin main",
        "unreadable Runner(taskset)",
    ),
    ("chrt -f 5 git-push origin main", "unreadable Runner(chrt)"),
    (
        "echo main | xargs git-push origin",
        "unreadable Runner(xargs)",
    ),
    ("ssh host 'git-push origin main'", "unreadable Runner(ssh)"),
    (
        "find . -exec git-push origin main \\;",
        "unreadable Runner(find)",
    ),
    (
        "/usr/lib/git-core/git-push --force origin main",
        "push origin main>main force",
    ),
    ("git-push origin main", "push origin main>main"),
    (
        "env \"$GIT\" push --force origin main",
        "unreadable Variable",
    ),
    ("command \"$GIT\" push", "unreadable Variable"),
    ("sudo \"$GIT\" push", "unreadable Variable"),
    ("{git,push} --force origin main", "unreadable Variable"),
    ("git pu{sh,} --force origin main", "unreadable Variable"),
    ("git {push,} origin main", "unreadable Variable"),
    (
        "echo --no-verify | xargs git push",
        "unreadable Runner(xargs)",
    ),
    ("xargs git push < list", "unreadable Runner(xargs)"),
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
    ("env -C a env -C b git push", "push ? @a/b"),
    ("env -C a -C b git push", "push ? @b"),
    ("env -C a -C /b git push", "push ? @/b"),
    ("env -C a env -C \"$D\" git push", "push ? @?"),
    ("env -C \"$D\" env -C /b git push", "push ? @?"),
    ("sudo -R /jail env -C /repo git push", "push ? @?"),
    ("env -C a sudo -D b git push", "push ? @a/b"),
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
    ("cd /x | true; git push", "push ?"),
    ("cd /x & git push", "push ?"),
    ("[ -d /x ] && cd /x; git push", "cd ? ; push ? @?"),
    ("mkdir -p x && cd x && git push", "cd ? ; push ? @?"),
    ("trap 'cd /x' EXIT; git push", "push ?"),
    (
        "if true; then eval 'cd /x'; fi; git push",
        "cd ? ; push ? @?",
    ),
    ("false || eval 'cd /x'; git push", "cd ? ; push ? @?"),
    (
        "export GIT_DIR=/other/.git; git push --force origin main",
        "cd ? ; push origin main>main force @?",
    ),
    (
        "GIT_DIR=/other; git push origin main",
        "cd ? ; push origin main>main @?",
    ),
    ("pushd +1; git push", "cd ? ; push ? @?"),
    ("CDPATH=/x cd foo; git push", "cd ? ; push ? @?"),
    ("CDPATH=/x cd ./foo; git push", "cd ./foo ; push ? @./foo"),
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
    ("trap 'git push' EXIT", "push ? @?"),
    ("trap 'git push' EXIT; cd x", "push ? @? ; cd x"),
    ("trap - EXIT", ""),
    ("trap -p", ""),
    ("trap \"$T\" EXIT", "unreadable ShellString"),
    ("bash <<'EOF'\ngit push\nEOF", "push ?"),
    ("bash -s <<EOF\ngit push $B\nEOF", "unreadable ShellString"),
    ("bash script.sh", ""),
    (
        "bash <<< 'git push --force origin main'",
        "push origin main>main force",
    ),
    (
        "echo 'git push --force origin main' | bash",
        "unreadable ShellString",
    ),
    ("cat script.sh | sh -s", "unreadable ShellString"),
    (
        "echo 'git push origin main' | { true; sh; }",
        "unreadable ShellString",
    ),
    (
        "echo 'git push origin main' | if true; then sh; fi",
        "unreadable ShellString",
    ),
    ("echo x | bash; git push", "unreadable ShellString ; push ?"),
    ("echo x | true; bash -c 'git push'", "push ?"),
    ("echo x | bash <<< 'git push'", "push ?"),
    (
        "printf 'git push' | bash 3<&0 <<< 'true' 0<&3",
        "unreadable ShellString",
    ),
    (
        "a | b | c | d | e | f | g | h | i | j | k | l | m | n | o | p | q | git push",
        "push ?",
    ),
    (
        "[[ -n $(git push --force origin main) ]]",
        "push origin main>main force",
    ),
    ("(( $(git push origin main) ))", "push origin main>main"),
    (
        "exit $(git push --force origin main)",
        "push origin main>main force",
    ),
    (
        "echo \"${X:-$(git push --force origin main)}\"",
        "push origin main>main force",
    ),
    (
        "a=($(git push --force origin main))",
        "push origin main>main force",
    ),
    (
        "echo $(( $(git push origin main) ))",
        "push origin main>main",
    ),
    (
        "case x in $(git push origin main)) ;; esac",
        "push origin main>main",
    ),
    (
        "for ((i=$(git push -f o m); i<1; i++)); do :; done",
        "push o m>m force",
    ),
    ("echo `echo \\`git push\\``", "unreadable Syntax"),
    ("git commit -m \"fix \\`x\\`\"", ""),
    (
        "echo '`'; x=`echo \\`git push --no-verify origin main\\``",
        "unreadable Syntax",
    ),
    (
        "cat <<'EOF'\n`\nEOF\nx=`echo \\`git push\\``",
        "unreadable Syntax",
    ),
    ("echo \"`echo \\`git push\\``\"", "unreadable Syntax"),
    (
        "cat <<EOF\n`echo \\`git push origin main\\``\nEOF",
        "unreadable Syntax",
    ),
    ("cat <<'EOF'\n`echo \\`git push\\``\nEOF", ""),
    ("echo $(echo \\`git push\\`)", ""),
    (
        "a=`date`; b=`date`; c=`date`; d=`date`; e=`date`; f=`date`; g=`date`; h=`date`; i=`date`; git push",
        "push ?",
    ),
    ("echo ${X:0:$(git push)}", "push ?"),
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
    ("a[$(git push)]=1", "push ?"),
    ("declare a[$(git push)]=1", "push ?"),
    ("echo \"${a[$(git push)]}\"", "push ?"),
    ("echo ${#a[$(git push)]}", "push ?"),
    ("echo ${a[$(git push)]:-x}", "push ?"),
    ("echo ${a[$(git push)]:1}", "push ?"),
    ("echo ${a[$(git push)]^^}", "push ?"),
    ("cat <<EOF\n$(( $(git push) + 1 ))\nEOF", "push ?"),
    ("cat <<EOF\n${a:-$(git push)}\nEOF", "push ?"),
    ("cat <<EOF\n${a[$(git push)]}\nEOF", "push ?"),
    ("cat <<EOF\n${a//x/$(git push)}\nEOF", "push ?"),
    ("[[ -v a[$(git push)] ]]", "push ?"),
    ("a=(1); [[ -R a[$(git push)] ]]", "push ?"),
    ("declare a[$(git push)]", "push ?"),
    ("a=x; echo ${a//x/$(git push)}", "push ?"),
    ("echo ${a/x/$(git push)}", "push ?"),
    ("echo ${a#$(git push)}", "push ?"),
    ("echo ${a%%$(git push)}", "push ?"),
    ("echo ${!a#$(git push)}", "push ?"),
    ("let \"a[$(git push)]++\"", "push ?"),
    ("let 'a[$(git push)]++'", "unreadable Runner(let)"),
    (
        "declare -i x='a[$(git push)]'",
        "unreadable Runner(declare)",
    ),
    ("declare 'a[$(git push)]=1'", "unreadable Runner(declare)"),
    ("typeset 'a[$(git push)]=1'", "unreadable Runner(typeset)"),
    ("unset 'a[$(git push)]'", "unreadable Runner(unset)"),
    ("printf -v 'a[$(git push)]' x", "unreadable Runner(printf)"),
    ("read 'a[$(git push)]' <<< x", "unreadable Runner(read)"),
    ("test -v 'a[$(git push)]'", "unreadable Runner(test)"),
    ("[ -v 'a[$(git push)]' ]", "unreadable Runner([)"),
    ("let 'a[`git push`]++'", "unreadable Runner(let)"),
    ("let 'a[1]++'", ""),
    ("printf '%s\\n' '$(git push)'", ""),
    ("a['$(git push)']=1", "unreadable Syntax"),
    ("echo ${a['$(git push)']}", "unreadable Syntax"),
    ("echo \"${a['$(git push)']}\"", "unreadable Syntax"),
    ("a=(['$(git push)']=1)", "unreadable Syntax"),
    ("(( 'a[$(git push)]' ))", "unreadable Syntax"),
    ("echo $(( '$(git push)' ))", "unreadable Syntax"),
    ("echo $(( '`git push`' ))", "unreadable Syntax"),
    ("echo $[ '$(git push)' ]", "unreadable Syntax"),
    (
        "for (( i='$(git push)';0; )); do :; done",
        "unreadable Syntax",
    ),
    ("[[ 1 -eq 'a[$(git push)]' ]]", "unreadable Syntax"),
    ("[[ -v 'a[$(git push)]' ]]", "unreadable Syntax"),
    ("[[ 'a[$(git push)]' == x ]]", ""),
    ("cat <<EOF\n$(( '$(git push)' ))\nEOF", "unreadable Syntax"),
    ("echo ${a:'$(git push)'}", "unreadable Syntax"),
    ("echo ${a:0:'$(git push)'}", "unreadable Syntax"),
    ("echo ${a[@]:'$(git push)'}", "unreadable Syntax"),
    ("read -p '$(git push)' x", "unreadable Runner(read)"),
    ("[ x = '$(git push)' ]", ""),
    ("(( a['1'] ))", ""),
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

/// The scan refuses before the parser runs, and closers the parser would not honour do not
/// lower its count.
#[test]
fn substitutions_nest_sixteen_deep_and_deeper_is_unreadable_without_parsing() {
    use crate::shell::scan::too_deep_or_long;
    let nest = |n: usize| format!("{}git push{}", "echo $(".repeat(n), ")".repeat(n));
    assert_eq!(show(&classify(&nest(16))), "push ?");
    assert_eq!(show(&classify(&nest(17))), "unreadable Syntax");
    assert!(too_deep_or_long(&nest(16)).is_none());
    assert!(too_deep_or_long(&nest(17)).is_some());
    assert!(too_deep_or_long(&"(".repeat(2000)).is_some());
    for hidden in [
        format!("echo {}git push{}", "$(: }} ".repeat(17), ")".repeat(17)),
        format!(
            "echo \"'\"; echo {}git push{} #'",
            "$(".repeat(17),
            ")".repeat(17)
        ),
        format!(
            "echo \" #\"; echo {}git push{}",
            "$(".repeat(17),
            ")".repeat(17)
        ),
        format!(
            "cat <<'EOF'\r\nhi\r\nEOF\r\necho {}x{}",
            "$(".repeat(17),
            ")".repeat(17)
        ),
        format!("(( 1<<2 ))\na{}0{}=1", "[a".repeat(17), "]".repeat(17)),
        format!(
            "echo {}git push{}",
            "$(echo \")\" ".repeat(4097),
            ")".repeat(4097)
        ),
    ] {
        assert!(too_deep_or_long(&hidden).is_some(), "{hidden}");
    }
}

/// Every shape the parser recurses on, filled to the size cap: the parser thread has the
/// stack for each, and a plain long script is not refused for its length. Arithmetic and
/// `${…}` bodies are the exception, capped because the parser is quadratic in them.
#[test]
fn every_shape_under_the_size_cap_parses_without_aborting() {
    let fill = |unit: &str, tail: &str| {
        let n = (super::MAX_BYTES - tail.len() - 1) / unit.len();
        format!("{}{tail}", unit.repeat(n))
    };
    assert_eq!(show(&classify(&fill("time ", "git push"))), "push ?");
    assert_eq!(show(&classify(&fill("true && ", "git push"))), "push ?");
    assert_eq!(show(&classify(&fill("true || ", "git push"))), "push ?");
    assert_eq!(show(&classify(&fill("true | ", "git push"))), "push ?");
    assert_eq!(show(&classify(&fill("x=1;", "git push"))), "push ?");
    let n = (super::MAX_BYTES - 16) / "if :; then ;fi".len();
    for shape in [
        fill("coproc ", "true"),
        fill("! ", "git push"),
        format!("{}git push{}", "if :; then ".repeat(n), ";fi".repeat(n)),
        format!("if :; then\n{}fi", "echo a | cat\n".repeat(4000)),
    ] {
        assert!(shape.len() <= super::MAX_BYTES);
        classify(&shape);
    }
    let arith = |unit: &str, n: usize| format!("(( {}1 ))", unit.repeat(n));
    assert_eq!(show(&classify(&arith("1+", 2000))), "");
    assert_eq!(show(&classify(&arith("a=", 2000))), "");
    assert_eq!(
        show(&classify(&format!("x=${{a:{}1}}", "1+".repeat(2000)))),
        ""
    );
    for over in [
        arith("1+", 2100),
        arith("a?", 2100),
        arith("!", 4200),
        format!("(( ((1)) + {}1 ))", "1+".repeat(30000)),
        format!("x=${{a:{}1}}", "1+".repeat(30000)),
        format!("echo ${{a[{}1]}}", "1+".repeat(2100)),
        format!("(( ${{x:-]}} + {}1 ))", "1+".repeat(2100)),
        format!("(( {}0{} ))", "a[".repeat(17), "]".repeat(17)),
        format!("echo ${{{}0{}}}", "a[".repeat(17), "]".repeat(17)),
        format!("cat <<EOF\n{}x{}\nEOF", "$(".repeat(17), ")".repeat(17)),
    ] {
        assert_eq!(show(&classify(&over)), "unreadable Syntax", "{over}");
    }
}

/// Balanced brackets in text cost nothing, and only arithmetic and `${…}` bodies are capped,
/// so JSON, code and long tests read; an unbalanced quoted bracket may refuse, never hide.
#[test]
fn balanced_text_reads_and_only_arithmetic_bodies_are_capped() {
    let files = (0..40)
        .map(|n| format!("-e /workspace/example-app/src/file{n}.rs"))
        .collect::<Vec<_>>()
        .join(" && ");
    assert_eq!(
        show(&classify(&format!("[[ {files} ]] && git push origin main"))),
        "push origin main>main"
    );
    let items = (0..120)
        .map(|n| format!("  {{\"id\": {n}, \"tags\": [\"a\", \"b\"]}}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let json = format!("cat > data.json <<'EOF'\n[\n{items}\n]\nEOF\ngit push origin main");
    assert_eq!(show(&classify(&json)), "push origin main>main");
    let code = "x = compute(a, b)\n".repeat(80);
    let py = format!("python3 - <<'EOF'\nprint((a + b) * c)\n{code}EOF\ngit push");
    assert_eq!(show(&classify(&py)), "push ?");
    let raw = format!("cat <<EOF\n[[[[ (((( {{{{\n{code}EOF\ngit push");
    assert_eq!(show(&classify(&raw)), "push ?");
    let tail = "true; ".repeat(900);
    assert_eq!(
        show(&classify(&format!("grep -E '((a|b)c)' f; {tail}git push"))),
        "push ?"
    );
    assert_eq!(
        show(&classify(&format!("echo '[' ; {tail}git push"))),
        "push ?"
    );
    assert_eq!(
        show(&classify(&format!("# see [1] (\n{tail}git push"))),
        "push ?"
    );
    let test = format!("[ {} ]; git push", "-n x -a ".repeat(600));
    assert_eq!(show(&classify(&test)), "push ?");
    assert_eq!(
        show(&classify(&format!("echo '{}'", "(".repeat(40)))),
        "unreadable Syntax"
    );
    let quoted = format!("cat <<'EOF'\n{}\nEOF", "$(".repeat(40));
    assert_eq!(show(&classify(&quoted)), "unreadable Syntax");
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
