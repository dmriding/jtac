#![cfg(test)]
//! The one table (DESIGN.md section 11.4): every spelling the classifier must read, bypasses
//! first. A new bypass is a new row, with the fix.

use super::bypasses::BYPASSES;
use super::{Action, Dir, Force, Spec, Unread, classify};

fn show(actions: &[Action]) -> String {
    let mut lines = Vec::new();
    for a in actions {
        lines.push(match a {
            Action::Push(p) => {
                let mut s = format!("push {}", p.remote.as_deref().unwrap_or("?"));
                let specs = p.sources.iter().zip(&p.destinations).zip(&p.specs);
                for ((src, dst), spec) in specs {
                    match spec {
                        Spec::Spelled => s.push_str(&format!(" {src}>{dst}")),
                        Spec::Forced => s.push_str(&format!(" +{src}")),
                        Spec::Tag => {
                            let name = dst.strip_prefix("refs/tags/").unwrap_or(dst);
                            s.push_str(&format!(" tag {name}"));
                        }
                        Spec::Bare => s.push_str(&format!(" {src}")),
                    }
                }
                // A `:dst` keeps its slot, so a deletion between two updates reads in order
                // (`git push origin :old main` is `>old main>main`).
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
                    (p.tags, "tags"),
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

/// Force forms.
const FORCE: &[(&str, &str)] = &[
    ("git push -f origin main", "push origin main force"),
    ("git push --force origin main", "push origin main force"),
    (
        "git push --force-with-lease origin main",
        "push origin main lease",
    ),
    (
        "git push --force-with-lease=main:abc origin main",
        "push origin main lease",
    ),
    (
        "git push --force-with-lease --force origin main",
        "push origin main force",
    ),
    ("git push origin +main", "push origin +main force"),
    ("git push -fu origin main", "push origin main force"),
    ("git push --mirror origin", "push origin force all"),
    ("git push --mirr origin", "push origin force all"),
    ("git push --m origin", "push origin force all"),
    ("git push --de origin main", "push origin >main delete"),
    ("git push --b origin", "push origin all"),
    ("git push --al origin", "push origin all"),
    ("git push --forc origin main", "push origin main"),
    (
        "git push --force-if-includes origin main",
        "push origin main",
    ),
    (
        "git push --no-force-with-lease origin main",
        "push origin main",
    ),
    (
        "git push --force --no-force-with-lease origin main",
        "push origin main force",
    ),
    (
        "git push --force-with-lease --no-force origin main",
        "push origin main lease",
    ),
    (
        "git push --force --force-with-lease --no-force origin main",
        "push origin main lease",
    ),
    ("git push --mirror --no-force origin", "push origin all"),
    (
        "git push --no-force --mirror origin",
        "push origin force all",
    ),
    (
        "git push --mirror --no-mirror origin main",
        "push origin main",
    ),
    (
        "git push -f --mirror --no-mirror origin main",
        "push origin main",
    ),
    (
        "git push --mirror --force-with-lease --no-mirror origin main",
        "push origin main lease",
    ),
    ("git push -fofoo=bar origin main", "push origin main force"),
    ("git push -o=ci.skip origin main", "push origin main"),
    ("git push -n --no-dry-run origin main", "push origin main"),
    ("git push -n --no-dry origin main", "push origin main"),
    ("git push -f --no-force origin main", "push origin main"),
    ("git push --mirror --no-mirror origin", "push origin"),
    ("git push -d --no-delete origin main", "push origin main"),
    ("git push --force-w origin main", "push origin main lease"),
    (
        "git push -4f origin HEAD~1:main",
        "push origin HEAD~1>main force",
    ),
    ("git push --del origin old", "push origin >old delete"),
    ("git push --dry origin main", "push origin main dry"),
    ("git push --all origin", "push origin all"),
    ("git push origin --tags", "push origin tags"),
    ("git push --tags --no-tags origin main", "push origin main"),
    (
        "git push origin 'refs/heads/*'",
        "push origin refs/heads/* all",
    ),
];

/// Deletions and refspecs.
const REFSPECS: &[(&str, &str)] = &[
    ("git push -d origin old", "push origin >old delete"),
    ("git push --delete origin a b", "push origin >a >b delete"),
    ("git push origin :old", "push origin >old delete"),
    ("git push origin main:main", "push origin main>main"),
    ("git push origin main :old", "push origin main >old delete"),
    ("git push origin :old main", "push origin >old main delete"),
    ("git push -fd origin old", "push origin >old force delete"),
    (
        "git push origin main:refs/heads/x",
        "push origin main>refs/heads/x",
    ),
    (
        "git push origin HEAD:main topic",
        "push origin HEAD>main topic",
    ),
    ("git push origin tag v1", "push origin tag v1"),
    ("git push --repo=origin", "push origin"),
    ("git push --repo origin main", "push main"),
    ("git push origin \"$BRANCH\"", "push origin $BRANCH opaque"),
    (
        "git push origin $(git branch --show-current)",
        "push origin $(git branch --show-current) opaque",
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
        "push origin main force",
    ),
    ("A=1 B=2 git push", "push ?"),
    ("sudo -u git -H git push origin main", "push origin main"),
    ("env -u git git push origin main", "push origin main"),
    ("flock /tmp/lock git push", "push ?"),
    (
        "flock /tmp/lock -c 'git push --force origin main'",
        "push origin main force",
    ),
    ("flock -w 5 /tmp/lock git push", "push ?"),
    (
        "flock /tmp/lock --command 'git push --force origin main'",
        "push origin main force",
    ),
    (
        "flock -s /tmp/lock git push origin main",
        "push origin main",
    ),
    (
        "env --default-signal git push origin main",
        "push origin main",
    ),
    (
        "env --ignore-signal=INT git push --force origin main",
        "push origin main force",
    ),
    ("env --unse git git push origin main", "push origin main"),
    (
        "env --chd=/elsewhere git push origin main",
        "push origin main @/elsewhere",
    ),
    ("env --c /elsewhere git push", "push ? @/elsewhere"),
    ("sudo --us git git push origin main", "push origin main"),
    ("sudo --ch /x git push", "push ? @?"),
    (
        "busybox env git push --force origin main",
        "push origin main force",
    ),
    ("busybox timeout 5 git push origin main", "push origin main"),
    ("busybox $APPLET git push", "unreadable Variable"),
    ("café", ""),
    ("éabc", ""),
    ("/opt/café/git push origin main", "push origin main"),
    ("Git-Push origin main", "push origin main"),
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
        "push origin main force",
    ),
    ("git-push origin main", "push origin main"),
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
    ("sudo -R /jail git -C /x push", "push ? @?"),
    ("sudo -R /jail env -C /repo git -C /x push", "push ? @?"),
    ("sudo --chroot=/jail -- git -C /x push", "push ? @?"),
    ("env -C \"$D\" git -C /x push", "push ? @/x"),
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
        "cd ? ; push origin main force @?",
    ),
    (
        "GIT_DIR=/other; git push origin main",
        "cd ? ; push origin main @?",
    ),
    ("pushd +1; git push", "cd ? ; push ? @?"),
    ("CDPATH=/x cd foo; git push", "cd ? ; push ? @?"),
    ("CDPATH=/x cd ./foo; git push", "cd ./foo ; push ? @./foo"),
];

/// Indirection.
const INDIRECTION: &[(&str, &str)] = &[
    (
        "bash -lc 'git push -f origin main'",
        "push origin main force",
    ),
    (
        "sh -c 'cd ../x && git push'; git push",
        "push ? @../x ; push ?",
    ),
    ("cd a && bash -c 'cd b && git push'", "cd a ; push ? @a/b"),
    ("bash -c \"git push $X\"", "unreadable ShellString"),
    ("bash -c \"$SCRIPT\"", "unreadable ShellString"),
    ("busybox sh -c 'git push'", "push ?"),
    ("eval git push origin main", "push origin main"),
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
        "push origin main force",
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
        "push origin main force",
    ),
    ("(( $(git push origin main) ))", "push origin main"),
    (
        "exit $(git push --force origin main)",
        "push origin main force",
    ),
    (
        "echo \"${X:-$(git push --force origin main)}\"",
        "push origin main force",
    ),
    (
        "a=($(git push --force origin main))",
        "push origin main force",
    ),
    ("echo $(( $(git push origin main) ))", "push origin main"),
    (
        "case x in $(git push origin main)) ;; esac",
        "push origin main",
    ),
    (
        "for ((i=$(git push -f o m); i<1; i++)); do :; done",
        "push o m force",
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
        "push origin $b opaque",
    ),
    (
        "while read b; do git push origin \"$b\"; done < list",
        "push origin $b opaque",
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
    ("let 'a[$(git push)]++'", "unreadable Syntax"),
    ("let 'a[$(git -C /repo push)]++'", "unreadable Syntax"),
    ("let 'a[$(git push)]++'$z", "unreadable Syntax"),
    ("let \"a[\\$(git push)]++\"", "unreadable Syntax"),
    ("declare -i x='a[$(git push)]'", "unreadable Syntax"),
    ("declare 'a[$(git push)]=1'", "unreadable Syntax"),
    ("typeset 'a[$(git push)]=1'", "unreadable Syntax"),
    ("f() { local 'a[$(git push)]=1'; }", "unreadable Syntax"),
    ("unset 'a[$(git push)]'", "unreadable Syntax"),
    ("unset -v 'a[$(git push)]'", "unreadable Syntax"),
    ("printf -v 'a[$(git push)]' x", "unreadable Syntax"),
    ("read 'a[$(git push)]' <<< x", "unreadable Syntax"),
    ("test -v 'a[$(git push)]'", "unreadable Syntax"),
    ("[ -v 'a[$(git push)]' ]", "unreadable Syntax"),
    ("[ -R 'a[$(git push)]' ]", "unreadable Syntax"),
    ("let 'a[`git push`]++'", "unreadable Syntax"),
    ("let 'a[1]++'", ""),
    ("export 'a[$(git push)]=1'", ""),
    ("readonly 'a[$(git push)]=1'", ""),
    (
        "mapfile -c 1 -C 'git push; :' <<< x",
        "unreadable Runner(mapfile)",
    ),
    (
        "readarray -C 'git push' -c 1 <<< x",
        "unreadable Runner(readarray)",
    ),
    ("mapfile -t lines <<< x", ""),
    (
        "mapfile -tC 'git push' -c1 a <<< x",
        "unreadable Runner(mapfile)",
    ),
    (
        "readarray -C'git push' -c1 a <<< x",
        "unreadable Runner(readarray)",
    ),
    (
        "mapfile -C \"$cb\" -c 1 a < f",
        "unreadable Runner(mapfile)",
    ),
    ("PS4='$(git push) ' bash -xc :", "unreadable Prompt"),
    ("PS4='$(git push) '; set -x; :", "unreadable Prompt"),
    ("export PS4='`git push` '", "unreadable Prompt"),
    ("PS4='+ ' bash -xc :", ""),
    ("echo PS4='$(git push)'", ""),
    ("PS4='\\044(git push) '; set -x; :", "unreadable Prompt"),
    ("PS4='\\$(git push) '; set -x; :", "unreadable Prompt"),
    (
        "y='$(git push)'; PS4='${y@P} '; set -x; :",
        "unreadable Prompt",
    ),
    ("env PS4='$(git push) ' bash -xc :", "unreadable Prompt"),
    ("env -i PS4='$(git push) ' bash -xc :", "unreadable Prompt"),
    ("declare PS4='$(git push) '", "unreadable Prompt"),
    ("typeset -x PS4='`git push` '", "unreadable Prompt"),
    ("export PS4=$'\\U00000024(git push) '", "unreadable Prompt"),
    ("PS4='\\033[1m+ '; set -x; :", ""),
    ("PS4='${LINENO}: ' bash -xc :", ""),
    ("x='$(git push)'; : \"${x@P}\"", "unreadable Syntax"),
    ("echo ${x@Q}", ""),
    ("x=0; echo $(( $x+'$(git push)' ))", "unreadable Syntax"),
    ("x=0; a[$x+'$(git push)']=1", "unreadable Syntax"),
    ("x=0; echo ${a:$x'+$(git push)'}", "unreadable Syntax"),
    (
        "for ((i=$x'+$(git push)'; i<0;)); do :; done",
        "unreadable Syntax",
    ),
    ("[[ $x+'a[$(git push)]' -eq 1 ]]", "unreadable Syntax"),
    ("x=; [[ -v \"$x\"'a[$(git push)]' ]]", "unreadable Syntax"),
    ("x=0; let \"$x\"'+a[$(git push)]'", "unreadable Syntax"),
    (
        "x=0; declare -i v=\"$x\"'+a[$(git push)]'",
        "unreadable Syntax",
    ),
    ("printf -v \"$x\"'a[$(git push)]' y", "unreadable Syntax"),
    ("(( $'\\x24(git push)' ))", "unreadable Syntax"),
    ("echo $(( $'\\x24(git push)' ))", "unreadable Syntax"),
    ("echo ${a[$'\\044(git push)']}", "unreadable Syntax"),
    ("echo $(( $'\\u0024(git push)' ))", "unreadable Syntax"),
    ("(( $'\\x60git push\\x60' ))", "unreadable Syntax"),
    ("let $'a[\\x24(git push)]=1'", "unreadable Syntax"),
    ("echo $'\\x24(git push)'", ""),
    ("echo $(( $'\\n' + 1 ))", ""),
    ("f() { local msg=\"run \\`git push\\` when done\"; }", ""),
    (
        "declare -r HELP='usage: $(basename \"$0\"), then git push'",
        "",
    ),
    ("typeset note='see `git push --help`'", ""),
    ("declare -ri n='a[$(git push)]'", "unreadable Syntax"),
    ("let 'x=a[$''(git push)]'", "unreadable Syntax"),
    ("let x=a\\[\\$\\(git\\ push\\)\\]", "unreadable Syntax"),
    ("declare -i x='a[$''(git push)]'", "unreadable Syntax"),
    ("read 'a[$''(git push)]' <<< x", "unreadable Syntax"),
    ("[ -v 'a[$''(git push)]' ]", "unreadable Syntax"),
    ("x=1 let 'y=a[$''(git push)]'", "unreadable Syntax"),
    ("coproc let 'x=a[$''(git push)]'", "unreadable Syntax"),
    ("(( $'\\x24'\"(git push)\" ))", "unreadable Syntax"),
    ("(( $'\\U00000024(git push)' ))", "unreadable Syntax"),
    ("builtin let 'x=a[$(git push)]'", "unreadable Syntax"),
    ("command let 'x=a[$(git push)]'", "unreadable Syntax"),
    ("command -p let 'x=a[$(git push)]'", "unreadable Syntax"),
    ("builtin read 'a[$(git push)]' <<< x", "unreadable Syntax"),
    ("command [ -v 'a[$(git push)]' ]", "unreadable Syntax"),
    ("builtin printf -v 'a[$(git push)]' x", "unreadable Syntax"),
    ("builtin declare -i x='a[$(git push)]'", "unreadable Syntax"),
    ("command declare 'a[$(git push)]=1'", "unreadable Syntax"),
    ("declare -a a='($(git push))'", "unreadable Syntax"),
    ("declare -A m='([$(git push)]=1)'", "unreadable Syntax"),
    (
        "f() { local -a a='([$(git push)]=1)'; }",
        "unreadable Syntax",
    ),
    ("typeset -a a='(`git push`)'", "unreadable Syntax"),
    ("declare -ga a='($(git push))'", "unreadable Syntax"),
    ("declare -a 'a=($(git push))'", "unreadable Syntax"),
    ("declare -a a=\"(\\$(git push))\"", "unreadable Syntax"),
    ("declare \"-i\" x='a[$(git push)]'", "unreadable Syntax"),
    ("readonly -a a='($(git push))'", "unreadable Syntax"),
    ("declare -a a=('$(git push)')", ""),
    ("declare -a a=(one two)", ""),
    ("printf -v x 'run `git push` first'", ""),
    ("printf -v 'a[$(git push)]' '%s' x", "unreadable Syntax"),
    ("printf -vx 'run `git push`'", ""),
    ("compgen -W '$(git push)' x", "unreadable Syntax"),
    ("compgen -C 'git push' x", "unreadable Runner(compgen)"),
    ("compgen -W 'one two' o", ""),
    ("sleep 0 & wait -n -p 'a[$(git push)]'", "unreadable Syntax"),
    ("y='$(git push)'; x=y; : \"${!x@P}\"", "unreadable Syntax"),
    (
        "unset PS4; : ${PS4='$(git push)'}; set -x; :",
        "unreadable Syntax",
    ),
    (": ${PS4:-x}", ""),
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
    ("read -p '$(git push)' x", "unreadable Syntax"),
    (
        "z=; a=(0); [[ -v 'a[$(git push)]'$z ]]",
        "unreadable Syntax",
    ),
    ("echo \"${a['$(git push)']}\"$z", "unreadable Syntax"),
    ("[ x = '$(git push)' ]", ""),
    ("(( a['1'] ))", ""),
    ("g=git; $g push", "unreadable Variable"),
    ("$HOME/bin/tool build", "unreadable Variable"),
    ("cat <<EOF > f\n$(git push)\nEOF", "push ?"),
    ("cat 3<<'EOF'\ngit push\nEOF", "unreadable Syntax"),
    ("[[ -n x ]] && git push", "push ?"),
    ("(( 1 + 1 )); git push", "push ?"),
    ("exit 1; git push", "push ?"),
    ("x=${a:-$( )}; echo $( # nothing\n)", ""),
    ("x=$(\\\n); echo \\$(x) \"\\$(y)\" '$(z)' a\\`b", ""),
    (
        "git commit -m \"handle <(x) in the reader\"; echo \"usage: diff <(a) >(b)\"",
        "",
    ),
    ("echo ${PWD#${PWD#${PWD#${PWD#$(git push)}}}}", "push ?"),
    (
        "tag=${BRANCH//\\//-}; echo \"${N:-\\$HOME}\" ${a:-\\*}; git push",
        "push ?",
    ),
    (
        "echo a\\\\b \"c\\\"d\" \"${x//\\\"/\\\\\\\"}\" ${x//\\\\/\\\\\\\\}; git push",
        "push ?",
    ),
    ("x=\"${N:-\\$x}\"; git push", "push ?"),
    (
        "out=\"${x//\\\\/\\\\\\\\}\"; out=\"${out//\\\"/\\\\\\\"}\"; echo \"{\\\"m\\\":\\\"$out\\\"}\"; git push",
        "push ?",
    ),
    (
        "echo \"${x/#\\~/$HOME}\" \"${x//\\$/\\\\$}\"; git push",
        "push ?",
    ),
    (
        "echo \"\\\"$HOME\\\"\" \"{\\\"k\\\":\\\"${name}\\\",\\\"d\\\":\\\"$(date)\\\"}\"; git push",
        "unreadable Syntax",
    ),
    (
        "echo \"a\\\nb${x}\" \"\\\"${a@Q}\\\"\" \"\\\"${#a}\\\"\"; git push",
        "push ?",
    ),
    ("echo \"${N:-'a'\\\"}\"; git push", "push ?"),
    (
        "curl -d \"{\\\"text\\\":\\\"${text//\\\"/\\\\\\\"}\\\",\\\"user\\\":\\\"$USER\\\"}\" https://example.com; git push",
        "push ?",
    ),
    (
        "echo \"${p//\\\\//}/$f\" \"Path: ${WIN//\\\\//} -> $DEST\"; sed \"s/${old//\\//\\\\/}/$new/\" f; git push",
        "push ?",
    ),
    ("echo \"${a:-<(x)}\" \"${a:=<(y)}\" \"${a:+<(z)}\"", ""),
    ("x=$((a>(b)?a:b)); cat <<EOF\n${a:-<(x)} <(y)\nEOF", ""),
    (
        "echo $(( ${a:-<(x)} )) ${PWD:${a:-<(x)}}; for ((i=${a:-<(x)};0;)); do :; done",
        "",
    ),
    (
        "(( ${a:-<(x)} )); echo ${PWD#\\/} ${PWD/x/y} 'm[${a:-<(x)}]' \"m[${a:-<(x)}]\"",
        "",
    ),
    (
        "git commit -m \"split on $'\\n'\"; echo $\"hi\" && git push",
        "push ?",
    ),
    (
        "grep -n \"^fn main$\" src/lib.rs; rg \"TODO$\"; sed -i \"/^$/d\" f",
        "",
    ),
    (
        "[[ $v =~ ^[a-z]+$ ]]; case $x in *$) :;; esac; ls [$]x; cat <<< \"a$\"",
        "",
    ),
    (
        "echo ${a:-$(git push)} ${b[$(git push)]} $(( $(git push) ))",
        "push ? ; push ? ; push ?",
    ),
    (
        "msg=\"\\\nHello $USER\nbye\"; echo \"\\\n$(date)\"; git push",
        "push ?",
    ),
    ("cat <<-E\n\tx\n\tE\ngit push", "push ?"),
    ("cat <<'E'\nx\\\nE\ngit push", "unreadable Syntax"),
    ("echo \\$\\\n{y}; git push", "push ?"),
    (
        "curl -d \"{\\\"text\\\":\\\"$(cat $FILE)\\\"}\" https://example.com; git push",
        "unreadable Syntax",
    ),
    (
        "echo \"{\\\"k\\\":\\\"$(basename $PWD)\\\"}\" \"\\\"\\\"\\\"$(ls $HOME)\"; git push",
        "unreadable Syntax",
    ),
    ("git push; cat <<'E'\nhello", "push ?"),
    ("cat <<E; cat <<F\nx\nE\ny\nF\ngit push", "push ?"),
    ("x=$(cat <<E\nx\nE\n); git push", "push ?"),
    ("cat <<\\E\nx\nE\ngit push", "push ?"),
    ("cat <<\\E\n$(x)\nE\ngit push", "push ?"),
    ("cat <<EOF\nEOF)\nEOF\ngit push", "push ?"),
    (
        "cat <<'A'\n$(gi'\\\n't push)\nA\ngit push",
        "unreadable Syntax",
    ),
    ("bash <<EOF\nr=\"\\$(git push)\"\nEOF", "push ?"),
    ("bash <<A\ngi'\\\n't push\nA", "push ?"),
    ("cat <<A\n$(echo a\n\tb)\nA\ngit push", "push ?"),
    ("bash <<A\ngit pu\\\\sh\nA", "push ?"),
    ("bash <<A >/dev/null\ngit push\nA", "push ?"),
    ("bash <<A >>log 2>&1\ngit push\nA", "push ?"),
    ("bash <<A &>/dev/null\ngit push\nA", "push ?"),
    ("bash <<A {fd}>/dev/null\ngit push\nA", "push ?"),
    ("bash <<'A'\ngit pu\\\\sh\nA", ""),
    ("echo $(( 1 + 2 )) $(( )); git push", "push ?"),
    ("bash <<A <&0\ngit push\nA", "unreadable ShellString"),
    ("bash <<A 3<&0 <&3\ngit push\nA", "unreadable ShellString"),
    ("bash <<A <&-\ngit push\nA", "unreadable ShellString"),
    ("bash 3<<<x; git push", "unreadable Syntax"),
    ("bash <<<'git push' {fd}</dev/null", "push ?"),
    ("bash <&0", "unreadable ShellString"),
    ("bash 0</dev/null", "unreadable ShellString"),
    ("bash <&-", "unreadable ShellString"),
    ("bash <<<'git push' 0>&0", "unreadable ShellString"),
    ("bash <<<'git push' 3<&0 0>&3", "unreadable ShellString"),
    (
        "cat <<A >\"$(bash)\"\ngit push\nA",
        "unreadable ShellString",
    ),
    ("cat <<<'git push' >\"$(bash)\"", "unreadable ShellString"),
    ("cat 3<\"$(bash)\" <<<'git push'", "unreadable ShellString"),
    ("git push > \"$(mktemp)\"", "push ?"),
    ("echo 'git push' > >(bash)", "unreadable ShellString"),
    ("bash < <(echo 'git push')", "unreadable ShellString"),
    ("cp <(echo 'git push') >(bash)", "unreadable ShellString"),
    ("cat <<\\\n-A\nx\nA\ngit push\n-A", "push ?"),
    ("bash <<-A\n\tgi\\\n\tt push\n\tA", ""),
    ("bash -c 'gi\\\nt push'", "push ?"),
    ("echo 'a \\\nb' $'c\\\nd'", ""),
    ("eval 'gi\\\nt push'", "push ?"),
    ("bash <<A\ngit 'pu'\\\n'sh'\nA", "push ?"),
    ("$'git'\\\n push", "push ?"),
    ("echo `# c \\\ngit push\n`", ""),
    ("cat <<E\n$( # c \\\ngit push\n)\nE", ""),
    ("(( 2#1 \\\n+ 1 ))", ""),
    ("echo a#b \\\ngit push", "unreadable Runner(echo)"),
    ("python3<<EOF\nprint(1)\nEOF", ""),
    ("cat file2<<EOF\nx\nEOF", ""),
    (
        "git commit -m \"Bump to $(cat VERSION) \\\"stable\\\"\"",
        "",
    ),
    ("echo \"$(date) \\\"x\\\"\"", ""),
    ("echo \"${N:-\\$HOME}\" ${M:-\\${HOME}}", ""),
];

/// Unreadable syntax.
const SYNTAX: &[(&str, &str)] = &[
    ("git push \"unterminated", "unreadable Syntax"),
    ("git push 'x", "unreadable Syntax"),
    ("git push $(", "unreadable Syntax"),
    ("git push; fi", "unreadable Syntax"),
    (
        "x=${a:-$(git push; if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi; fi)}",
        "unreadable Syntax",
    ),
    (
        "x=${a:-$(git push; if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi; fi; fi; fi)}",
        "unreadable Syntax",
    ),
    (
        "echo \"${remote:-$(git push --no-verify origin HEAD; if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi; fi)}\"",
        "unreadable Syntax",
    ),
    (
        "a[$(a[$(a[$(a[$(a[$(git push)]=1)]=1)]=1)]=1)]=1",
        "unreadable Syntax",
    ),
    (
        "echo $(( $(if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi; fi; git push) ))",
        "unreadable Syntax",
    ),
    (
        "echo ${a/x/$(if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi; fi; git push)}",
        "unreadable Syntax",
    ),
    (
        "echo ${a[$(if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi; fi; git push)]}",
        "unreadable Syntax",
    ),
    (
        "x=${a:-$(if :; then if :; then if :; then if :; then if :; then if :; then if :; then echo ${b:-$(git push)}; fi; fi; fi; fi; fi; fi; fi)}",
        "unreadable Syntax",
    ),
    (
        "x=${a:-$(if :; then if :; then if :; then if :; then if :; then if :; then if :; then echo \"${b:-\"`git push --no-verify`\"}\"; fi; fi; fi; fi; fi; fi; fi)}",
        "unreadable Syntax",
    ),
    (
        "x=${a:-$(if :; then if :; then if :; then if :; then if :; then if :; then if :; then (( $(git push) )); fi; fi; fi; fi; fi; fi; fi)}",
        "unreadable Syntax",
    ),
    (
        "x=${a:-$(cat <(git push; if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi))}",
        "unreadable Syntax",
    ),
    ("(( $(git push) 1 ))", "unreadable Syntax"),
    ("(( a[ $(git push) ))", "unreadable Syntax"),
    ("cat ${b:-<(git push)}", "unreadable Syntax"),
    ("x=${a:=<(cd /x && git push)}", "unreadable Syntax"),
    (
        "x=${a:-`git push; if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi; fi`}",
        "unreadable Syntax",
    ),
    (
        "cat <<EOF\n${a:-$(git push; if :; then if :; then if :; then if :; then if :; then if :; then if :; then if :; then :; fi; fi; fi; fi; fi; fi; fi; fi)}\nEOF",
        "unreadable Syntax",
    ),
    (
        "echo ${PWD#${PWD#${PWD#${PWD#${PWD#$(git push)}}}}}",
        "unreadable Syntax",
    ),
    (
        "x=${a:-$(if :; then if :; then if :; then if :; then if :; then if :; then if :; then echo ${b:-$\\\n(git push)}; fi; fi; fi; fi; fi; fi; fi)}",
        "unreadable Syntax",
    ),
    (
        "x=${a:-$(if :; then if :; then if :; then if :; then if :; then if :; then if :; then echo ${b:-<\\\n(git push)}; fi; fi; fi; fi; fi; fi; fi)}",
        "unreadable Syntax",
    ),
    (
        "y='$(git push)'; x=${a:-$(if :; then if :; then if :; then if :; then if :; then if :; then if :; then echo ${b:-${y@P}}; fi; fi; fi; fi; fi; fi; fi)}",
        "unreadable Syntax",
    ),
    ("echo ${b:-$\\\n(git push)}", "push ?"),
    ("echo \"${PWD/x/<(git push)}\"", "unreadable Syntax"),
    ("echo \"${PWD//x/>(git push)}\"", "unreadable Syntax"),
    ("echo \"${PWD/#x/<(git push)}\"", "unreadable Syntax"),
    ("echo \"${PWD#${a:-<(git push)}}\"", "unreadable Syntax"),
    (
        "echo \"${PWD,<(git push)}\" \"${PWD^^<(git push)}\"",
        "unreadable Syntax",
    ),
    ("echo $(( ${PWD/x/<(git push)} ))", "unreadable Syntax"),
    ("echo `echo ${a:-\\$(git push)}`", "unreadable Syntax"),
    (
        "echo `echo ${PWD#${PWD#${PWD#${PWD#${PWD#\\$(git push)}}}}}`",
        "unreadable Syntax",
    ),
    ("echo `echo \\\\$(git push)`", "unreadable Syntax"),
    (
        "echo `echo \"\\\\\" '$(git push)' \"\\\\\"`",
        "unreadable Syntax",
    ),
    ("[[ 1 -eq ${a:-<(git push)} ]]", "unreadable Syntax"),
    ("[[ -v ${a:-<(git push)} ]]", "unreadable Syntax"),
    ("m=([<(git push)]=1)", "unreadable Syntax"),
    ("declare -A m; m[${a:-<(git push)}]=1", "unreadable Syntax"),
    (
        "declare -A m; echo \"${m[${a:-<(git push)}]}\"",
        "unreadable Syntax",
    ),
    (
        "declare -A m; (( m[${a:-<(git push)}] ))",
        "unreadable Syntax",
    ),
    ("echo \"$(( m[${a:-<(git push)}] ))\"", "unreadable Syntax"),
    (
        "cat <<EOF\n${m[${a:-<(git push)}]}\nEOF",
        "unreadable Syntax",
    ),
    ("echo ${PWD:${m[${a:-<(git push)}]}}", "unreadable Syntax"),
    (
        "declare -A m; cat <<EOF\n$(( m[${a:-<(git push)}] ))\nEOF",
        "unreadable Syntax",
    ),
    (
        "declare -A m; let 'm[${a:-<(git push)}]=1'",
        "unreadable Syntax",
    ),
    ("unset 'm[${a:->(git push)}]'", "unreadable Syntax"),
    ("[[ -v 'm[${a:-<(git push)}]' ]]", "unreadable Syntax"),
    ("printf -v 'm[${a:-<(git push)}]' x", "unreadable Syntax"),
    ("echo ${PWD/\\/$(git push)/y}", "unreadable Syntax"),
    ("x=\"${PWD//\\/<(git push)/}\"", "unreadable Syntax"),
    ("echo ${N:-\\$x$(git push)}", "unreadable Syntax"),
    ("echo \"${N:-\\\\$(git push)}\"", "unreadable Syntax"),
    ("echo \"${PWD#\\$x$(git push)}\"", "unreadable Syntax"),
    ("echo ${N:-${M:-\\$x$(git push)}}", "unreadable Syntax"),
    ("(echo ${PWD/\\/$(git push)/y})", "unreadable Syntax"),
    ("echo ${PWD/#x/\\/$(git push)}", "push ?"),
    ("echo \"${PWD/x/\\\\$(git push)}\"", "unreadable Syntax"),
    (
        "echo ${N:-\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\$(git push)}",
        "unreadable Syntax",
    ),
    (
        "declare -A m; echo ${m[\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\$(git push)]}",
        "unreadable Syntax",
    ),
    (
        "echo ${M:-\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\}${N:-$(git push)}",
        "unreadable Syntax",
    ),
    (
        "echo ${N:-\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\$(git push)}\"x\"",
        "unreadable Syntax",
    ),
    (
        "echo \"${N:-'\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"'$(git push)}\"",
        "unreadable Syntax",
    ),
    (
        "echo ${x:-a}; echo ${N:-\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\$(git push)}",
        "unreadable Syntax",
    ),
    (
        "case x in ${N:-\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\$(git push)}) ;; esac",
        "unreadable Syntax",
    ),
    (
        "case x in y|${N:-\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\\$(git push)}\"x\") ;; esac",
        "unreadable Syntax",
    ),
    ("echo \"\\\"${X:-$(git push)}\\\"\"", "unreadable Syntax"),
    ("echo \"a\\\nb${N:-$(git push)}\"", "push ?"),
    ("echo $\"\\\"${N:-$(git push)}\"", "unreadable Syntax"),
    (
        "echo \"${N:-\\''\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"'$(git push)}\"",
        "unreadable Syntax",
    ),
    (
        "echo \"${N:-\"'\"'\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"'$(git push)}\"",
        "unreadable Syntax",
    ),
    ("echo ${x:-a}\\\n\\\n${N:-$(git push)}", "push ?"),
    (
        "x=\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"${m[$(git push)]}\"",
        "unreadable Syntax",
    ),
    (
        "x=\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"\\\"$((1+$(git push)))\"",
        "unreadable Syntax",
    ),
    ("echo \"\\\n${N:-$(git push)}\"", "push ?"),
    ("echo \"${x}\\\n\\\n${N:-$(git push)}\"", "push ?"),
    (
        "echo \"\\\n\\\n\\\n\\\n\\\n\\\n\\\n\\\n\\\n\\\n`echo \\$(git push)`\"",
        "unreadable Syntax",
    ),
    (
        "echo \"\\\"$(echo a)é`echo \\$(git push)`\"",
        "unreadable Syntax",
    ),
    ("echo \"$\\\n{y}${a[$(git push)]}\"", "push ?"),
    ("echo $\\\n{y}${a[$(git push)]}\"z\"", "push ?"),
    (
        "echo \"\\\"\\\"\\\"\\\"\\\"z$[1+$(git push)]\"",
        "unreadable Syntax",
    ),
    ("echo \"\\\"${a[$(git push)]}\"", "unreadable Syntax"),
    ("echo \"\\\"$(date)\\\" `git push`\"", "unreadable Syntax"),
    ("eval $\\\n(printf \"git push\")", "unreadable Eval"),
    ("cat <<E\nx\\\nE\n: '$(git push)'\nE", "push ?"),
    ("cat <<E\nx\nE \n: '$(git push)'\nE", "unreadable Syntax"),
    (
        "cat <<-E\n\tx\n\tE\t\n: '$(git push)'\nE",
        "unreadable Syntax",
    ),
    (
        "cat <<E; cat <<F\nx\nE\n$(git push)\nF",
        "unreadable Syntax",
    ),
    ("cat <<{E}\n$(git push)\n{E}", "unreadable Syntax"),
    ("cat <<EF\nE\\\nF\ngit push\nEF", "push ?"),
    ("x=$(cat <<E\nx\nE)\ngit push\nE\n)", "unreadable Syntax"),
    ("echo \"\\\n$${\\\nx}${x:-$(git push)}\"", "push ?"),
    (
        "echo \"$'a'$\"a\"\\\\\\\né\\\n${\\\nx}$[1+$(git push)]${x:-'\\\"'}$$\"",
        "unreadable Syntax",
    ),
    ("cat <<\"E\\$\"\nx\nE$\ngit push\nE\\$", "unreadable Syntax"),
    ("cat <<E\\\n\n$(git push)\nE\\", "unreadable Syntax"),
    ("cat <<é\"\\$\"\nx\né$\ngit push", "unreadable Syntax"),
    ("cat <<A\n$(gi'\\\n't push)\nA", "push ?"),
    (
        "cat <<-A\n$(cat <<B\n\tB\ngit push\nB\n)\nA",
        "unreadable Syntax",
    ),
    ("echo `'gi\\\nt' push`", "push ?"),
    ("cat <<A <<EF\na\nA\nE\\\nF\ngit push\nEF", "push ?"),
    ("cat <<EF\nE\\\nF\ngit push", "push ?"),
    (
        "echo \"$(echo \"\\\"\")${x:-\\\"}\\\n\\\n\\\né$[1+$(git push)]${x:-\"\\\"\"}${\\\nx}\"",
        "unreadable Syntax",
    ),
    ("echo ${x\\\n:-$(git push)}", "push ?"),
    ("{\\\n git push; }", "push ?"),
    ("cat <<A\n$((1+$(gi'\\\n't push)))\nA", "push ?"),
    ("cat <<A\n${y\\\n:-$(git push)}\nA", "push ?"),
    ("echo $((true) && git push)", "unreadable Syntax"),
    ("cat <<A\n$((true) && git push)\nA", "unreadable Syntax"),
    ("echo \"$(c\\\nase x in x) git push;; esac)\"", "push ?"),
    ("bash <<-A\ngit\\\n\tpush\nA", "push ?"),
    ("git 3<<<x push", "unreadable Syntax"),
    ("bash <<A 3<<B\ngit push\nA\necho\nB", "unreadable Syntax"),
    ("{ bash; } <<<'git push'", "unreadable ShellString"),
    ("( bash ) <<A\ngit push\nA", "unreadable ShellString"),
    (
        "if true; then bash; fi <<A\ngit push\nA",
        "unreadable ShellString",
    ),
    ("bash <&3", "unreadable ShellString"),
    ("# note \\\ngit push", "unreadable Syntax"),
    ("echo x # note \\\ngit push", "unreadable Syntax"),
    ("[\\\n[ 'a[$(git push)]' -eq 1 ]]", "unreadable Syntax"),
    ("(\\\n( x = 'a[$(git push)]' ))", "unreadable Syntax"),
    ("x=$(\\\n( 1 )); git push", "push ?"),
    ("echo \"$(x)\\\"b\"; git push", "push ?"),
    ("echo \"${x:-\\$(git push)}\"", "unreadable Syntax"),
    ("echo ${PWD/\\/$(git push)/y}", "unreadable Syntax"),
    ("x=\"${PWD//\\/<(git push)/}\"", "unreadable Syntax"),
    ("echo \"${PWD#\\$x$(git push)}\"", "unreadable Syntax"),
    ("cat <<E\\$\ngit push\nE$", "unreadable Syntax"),
    ("cat <<{E}\ngit push\n{E}", "unreadable Syntax"),
    ("cat <<E\\\nF\ngit push\nEF", "unreadable Syntax"),
    ("bash -c '# c\\\ngit push'", "unreadable Syntax"),
    ("eval '# c\\\ngit push'", "unreadable Syntax"),
    (
        "cat <<<'git push' <<A\n$(bash)\nA",
        "unreadable ShellString",
    ),
    ("x=( [0]=a # c \\\n[1]=$(git push)\n)", "unreadable Syntax"),
    (
        "declare -A m=( [k]=v # c \\\n[j]=$(git push)\n)",
        "unreadable Syntax",
    ),
    ("m=( [k]=v # c \\\nx $(git push)\n)", "unreadable Syntax"),
    ("#!/bin/sh \\\ngit push", "unreadable Syntax"),
    ("echo ${M:-\\${$(git push)}", "unreadable Syntax"),
    ("x=${M:-\\${x$(git push)}", "unreadable Syntax"),
    ("echo ${M:-\\${#$(git push)}", "unreadable Syntax"),
    ("export x=${M:-\\${!$(git push)}", "unreadable Syntax"),
    (
        "N=abc; echo \"${N/a/\\${$(git push)}\"",
        "unreadable Syntax",
    ),
    ("N=abc; echo ${N^\\${$(git push)}", "unreadable Syntax"),
    ("N=abc; echo ${N#\\${x$(git push)}", "unreadable Syntax"),
    ("echo ${M:-$((1))\\${$(git push)}", "unreadable Syntax"),
    ("eval $'git push #\\\n\\''", "unreadable Syntax"),
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
    assert_eq!(show(&classify(&three)), "push origin main");
    assert_eq!(show(&classify(&nest(&three))), "unreadable Nesting");
}

/// The scan refuses before the parser runs, and closers the parser would not honour do not
/// lower its count.
#[test]
fn substitutions_nest_twelve_deep_and_deeper_is_unreadable_without_parsing() {
    use crate::shell::scan::too_deep_or_long;
    let nest = |n: usize| format!("{}git push{}", "echo $(".repeat(n), ")".repeat(n));
    assert_eq!(show(&classify(&nest(12))), "push ?");
    assert_eq!(show(&classify(&nest(13))), "unreadable Syntax");
    assert!(too_deep_or_long(&nest(12)).is_none());
    assert!(too_deep_or_long(&nest(13)).is_some());
    assert!(too_deep_or_long(&"(".repeat(2000)).is_some());
    for hidden in [
        format!("echo {}git push{}", "$(: }} ".repeat(13), ")".repeat(13)),
        format!(
            "echo \"'\"; echo {}git push{} #'",
            "$(".repeat(13),
            ")".repeat(13)
        ),
        format!(
            "echo \" #\"; echo {}git push{}",
            "$(".repeat(13),
            ")".repeat(13)
        ),
        format!(
            "cat <<'EOF'\r\nhi\r\nEOF\r\necho {}x{}",
            "$(".repeat(13),
            ")".repeat(13)
        ),
        format!("(( 1<<2 ))\na{}0{}=1", "[a".repeat(13), "]".repeat(13)),
        format!(
            "echo {}git push{}",
            "$(echo \")\" ".repeat(4097),
            ")".repeat(4097)
        ),
    ] {
        assert!(too_deep_or_long(&hidden).is_some(), "{hidden}");
    }
}

/// Every shape the parser recurses on, filled to the size cap, is read or refused with a
/// reason, never aborted, and a plain long script is not refused for its length. The parser
/// stops an operator chain at 1000 links and keyword nesting at 100 in every build; between
/// this reader's bracket cap and that, a debug build refuses on its stack budget where a
/// release build reads, so nothing in that band is asserted. Arithmetic and `${…}` bodies
/// are capped here because the parser is quadratic in them.
#[test]
fn every_shape_under_the_size_cap_parses_without_aborting() {
    let fill = |unit: &str, tail: &str| {
        let n = (super::MAX_BYTES - tail.len() - 1) / unit.len();
        format!("{}{tail}", unit.repeat(n))
    };
    let chain = |unit: &str, n: usize| format!("{}git push", unit.repeat(n));
    assert_eq!(show(&classify(&fill("time ", "git push"))), "push ?");
    assert_eq!(show(&classify(&fill("x=1;", "git push"))), "push ?");
    assert_eq!(show(&classify(&fill("true; ", "git push"))), "push ?");
    for unit in ["true && ", "true || ", "true | "] {
        assert_eq!(show(&classify(&chain(unit, 1000))), "push ?", "{unit}");
        assert_eq!(
            show(&classify(&chain(unit, 1001))),
            "unreadable Syntax",
            "{unit}"
        );
        assert_eq!(
            show(&classify(&fill(unit, "git push"))),
            "unreadable Syntax"
        );
    }
    let keyword = |n: usize| format!("{}git push{}", "if :; then ".repeat(n), ";fi".repeat(n));
    assert_eq!(show(&classify(&keyword(12))), "push ?");
    assert_eq!(show(&classify(&keyword(101))), "unreadable Syntax");
    let n = (super::MAX_BYTES - 16) / "if :; then ;fi".len();
    for shape in [
        fill("coproc ", "true"),
        fill("! ", "git push"),
        keyword(n),
        format!("if :; then\n{}fi", "echo a | cat\n".repeat(4000)),
    ] {
        assert!(shape.len() <= super::MAX_BYTES);
        classify(&shape);
    }
    let arith = |unit: &str, n: usize| format!("(( {}1 ))", unit.repeat(n));
    assert_eq!(show(&classify(&arith("1+", 1000))), "");
    assert_eq!(show(&classify(&arith("a=", 100))), "");
    assert_eq!(
        show(&classify(&format!("x=${{a:{}1}}", "1+".repeat(1000)))),
        ""
    );
    for over in [
        arith("1+", 1001),
        arith("a=", 101),
        format!("x=${{a:{}1}}", "1+".repeat(1001)),
        arith("1+", 2100),
        arith("a?", 2100),
        arith("!", 4200),
        format!("(( ((1)) + {}1 ))", "1+".repeat(30000)),
        format!("x=${{a:{}1}}", "1+".repeat(30000)),
        format!("echo ${{a[{}1]}}", "1+".repeat(2100)),
        format!("(( ${{x:-]}} + {}1 ))", "1+".repeat(2100)),
        format!("(( {}0{} ))", "a[".repeat(13), "]".repeat(13)),
        format!("echo ${{{}0{}}}", "a[".repeat(13), "]".repeat(13)),
        format!("cat <<EOF\n{}x{}\nEOF", "$(".repeat(13), ")".repeat(13)),
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
        "push origin main"
    );
    let items = (0..120)
        .map(|n| format!("  {{\"id\": {n}, \"tags\": [\"a\", \"b\"]}}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let json = format!("cat > data.json <<'EOF'\n[\n{items}\n]\nEOF\ngit push origin main");
    assert_eq!(show(&classify(&json)), "push origin main");
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
