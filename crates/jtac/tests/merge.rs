//! The merge job end to end (DESIGN.md sections 15 and 15.1, M16): a developer's clone, a
//! forge stood in by a bare repository, the default-branch job's clone holding the CI key,
//! and a second clone that reads `jtac audit main` with nothing but what the remote holds.

use std::path::{Path, PathBuf};
use std::process::Command;

use jtac_core::{Covers, GitOid, Root};
use jtac_testkit::Repo;
use serde_json::Value;

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").replace('\\', "/")
}

fn key(dir: &Path, name: &str, email: &str) -> PathBuf {
    let key = dir.join(".git").join(name);
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

fn signers_line(key: &Path, email: &str, option: &str) -> String {
    let pubkey = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let pubkey: Vec<&str> = pubkey.split_whitespace().take(2).collect();
    format!("{email} {option}{}\n", pubkey.join(" "))
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

fn jtac(repo: &Repo, args: &[&str]) -> Ran {
    let out = repo.command(env!("CARGO_BIN_EXE_jtac"), args);
    Ran {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

fn ok(repo: &Repo, args: &[&str]) -> String {
    let ran = jtac(repo, args);
    assert_eq!(ran.code, 0, "{args:?}: {}{}", ran.stdout, ran.stderr);
    ran.stdout
}

fn json(repo: &Repo, args: &[&str]) -> (i32, Value, String) {
    let ran = jtac(repo, args);
    let v = serde_json::from_str(&ran.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}{}", ran.stdout, ran.stderr));
    (ran.code, v, ran.stderr)
}

fn root_ref(commit: &str) -> String {
    format!("refs/jtac/roots/{}/{commit}", &commit[..2])
}

fn failed_ref(commit: &str) -> String {
    format!("refs/jtac/failed/{}/{commit}", &commit[..2])
}

fn read_at(repo: &Repo, name: &str) -> Root {
    let out = repo.command("git", &["cat-file", "blob", &format!("{name}:root.json")]);
    assert!(out.status.success(), "{name} holds no document");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn read_root(repo: &Repo, commit: &str) -> Root {
    read_at(repo, &root_ref(commit))
}

fn read_record(repo: &Repo, commit: &str) -> Root {
    read_at(repo, &failed_ref(commit))
}

/// Replace `commit`'s failure record ref with `record`, signed by `key` in the CI namespace.
fn put_record(repo: &Repo, key: &Path, commit: &str, record: &Root) {
    let json = String::from_utf8(record.canonical_bytes().unwrap()).unwrap();
    repo.put_record(key, "jtac-ci-root-v1", commit, &json, None);
}

/// Replace `commit`'s root ref with `root`, freshly signed by `key` in the namespace its
/// shape picks.
fn put(repo: &Repo, key: &Path, commit: &str, root: &Root) {
    let json = String::from_utf8(root.canonical_bytes().unwrap()).unwrap();
    let ns = match root.from {
        Some(_) => "jtac-ci-root-v1",
        None => "jtac-root-v1",
    };
    repo.put_root(key, ns, commit, &json, None);
}

fn g(hex: &str) -> GitOid {
    format!("sha1:{hex}").parse().unwrap()
}

fn sha(hex: &str) -> String {
    format!("sha1:{hex}")
}

fn row<'a>(v: &'a Value, commit: &str) -> &'a Value {
    v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["commit"] == sha(commit))
        .unwrap_or_else(|| panic!("no row for {commit}: {v}"))
}

const CI_OPTION: &str = "namespaces=\"jtac-ci-root-v1,jtac-release-v1\" ";
const ROOTS: &str = "refs/jtac/roots/*:refs/jtac/roots/*";
const JTAC: &str = "+refs/jtac/*:refs/jtac/*";

/// The forge: `origin` is the base repository, `dev` the developer's clone with the
/// developer key, `ci` the default-branch job's clone with the CI key, `main` the adoption
/// commit, rooted by the developer and pushed with its root.
struct Forge {
    origin: Repo,
    dev: Repo,
    ci: Repo,
    dev_key: PathBuf,
    ci_key: PathBuf,
    main: String,
}

fn forge() -> Forge {
    forge_with(true)
}

/// `signers` false leaves `.jtac/signers` out of the adoption commit: the first pull request
/// brings it, and the old tip trusts nobody.
fn forge_with(signers: bool) -> Forge {
    let origin = Repo::bare(&[]);
    let dev = Repo::clone_of(&origin);
    let policy = format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n",
        me()
    );
    dev.write(".jtac/policy.toml", policy.as_bytes());
    dev.write(".gitignore", b"*.profraw\n");
    dev.write("src/lib.rs", b"#[test]\nfn keeps() {}\nfn a() {}\n");
    let dev_key = key(dev.path(), "dev", "dev@example.com");
    let ci_key = key(dev.path(), "ci", "ci@example.com");
    let lines = format!(
        "{}{}",
        signers_line(&dev_key, "dev@example.com", ""),
        signers_line(&ci_key, "ci@example.com", CI_OPTION)
    );
    dev.write(".git/signers", lines.as_bytes());
    if signers {
        dev.write(".jtac/signers", lines.as_bytes());
    }
    let path = dev_key.to_string_lossy().replace('\\', "/");
    dev.git(&["config", "jtac.signingkey", &path]);
    dev.git(&["config", "user.email", "dev@example.com"]);
    let main = dev.commit("adopt");
    ok(&dev, &["check"]);
    dev.git(&["push", "-q", "-u", "origin", "main"]);
    dev.git(&["push", "-q", "origin", ROOTS]);
    let ci = Repo::clone_of(&origin);
    ci.git(&["config", "--add", "remote.origin.fetch", JTAC]);
    ci.git(&["fetch", "-q", "origin"]);
    let path = ci_key.to_string_lossy().replace('\\', "/");
    ci.git(&["config", "jtac.signingkey", &path]);
    ci.git(&["config", "user.email", "ci@example.com"]);
    Forge {
        origin,
        dev,
        ci,
        dev_key,
        ci_key,
        main,
    }
}

impl Forge {
    /// `n` commits on a branch `name` off `main` in the developer's clone, each adding a file,
    /// with `jtac check` at the tip. Oldest first.
    fn feature(&self, name: &str, n: usize) -> Vec<String> {
        self.dev.git(&["switch", "-q", "-c", name, "main"]);
        let commits: Vec<String> = (0..n)
            .map(|i| {
                self.dev
                    .write(&format!("{name}-{i}.txt"), format!("{i}\n").as_bytes());
                self.dev.commit(&format!("{name} {i}"))
            })
            .collect();
        ok(&self.dev, &["check"]);
        commits
    }

    /// The developer pushes `branch` to `remote` and, unless the hook was bypassed, the roots.
    fn push(&self, remote: &str, branch: &str, roots: bool) {
        self.dev.git(&["push", "-q", remote, branch]);
        if roots {
            self.dev.git(&["push", "-q", remote, ROOTS]);
        }
    }

    /// The job's clone fetches `remote`'s branches and roots.
    fn fetch(&self, remote: &str) {
        self.ci.git(&["fetch", "-q", remote]);
        self.ci.git(&["fetch", "-q", remote, JTAC]);
    }

    /// The forge stands `main` at `origin/main`.
    fn at_main(&self) {
        self.ci
            .git(&["checkout", "-q", "-B", "main", "origin/main"]);
    }

    fn merge(&self, head: &str) -> String {
        self.at_main();
        self.ci
            .git(&["merge", "-q", "--no-ff", "-m", "merge", head]);
        self.ci.git(&["rev-parse", "HEAD"])
    }

    fn squash(&self, head: &str) -> String {
        self.at_main();
        self.ci.git(&["merge", "-q", "--squash", head]);
        self.ci
            .git(&["commit", "-q", "--no-gpg-sign", "-m", "squash"]);
        self.ci.git(&["rev-parse", "HEAD"])
    }

    fn rebase(&self, head: &str) -> String {
        self.at_main();
        self.ci.git(&["checkout", "-q", "--detach", head]);
        self.ci.git(&["rebase", "-q", "main"]);
        self.ci.git(&["update-ref", "refs/heads/main", "HEAD"]);
        self.ci.git(&["checkout", "-q", "main"]);
        self.ci.git(&["rev-parse", "HEAD"])
    }

    /// The forge pushes the branch, then the job runs on it.
    fn job(&self, head: &str, old: &str) -> (i32, Value, String) {
        self.ci.git(&["push", "-q", "origin", "main"]);
        json(
            &self.ci,
            &[
                "check", "--from", head, "--base", old, "--push", "origin", "--json",
            ],
        )
    }

    /// A second clone of `origin` with its roots fetched.
    fn second(&self) -> Repo {
        let second = Repo::clone_of(&self.origin);
        second.git(&["fetch", "-q", "origin", JTAC]);
        second
    }

    /// `jtac audit main --json` in a second clone.
    fn audit(&self) -> Value {
        let (code, v, err) = json(&self.second(), &["audit", "main", "--json"]);
        assert_eq!(code, 0, "{v}{err}");
        v
    }
}

fn principal(row: &Value) -> &str {
    row["signer"]["principal"].as_str().unwrap_or_default()
}

/// A merge commit, a squash and a rebase are each a commit the forge made: covered by the CI
/// principal with the head verified, its root held, every rebased commit covered by the merge
/// root, and the adoption commit by the developer's key. The merge's tree is not the head's,
/// so the head's checks ran in a worktree; the squash's is, so CI's own nodes stood in.
#[test]
fn forge_made_commits_are_covered_by_the_ci_principal_with_the_head_verified() {
    let f = forge();
    let b = f.feature("b", 2);
    f.push("origin", "b", true);
    f.fetch("origin");
    let squash = f.squash("origin/b");
    assert_eq!(
        f.ci.git(&["rev-parse", "HEAD^{tree}"]),
        f.ci.git(&["rev-parse", &format!("{}^{{tree}}", b[1])]),
        "a squash onto an unmoved base has the head's tree"
    );
    let (code, v, err) = f.job(&b[1], &f.main);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    assert_eq!(
        f.ci.git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1,
        "the head's checks hit on the tip's tree: no worktree"
    );

    let a = f.feature("a", 2);
    f.push("origin", "a", true);
    f.fetch("origin");
    let merge = f.merge("origin/a");
    assert_ne!(
        f.ci.git(&["rev-parse", "HEAD^{tree}"]),
        f.ci.git(&["rev-parse", &format!("{}^{{tree}}", a[1])]),
        "the merge's tree is not the head's: its checks run in a worktree"
    );
    let (code, v, err) = f.job(&a[1], &squash);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["head"], sha(&a[1]), "{v}");
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    assert_eq!(v["head_root"]["trust"], "trusted", "{v}");
    assert_eq!(v["pushed"].as_array().map(Vec::len), Some(2), "{v}");
    assert!(
        err.contains("sent 1 root(s)"),
        "the head's root was already there: {err}"
    );
    assert_eq!(
        f.ci.git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1,
        "the head's worktree is removed and pruned"
    );
    let registered = f.ci.path().join(".git/worktrees");
    assert!(
        !registered.exists() || registered.read_dir().unwrap().next().is_none(),
        "no worktree registration is left behind"
    );
    let root = read_root(&f.ci, &merge);
    assert_eq!(root.base, Some(g(&squash)));
    assert_eq!(root.parents.len(), 1);
    assert_eq!(root.parents[0].commit, g(&squash));
    assert_eq!(
        root.covers,
        Covers::List(vec![g(&a[0]), g(&a[1]), g(&merge)])
    );
    assert_eq!(
        root.from.as_ref().unwrap().root,
        Some(read_root(&f.dev, &a[1]).id().unwrap())
    );
    assert!(ok(&f.ci, &["replay", &merge]).contains("CLEARED HOT"));

    let c = f.feature("c", 3);
    f.push("origin", "c", true);
    f.fetch("origin");
    let rebased = f.rebase("origin/c");
    let (code, v, err) = f.job(&c[2], &merge);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    let root = read_root(&f.ci, &rebased);
    assert!(
        matches!(&root.covers, Covers::List(l) if l.len() == 3),
        "{root:?}"
    );

    let v = f.audit();
    for tip in [&merge, &squash, &rebased] {
        let r = row(&v, tip);
        assert_eq!(r["state"], "covered", "{r}");
        assert_eq!(r["via"], "own", "{r}");
        assert_eq!(principal(r), "ci@example.com", "{r}");
        assert_eq!(r["trust"], "trusted", "{r}");
        assert_eq!(r["head"]["label"], "verified", "{r}");
        assert_eq!(r["head"]["held"], true, "{r}");
        assert_eq!(r["head"]["signer"]["principal"], "dev@example.com", "{r}");
        assert_eq!(r["head"]["trust"], "trusted", "{r}");
    }
    assert_eq!(row(&v, &squash)["head"]["commit"], sha(&b[1]));
    let rebased_commits: Vec<String> = (0..3)
        .map(|i| f.ci.git(&["rev-parse", &format!("{rebased}~{i}")]))
        .collect();
    for c in &rebased_commits[1..] {
        let r = row(&v, c);
        assert_eq!(r["state"], "covered", "{r}");
        assert_eq!(r["via"], "covers", "{r}");
        assert_eq!(r["by"], sha(&rebased), "{r}");
    }
    let adopt = row(&v, &f.main);
    assert_eq!(adopt["via"], "own");
    assert_eq!(principal(adopt), "dev@example.com");
    assert!(adopt.get("head").is_none(), "{adopt}");
    assert_eq!(v["totals"]["absent"], 0, "{v}");
    let text = ok(&f.ci, &["audit", "main"]);
    assert!(
        text.contains("verified, root held by dev@example.com (trusted)"),
        "{text}"
    );
}

