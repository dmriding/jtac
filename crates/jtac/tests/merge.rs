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

fn read_root(repo: &Repo, commit: &str) -> Root {
    let out = repo.command(
        "git",
        &[
            "cat-file",
            "blob",
            &format!("{}:root.json", root_ref(commit)),
        ],
    );
    assert!(out.status.success(), "{commit} has no root");
    serde_json::from_slice(&out.stdout).unwrap()
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
    let signers = format!(
        "{}{}",
        signers_line(&dev_key, "dev@example.com", ""),
        signers_line(&ci_key, "ci@example.com", CI_OPTION)
    );
    dev.write(".jtac/signers", signers.as_bytes());
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
    assert_eq!(read_root(&f.ci, &merge).signer.principal, "ci@example.com");
}
