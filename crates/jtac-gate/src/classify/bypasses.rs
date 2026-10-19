//! Hook bypasses, every spelling (DESIGN.md section 11.4): the rows of the spellings table
//! that skip a hook, kept out of the test module so `jtac bench` attacks each one through the
//! classifier. A new bypass is a new row here, with its reading.

/// The scripts the table marks as a hook bypass, for a caller that attacks each one.
pub fn bypasses() -> Vec<&'static str> {
    BYPASSES
        .iter()
        .filter(|(_, want)| want.contains("bypass"))
        .map(|(script, _)| *script)
        .collect()
}

pub(crate) const BYPASSES: &[(&str, &str)] = &[
    ("git push --no-verify", "push ? bypass=NoVerify"),
    (
        "git push origin main --no-verify",
        "push origin main bypass=NoVerify",
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
        "push origin main bypass=ConfigFlag",
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
        "push origin main bypass=NoVerify",
    ),
    ("git commit --no-verif -m x", "bypass NoVerify"),
    ("git merge --no-v topic", "bypass NoVerify"),
    (
        "git --config-env core.hooksPath=HP push origin main",
        "push origin main bypass=ConfigEnvFlag",
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
        "push origin main bypass=ConfigEnvVar",
    ),
    (
        "f() { local -x GIT_CONFIG_PARAMETERS=\"'core.hooksPath=/dev/null'\"; git push origin main; }; f",
        "bypass ConfigParameters ; push origin main",
    ),
    (
        "read -r GIT_CONFIG_PARAMETERS <<< x; export GIT_CONFIG_PARAMETERS; git push origin main",
        "unreadable Variable ; push origin main",
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
        "push origin main bypass=NoVerify",
    ),
    (
        "git push --no-verify --verify origin main",
        "push origin main",
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
        "bypass ConfigParameters ; push origin main",
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
    ("git push --dry-run origin main", "push origin main dry"),
    ("git push -o ci.skip origin main", "push origin main"),
    ("git push -on origin main", "push origin main"),
    ("git push origin -- --no-verify", "push origin --no-verify"),
    ("git push # --no-verify", "push ?"),
    ("git commit -m '--no-verify'", ""),
];