/// A fork's head: its roots live in the fork, and CI forwards the head's root ref into the
/// base repository when it agreed and never otherwise, so a second clone sees `head root not
/// held` beside `disagreed` and `lied`.
#[test]
fn a_fork_head_root_enters_the_base_repository_only_when_ci_found_it_agreeing() {
    let f = forge();
    let fork = Repo::bare(&[]);
    let url = fork.path().to_string_lossy().replace('\\', "/");
    f.dev.git(&["remote", "add", "fork", &url]);
    f.ci.git(&["remote", "add", "fork", &url]);

    let a = f.feature("a", 1);
    f.push("fork", "a", true);
    f.fetch("fork");
    let merge = f.merge("fork/a");
    let (code, v, err) = f.job(&a[0], &f.main);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    assert!(err.contains("sent 2 root(s)"), "{err}");
    let held = f
        .origin
        .command("git", &["rev-parse", "--verify", "-q", &root_ref(&a[0])]);
    assert!(
        held.status.success(),
        "the head's root ref reached the base repository"
    );

    let b = f.feature("b", 1);
    let mut lie = read_root(&f.dev, &b[0]);
    lie.counts.pass = 0;
    put(&f.dev, &f.dev_key, &b[0], &lie);
    f.push("fork", "b", true);
    f.fetch("fork");
    let merge_b = f.merge("fork/b");
    let (code, v, err) = f.job(&b[0], &merge);
    assert_eq!(code, 0, "disagreement never stops the merge root: {v}{err}");
    assert_eq!(v["from"]["compared"], "disagreed", "{v}");
    assert_eq!(v["pushed"].as_array().map(Vec::len), Some(1), "{v}");
    let notes = v["verdict"]["notes"].to_string();
    assert!(
        notes.contains("is not forwarded: CI found it disagreed"),
        "{notes}"
    );
    let held = f
        .origin
        .command("git", &["rev-parse", "--verify", "-q", &root_ref(&b[0])]);
    assert!(
        !held.status.success(),
        "a disagreeing root enters no ref every clone fetches"
    );

    let c = f.feature("c", 1);
    let mut lie = read_root(&f.dev, &c[0]);
    lie.policy.hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        .parse()
        .unwrap();
    put(&f.dev, &f.dev_key, &c[0], &lie);
    f.push("fork", "c", true);
    f.fetch("fork");
    let merge_c = f.merge("fork/c");
    let (code, v, err) = f.job(&c[0], &merge_b);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "lied", "{v}");

    let v = f.audit();
    let r = row(&v, &merge);
    assert_eq!(principal(r), "ci@example.com", "{r}");
    assert_eq!(r["head"]["label"], "verified", "{r}");
    assert_eq!(r["head"]["held"], true, "{r}");
    let r = row(&v, &merge_b);
    assert_eq!(r["state"], "covered", "{r}");
    assert_eq!(r["head"]["label"], "disagreed", "{r}");
    assert_eq!(r["head"]["held"], false, "{r}");
    let r = row(&v, &merge_c);
    assert_eq!(r["head"]["label"], "lied", "{r}");
    assert_eq!(r["head"]["held"], false, "{r}");
    let text = ok(&f.second(), &["audit", "main"]);
    assert!(text.contains("disagreed, root not held"), "{text}");
    for head in [&b[0], &c[0]] {
        let r = row(&v, head);
        assert_eq!(
            r["state"], "covered",
            "the merge root covers the fork's commit: {r}"
        );
        assert_eq!(r["via"], "covers", "{r}");
    }
}

/// What `from` records is what the merge job found: a head pushed around the hook reads
/// `head absent` whatever root lands later, one from another gate version `head unverified`,
/// one naming a tree not its commit's `head invalid`.
#[test]
fn the_head_label_is_what_the_merge_job_found_and_a_root_published_later_changes_nothing() {
    let f = forge();
    let a = f.feature("a", 1);
    f.push("origin", "a", false);
    f.fetch("origin");
    let merge_a = f.merge("origin/a");
    let (code, v, err) = f.job(&a[0], &f.main);
    assert_eq!(code, 0, "{v}{err}");
    assert!(v["from"].get("compared").is_none(), "{v}");
    assert!(v["head_root"].is_null(), "{v}");
    f.dev.git(&["push", "-q", "origin", ROOTS]);

    let b = f.feature("b", 1);
    let mut old = read_root(&f.dev, &b[0]);
    old.gate.version = "0.0.1".to_owned();
    put(&f.dev, &f.dev_key, &b[0], &old);
    f.push("origin", "b", true);
    f.fetch("origin");
    let merge_b = f.merge("origin/b");
    let (code, v, err) = f.job(&b[0], &merge_a);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "unverifiable", "{v}");

    let c = f.feature("c", 1);
    let mut bad = read_root(&f.dev, &c[0]);
    bad.tree = g(&f.dev.git(&["rev-parse", &format!("{}^{{tree}}", f.main)]));
    put(&f.dev, &f.dev_key, &c[0], &bad);
    f.push("origin", "c", true);
    f.fetch("origin");
    let merge_c = f.merge("origin/c");
    let (code, v, err) = f.job(&c[0], &merge_b);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "invalid", "{v}");

    let v = f.audit();
    let r = row(&v, &merge_a);
    assert_eq!(r["head"]["label"], "absent", "{r}");
    assert_eq!(r["head"]["held"], false, "{r}");
    assert_eq!(
        row(&v, &a[0])["via"],
        "own",
        "the root published later stands for its own commit"
    );
    let r = row(&v, &merge_b);
    assert_eq!(r["head"]["label"], "unverified", "{r}");
    assert_eq!(r["head"]["held"], true, "{r}");
    assert_eq!(r["head"]["trust"], "trusted", "{r}");
    let r = row(&v, &merge_c);
    assert_eq!(r["head"]["label"], "invalid", "{r}");
    assert_eq!(r["head"]["held"], true, "{r}");
    assert_eq!(
        row(&v, &c[0])["state"],
        "covered",
        "by the merge root's covers"
    );
    assert_eq!(row(&v, &c[0])["via"], "covers");
    let text = ok(&f.ci, &["audit", "main"]);
    assert!(
        text.contains(&format!("head {} absent", &a[0][..12])),
        "{text}"
    );
}

