//! `jtac hook pre-push` and `jtac status` against throwaway clones of a bare remote (DESIGN.md
//! section 11.3): every line git can hand the hook, a real `git push` through an installed
//! hook, the root push ahead of the code, and `status` asking the same decision.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jtac_core::{Covers, FromHead, GitOid, Root};
use jtac_testkit::Repo;
use serde_json::Value;

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").replace('\\', "/")
}

/// The binary under coverage drops a profile file where it runs; ignored, it leaves the tree
/// clean for the next check.
fn with_policy(repo: &Repo, extra: &str) {
    let policy = format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n{extra}",
        me()
    );
    repo.write(".jtac/policy.toml", policy.as_bytes());
    repo.write(".gitignore", b"*.profraw\n");
}

/// A key under `.git/`, where the work tree stays clean, named in the local config.
fn keyed(repo: &Repo) -> PathBuf {
    let key = new_key(repo, "key", "dev@example.com");
    let path = key.to_string_lossy().replace('\\', "/");
    repo.git(&["config", "jtac.signingkey", &path]);
    repo.git(&["config", "user.email", "dev@example.com"]);
    key
}

fn new_key(repo: &Repo, name: &str, email: &str) -> PathBuf {
    let key = repo.path().join(".git").join(name);
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", email, "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    key
}