/// A direct push gets no CI root: with a tree of its own it reads absent and the next merge
/// root's `covers` begins after it; with a tree a root already judged it reads covered by
/// tree. A head cleared by a trusted commit waiver merges under the same waiver.
#[test]
fn a_direct_push_reads_absent_or_by_tree_and_a_head_waiver_declares_the_merge_off() {
    let f = forge();
    let a = f.feature("a", 1);
    f.dev.git(&["switch", "-q", "main"]);
    f.dev.git(&[
        "commit",
        "-q",
        "--allow-empty",
        "--no-gpg-sign",
        "-m",
        "empty",
    ]);
    let empty = f.dev.git(&["rev-parse", "HEAD"]);
    f.dev.write("direct.txt", b"d\n");
    let direct = f.dev.commit("direct");
    f.push("origin", "main", false);
    f.dev.git(&["switch", "-q", "a"]);
    f.dev.write("a-0.txt", b"changed\n");
    let a1 = f.dev.commit("a 1");
    ok(&f.dev, &["waive", "HEAD", "--reason", "approved as pushed"]);
    ok(&f.dev, &["check"]);
    assert_eq!(read_root(&f.dev, &a1).state, jtac_core::State::DeclaredOff);
    f.push("origin", "a", true);
    f.fetch("origin");
    let merge = f.merge("origin/a");
    let (code, v, err) = f.job(&a1, &direct);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["verdict"]["state"], "declared_off", "{v}");
    assert_eq!(
        v["verdict"]["waivers"].as_array().map(Vec::len),
        Some(1),
        "{v}"
    );
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    let root = read_root(&f.ci, &merge);
    assert_eq!(root.base, Some(g(&direct)));
    assert!(
        root.parents.is_empty(),
        "the old tip has no root: a visible break"
    );
    assert_eq!(root.covers, Covers::List(vec![g(&a[0]), g(&a1), g(&merge)]));

    let v = f.audit();
    let r = row(&v, &direct);
    assert_eq!(r["state"], "absent", "{r}");
    let r = row(&v, &empty);
    assert_eq!(r["state"], "covered", "{r}");
    assert_eq!(r["via"], "tree", "{r}");
    assert_eq!(r["by"], sha(&f.main), "{r}");
    let r = row(&v, &merge);
    assert_eq!(r["state"], "declared_off", "{r}");
    assert_eq!(r["head"]["label"], "verified", "{r}");
    assert_eq!(
        v["waivers"].as_array().map(Vec::len),
        Some(2),
        "head's and merge's: {v}"
    );
}

/// Past the bound a merge root records `base..tip`, so a side line merged inside the pull
/// request reads covered too; the developer's own root, bounded the ancestry-path way, would
/// not have covered it.
#[test]
fn a_pull_request_past_the_covers_bound_with_a_side_line_inside_reads_covered_throughout() {
    let f = forge();
    f.dev.git(&["switch", "-q", "-c", "a", "main"]);
    f.dev.git(&["switch", "-q", "-c", "side", "a"]);
    f.dev.write("side.txt", b"s\n");
    let side = f.dev.commit("side");
    f.dev.git(&["switch", "-q", "a"]);
    for i in 0..258 {
        f.dev.git(&[
            "commit",
            "-q",
            "--allow-empty",
            "--no-gpg-sign",
            "-m",
            &format!("a {i}"),
        ]);
    }
    f.dev
        .git(&["merge", "-q", "--no-ff", "-m", "side in", "side"]);
    let head = f.dev.git(&["rev-parse", "HEAD"]);
    f.push("origin", "a", false);
    f.fetch("origin");
    let merge = f.merge("origin/a");
    let (code, v, err) = f.job(&head, &f.main);
    assert_eq!(code, 0, "{v}{err}");
    let root = read_root(&f.ci, &merge);
    assert_eq!(
        root.covers,
        Covers::Range {
            base: g(&f.main),
            tip: g(&merge)
        }
    );
    let v = f.audit();
    assert_eq!(v["totals"]["absent"], 0, "{}", v["totals"]);
    assert_eq!(v["totals"]["commits"], 262, "{}", v["totals"]);
    let r = row(&v, &side);
    assert_eq!(r["via"], "covers", "{r}");
    assert_eq!(r["by"], sha(&merge), "{r}");
}

/// The job refuses, with exit 2 and three parts, a head that is HEAD itself (a fast-forward,
/// which gets no merge root), a head that is no commit, an old tip that is not behind HEAD,
/// and a developer key's run is a root readers see as an unknown signer's, noted.
#[test]
fn the_merge_job_refuses_a_fast_forward_an_unknown_head_and_a_base_not_behind_head() {
    let f = forge();
    let a = f.feature("a", 1);
    f.push("origin", "a", true);
    f.fetch("origin");
    f.at_main();
    f.ci.git(&["merge", "-q", "--ff-only", "origin/a"]);
    let ran = jtac(&f.ci, &["check", "--from", &a[0], "--base", &f.main]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stderr.contains("is HEAD itself"), "{}", ran.stderr);
    assert!(ran.stderr.contains("jtac ci --base"), "{}", ran.stderr);
    let ran = jtac(&f.ci, &["check", "--from", "nope", "--base", &f.main]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.contains("does not name a commit"),
        "{}",
        ran.stderr
    );
    let b = f.feature("b", 1);
    f.push("origin", "b", true);
    f.fetch("origin");
    let merge = f.merge("origin/a");
    for base in [b[0].as_str(), merge.as_str(), a[0].as_str()] {
        let ran = jtac(&f.ci, &["check", "--from", &a[0], "--base", base]);
        assert_eq!(ran.code, 2, "{base}: {}", ran.stderr);
        assert!(
            ran.stderr
                .contains("is not on the first-parent line of HEAD"),
            "{}",
            ran.stderr
        );
        assert!(
            ran.stderr.contains(&format!("--from {}", a[0])),
            "{}",
            ran.stderr
        );
    }
    let ran = jtac(&f.ci, &["check", "--from", &f.main, "--base", &f.main]);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(ran.stderr.contains("is reachable from"), "{}", ran.stderr);
    let ran = jtac(
        &f.ci,
        &["check", "--from", &a[0], "--base", "x\nfence: forged"],
    );
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert_eq!(ran.stderr.lines().count(), 3, "{}", ran.stderr);
    let (code, v, err) = json(
        &f.ci,
        &[
            "check", "--from", &a[0], "--base", &f.main, "--push", "nowhere", "--json",
        ],
    );
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["pass"], true, "{v}");
    assert_eq!(v["pushed"], serde_json::json!([]), "{v}");
    assert!(err.contains("did not reach nowhere"), "{err}");
    let (code, v, _) = json(
        &f.ci,
        &[
            "check", "--from", &a[0], "--base", &f.main, "--push", "origin", "--json",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["pushed"], serde_json::json!([]), "a standing root: {v}");
    assert!(
        v["verdict"]["notes"]
            .to_string()
            .contains("nothing was pushed"),
        "{v}"
    );
    f.ci.git(&["update-ref", "-d", &root_ref(&merge)]);
    let path = f.dev_key.to_string_lossy().replace('\\', "/");
    f.ci.git(&["config", "jtac.signingkey", &path]);
    f.ci.git(&["config", "user.email", "dev@example.com"]);
    let (code, v, _) = json(
        &f.ci,
        &["check", "--from", &a[0], "--base", &f.main, "--json"],
    );
    assert_eq!(code, 0, "{v}");
    let notes = v["verdict"]["notes"].to_string();
    assert!(
        notes.contains("is not named for namespace jtac-ci-root-v1"),
        "{notes}"
    );
    let ran = jtac(&f.ci, &["verify", "--json"]);
    let v: Value = serde_json::from_str(&ran.stdout).unwrap();
    assert_eq!(v["unknown_signers"], 1, "{v}");
    let _ = &f.ci_key;
}

/// The head's checks run on the head's tree under the head's own policy: a policy the pull
/// request adds requires `marker.txt`, which main deletes meanwhile, so a run on the merge
/// tip's checkout would disagree. A head root with `from` whose declared base names a policy
/// that is neither the head's nor the branch's is unverifiable in the keyed job, while the
/// pull request job still compares it (decision 47).
#[test]
fn the_head_s_checks_run_on_its_own_tree_and_under_no_policy_but_its_own_or_the_branch_s() {
    let f = forge();
    f.dev.write("marker.txt", b"m\n");
    f.dev.commit("marker");
    ok(&f.dev, &["check"]);
    f.push("origin", "main", true);
    let policy = std::fs::read_to_string(f.dev.path().join(".jtac/policy.toml")).unwrap();
    let marker = format!(
        "{policy}[checks.marker]\nrun = [\"git\", \"rev-parse\", \"--verify\", \"-q\", \
         \"HEAD:marker.txt\"]\n"
    );
    f.dev.git(&["switch", "-q", "-c", "a", "main"]);
    f.dev.write(".jtac/policy.toml", marker.as_bytes());
    let head = f.dev.commit("policy requires the marker");
    ok(
        &f.dev,
        &[
            "waive",
            "--rule",
            "JT-PROT-001",
            "--path",
            ".jtac/",
            "--expires",
            "7d",
            "--reason",
            "reviewed",
        ],
    );
    ok(&f.dev, &["check"]);
    assert_eq!(
        read_root(&f.dev, &head).counts.checks,
        3,
        "integrity, ok, marker"
    );
    f.push("origin", "a", true);
    f.dev.git(&["switch", "-q", "main"]);
    f.dev.remove("marker.txt");
    let deleted = f.dev.commit("marker gone");
    ok(&f.dev, &["check"]);
    f.push("origin", "main", true);
    f.fetch("origin");
    let merge = f.merge("origin/a");
    assert!(!f.ci.path().join("marker.txt").exists());
    let (code, v, err) = f.job(&head, &deleted);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["policy_from"], "base", "{v}");
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    assert_eq!(v["head_root"]["run"]["counts"]["pass"], 3, "{v}");
    assert_eq!(
        read_root(&f.ci, &merge).levers.len(),
        1,
        "the head's rule waiver applied"
    );

    f.dev.git(&["fetch", "-q", "origin"]);
    f.dev.git(&["fetch", "-q", "origin", JTAC]);
    f.dev.git(&["switch", "-q", "-c", "b", "origin/main"]);
    f.dev.write("marker.txt", b"m\n");
    let foreign = format!(
        "{policy}[checks.extra]\nrun = [\"{}\", \"--version\"]\n",
        me()
    );
    f.dev.write(".jtac/policy.toml", foreign.as_bytes());
    let b1 = f.dev.commit("foreign policy");
    let (_, preview, _) = json(&f.dev, &["check", "--preview", "--json"]);
    let foreign_hash = preview["policy"].as_str().unwrap().to_owned();
    f.dev.write(".jtac/policy.toml", marker.as_bytes());
    let b2 = f.dev.commit("the branch's policy again, marker back");
    ok(&f.dev, &["check"]);
    let mut forged = read_root(&f.dev, &b2);
    forged.base = Some(g(&b1));
    forged.covers = Covers::List(vec![g(&b2)]);
    forged.parents = vec![];
    forged.policy.hash = foreign_hash.parse().unwrap();
    forged.counts.pass -= 1;
    forged.from = Some(jtac_core::FromHead {
        head: g(&b1),
        root: None,
        compared: None,
    });
    put(&f.dev, &f.dev_key, &b2, &forged);
    let (code, v, _) = json(&f.dev, &["ci", "--base", "origin/main", "--json"]);
    assert_eq!(code, 1, "the pull request job compares it: {v}");
    assert_eq!(v["head_root"]["compared"], "disagreed", "{v}");
    f.push("origin", "b", true);
    f.fetch("origin");
    f.merge("origin/b");
    let (code, v, err) = f.job(&b2, &merge);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "unverifiable", "{v}");
    assert!(
        v["head_root"]["why"]
            .as_str()
            .unwrap()
            .contains("holds the CI key"),
        "{v}"
    );
}

/// A root already on the forge's tip is kept only as CI's own for this head: a developer's
/// root pushed onto the merge commit around the job is refused with the ref to delete, the
/// job passes once it is gone, and a re-run finds CI's own root and keeps it.
#[test]
fn a_root_already_on_the_forge_s_tip_is_kept_only_as_ci_s_own_for_this_head() {
    let f = forge();
    let a = f.feature("a", 1);
    f.push("origin", "a", true);
    f.fetch("origin");
    let merge = f.merge("origin/a");
    f.ci.git(&["push", "-q", "origin", "main"]);
    let mut forged = read_root(&f.ci, &a[0]);
    forged.commit = g(&merge);
    forged.tree = g(&f.ci.git(&["rev-parse", &format!("{merge}^{{tree}}")]));
    forged.covers = Covers::List(vec![g(&a[0]), g(&merge)]);
    put(&f.ci, &f.dev_key, &merge, &forged);
    let ran = jtac(
        &f.ci,
        &[
            "check", "--from", &a[0], "--base", &f.main, "--push", "origin",
        ],
    );
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr
            .contains("already carries a root this job did not write")
            && ran.stderr.contains("no `from`")
            && ran.stderr.contains(&format!(":{}", root_ref(&merge)))
            && ran.stderr.contains("fence: "),
        "{}",
        ran.stderr
    );
    f.ci.git(&["update-ref", "-d", &root_ref(&merge)]);
    let (code, v, err) = f.job(&a[0], &f.main);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    let (code, v, err) = f.job(&a[0], &f.main);
    assert_eq!(code, 0, "{v}{err}");
    let notes = v["verdict"]["notes"].to_string();
    assert!(notes.contains("HEAD already has a root; it stands"), "{v}");
    let own = read_root(&f.ci, &merge);
    assert_eq!(own.signer.principal, "ci@example.com");
    let refused = |root: &Root, key: &Path, why: &str| {
        put(&f.ci, key, &merge, root);
        let ran = jtac(&f.ci, &["check", "--from", &a[0], "--base", &f.main]);
        assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
        assert!(ran.stderr.contains(why), "{}", ran.stderr);
    };
    let mut other = own.clone();
    other.from.as_mut().unwrap().head = g(&f.main);
    refused(&other, &f.ci_key, "its `from` names head");
    let mut dev = own.clone();
    dev.signer = read_root(&f.ci, &a[0]).signer;
    refused(&dev, &f.dev_key, "not named for namespace jtac-ci-root-v1");
    put(&f.ci, &f.ci_key, &merge, &own);
    let (code, v, err) = f.job(&a[0], &f.main);
    assert_eq!(code, 0, "{v}{err}");
}

/// The adoption merge's old tip names no key, so CI's own root verifies as an unknown
/// signer's there; a re-run keeps it all the same, since it is the job's own key's.
#[test]
fn a_re_run_on_the_adoption_merge_keeps_ci_s_own_root() {
    let f = forge_with(false);
    f.dev.git(&["switch", "-q", "-c", "a", "main"]);
    let lines = std::fs::read(f.dev.path().join(".git").join("signers")).unwrap();
    f.dev.write(".jtac/signers", &lines);
    let a = f.dev.commit("signers");
    f.push("origin", "a", false);
    f.fetch("origin");
    let merge = f.merge("origin/a");
    let (code, v, err) = f.job(&a, &f.main);
    assert_eq!(code, 0, "{v}{err}");
    assert!(
        v["verdict"]["notes"]
            .to_string()
            .contains("not named for namespace"),
        "{v}"
    );
    let (code, v, err) = f.job(&a, &f.main);
    assert_eq!(code, 0, "{v}{err}");
    assert!(
        v["verdict"]["notes"]
            .to_string()
            .contains("HEAD already has a root; it stands"),
        "{v}"
    );
    assert_eq!(read_root(&f.ci, &merge).signer.principal, "ci@example.com");
}

/// A root pushed onto the forge's commit between the job's fetch and its push: the remote
/// refuses the merge root, the job reads back what is there and judges it as standing. A
/// developer's is the fence; a concurrent run's own is kept and this run's stays unsent,
/// the local ref untouched; a remote that refuses the ref and holds none is its own fence.
#[test]
fn a_root_pushed_onto_the_tip_while_the_job_runs_is_judged_as_standing() {
    let f = forge();
    let a = f.feature("a", 1);
    f.push("origin", "a", true);
    f.fetch("origin");
    let merge = f.merge("origin/a");
    f.ci.git(&["push", "-q", "origin", "main"]);
    f.dev.git(&["fetch", "-q", "origin"]);
    let mut forged = read_root(&f.dev, &a[0]);
    forged.commit = g(&merge);
    forged.tree = g(&f.dev.git(&["rev-parse", &format!("{merge}^{{tree}}")]));
    forged.covers = Covers::List(vec![g(&a[0]), g(&merge)]);
    put(&f.dev, &f.dev_key, &merge, &forged);
    f.dev.git(&["push", "-q", "origin", ROOTS]);
    let ran = jtac(
        &f.ci,
        &[
            "check", "--from", &a[0], "--base", &f.main, "--push", "origin",
        ],
    );
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr.contains("no `from`")
            && ran
                .stderr
                .contains("origin holds it, pushed after this run fetched")
            && ran.stderr.contains(&format!(":{}", root_ref(&merge))),
        "{}",
        ran.stderr
    );
    f.dev
        .git(&["push", "-q", "origin", &format!(":{}", root_ref(&merge))]);
    f.ci.git(&["update-ref", "-d", &root_ref(&merge)]);
    let (code, v, err) = f.job(&a[0], &f.main);
    assert_eq!(code, 0, "{v}{err}");
    assert!(v["pushed"].to_string().contains(&root_ref(&merge)), "{v}");
    let first = read_root(&f.ci, &merge);
    f.ci.git(&["update-ref", "-d", &root_ref(&merge)]);
    let (code, v, err) = f.job(&a[0], &f.main);
    assert_eq!(code, 0, "{v}{err}");
    let notes = v["verdict"]["notes"].to_string();
    assert!(
        notes.contains("origin already holds CI's own root for this head"),
        "{v}"
    );
    assert!(!v["pushed"].to_string().contains(&root_ref(&merge)), "{v}");
    assert_eq!(read_root(&f.second(), &merge), first);
    assert_ne!(read_root(&f.ci, &merge), first);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.dev
            .git(&["push", "-q", "origin", &format!(":{}", root_ref(&merge))]);
        f.ci.git(&["update-ref", "-d", &root_ref(&merge)]);
        let hook = f.origin.write(
            "hooks/pre-receive",
            b"#!/bin/sh\nwhile read old new ref; do case \"$ref\" in refs/jtac/roots/*) exit 1;; \
              esac; done\nexit 0\n",
        );
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ran = jtac(
            &f.ci,
            &[
                "check", "--from", &a[0], "--base", &f.main, "--push", "origin",
            ],
        );
        assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
        assert!(
            ran.stderr.contains("holds no root for the tip")
                && ran.stderr.contains(&format!("origin {}", root_ref(&merge))),
            "{}",
            ran.stderr
        );
    }
}

/// A repository whose `main` reads `e2e` from CI: `first` is the merge that brought the
/// policy and the definition file, judged under the old tip's policy as any policy change
/// is; `second` merges `b`, which holds the developer's partial root, and is pushed with no
/// job run on it yet.
struct Reading {
    f: Forge,
    a: String,
    first: String,
    b: String,
    second: String,
}

const WORKFLOW: &str = ".github/workflows/ci.yml";

fn reading() -> Reading {
    reading_with("")
}