/// One allowed-signers line, `options` with its trailing space or empty.
fn signers_line(key: &Path, email: &str, options: &str) -> String {
    let pubkey = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let pubkey: Vec<&str> = pubkey.split_whitespace().take(2).collect();
    format!("{email} {options}{}\n", pubkey.join(" "))
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Ran {
    fn of(out: std::process::Output) -> Self {
        Self {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }
}

fn jtac(repo: &Repo, args: &[&str]) -> Ran {
    Ran::of(repo.command(env!("CARGO_BIN_EXE_jtac"), args))
}

/// The hook as git runs it for `origin`: remote and URL as arguments, the ref lines on stdin.
fn hook(repo: &Repo, stdin: &str, extra: &[(&str, &str)]) -> Ran {
    hook_to(repo, "origin", stdin.as_bytes(), extra)
}

fn hook_to(repo: &Repo, remote: &str, stdin: &[u8], extra: &[(&str, &str)]) -> Ran {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jtac"))
        .args([
            "hook",
            "pre-push",
            remote,
            "https://example.com/example-app.git",
        ])
        .current_dir(repo.path())
        .env_clear()
        .envs(repo.env())
        .envs(extra.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    Ran::of(child.wait_with_output().unwrap())
}

/// A mirror remote deletes remote refs the clone lacks without a line on stdin, so a
/// protected branch would go unjudged; the push is refused on `remote.<name>.mirror` in any
/// of git's spellings, with empty stdin too, and when `GIT_CONFIG` points `git config` at
/// another file (issue 32; `git push` never reads it). The command the refusal names clears
/// the key where it is set, a second value and a global copy included (issue 33). A push of
/// a `refs/remotes/` ref by refspec is not a mirror and passes on its own merits.
#[test]
fn a_mirror_push_is_refused_because_git_hides_its_deletions() {
    let Checked {
        origin,
        repo,
        base,
        covered,
    } = checked();
    install_hook(&repo);
    repo.git(&["switch", "-q", "-c", "topic"]);
    repo.git(&["branch", "-q", "-D", "main"]);
    repo.git(&["update-ref", "-d", "refs/remotes/origin/main"]);
    for spelling in ["true", "yes", "on", "1"] {
        repo.git(&["config", "remote.origin.mirror", spelling]);
        let out = repo.command("git", &["push", "origin"]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{spelling}: {stderr}");
        assert!(
            stderr.contains("remote.origin.mirror"),
            "{spelling}: {stderr}"
        );
        assert_eq!(origin.git(&["rev-parse", "main"]), base, "main survived");
        let ran = hook(&repo, "", &[]);
        assert_eq!(ran.code, 2, "{spelling}, empty stdin: {}", ran.stderr);
    }
    repo.git(&["config", "--unset", "remote.origin.mirror"]);
    repo.git(&[
        "push",
        "-q",
        "origin",
        &format!("{covered}:refs/heads/keep"),
    ]);
    repo.git(&["config", "remote.origin.mirror", "true"]);
    let elsewhere = Repo::empty();
    let empty = elsewhere.write("empty", b"");
    let env = [("GIT_CONFIG", empty.to_str().unwrap())];
    assert!(
        !repo
            .command_with("git", &["push", "origin"], &env)
            .status
            .success()
    );
    let kept = origin.command("git", &["rev-parse", "--verify", "-q", "refs/heads/keep"]);
    assert!(
        kept.status.success(),
        "GIT_CONFIG hid the mirror: keep was deleted"
    );
    repo.git(&["config", "--add", "remote.origin.mirror", "yes"]);
    let clears = |ran: Ran, ends: &str, env: &[(&str, &str)]| {
        let (_, rest) = ran
            .stderr
            .split_once("Clear it: run `git config --file ")
            .unwrap();
        let (shown, rest) = rest
            .split_once(" --unset-all remote.origin.mirror`.")
            .unwrap();
        let file = shown.trim_matches('\'').replace("'\\''", "'");
        assert!(
            ran.code == 2 && rest.contains("`git push origin <branch>`"),
            "{}",
            ran.stderr
        );
        assert!(
            std::path::Path::new(&file).is_absolute() && file.ends_with(ends),
            "{file}"
        );
        let cmd = format!("git config --file {shown} --unset-all remote.origin.mirror");
        #[cfg(unix)]
        let out = repo.command("sh", &["-c", &format!("cd sub && {cmd}")]);
        #[cfg(not(unix))]
        let out = repo.command(
            "git",
            &[
                "-C",
                "sub",
                "config",
                "--file",
                &file,
                "--unset-all",
                "remote.origin.mirror",
            ],
        );
        assert!(
            out.status.success(),
            "{cmd}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(hook(&repo, "", env).code, 0);
    };
    std::fs::create_dir_all(repo.path().join("sub")).unwrap();
    clears(hook(&repo, "", &[]), "/.git/config", &[]);
    let mirrored = b"[remote \"origin\"]\n\tmirror = true\n";
    let global = elsewhere.write("gitconfig", mirrored);
    let env = [("GIT_CONFIG_GLOBAL", global.to_str().unwrap())];
    clears(hook(&repo, "", &env), "gitconfig", &env);
    let included = elsewhere.write("included", mirrored);
    repo.git(&["config", "include.path", included.to_str().unwrap()]);
    clears(hook(&repo, "", &[]), "included", &[]);
    #[cfg(unix)]
    {
        let odd = elsewhere.write("tab\thé'x", mirrored);
        repo.git(&["config", "include.path", odd.to_str().unwrap()]);
        clears(hook(&repo, "", &[]), "tab\thé'x", &[]);
    }
    repo.git(&["config", "--unset", "include.path"]);
    let url = repo.git(&["remote", "get-url", "origin"]);
    repo.git(&["remote", "add", "x;id;#", &url]);
    repo.git(&["config", "remote.x;id;#.mirror", "true"]);
    let ran = hook_to(&repo, "x;id;#", b"", &[]);
    let want = "--unset-all 'remote.x;id;#.mirror'`. Then push branches by name, `git push \
                'x;id;#' <branch>`";
    assert!(ran.code == 2 && ran.stderr.contains(want), "{}", ran.stderr);
    let out = repo.command(
        "git",
        &["-c", "remote.origin.mirror=true", "push", "origin"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let flag = "Clear it: drop `remote.origin.mirror` from the push's `-c` flags or `GIT_CONFIG_*` environment.";
    assert!(!out.status.success() && stderr.contains(flag), "{stderr}");
    let ran = hook(
        &repo,
        &format!("refs/remotes/origin/keep {covered} refs/remotes/origin/keep {base}\n"),
        &[],
    );
    assert_eq!(ran.code, 0, "{}", ran.stderr);
}

/// A ref under `refs/jtac/roots/` nobody wrote through jtac is noise: the hook, `status`,
/// and `check` keep judging and say what they skipped, once per push (issue 30).
#[test]
fn a_malformed_root_ref_is_skipped_with_a_note_not_a_refusal() {
    let Checked {
        repo,
        base,
        covered,
        ..
    } = checked();
    let junk = "refs/jtac/roots/zz/junk";
    repo.git(&["update-ref", junk, &base]);
    let note = format!("skipped {junk}:");
    let ran = hook(
        &repo,
        &format!(
            "refs/heads/main {covered} refs/heads/main {base}\n\
             refs/heads/feat {covered} refs/heads/feat {base}\n"
        ),
        &[],
    );
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert_eq!(ran.stderr.matches(&note).count(), 1, "{}", ran.stderr);
    let (ran, json) = status(&repo);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(json["notes"].to_string().contains(&note), "{json}");
    let ran = jtac(&repo, &["check", "--preview"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.contains(&note), "{}", ran.stdout);
}

/// A ref name git accepts and this release cannot read must refuse, never pass unjudged.
#[test]
fn non_utf8_stdin_refuses_rather_than_passes() {
    let Checked {
        origin: _origin,
        repo,
        base,
        covered,
    } = checked();
    let mut stdin = b"refs/heads/caf\xe9 ".to_vec();
    stdin.extend_from_slice(format!("{covered} refs/heads/caf\u{e9} {base}\n").as_bytes());
    let ran = hook_to(&repo, "origin", &stdin, &[]);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(
        ran.stderr.contains("could not read its stdin"),
        "{}",
        ran.stderr
    );
    assert!(ran.stderr.contains("negotiation: "), "{}", ran.stderr);
}

/// The script `jtac init` writes, pointing at the binary under test.
fn install_hook(repo: &Repo) {
    let hooks = repo
        .path()
        .join(repo.git(&["rev-parse", "--git-path", "hooks"]));
    std::fs::create_dir_all(&hooks).unwrap();
    let path = hooks.join("pre-push");
    let script = format!(
        "#!/bin/sh\n# jtac-hook v1\nexec '{}' hook pre-push \"$@\"\n",
        me()
    );
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn root_ref(commit: &str) -> String {
    format!("refs/jtac/roots/{}/{commit}", &commit[..2])
}

/// `base` on origin's `main` under the default fences, a key, and `covered` on top with a root.
struct Checked {
    origin: Repo,
    repo: Repo,
    base: String,
    covered: String,
}

fn checked() -> Checked {
    let origin = Repo::bare(&[]);
    let repo = Repo::clone_of(&origin);
    with_policy(&repo, "");
    repo.write("src/lib.rs", b"fn a() {}\n");
    let base = repo.commit("base");
    repo.git(&["push", "-q", "-u", "origin", "main"]);
    keyed(&repo);
    repo.write("a.txt", b"a\n");
    let covered = repo.commit("covered");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    Checked {
        origin,
        repo,
        base,
        covered,
    }
}

/// Beside `checked()`: `twin` has `covered`'s tree and no root, `bare` adds a file and has no
/// root, `allow` and `allow2` are orphans whose policy lets a rewind through, each with a
/// root, `nopolicy` is the empty tree.
struct Branches {
    twin: String,
    bare: String,
    allow: String,
    allow2: String,
    nopolicy: String,
}

fn branches(c: &Checked) -> Branches {
    let repo = &c.repo;
    let tree = format!("{}^{{tree}}", c.covered);
    let twin = repo.git(&["commit-tree", &tree, "-p", &c.base, "-m", "twin"]);
    repo.git(&["switch", "-q", "-c", "side", &c.base]);
    repo.write("b.txt", b"b\n");
    let bare = repo.commit("no root");
    repo.git(&["switch", "-q", "--orphan", "allow"]);
    with_policy(repo, "[fences]\nforce_push_protected = \"allow\"\n");
    let allow = repo.commit("allows a rewind");
    assert_eq!(jtac(repo, &["check"]).code, 0);
    repo.git(&["switch", "-q", "--orphan", "allow2"]);
    with_policy(repo, "[fences]\nforce_push_protected = \"allow\"\n");
    repo.write("c.txt", b"c\n");
    let allow2 = repo.commit("also allows a rewind");
    assert_eq!(jtac(repo, &["check"]).code, 0);
    repo.git(&["switch", "-q", "main"]);
    let nopolicy = repo.git(&["commit-tree", EMPTY_TREE, "-m", "no policy"]);
    Branches {
        twin,
        bare,
        allow,
        allow2,
        nopolicy,
    }
}

struct Row {
    claim: &'static str,
    stdin: String,
    guard: bool,
    code: i32,
    wants: Vec<String>,
}

fn row(claim: &'static str, stdin: String, code: i32, wants: &[&str]) -> Row {
    Row {
        claim,
        stdin,
        guard: false,
        code,
        wants: wants.iter().map(|w| (*w).to_owned()).collect(),
    }
}

/// The table, one row per line git can write.
fn table(c: &Checked, b: &Branches) -> Vec<Row> {
    let Branches {
        twin,
        bare,
        allow,
        allow2,
        nopolicy,
    } = b;
    let (base, covered) = (&c.base, &c.covered);
    let (z, unseen) = ("0".repeat(40), "1".repeat(40));
    let main = |local: &str, old: &str| format!("refs/heads/main {local} refs/heads/main {old}\n");
    let evidence = root_ref(covered);
    vec![
        row("nothing to push", String::new(), 0, &[]),
        row(
            "deleting an unprotected branch passes without evidence",
            format!("(delete) {z} refs/heads/side {bare}\n"),
            0,
            &["jtac: refs/heads/side: a deletion needs no evidence"],
        ),
        row(
            "deleting a protected branch is the fence: recreating it would rewind it",
            format!("(delete) {z} refs/heads/main {base}\n"),
            1,
            &[
                "deletes a protected branch",
                "fence: No command clears this",
            ],
        ),
        row(
            "deleting a branch whose tip this clone lacks refuses, naming git fetch",
            format!("(delete) {z} refs/heads/side {unseen}\n"),
            1,
            &["negotiation: Run `git fetch origin refs/heads/side`"],
        ),
        row(
            "a tag passes without evidence",
            format!("refs/tags/v1 {bare} refs/tags/v1 {z}\n"),
            0,
            &["jtac: refs/tags/v1: a tag needs no evidence"],
        ),
        row(
            "a root ref is evidence, not code",
            format!("{evidence} {covered} {evidence} {z}\n"),
            0,
            &["evidence, not code"],
        ),
        row(
            "a tip with no root is refused, naming jtac check",
            main(bare, base),
            1,
            &[
                &format!("{bare} has no root"),
                "negotiation: Run `jtac check`",
            ],
        ),
        row(
            "a covered tip passes and its root goes first",
            main(covered, base),
            0,
            &[
                &format!("{covered} has a root, covered"),
                "sent 1 root(s) to origin",
            ],
        ),
        row(
            "a local ref with a space in it is read from the right",
            format!("main@{{0 days ago}} {covered} refs/heads/main {base}\n"),
            0,
            &[&format!("{covered} has a root, covered")],
        ),
        row(
            "a root for the same tree on another commit passes and is named",
            format!("refs/heads/topic {twin} refs/heads/topic {z}\n"),
            0,
            &[&format!("{covered}'s root judged the same tree")],
        ),
        row(
            "a rewound protected branch is the fence, root or not",
            main(base, covered),
            1,
            &["fence: No command clears this", "bigger `--force`"],
        ),
        row(
            "an old oid this clone lacks refuses, naming git fetch",
            main(covered, &unseen),
            1,
            &["negotiation: Run `git fetch origin refs/heads/main`"],
        ),
        row(
            "an old oid this clone lacks refuses on an unprotected branch too",
            format!("refs/heads/side {covered} refs/heads/side {unseen}\n"),
            1,
            &[
                "half the fence",
                "negotiation: Run `git fetch origin refs/heads/side`",
            ],
        ),
        row(
            "an orphan whose policy allows the rewind is still fenced by the branch",
            main(allow, covered),
            1,
            &[
                "fence: No command clears this",
                "at the branch's tip or at yours",
            ],
        ),
        row(
            "a rewind both tips allow is judged on its root",
            main(allow2, allow),
            0,
            &[&format!("{allow2} has a root, covered")],
        ),
        Row {
            guard: true,
            ..row(
                "the guard runs nothing and says nothing",
                main(bare, base),
                0,
                &[],
            )
        },
        row(
            "a line that is not four fields cannot be judged",
            "refs/heads/main deadbeef\n".into(),
            2,
            &["cannot read the line"],
        ),
        row(
            "a tip without a policy file is refused, naming jtac init",
            format!("refs/heads/topic {nopolicy} refs/heads/topic {z}\n"),
            2,
            &["`refs/heads/topic`: ", "jtac init"],
        ),
        row(
            "the first refusal gets three parts, a later one a line, and a pass still prints",
            [
                main(bare, base),
                main(covered, &unseen),
                main(covered, base),
            ]
            .concat(),
            1,
            &[
                "negotiation: Run `jtac check`",
                "jtac: `refs/heads/main`: the remote's tip",
                &format!("{covered} has a root, covered"),
            ],
        ),
    ]
}

#[test]
fn every_line_git_can_hand_the_hook_is_judged_as_section_11_3_says() {
    let c = checked();
    let b = branches(&c);
    let (base, covered) = (&c.base, &c.covered);
    let main = |local: &str, old: &str| format!("refs/heads/main {local} refs/heads/main {old}\n");
    let evidence = root_ref(covered);
    for r in table(&c, &b) {
        let extra: &[(&str, &str)] = if r.guard { &[("JTAC_HOOK", "1")] } else { &[] };
        let ran = hook(&c.repo, &r.stdin, extra);
        let claim = r.claim;
        assert_eq!(ran.code, r.code, "{claim}: {}", ran.stderr);
        assert_eq!(ran.stdout, "", "{claim}: the hook owns no stdout");
        if r.wants.is_empty() {
            assert_eq!(ran.stderr, "", "{claim}");
        }
        for want in &r.wants {
            assert!(
                ran.stderr.contains(want),
                "{claim}: {want:?} in {}",
                ran.stderr
            );
        }
    }
    assert_eq!(
        c.origin.git(&["rev-parse", &evidence]),
        c.repo.git(&["rev-parse", &evidence]),
        "the passing row pushed the root"
    );
    assert_eq!(
        c.origin.git(&["rev-parse", "main"]),
        *base,
        "the hook never pushes code"
    );
    let lost = hook_to(&c.repo, "nowhere", main(covered, base).as_bytes(), &[]);
    assert_eq!(
        lost.code, 0,
        "a root push that fails never refuses: {}",
        lost.stderr
    );
    for want in [
        "jtac: WARNING: the roots did not reach nowhere",
        &evidence,
        "next: git push nowhere 'refs/jtac/roots/*:refs/jtac/roots/*'",
    ] {
        assert!(lost.stderr.contains(want), "{want:?} in {}", lost.stderr);
    }
    c.repo
        .git(&["config", "gpg.ssh.program", "jtac-no-such-tool"]);
    let blind = hook(&c.repo, &main(covered, base), &[]);
    assert_eq!(
        blind.code, 2,
        "an unchecked signature never passes: {}",
        blind.stderr
    );
    assert!(
        blind.stderr.contains("cannot check a root's signature"),
        "{}",
        blind.stderr
    );
}

#[test]
fn a_root_whose_signature_was_replaced_is_absent_with_a_note() {
    let Checked {
        repo,
        base,
        covered,
        ..
    } = checked();
    let name = root_ref(&covered);
    let json = repo.git(&["rev-parse", &format!("{name}:root.json")]);
    let garbage = repo.write(".git/garbage", b"not a signature\n");
    let garbage = garbage.to_string_lossy().into_owned();
    let sig = repo.git(&["hash-object", "-w", "--", &garbage]);
    let listing = format!("100644 blob {json}\troot.json\n100644 blob {sig}\troot.sig\n");
    let listing_path = repo.write(".git/listing", listing.as_bytes());
    let tree = Ran::of(
        Command::new("git")
            .args(["mktree"])
            .current_dir(repo.path())
            .env_clear()
            .envs(repo.env())
            .stdin(std::fs::File::open(listing_path).unwrap())
            .output()
            .unwrap(),
    );
    let forged = repo.git(&["commit-tree", tree.stdout.trim(), "-m", "forged"]);
    repo.git(&["update-ref", &name, &forged]);
    let ran = hook(
        &repo,
        &format!("refs/heads/main {covered} refs/heads/main {base}\n"),
        &[],
    );
    assert_eq!(ran.code, 1, "{}", ran.stderr);
    assert!(
        ran.stderr.contains(&format!(
            "jtac: note: {covered}'s root is not counted: its signature does not hold"
        )),
        "{}",
        ran.stderr
    );
    assert!(ran.stderr.contains("has no root"), "{}", ran.stderr);
    assert!(
        ran.stderr.contains(&format!(
            "run `git update-ref -d {name}`, then `jtac check`"
        )),
        "{}",
        ran.stderr
    );
}

/// A graft rewrites the parents git shows, so a rewind reads as a fast-forward; the hook and
/// `status` refuse it as `check` does.
#[test]
fn a_grafts_file_refuses_the_hook_and_status() {
    let Checked {
        origin: _origin,
        repo,
        base,
        covered,
    } = checked();
    repo.git(&["push", "-q", "origin", "main"]);
    repo.git(&["reset", "-q", "--hard", &base]);
    repo.write("c.txt", b"c\n");
    let rework = repo.commit("rework");
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    let grafts = repo.write(
        ".git/info/grafts",
        format!("{rework} {covered}\n").as_bytes(),
    );
    let ran = hook(
        &repo,
        &format!("refs/heads/main {rework} refs/heads/main {covered}\n"),
        &[],
    );
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    let named = std::fs::canonicalize(&grafts).unwrap();
    let named = named.to_string_lossy().replace('\\', "/");
    let named = named.strip_prefix("//?/").unwrap_or(&named);
    assert!(ran.stderr.contains(named), "{named}: {}", ran.stderr);
    let ran = jtac(&repo, &["status"]);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(ran.stderr.contains("grafts"), "{}", ran.stderr);
}

#[test]
fn a_real_push_is_refused_before_check_and_carries_its_root_after() {
    let origin = Repo::bare(&[]);
    let repo = Repo::clone_of(&origin);
    with_policy(&repo, "");
    let base = repo.commit("base");
    repo.git(&["push", "-q", "-u", "origin", "main"]);
    keyed(&repo);
    install_hook(&repo);
    repo.write("a.txt", b"a\n");
    let tip = repo.commit("tip");
    let refused = Ran::of(repo.command("git", &["push", "origin", "main"]));
    assert_ne!(refused.code, 0, "{}", refused.stderr);
    assert!(
        refused.stderr.contains("Run `jtac check`"),
        "{}",
        refused.stderr
    );
    assert_eq!(
        origin.git(&["rev-parse", "main"]),
        base,
        "the branch did not move"
    );
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    let pushed = Ran::of(repo.command("git", &["push", "origin", "main"]));
    assert_eq!(pushed.code, 0, "{}", pushed.stderr);
    assert!(
        pushed.stderr.contains("has a root, covered"),
        "{}",
        pushed.stderr
    );
    assert_eq!(origin.git(&["rev-parse", "main"]), tip);
    assert_eq!(
        origin.git(&["rev-parse", &root_ref(&tip)]),
        repo.git(&["rev-parse", &root_ref(&tip)]),
        "the remote holds the clone's root object"
    );
}

#[test]
fn a_root_the_remote_refuses_warns_and_the_code_push_still_lands() {
    let Checked {
        origin,
        repo,
        covered,
        ..
    } = checked();
    let other = repo.git(&["commit-tree", EMPTY_TREE, "-m", "another clone's root"]);
    let name = root_ref(&covered);
    repo.git(&["push", "-q", "origin", &format!("{other}:{name}")]);
    install_hook(&repo);
    let ran = Ran::of(repo.command("git", &["push", "origin", "main"]));
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    for want in [
        "jtac: WARNING: the roots did not reach origin",
        &name,
        "next: git push origin 'refs/jtac/roots/*:refs/jtac/roots/*'",
    ] {
        assert!(ran.stderr.contains(want), "{want:?} in {}", ran.stderr);
    }
    assert_eq!(
        origin.git(&["rev-parse", "main"]),
        covered,
        "the code landed"
    );
    assert_eq!(
        origin.git(&["rev-parse", &name]),
        other,
        "the remote's root stands"
    );
}

fn status(repo: &Repo) -> (Ran, Value) {
    let ran = jtac(repo, &["status"]);
    let json = jtac(repo, &["status", "--json"]);
    assert_eq!(json.code, ran.code, "{}", json.stderr);
    (ran, serde_json::from_str(&json.stdout).unwrap())
}

#[test]
fn status_passes_on_a_tree_match_and_names_the_commit_whose_root_judged_it() {
    let Checked { repo, covered, .. } = checked();
    repo.git(&[
        "commit",
        "-q",
        "--amend",
        "--no-gpg-sign",
        "-m",
        "same tree",
    ]);
    let (ran, v) = status(&repo);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout.contains(&format!(
            "has no root of its own; {covered}'s root judged the same tree"
        )),
        "{}",
        ran.stdout
    );
    assert_eq!(v["pass"], true);
    assert_eq!(v["root"], Value::Null);
    assert_eq!(v["tree_match"], format!("sha1:{covered}"));
}

#[test]
fn status_reads_a_rewind_of_a_protected_upstream_as_the_fence() {
    let Checked {
        origin: _origin,
        repo,
        base,
        ..
    } = checked();
    repo.git(&["push", "-q", "origin", "main"]);
    repo.git(&["reset", "-q", "--hard", &base]);
    repo.write("c.txt", b"c\n");
    repo.commit("rework");
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    let (ran, v) = status(&repo);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout.contains("PUSH WOULD BE REFUSED: HEAD") && ran.stdout.contains("rewinds"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("  fence   "), "{}", ran.stdout);
    assert_eq!(v["pass"], false);
    assert_eq!(v["refused"], "rewind");
    assert!(v["root"].as_str().unwrap().starts_with("sha256:"), "{v}");
}

/// The push goes where `branch.<name>.merge` points, not to a branch named like the local
/// one: `fix` tracking `main` rewinds a protected branch.
#[test]
fn status_judges_the_branch_head_merges_with_not_the_local_name() {
    let Checked {
        origin: _origin,
        repo,
        base,
        ..
    } = checked();
    repo.git(&["push", "-q", "origin", "main"]);
    repo.git(&["switch", "-q", "-c", "fix", &base]);
    repo.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
    repo.write("c.txt", b"c\n");
    repo.commit("rework");
    assert_eq!(jtac(&repo, &["check"]).code, 0);
    let (ran, v) = status(&repo);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert_eq!(v["refused"], "rewind", "{v}");
}

/// Origin's `main` at `base` names the dev key plain and the CI key for the CI namespace;
/// `covered` has the dev's root; `merge`, HEAD, sits on `covered` with a tree of its own and
/// no root yet.
struct CiFix {
    repo: Repo,
    dev: PathBuf,
    ci: PathBuf,
    base: String,
    covered: String,
    merge: String,
}

const CI_OPTION: &str = "namespaces=\"jtac-ci-root-v1,jtac-release-v1\" ";

fn ci_fixture() -> CiFix {
    let origin = Repo::bare(&[]);
    let repo = Repo::clone_of(&origin);
    with_policy(&repo, "");
    let dev = keyed(&repo);
    let ci = new_key(&repo, "ci", "ci@example.com");
    let signers = format!(
        "{}{}",
        signers_line(&dev, "dev@example.com", ""),
        signers_line(&ci, "ci@example.com", CI_OPTION)
    );
    repo.write(".jtac/signers", signers.as_bytes());
    repo.write("src/lib.rs", b"fn a() {}\n");
    let base = repo.commit("base");
    repo.git(&["push", "-q", "-u", "origin", "main"]);
    repo.git(&["remote", "set-head", "origin", "--auto"]);
    repo.write("a.txt", b"a\n");
    let covered = repo.commit("covered");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    repo.write("m.txt", b"m\n");
    let merge = repo.commit("merge");
    std::mem::forget(origin);
    CiFix {
        repo,
        dev,
        ci,
        base,
        covered,
        merge,
    }
}

fn g(hex: &str) -> GitOid {
    format!("sha1:{hex}").parse().unwrap()
}

fn fingerprint(key: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .arg("-lf")
        .arg(key.with_extension("pub"))
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned()
}

impl CiFix {
    /// `merge`'s root with `from` naming `covered`, signed by `key` as `email` in the CI
    /// namespace.
    fn root_from(&self, key: &Path, email: &str) {
        let json = self.repo.git(&[
            "cat-file",
            "-p",
            &format!("{}:root.json", root_ref(&self.covered)),
        ]);
        let mut root: Root = serde_json::from_str(&json).unwrap();
        root.commit = g(&self.merge);
        root.tree = g(&self
            .repo
            .git(&["rev-parse", &format!("{}^{{tree}}", self.merge)]));
        root.signer.principal = email.to_owned();
        root.signer.fingerprint = fingerprint(key);
        root.base = Some(g(&self.base));
        root.covers = Covers::List(vec![g(&self.merge)]);
        root.parents = vec![];
        root.from = Some(FromHead {
            head: g(&self.covered),
            root: None,
            compared: None,
        });
        let canonical = String::from_utf8(root.canonical_bytes().unwrap()).unwrap();
        self.repo
            .put_root(key, "jtac-ci-root-v1", &self.merge, &canonical, None);
    }

    fn line(&self, remote_ref: &str, old: &str) -> String {
        format!("{remote_ref} {} {remote_ref} {old}\n", self.merge)
    }
}

/// DESIGN.md section 11.3: a root with `from` passes the hook only under a signer the remote's
/// default branch tip's signers, or the old tip's, name for the CI namespace.
#[test]
fn a_from_root_passes_the_hook_only_under_a_signer_the_remote_names_for_the_ci_namespace() {
    let f = ci_fixture();
    let zero = "0".repeat(40);
    let new_branch = f.line("refs/heads/feature", &zero);
    f.root_from(&f.ci, "ci@example.com");
    let ran = hook(&f.repo, &new_branch, &[]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stderr
            .contains(&format!("{} has a root, covered", f.merge)),
        "{}",
        ran.stderr
    );
    let ran = hook_to(
        &f.repo,
        "https://example.com/example-app.git",
        new_branch.as_bytes(),
        &[],
    );
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(
        ran.stderr.contains("this clone holds neither")
            && ran.stderr.contains("git remote add <name> <url>"),
        "{}",
        ran.stderr
    );
    let update = f.line("refs/heads/main", &f.covered);
    let ran = hook_to(
        &f.repo,
        "https://example.com/example-app.git",
        update.as_bytes(),
        &[],
    );
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    f.repo
        .git(&["symbolic-ref", "-d", "refs/remotes/origin/HEAD"]);
    let ran = hook(&f.repo, &new_branch, &[]);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(
        ran.stderr.contains("git remote set-head origin --auto"),
        "{}",
        ran.stderr
    );
    f.repo.git(&["remote", "set-head", "origin", "--auto"]);
    f.repo.git(&["update-ref", "-d", &root_ref(&f.merge)]);
    f.root_from(&f.dev, "dev@example.com");
    let ran = hook(&f.repo, &new_branch, &[]);
    assert_eq!(ran.code, 1, "{}", ran.stderr);
    assert!(
        ran.stderr.contains(
            "it has `from`, a CI root, and its signer is not named for namespace \
             jtac-ci-root-v1 by origin/HEAD:.jtac/signers"
        ),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr
            .contains(&format!("git update-ref -d {}", root_ref(&f.merge))),
        "{}",
        ran.stderr
    );
}

#[test]
fn status_asks_the_hook_s_question_of_a_from_root_at_head() {
    let f = ci_fixture();
    f.root_from(&f.ci, "ci@example.com");
    let (ran, v) = status(&f.repo);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert_eq!(v["pass"], true, "{v}");
    f.repo.git(&["update-ref", "-d", &root_ref(&f.merge)]);
    f.root_from(&f.dev, "dev@example.com");
    let (ran, v) = status(&f.repo);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert_eq!(v["pass"], false, "{v}");
    assert!(
        v["notes"].as_array().unwrap().iter().any(|n| n
            .as_str()
            .unwrap()
            .contains("not named for namespace jtac-ci-root-v1")),
        "{v}"
    );
}