/// `pin` is the rest of the `ci` table: the job, where a test names one.
fn reading_with(pin: &str) -> Reading {
    let f = forge();
    f.dev.git(&["switch", "-q", "-c", "a", "main"]);
    let read = format!(
        "version = 2\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n\
         [checks.e2e]\nci = {{ workflow = \"{WORKFLOW}\"{pin} }}\n",
        me()
    );
    f.dev.write(".jtac/policy.toml", read.as_bytes());
    f.dev.write(WORKFLOW, b"on: push\n");
    let a = f.dev.commit("read e2e from ci");
    let ran = jtac(&f.dev, &["check"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.contains("JT-PROT-001"), "{}", ran.stdout);
    f.push("origin", "a", false);
    f.fetch("origin");
    let first = f.merge("origin/a");
    f.ci.git(&["push", "-q", "origin", "main"]);
    let (code, v, err) = json(
        &f.ci,
        &[
            "check",
            "--from",
            &a,
            "--base",
            &f.main,
            "--push",
            "origin",
            "--json",
            "--results",
            "no-such-file.json",
            "--branch",
            "main",
        ],
    );
    assert_eq!(
        (code, &v["policy_from"]),
        (1, &Value::from("base")),
        "{v}{err}"
    );
    let notes = v["verdict"]["notes"].to_string();
    assert!(notes.contains("`--results` was not read"), "{notes}");
    f.dev.git(&["fetch", "-q", "origin"]);
    f.dev.git(&["switch", "-q", "-c", "b", "origin/main"]);
    f.dev.write("b.txt", b"b\n");
    let b = f.dev.commit("b");
    let ran = jtac(&f.dev, &["check", "--base", "origin/main"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    assert!(ran.stdout.contains("     e2e unread  "), "{}", ran.stdout);
    f.push("origin", "b", true);
    f.fetch("origin");
    let second = f.merge("origin/b");
    f.ci.git(&["push", "-q", "origin", "main"]);
    Reading {
        f,
        a,
        first,
        b,
        second,
    }
}

/// The results file as a forge adapter writes it, kept outside the work tree: one push run
/// of the definition file for `commit` on `branch`, by the run's own conclusion. Named
/// relative to the clone, so a refusal prints it the same on every platform: a temp dir's
/// `~` or `:` would print quoted.
fn results(repo: &Repo, commit: &str, branch: &str, concluded: Option<&str>) -> String {
    results_of(repo, commit, branch, concluded, "")
}

/// `jobs` is the run's job list as the file holds it.
fn results_of(
    repo: &Repo,
    commit: &str,
    branch: &str,
    concluded: Option<&str>,
    jobs: &str,
) -> String {
    let end = match concluded {
        Some(word) => format!(r#""status":"completed","conclusion":"{word}""#),
        None => r#""status":"in_progress""#.to_owned(),
    };
    let text = format!(
        r#"{{"v":1,"forge":"github","runs":[{{"workflow":"{WORKFLOW}","commit":"sha1:{commit}","event":"push","ref":"{branch}","id":1234,"attempt":2,{end},"jobs":[{jobs}]}}]}}"#
    );
    let word = concluded.unwrap_or("going");
    let name = format!(".git/runs-{}-{branch}-{word}.json", &commit[..7]);
    repo.write(&name, text.as_bytes());
    name
}

impl Reading {
    /// The merge job on `second` with `more` flags, as text.
    fn job(&self, more: &[&str]) -> Ran {
        let base = [
            "check",
            "--from",
            &self.b,
            "--base",
            &self.first,
            "--push",
            "origin",
        ];
        jtac(&self.f.ci, &[&base[..], more].concat())
    }

    fn state(&self, commit: &str) -> (String, String) {
        let v = self.f.audit();
        let word = |key: &str| row(&v, commit)[key].as_str().unwrap_or("none").to_owned();
        (word("state"), word("ci"))
    }
}

/// DESIGN.md section 15.2, invariant of the merge job: every `ci` check is read before
/// anything is written, so with no file, a file that does not read, a run still going, a
/// run for another commit or another branch, the job answers with its own command and
/// the merge has neither a root nor a failure record. The answer is 3 for the run still
/// going, which an adapter waits out, and 2 for every other, which fails its job.
#[test]
fn a_result_missing_or_pending_writes_nothing_and_a_pending_one_answers_3() {
    let r = reading();
    let ci = &r.f.ci;
    let ran = r.job(&[]);
    assert_eq!((ran.code, ran.stdout.as_str()), (2, ""), "{}", ran.stderr);
    let what = "jtac: Check `e2e` in the `.jtac/policy.toml` the merge job judges under is read \
                from CI, and this run was given no results file.";
    assert!(ran.stderr.starts_with(what), "{}", ran.stderr);
    let how = format!(
        "negotiation: Run `jtac check --from {} --base {} --results <file> --branch <branch> \
         --push origin`",
        r.b, r.first
    );
    assert!(ran.stderr.contains(&how), "{}", ran.stderr);
    ci.write(".git/garbage.json", b"{\"v\":2}");
    let garbage = ".git/garbage.json".to_owned();
    let rows = [
        (
            results(ci, &r.second, "main", None),
            "main",
            "`e2e` is pending (.github/workflows/ci.yml)",
        ),
        (
            results(ci, &r.first, "main", Some("success")),
            "main",
            "`e2e` is missing",
        ),
        (
            results(ci, &r.second, "main", Some("success")),
            "release",
            "`e2e` is missing",
        ),
        (
            results(ci, &r.second, "main", Some("success")),
            "",
            "`--branch` is empty.",
        ),
        (
            garbage,
            "main",
            "garbage.json was not read: not a version 1 results file",
        ),
        (
            "no-such-file.json".to_owned(),
            "main",
            "no-such-file.json was not read: ",
        ),
    ];
    for (file, branch, want) in &rows {
        let ran = r.job(&["--results", file, "--branch", branch]);
        let code = if want.contains("is pending") { 3 } else { 2 };
        assert_eq!(
            (ran.code, ran.stdout.as_str()),
            (code, ""),
            "{want}: {}",
            ran.stderr
        );
        assert!(ran.stderr.contains(want), "{want}: {}", ran.stderr);
        assert_eq!(ran.stderr.lines().count(), 3, "{want}: {}", ran.stderr);
        let again = format!("--results {file} --branch");
        assert!(ran.stderr.contains(&again), "{want}: {}", ran.stderr);
    }
    let refs = r.f.second().git(&["for-each-ref", "refs/jtac"]);
    assert!(!refs.contains(&r.second), "{refs}");
    assert_eq!(r.state(&r.second), ("absent".into(), "none".into()));
    assert_eq!(r.state(&r.b).0, "partial");

    r.f.dev.git(&["fetch", "-q", "origin"]);
    r.f.dev.git(&["switch", "-q", "-c", "d", "origin/main"]);
    r.f.dev.git(&["rm", "-q", WORKFLOW]);
    let d = r.f.dev.commit("drop the definition file");
    r.f.push("origin", "d", false);
    r.f.fetch("origin");
    let third = r.f.merge("origin/d");
    ci.git(&["push", "-q", "origin", "main"]);
    let file = results(ci, &third, "main", Some("success"));
    let flags = ["--from", &d, "--base", &r.second, "--results", &file];
    let ran = jtac(
        ci,
        &[&["check"], &flags[..], &["--branch", "main"]].concat(),
    );
    assert_eq!((ran.code, ran.stdout.as_str()), (2, ""), "{}", ran.stderr);
    let what = format!(
        "jtac: Check `e2e` is read from {WORKFLOW}, and commit {}'s tree holds no regular \
         file there.",
        &third[..12]
    );
    assert!(ran.stderr.starts_with(&what), "{}", ran.stderr);
}

/// DESIGN.md section 15.2, M20's done-when: the merge job takes `e2e` from the results
/// file and writes a version 3 root that holds its node, a second clone reads the merge
/// and the pull request's commit covered through it beside the developer's partial root,
/// and `replay` holds the root on the job's store. The head's root, which left `e2e`
/// unread, still compares agreed: CI's reading is no node of its run (issue 161 note 3).
/// A run that concluded `failure` fails the verdict, named by the forge's word.
#[test]
fn the_merge_job_reads_each_ci_check_from_the_results_file_and_the_merge_reads_covered() {
    let r = reading();
    let ci = &r.f.ci;
    let file = results(ci, &r.second, "main", Some("success"));
    let read = ["--results", file.as_str(), "--branch", "main"];
    let ran = r.job(&[&read[..], &["--json"]].concat());
    let v: Value =
        serde_json::from_str(&ran.stdout).unwrap_or_else(|e| panic!("{e}: {}", ran.stderr));
    assert_eq!(ran.code, 0, "{v}{}", ran.stderr);
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    let blob = ci.git(&["rev-parse", &format!("{}:{WORKFLOW}", r.second)]);
    let want = serde_json::json!([{
        "id": "e2e",
        "outcome": "pass",
        "observed": {
            "forge": "github", "workflow": WORKFLOW, "blob": sha(&blob),
            "commit": sha(&r.second), "run": 1234, "attempt": 2, "conclusion": "success",
        },
    }]);
    assert_eq!(v["verdict"]["observed"], want, "{v}");
    assert_eq!(v["verdict"]["unread"], serde_json::json!([]), "{v}");
    let root = read_root(ci, &r.second);
    assert_eq!(serde_json::to_value(root.v).unwrap(), 3);
    assert_eq!((root.observed.len(), root.unread.len()), (1, 0));
    assert_eq!((root.counts.checks, root.counts.pass), (3, 3));
    let text = r.job(&read).stdout;
    assert!(text.contains("3 of 3 checks pass"), "{text}");
    let line = format!("     e2e pass  CI concluded `success`: {WORKFLOW}, run 1234 attempt 2\n");
    assert!(text.contains(&line), "{text}");
    let replayed = jtac(ci, &["replay", &r.second]);
    assert_eq!(replayed.code, 0, "{}{}", replayed.stdout, replayed.stderr);
    assert_eq!(r.state(&r.second), ("covered".into(), "covered".into()));
    let v = r.f.audit();
    let pr = row(&v, &r.b);
    assert_eq!(
        (&pr["state"], &pr["via"], &pr["by"]),
        (&"covered".into(), &"covers".into(), &sha(&r.second).into()),
        "{v}"
    );
    assert_eq!(row(&v, &r.a)["state"], "absent", "{v}");

    r.f.dev.git(&["fetch", "-q", "origin"]);
    r.f.dev.git(&["switch", "-q", "-c", "c", "origin/main"]);
    r.f.dev.write("c.txt", b"c\n");
    let c = r.f.dev.commit("c");
    ok(&r.f.dev, &["check", "--base", "origin/main"]);
    r.f.push("origin", "c", true);
    r.f.fetch("origin");
    let third = r.f.merge("origin/c");
    ci.git(&["push", "-q", "origin", "main"]);
    let file = results(ci, &third, "main", Some("failure"));
    let ran = jtac(
        ci,
        &[
            "check",
            "--from",
            &c,
            "--base",
            &r.second,
            "--push",
            "origin",
            "--results",
            &file,
            "--branch",
            "main",
        ],
    );
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    let what = format!("check `e2e` fail: CI concluded `failure` ({WORKFLOW}, run 1234 attempt 2)");
    assert!(ran.stdout.contains(&what), "{}", ran.stdout);
    let waive = format!("`jtac waive {third} --ci --reason <why>`");
    assert!(ran.stdout.contains(&waive), "{}", ran.stdout);
    assert_eq!(r.state(&third), ("absent".into(), "failed".into()));
}

/// Decision 65: a head that is itself a forge's merge holds a CI root with results read
/// for its own commit, which no later job can read again, so neither gate says it agreed
/// and neither fails on it: the pull request job passes, and the merge root of the branch
/// it lands on records `unverifiable`. Under `--require-local-root` it fails as a missing
/// root does, and the next step never deletes the merge job's root.
#[test]
fn a_head_root_that_read_ci_for_its_own_commit_compares_unverifiable_at_both_gates() {
    let r = reading();
    let ci = &r.f.ci;
    let file = results(ci, &r.second, "main", Some("success"));
    let ran = r.job(&["--results", &file, "--branch", "main"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    ci.git(&["checkout", "-q", "--detach", &r.second]);
    let (code, v, err) = json(ci, &["ci", "--base", &r.first, "--json"]);
    assert_eq!(code, 0, "{v}{err}");
    let head = &v["head_root"];
    assert_eq!(
        (&head["compared"], &head["trust"]),
        (&"unverifiable".into(), &"trusted".into()),
        "{v}"
    );
    assert!(
        head["why"]
            .as_str()
            .unwrap()
            .contains("results read from CI for its own commit"),
        "{v}"
    );
    let flagged = ["ci", "--base", &r.first, "--require-local-root", "--json"];
    let (code, v, err) = json(ci, &flagged);
    assert_eq!(code, 1, "{v}{err}");
    let fix = v["reasons"][0]["how"].as_str().unwrap();
    let run = format!(
        "Run `jtac ci --base {}` without `--require-local-root`",
        r.first
    );
    assert!(fix.starts_with(&run), "{fix}");
    assert!(
        fix.contains(". Do not delete refs/jtac/roots/") && !fix.contains("update-ref"),
        "{fix}"
    );
    ci.git(&["checkout", "-q", "-B", "release", &r.first]);
    ci.git(&["merge", "-q", "--no-ff", "-m", "promote", &r.second]);
    let landed = ci.git(&["rev-parse", "HEAD"]);
    ci.git(&["push", "-q", "origin", "release"]);
    let file = results(ci, &landed, "release", Some("success"));
    let (code, v, err) = json(
        ci,
        &[
            "check",
            "--from",
            &r.second,
            "--base",
            &r.first,
            "--push",
            "origin",
            "--json",
            "--results",
            &file,
            "--branch",
            "release",
        ],
    );
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["from"]["compared"], "unverifiable", "{v}");
    let root = read_root(ci, &landed);
    assert_eq!(
        (root.observed.len(), root.from.map(|f| f.head)),
        (1, Some(g(&r.second)))
    );
}

/// Decision 65: under `--require-local-root` an unverifiable root is told to leave its ref
/// alone only when a trusted CI key signed it. One the developer's key signed with a `from`
/// is nobody's merge root, and is told to write its root again, as before.
#[test]
fn an_unverifiable_root_is_a_merge_job_s_only_when_a_trusted_ci_key_signed_it() {
    let f = forge();
    let tip = f.feature("feat", 1).pop().unwrap();
    f.push("origin", "feat", true);
    f.fetch("origin");
    let merge = f.merge(&tip);
    let (code, v, err) = f.job(&tip, &f.main);
    assert_eq!(code, 0, "{v}{err}");
    let written = read_root(&f.ci, &merge);
    let mut old = written.clone();
    old.gate.version = "0.0.1".into();
    let mut far = written.clone();
    far.from.as_mut().unwrap().head = g("abababababababababababababababababababab");
    let by_dev = |root: &Root| {
        let mut root = root.clone();
        root.signer = read_root(&f.dev, &tip).signer;
        root
    };
    let (old_dev, far_dev) = (by_dev(&old), by_dev(&far));
    let keep = format!(". Do not delete {} or run `jtac check`", root_ref(&merge));
    let drop = format!(
        "Run `jtac ci --base {}` without `--require-local-root`",
        f.main
    );
    let again = "delete the ref with `git update-ref -d ";
    let fetch = format!(
        "run `jtac ci --base {} --require-local-root` again{keep}",
        f.main
    );
    for (root, key, trust, starts, holds, lacks) in [
        (
            &old,
            &f.ci_key,
            "trusted",
            drop.as_str(),
            keep.as_str(),
            again,
        ),
        (
            &far,
            &f.ci_key,
            "trusted",
            "Fetch the history",
            &fetch,
            again,
        ),
        (
            &old_dev,
            &f.dev_key,
            "unknown",
            "With jtac ",
            again,
            "Do not",
        ),
        (
            &far_dev,
            &f.dev_key,
            "unknown",
            "With jtac ",
            again,
            "Do not",
        ),
    ] {
        put(&f.ci, key, &merge, root);
        let flagged = ["ci", "--base", &f.main, "--require-local-root", "--json"];
        let (code, v, err) = json(&f.ci, &flagged);
        assert_eq!(code, 1, "{v}{err}");
        let head = &v["head_root"];
        assert_eq!(
            (&head["compared"], &head["trust"]),
            (&"unverifiable".into(), &trust.into()),
            "{v}"
        );
        let how = v["reasons"][0]["how"].as_str().unwrap();
        assert!(how.starts_with(starts), "{trust}: {how}");
        assert!(
            how.contains(holds) && !how.contains(lacks),
            "{trust}: {how}"
        );
    }
}

/// DESIGN.md section 15.2: a root's `observed` is the checks read from CI among its nodes,
/// so a merge root signed without them does not replay, though its evidence, counts and
/// state all recompute.
#[test]
fn replay_holds_a_merge_root_s_observed_to_the_checks_read_from_ci_among_its_nodes() {
    let r = reading();
    let ci = &r.f.ci;
    let file = results(ci, &r.second, "main", Some("success"));
    let ran = r.job(&["--results", &file, "--branch", "main"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let real = read_root(ci, &r.second);
    let hex = |root: &Root| root.id().unwrap().to_string().replace("sha256:", "");
    let store = ci.path().join(".git/jtac/roots");
    let nodes = std::fs::read(store.join(hex(&real)).join("nodes.json")).unwrap();
    let silent = Root {
        v: serde_json::from_value(2.into()).unwrap(),
        observed: Vec::new(),
        ..real
    };
    put(ci, &r.f.ci_key, &r.second, &silent);
    std::fs::create_dir_all(store.join(hex(&silent))).unwrap();
    std::fs::write(store.join(hex(&silent)).join("nodes.json"), nodes).unwrap();
    let (code, v, err) = json(ci, &["replay", &r.second, "--json"]);
    assert_eq!(code, 1, "{v}{err}");
    for part in ["evidence", "counts", "state"] {
        assert_eq!(v[part]["recorded"], v[part]["recomputed"], "{part}: {v}");
    }
    let want = "the root's `observed` is not the checks read from CI among its nodes";
    assert_eq!(v["mismatches"], serde_json::json!([want]), "{v}");
}

const E2E_JOB: &str = r#"{"name":"e2e","conclusion":"failure"}"#;

impl Reading {
    /// The merge job on `second` with a results file that concluded `word`, as JSON.
    fn concluded(&self, word: &str, jobs: &str, push: bool) -> (i32, Value, String) {
        let file = results_of(&self.f.ci, &self.second, "main", Some(word), jobs);
        let flags = ["check", "--from", &self.b, "--base", &self.first];
        let read = ["--results", file.as_str(), "--branch", "main", "--json"];
        let push = match push {
            true => &["--push", "origin"][..],
            false => &[],
        };
        json(&self.f.ci, &[&flags[..], &read[..], push].concat())
    }
}

/// DESIGN.md section 15.2, M20's done-when: a merge whose listed job failed gets a failure
/// record, the root the job would have written with the state `failed`, and a second clone
/// reads the merge `failed` with the job and the forge's word, though its tree is its
/// head's and the head holds a developer root. `release` refuses it with and without
/// `--require-ci` and says it failed. A re-run that fails again keeps the record.
#[test]
fn a_merge_whose_listed_job_failed_reads_failed_with_the_job_named_and_release_refuses_it() {
    let r = reading_with(", job = \"e2e\"");
    let ci = &r.f.ci;
    let (code, v, err) = r.concluded("failure", E2E_JOB, true);
    assert_eq!(code, 1, "{v}{err}");
    assert_eq!(v["verdict"]["root"], Value::Null, "{v}");
    assert_eq!(v["from"]["compared"], "agreed", "{v}");
    let name = failed_ref(&r.second);
    assert!(v["pushed"].to_string().contains(&name), "{v}");
    assert!(err.contains("sent 1 failure record(s) to origin"), "{err}");
    let record = read_record(ci, &r.second);
    assert_eq!(v["record"], record.id().unwrap().to_string(), "{v}");
    assert_eq!(serde_json::to_value(record.v).unwrap(), 3);
    assert_eq!(serde_json::to_value(record.state).unwrap(), "failed");
    assert_eq!(record.covers, Covers::List(vec![g(&r.b), g(&r.second)]));
    assert_eq!(record.from.as_ref().unwrap().head, g(&r.b));
    assert_eq!(record.signer.principal, "ci@example.com");
    assert_eq!((record.observed.len(), record.counts.pass), (1, 2));
    assert_eq!(ci.git(&["for-each-ref", &root_ref(&r.second)]), "");

    let second = r.f.second();
    let (code, v, err) = json(&second, &["audit", "main", "--json"]);
    assert_eq!(code, 0, "{v}{err}");
    let merge = row(&v, &r.second);
    assert_eq!(
        (&merge["state"], &merge["ci"]),
        (&"absent".into(), &"failed".into()),
        "{v}"
    );
    let shown = &merge["ci_record"];
    assert_eq!(shown["commit"], sha(&r.second), "{v}");
    assert_eq!(shown["observed"][0]["observed"]["job"], "e2e", "{v}");
    assert_eq!(
        shown["observed"][0]["observed"]["conclusion"], "failure",
        "{v}"
    );
    assert_eq!(row(&v, &r.b)["state"], "partial", "{v}");
    assert_eq!(row(&v, &r.b)["ci"], Value::Null, "{v}");
    assert_eq!(row(&v, &r.first)["ci"], "failed", "{v}");
    let text = ok(&second, &["audit", "main"]);
    let line = format!(
        "ci failed: its own failure record: check `e2e` fail, CI concluded `failure` (job e2e \
         of {WORKFLOW}, run 1234 attempt 2); checks 2/3 pass, findings 0 blocker"
    );
    assert!(text.contains(&line), "{text}");
    assert!(
        text.contains("its own failure record: checks 2/2 pass, findings 0 blocker 2 major"),
        "{text}"
    );

    second.git(&["tag", "-a", "-m", "t1", "t1", &r.second]);
    for flag in [None, Some("--require-ci")] {
        let args = ["release", "t1", "--from", &r.f.main, "--json"];
        let args: Vec<&str> = args.into_iter().chain(flag).collect();
        let (code, v, err) = json(&second, &args);
        assert_eq!(code, 1, "{v}{err}");
        let failed = v["failed"].to_string();
        assert!(
            failed.contains(&r.second) && failed.contains(&r.first),
            "{v}"
        );
        assert!(!v["absent"].to_string().contains(&r.second), "{v}");
    }
    let text = jtac(&second, &["release", "t1", "--from", &r.f.main]).stdout;
    let line = format!("  failed    {} CI's verdict on its merge failed", r.second);
    assert!(text.contains(&line), "{text}");

    let held = ci.git(&["rev-parse", &name]);
    let (code, v, err) = r.concluded("failure", E2E_JOB, true);
    assert_eq!(code, 1, "{v}{err}");
    let notes = v["verdict"]["notes"].to_string();
    assert!(
        notes.contains("already holds CI's own failure record"),
        "{notes}"
    );
    assert!(
        notes.contains("this run wrote no failure record"),
        "{notes}"
    );
    assert_eq!(v["record"], record.id().unwrap().to_string(), "{v}");
    assert_eq!(ci.git(&["rev-parse", &name]), held);
}

/// DESIGN.md section 15.2 and decision 66: a later attempt that passes writes the root as
/// usual, the record stays beside it, and the merge reads covered with the failure noted;
/// a re-run that fails after a root stands writes its record beside that root, and the
/// reading does not move.
#[test]
fn a_failed_merge_reads_covered_with_the_failure_noted_once_a_later_attempt_passes() {
    let r = reading();
    let ci = &r.f.ci;
    let (code, v, err) = r.concluded("failure", "", true);
    assert_eq!(code, 1, "{v}{err}");
    assert_eq!(r.state(&r.second), ("absent".into(), "failed".into()));
    let (code, v, err) = r.concluded("success", "", true);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["record"], Value::Null, "{v}");
    let pushed = v["pushed"].to_string();
    assert!(pushed.contains(&root_ref(&r.second)), "{v}");
    let v = r.f.audit();
    let merge = row(&v, &r.second);
    assert_eq!(
        (&merge["state"], &merge["ci"]),
        (&"covered".into(), &"covered".into()),
        "{v}"
    );
    assert_eq!(merge["ci_record"]["commit"], sha(&r.second), "{v}");
    assert_eq!(row(&v, &r.b)["state"], "covered", "{v}");
    let second = r.f.second();
    let text = ok(&second, &["audit", "main"]);
    let line = "ci note: another verdict failed: its own failure record: check `e2e` fail, CI \
                concluded `failure`";
    assert!(text.contains(line), "{text}");
    assert!(
        second
            .git(&["for-each-ref", "refs/jtac"])
            .contains(&failed_ref(&r.second))
    );

    r.f.dev.git(&["fetch", "-q", "origin"]);
    r.f.dev.git(&["switch", "-q", "-c", "c", "origin/main"]);
    r.f.dev.write("c.txt", b"c\n");
    let c = r.f.dev.commit("c");
    ok(&r.f.dev, &["check", "--base", "origin/main"]);
    r.f.push("origin", "c", true);
    r.f.fetch("origin");
    let third = r.f.merge("origin/c");
    ci.git(&["push", "-q", "origin", "main"]);
    let attempt = |word: &str| {
        let file = results(ci, &third, "main", Some(word));
        let flags = [
            "check", "--from", &c, "--base", &r.second, "--push", "origin",
        ];
        let read = ["--results", file.as_str(), "--branch", "main", "--json"];
        json(ci, &[&flags[..], &read[..]].concat())
    };
    let (code, v, err) = attempt("success");
    assert_eq!(code, 0, "{v}{err}");
    let root = read_root(ci, &third);
    let (code, v, err) = attempt("failure");
    assert_eq!(code, 1, "{v}{err}");
    assert!(v["pushed"].to_string().contains(&failed_ref(&third)), "{v}");
    assert_eq!(
        read_root(ci, &third),
        root,
        "the root that stood still stands"
    );
    let record = read_record(ci, &third);
    assert_eq!(record.parents, root.parents);
    let v = r.f.audit();
    let merge = row(&v, &third);
    assert_eq!(
        (&merge["state"], &merge["ci"]),
        (&"covered".into(), &"covered".into()),
        "{v}"
    );
    assert_eq!(
        merge["ci_record"]["record"],
        record.id().unwrap().to_string(),
        "{v}"
    );
}

/// DESIGN.md section 9.5: the verdict that failed judged the whole diff from the old tip,
/// so every commit of a failed rebase reads `failed` to the CI gate, the earlier ones by
/// the tip's record, and none reads absent to it.
#[test]
fn every_commit_of_a_failed_rebase_reads_failed() {
    let f = forge();
    f.dev.git(&["switch", "-q", "-c", "a", "main"]);
    f.dev.write("a.txt", b"a\n");
    f.dev.commit("a");
    let mut policy = std::fs::read(f.dev.path().join(".jtac/policy.toml")).unwrap();
    policy.extend_from_slice(b"# a change to a protected path\n");
    f.dev.write(".jtac/policy.toml", &policy);
    let head = f.dev.commit("touch the policy");
    f.push("origin", "a", false);
    let z = f.feature("z", 1);
    f.push("origin", "z", true);
    f.fetch("origin");
    let old = f.merge("origin/z");
    let (code, v, err) = f.job(&z[0], &f.main);
    assert_eq!(code, 0, "{v}{err}");
    f.fetch("origin");
    let tip = f.rebase("origin/a");
    let earlier = f.ci.git(&["rev-parse", "HEAD~1"]);
    assert_eq!(f.ci.git(&["rev-parse", "HEAD~2"]), old);
    let (code, v, err) = f.job(&head, &old);
    assert_eq!(code, 1, "{v}{err}");
    assert!(
        v["verdict"]["reasons"].to_string().contains("JT-PROT-001"),
        "{v}"
    );
    assert!(v["pushed"].to_string().contains(&failed_ref(&tip)), "{v}");
    let record = read_record(&f.ci, &tip);
    assert_eq!(
        serde_json::to_value(record.v).unwrap(),
        3,
        "version 3 under a version 1 policy"
    );
    assert!(record.observed.is_empty());
    let second = f.second();
    let (code, v, err) = json(&second, &["audit", "main", "--json"]);
    assert_eq!(code, 0, "{v}{err}");
    for commit in [&earlier, &tip] {
        let r = row(&v, commit);
        assert_eq!(
            (&r["state"], &r["ci"]),
            (&"absent".into(), &"failed".into()),
            "{v}"
        );
        assert_eq!(r["ci_record"]["commit"], sha(&tip), "{v}");
    }
    assert_eq!(row(&v, &old)["ci"], "covered", "{v}");
    let (code, v, err) = json(&second, &["audit", "main~1", "--json"]);
    assert_eq!(code, 0, "{v}{err}");
    let below = row(&v, &earlier);
    assert_eq!(
        (&below["ci"], &below["ci_record"]["commit"]),
        (&"failed".into(), &sha(&tip).into()),
        "a range below the record's own commit: {v}"
    );
    let text = ok(&second, &["audit", "main"]);
    let reaches = format!(
        "ci failed: {}'s failure record reaches it: checks",
        &tip[..12]
    );
    assert!(text.contains(&reaches), "{text}");
}

/// Decision 66: the job looks at a record on the tip only when its own verdict failed, and
/// then keeps it only as CI's own for this head. One a developer's key signed, one for
/// another head and a ref that holds no record each stop the job with the ref to delete;
/// a verdict that passes writes its root beside any of them; and no reader stops on one:
/// `audit` and `release` answer as they did, the record a note and the merge not `failed`.
#[test]
fn a_record_that_is_not_ci_s_own_stops_the_job_on_a_failed_verdict_and_no_other_command() {
    let r = reading();
    let ci = &r.f.ci;
    let name = failed_ref(&r.second);
    let (code, v, err) = r.concluded("failure", "", false);
    assert_eq!(code, 1, "{v}{err}");
    let own = read_record(ci, &r.second);
    let mut dev = own.clone();
    dev.signer = read_root(ci, &r.b).signer;
    let mut other = own.clone();
    other.from.as_mut().unwrap().head = g(&r.a);

    put_record(ci, &r.f.dev_key, &r.second, &dev);
    ci.git(&["tag", "-a", "-m", "t1", "t1", &r.second]);
    let readers = |note: &str| {
        let (code, v, err) = json(ci, &["audit", "main", "--json"]);
        assert_eq!(code, 0, "{note}: {v}{err}");
        assert_eq!(row(&v, &r.second)["ci"], Value::Null, "{note}: {v}");
        assert!(v["notes"].to_string().contains(note), "{note}: {v}");
        let (code, v, err) = json(ci, &["release", "t1", "--from", &r.first, "--json"]);
        assert_eq!(code, 1, "{note}: {v}{err}");
        assert!(!v["failed"].to_string().contains(&r.second), "{note}: {v}");
    };
    readers("failure record counts for nothing");
    ci.git(&["update-ref", &name, &r.second]);
    readers("holds no failure record and is not read");

    let refused = |why: &str| {
        let file = results(ci, &r.second, "main", Some("failure"));
        let ran = r.job(&["--results", &file, "--branch", "main"]);
        assert_eq!(
            (ran.code, ran.stdout.as_str()),
            (2, ""),
            "{why}: {}",
            ran.stderr
        );
        let what = "already carries a failure record this job did not write";
        assert!(ran.stderr.contains(what), "{why}: {}", ran.stderr);
        assert!(ran.stderr.contains(why), "{why}: {}", ran.stderr);
        let delete = format!("`git push origin :{name}`");
        assert!(ran.stderr.contains(&delete), "{why}: {}", ran.stderr);
        assert!(ran.stderr.contains("\nfence: "), "{why}: {}", ran.stderr);
        assert_eq!(ran.stderr.lines().count(), 3, "{why}: {}", ran.stderr);
    };
    refused("its ref does not read");
    put_record(ci, &r.f.dev_key, &r.second, &dev);
    refused("not named for namespace jtac-ci-root-v1");
    put_record(ci, &r.f.ci_key, &r.second, &other);
    refused("its `from` names head");
    assert_eq!(
        r.f.second()
            .git(&["for-each-ref", "refs/jtac/failed"])
            .matches(&r.second)
            .count(),
        0
    );

    let (code, v, err) = r.concluded("success", "", false);
    assert_eq!(code, 0, "a verdict that passes reads no record: {v}{err}");
    assert_eq!(
        read_record(ci, &r.second),
        other,
        "and leaves the ref as it found it"
    );
    refused("its `from` names head");
    put_record(ci, &r.f.ci_key, &r.second, &own);
    let (code, v, err) = r.concluded("failure", "", false);
    assert_eq!(code, 1, "{v}{err}");
    let notes = v["verdict"]["notes"].to_string();
    assert!(
        notes.contains("already holds CI's own failure record"),
        "{notes}"
    );
}

/// A record pushed onto the forge's commit between the job's fetch and its push is judged
/// as a root there is (`a_root_pushed_onto_the_tip_while_the_job_runs_is_judged_as_standing`):
/// a developer's is the fence, with the ref to delete, and a concurrent run's own is kept.
#[test]
fn a_record_pushed_onto_the_tip_while_the_job_runs_is_judged_as_standing() {
    let r = reading();
    let ci = &r.f.ci;
    let name = failed_ref(&r.second);
    let (code, v, err) = r.concluded("failure", "", false);
    assert_eq!(code, 1, "{v}{err}");
    let mut forged = read_record(ci, &r.second);
    forged.signer = read_root(ci, &r.b).signer;
    r.f.dev.git(&["fetch", "-q", "origin"]);
    put_record(&r.f.dev, &r.f.dev_key, &r.second, &forged);
    let spec = format!("{name}:{name}");
    r.f.dev.git(&["push", "-q", "origin", &spec]);
    ci.git(&["update-ref", "-d", &name]);
    let file = results(ci, &r.second, "main", Some("failure"));
    let ran = r.job(&["--results", &file, "--branch", "main"]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr
            .contains("not named for namespace jtac-ci-root-v1")
            && ran
                .stderr
                .contains("origin holds it, pushed after this run fetched")
            && ran.stderr.contains(&format!("`git push origin :{name}`")),
        "{}",
        ran.stderr
    );
    r.f.dev.git(&["push", "-q", "origin", &format!(":{name}")]);
    ci.git(&["update-ref", "-d", &name]);
    let (code, v, err) = r.concluded("failure", "", true);
    assert_eq!(code, 1, "{v}{err}");
    assert!(v["pushed"].to_string().contains(&name), "{v}");
    let first = read_record(ci, &r.second);
    ci.git(&["update-ref", "-d", &name]);
    let (code, v, err) = r.concluded("failure", "", true);
    assert_eq!(code, 1, "{v}{err}");
    let notes = v["verdict"]["notes"].to_string();
    let kept = "origin already holds CI's own failure record for this head";
    assert!(notes.contains(kept), "{notes}");
    assert!(
        err.contains("WARNING: the failure record did not reach origin")
            && !err.contains("the roots did not reach"),
        "{err}"
    );
    assert!(!v["pushed"].to_string().contains(&name), "{v}");
    assert_eq!(read_record(&r.f.second(), &r.second), first);
}

/// Decision 66, issue 161 note 13: a tip whose first-parent line holds a commit of the pull
/// request's own, where a document with `from` would reach it. Three shapes: the head as
/// the first parent, an earlier commit of the pull request as the first parent with the
/// head second, and that commit as the only parent. The job refuses each for every head
/// that holds the line's first commit, names `jtac ci`, which judges the push and writes
/// nothing, and leaves no ref behind. A head ahead of HEAD holds HEAD itself, the line's
/// first commit when the old tip is HEAD's parent, and is refused the same way.
#[test]
fn the_merge_job_refuses_a_head_on_its_tip_s_first_parent_line() {
    let f = forge();
    let a = f.feature("a", 2);
    f.push("origin", "a", true);
    f.fetch("origin");
    f.at_main();
    let tree = f.ci.git(&["rev-parse", &format!("{}^{{tree}}", a[1])]);
    let made = ["commit-tree", "--no-gpg-sign", "-m", "merge", tree.as_str()];
    let (head, first, main) = (a[1].as_str(), a[0].as_str(), f.main.as_str());
    let shapes: [(&[&str], &[&str]); 3] = [
        (&["-p", head, "-p", main], &[first, head]),
        (&["-p", first, "-p", head], &[head]),
        (&["-p", first], &[head]),
    ];
    for (parents, heads) in shapes {
        let tip = f.ci.git(&[&made[..], parents].concat());
        f.ci.git(&["reset", "-q", "--hard", &tip]);
        for from in heads {
            let flags = ["check", "--from", from, "--base", main, "--push", "origin"];
            let ran = jtac(&f.ci, &flags);
            assert_eq!((ran.code, ran.stdout.as_str()), (2, ""), "{}", ran.stderr);
            let what = format!(
                "jtac: `--from {from}` holds {}, the first commit above the old tip on HEAD's \
                 first-parent line",
                &first[..12]
            );
            assert!(ran.stderr.starts_with(&what), "{}", ran.stderr);
            let how = format!("negotiation: Run `jtac ci --base {main}` on the tip instead");
            assert!(ran.stderr.contains(&how), "{}", ran.stderr);
            assert_eq!(ran.stderr.lines().count(), 3, "{}", ran.stderr);
        }
        let ran = jtac(&f.ci, &["ci", "--base", main]);
        assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
        assert!(!f.ci.git(&["for-each-ref", "refs/jtac"]).contains(&tip));
    }
    let tip = f.ci.git(&[&made[..], &["-p", main]].concat());
    f.ci.git(&["reset", "-q", "--hard", &tip]);
    let ahead = f.ci.git(&[&made[..], &["-p", tip.as_str()]].concat());
    let ran = jtac(&f.ci, &["check", "--from", &ahead, "--base", main]);
    let what = format!("jtac: `--from {ahead}` holds {}, the first", &tip[..12]);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(ran.stderr.starts_with(&what), "{}", ran.stderr);
}

/// Issue 177: `replay --record` recomputes a failure record as `replay` does a root and
/// holds it when the verdict fails again. Without the flag a commit that holds a record
/// and no root is refused, naming it, so a failed merge never answers 0; one with both
/// replays the root and names `--record`. `show` prints the record first, apart from what
/// a push sends. No record stops either: a second clone is told the nodes are not local, a
/// record signed over a verdict that passes is a mismatch, and a ref that holds none is a
/// note beside a root and nothing beside none.
#[test]
fn replay_and_show_read_a_failure_record_and_no_record_stops_either() {
    let r = reading();
    let ci = &r.f.ci;
    let name = failed_ref(&r.second);
    let (code, v, err) = r.concluded("failure", "", true);
    assert_eq!(code, 1, "{v}{err}");
    let record = read_record(ci, &r.second);
    let ran = jtac(ci, &["replay", &r.second]);
    let what = format!(
        "jtac: {} has no root in this clone, and holds a failure record.",
        r.second
    );
    assert_eq!((ran.code, ran.stdout.as_str()), (2, ""), "{}", ran.stderr);
    assert!(ran.stderr.starts_with(&what), "{}", ran.stderr);
    let how = format!("Run `jtac replay {} --record` to replay", r.second);
    assert!(ran.stderr.contains(&how), "{}", ran.stderr);
    let (code, v, err) = json(ci, &["replay", &r.second, "--record", "--json"]);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["document"], "failure record", "{v}");
    assert_eq!(v["root"], record.id().unwrap().to_string(), "{v}");
    assert_eq!(v["state"]["recorded"], "failed", "{v}");
    assert_eq!(v["state"]["recomputed"], "failed", "{v}");
    assert_eq!((&v["pass"], &v["matches"]), (&false.into(), &true.into()));
    let notes = v["notes"].to_string();
    assert!(notes.contains("it failed on: check `e2e` is Fail"), "{v}");
    assert!(!notes.contains("also holds a failure record"), "{v}");
    let text = ok(ci, &["replay", &r.second, "--record"]);
    let line = format!("REPLAYED: {}'s failure record replays to ABORT ", r.second);
    assert!(text.starts_with(&line), "{text}");

    let span = format!("{}..{}", r.first, r.second);
    let (code, v, err) = json(ci, &["show", "--json", &span]);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["records"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["records"][0]["name"], name.as_str(), "{v}");
    let bytes = ci.git(&["cat-file", "blob", &format!("{name}:root.json")]);
    assert_eq!(v["records"][0]["root_json"], bytes.as_str(), "{v}");
    assert!(!v["roots"].to_string().contains(&name), "{v}");
    let text = ok(ci, &["show", &span]);
    let first = text.lines().next().unwrap();
    assert!(
        first.ends_with("; 1 failure records are held and not sent"),
        "{text}"
    );
    let at = |line: &str| text.find(line).unwrap_or(usize::MAX);
    assert!(at(&format!("\nrecord {name} ")) < at("\nref "), "{text}");

    let ran = jtac(&r.f.second(), &["replay", &r.second, "--record"]);
    let what = format!(
        "jtac: the failure record for {} verifies, but its nodes are not in this clone's store.",
        r.second
    );
    assert_eq!((ran.code, ran.stdout.as_str()), (2, ""), "{}", ran.stderr);
    assert!(ran.stderr.starts_with(&what), "{}", ran.stderr);
    let ran = jtac(ci, &["replay", &r.b, "--record"]);
    let what = format!("jtac: {} has no failure record in this clone.", r.b);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(ran.stderr.starts_with(&what), "{}", ran.stderr);

    let (code, v, err) = r.concluded("success", "", true);
    assert_eq!(code, 0, "{v}{err}");
    let (code, v, err) = json(ci, &["replay", &r.second, "--json"]);
    assert_eq!((code, &v["document"]), (0, &"root".into()), "{v}{err}");
    let also = format!(
        "also holds a failure record at {name}; `jtac replay {} --record` replays it",
        r.second
    );
    assert!(v["notes"].to_string().contains(&also), "{v}");
    let (code, v, err) = json(ci, &["replay", &r.second, "--record", "--json"]);
    assert_eq!(code, 0, "{v}{err}");
    assert_eq!(v["document"], "failure record", "{v}");

    let real = read_root(ci, &r.second);
    let hex = |root: &Root| root.id().unwrap().to_string().replace("sha256:", "");
    let store = ci.path().join(".git/jtac/roots");
    let nodes = std::fs::read(store.join(hex(&real)).join("nodes.json")).unwrap();
    let forged = Root {
        state: serde_json::from_value("failed".into()).unwrap(),
        ..real
    };
    put_record(ci, &r.f.ci_key, &r.second, &forged);
    std::fs::create_dir_all(store.join(hex(&forged))).unwrap();
    std::fs::write(store.join(hex(&forged)).join("nodes.json"), nodes).unwrap();
    let (code, v, err) = json(ci, &["replay", &r.second, "--record", "--json"]);
    assert_eq!(code, 1, "{v}{err}");
    let want = [
        "the state differs",
        "the verdict is CLEARED HOT: a failure record holds a verdict that failed",
    ];
    assert_eq!(v["mismatches"], serde_json::json!(want), "{v}");

    ci.git(&["update-ref", &name, &r.second]);
    let (code, v, err) = json(ci, &["replay", &r.second, "--json"]);
    assert_eq!((code, &v["document"]), (0, &"root".into()), "{v}{err}");
    let junk = format!("{name} holds no failure record and is not read");
    assert!(v["notes"].to_string().contains(&junk), "{v}");
    let (code, v, err) = json(ci, &["show", "--json", &span]);
    assert_eq!(code, 0, "{v}{err}");
    let shown = &v["records"][0];
    assert_eq!(shown["root_json"], "", "{v}");
    let note = shown["note"].as_str().unwrap_or_default();
    assert!(note.starts_with("it does not hold a failure record"), "{v}");
    ci.git(&["update-ref", &failed_ref(&r.first), &r.second]);
    let ran = jtac(ci, &["replay", &r.first]);
    let what = format!("jtac: {} has no root in this clone.", r.first);
    assert_eq!(ran.code, 2, "{}", ran.stderr);
    assert!(ran.stderr.starts_with(&what), "{}", ran.stderr);
}
