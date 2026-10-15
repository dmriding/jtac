//! `jtac verify`, `replay`, `audit`, and `show` on a scripted history (DESIGN.md section
//! 11.9, M11): own roots, `covers` entries, a tree match, absence, each way a root is
//! invalid, a clone that holds roots but no nodes, replay mismatches, byte-exact `show`, and
//! trust read from HEAD rather than the work tree.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jtac_testkit::Repo;
use serde_json::Value;

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").replace('\\', "/")
}

fn with_policy(repo: &Repo) {
    let policy = format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n",
        me()
    );
    repo.write(".jtac/policy.toml", policy.as_bytes());
    repo.write(".gitignore", b"*.profraw\n");
}

/// A key under `.git/`, named in the local config with the principal.
fn keyed(repo: &Repo) -> PathBuf {
    let key = repo.path().join(".git").join("key");
    let out = Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "dev@example.com",
            "-f",
        ])
        .arg(&key)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = key.to_string_lossy().replace('\\', "/");
    repo.git(&["config", "jtac.signingkey", &path]);
    repo.git(&["config", "user.email", "dev@example.com"]);
    key
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

fn json(repo: &Repo, args: &[&str]) -> (i32, Value) {
    let ran = jtac(repo, args);
    assert!(ran.stderr.is_empty(), "{}", ran.stderr);
    (ran.code, serde_json::from_str(&ran.stdout).unwrap())
}

fn root_ref(commit: &str) -> String {
    format!("refs/jtac/roots/{}/{commit}", &commit[..2])
}

fn blob(repo: &Repo, spec: &str) -> String {
    let out = repo.command("git", &["cat-file", "blob", spec]);
    assert!(out.status.success(), "{spec}");
    String::from_utf8(out.stdout).unwrap()
}

/// `base` pushed to origin's `main`; `one` with a root; `two` without; `three` with a root
/// covering `two`; `twin` is `three`'s tree on `base`; `side` is `base`'s tree on `base`.
struct History {
    origin: Repo,
    repo: Repo,
    key: PathBuf,
    base: String,
    one: String,
    two: String,
    three: String,
    twin: String,
    side: String,
}

fn history() -> History {
    let origin = Repo::bare(&[]);
    let repo = Repo::clone_of(&origin);
    with_policy(&repo);
    repo.write("src/lib.rs", b"fn a() {}\n");
    let base = repo.commit("base");
    repo.git(&["push", "-q", "-u", "origin", "main"]);
    let key = keyed(&repo);
    repo.write("a.txt", b"a\n");
    let one = repo.commit("one");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    repo.write("b.txt", b"b\n");
    let two = repo.commit("two");
    repo.write("c.txt", b"c\n");
    let three = repo.commit("three");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let twin = repo.git(&[
        "commit-tree",
        &format!("{three}^{{tree}}"),
        "-p",
        &base,
        "-m",
        "twin",
    ]);
    repo.git(&["branch", "twin", &twin]);
    let side = repo.git(&[
        "commit-tree",
        &format!("{base}^{{tree}}"),
        "-p",
        &base,
        "-m",
        "side",
    ]);
    repo.git(&["branch", "side", &side]);
    History {
        origin,
        repo,
        key,
        base,
        one,
        two,
        three,
        twin,
        side,
    }
}

/// Rows by commit hex: `(state, via, by)`.
fn rows(v: &Value) -> Vec<(String, String, String, String)> {
    v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let s = |k: &str| {
                r[k].as_str()
                    .map(|s| s.trim_start_matches("sha1:").to_owned())
                    .unwrap_or_default()
            };
            (s("commit"), s("state"), s("via"), s("by"))
        })
        .collect()
}

#[test]
fn audit_assigns_own_covers_tree_and_absent_and_verify_fails_only_when_told_to() {
    let h = history();
    let (code, v) = json(&h.repo, &["audit", "--json"]);
    assert_eq!(code, 0);
    let own = |c: &str| {
        (
            c.to_owned(),
            "covered".to_owned(),
            "own".to_owned(),
            String::new(),
        )
    };
    assert_eq!(
        rows(&v),
        [
            own(&h.three),
            (
                h.two.clone(),
                "covered".into(),
                "covers".into(),
                h.three.clone()
            ),
            own(&h.one),
            (
                h.base.clone(),
                "absent".into(),
                String::new(),
                String::new()
            ),
        ]
    );
    let t = &v["totals"];
    assert_eq!(
        (
            t["commits"].as_u64(),
            t["covered"].as_u64(),
            t["absent"].as_u64()
        ),
        (Some(4), Some(3), Some(1))
    );
    assert_eq!(t["percent_covered"], 75);
    assert_eq!(t["invalid_roots"], 0);
    assert_eq!(t["unknown_signers"], 2);
    assert_eq!(v["rows"][1]["root"], v["rows"][0]["root"]);
    assert!(v["rows"][3]["root"].is_null());
    let (code, v) = json(&h.repo, &["audit", "--json", "twin"]);
    assert_eq!(code, 0);
    assert_eq!(
        rows(&v)[0],
        (
            h.twin.clone(),
            "covered".into(),
            "tree".into(),
            h.three.clone()
        )
    );
    assert_eq!(v["totals"]["by_tree"], 1);
    let ran = jtac(&h.repo, &["audit", "twin"]);
    assert!(
        ran.stdout
            .starts_with("AUDIT twin: 2 commits, 1 covered, 0 declared off, 1 absent"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("by tree, "), "{}", ran.stdout);
    let ran = jtac(&h.repo, &["verify"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stdout.starts_with("VERIFIED HEAD: 4 commits"),
        "{}",
        ran.stdout
    );
    let (code, v) = json(&h.repo, &["verify", "--json", "--require-coverage"]);
    assert_eq!(code, 1);
    assert_eq!(v["pass"], false);
    assert_eq!(
        v["absent_commits"],
        serde_json::json!([format!("sha1:{}", h.base)])
    );
    assert_eq!(v["roots"], 2);
    let ran = jtac(&h.repo, &["verify", "--require-coverage"]);
    assert!(ran.stdout.starts_with("INVALID HEAD"), "{}", ran.stdout);
    assert!(
        ran.stdout.contains(&format!("absent  {}", h.base)) && ran.stdout.contains("`jtac check`"),
        "{}",
        ran.stdout
    );
}

fn git_stdin(repo: &Repo, args: &[&str], stdin: &[u8]) -> String {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo.path())
        .env_clear()
        .envs(repo.env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Replace `commit`'s root ref with `json` and `sig`, or a fresh signature when `None`.
fn replace_root(repo: &Repo, key: &Path, commit: &str, json: &str, sig: Option<&str>) {
    let json_path = repo.write(".git/forged/root.json", json.as_bytes());
    let sig = match sig {
        Some(s) => s.to_owned(),
        None => {
            let sig_path = json_path.with_extension("json.sig");
            let _ = std::fs::remove_file(&sig_path);
            let out = Command::new("ssh-keygen")
                .args(["-Y", "sign", "-n", "jtac-root-v1", "-P", "", "-f"])
                .arg(key)
                .arg(&json_path)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            std::fs::read_to_string(sig_path).unwrap()
        }
    };
    let sig_path = repo.write(".git/forged/root.sig", sig.as_bytes());
    let j = repo.git(&["hash-object", "-w", "--", json_path.to_str().unwrap()]);
    let s = repo.git(&["hash-object", "-w", "--", sig_path.to_str().unwrap()]);
    let listing = format!("100644 blob {j}\troot.json\n100644 blob {s}\troot.sig\n");
    let tree = git_stdin(repo, &["mktree"], listing.as_bytes());
    let forged = repo.git(&["commit-tree", &tree, "-m", "forged"]);
    repo.git(&["update-ref", &root_ref(commit), &forged]);
}

#[test]
fn a_tampered_root_is_invalid_for_the_reason_verify_names_and_its_commits_read_absent() {
    let h = history();
    let name = root_ref(&h.three);
    let kept = h.repo.git(&["rev-parse", &name]);
    let original = blob(&h.repo, &format!("{name}:root.json"));
    let base_tree = h.repo.git(&["rev-parse", &format!("{}^{{tree}}", h.base)]);
    let parent_id = original
        .split("\"parents\":[{\"commit\":\"sha1:")
        .nth(1)
        .and_then(|rest| rest.split("\"root\":\"sha256:").nth(1))
        .map(|rest| rest[..64].to_owned())
        .unwrap();
    let zeros = "0".repeat(64);
    let cases: [(String, Option<&str>, String); 4] = [
        (
            original.clone(),
            Some("not a signature\n"),
            "its signature does not hold".into(),
        ),
        (
            original.replace(
                "\"covers\":[\"",
                &format!("\"covers\":[\"sha1:{}\",\"", h.side),
            ),
            None,
            format!(
                "it covers {}, which is not between its parents and its tip",
                h.side
            ),
        ),
        (
            original.replace(&parent_id, &zeros),
            None,
            format!(
                "it names sha256:{zeros} as the root of parent {}, and this clone holds sha256:{parent_id}",
                h.one
            ),
        ),
        (
            original.replace(
                &format!(
                    "\"tree\":\"sha1:{}\"",
                    h.repo.git(&["rev-parse", "HEAD^{tree}"])
                ),
                &format!("\"tree\":\"sha1:{base_tree}\""),
            ),
            None,
            format!("it names tree sha1:{base_tree}, not the commit's"),
        ),
    ];
    for (forged, sig, want) in &cases {
        assert!(
            sig.is_some() || *forged != original,
            "{want}: the edit did not apply"
        );
        replace_root(&h.repo, &h.key, &h.three, forged, *sig);
        let (code, v) = json(&h.repo, &["verify", "--json"]);
        assert_eq!(code, 1, "{want}");
        assert_eq!(
            v["invalid"].as_array().map(Vec::len),
            Some(1),
            "{want}: {v}"
        );
        assert_eq!(v["invalid"][0]["commit"], format!("sha1:{}", h.three));
        let why = v["invalid"][0]["why"].as_str().unwrap();
        assert!(why.contains(want.as_str()), "{why}");
        let (_, v) = json(&h.repo, &["audit", "--json"]);
        let got = rows(&v);
        assert_eq!(&got[0].1, "absent", "{want}: {got:?}");
        assert_eq!(&got[1].1, "absent", "{want}: {got:?}");
        assert!(
            v["rows"][0]["invalid"]
                .as_str()
                .unwrap()
                .contains(want.as_str()),
            "{v}"
        );
        assert_eq!(v["totals"]["invalid_roots"], 1);
        let ran = jtac(&h.repo, &["verify"]);
        assert!(
            ran.stdout.contains(&format!("invalid {} {}", h.three, why)),
            "{}",
            ran.stdout
        );
        h.repo.git(&["update-ref", &name, &kept]);
    }
    assert_eq!(jtac(&h.repo, &["verify"]).code, 0);
}

#[test]
fn a_missing_parent_root_is_a_note_and_a_clone_with_roots_but_no_nodes_cannot_replay() {
    let h = history();
    h.repo
        .git(&["push", "-q", "origin", "main", "+refs/jtac/*:refs/jtac/*"]);
    let other = Repo::clone_of(&h.origin);
    other.git(&["fetch", "-q", "origin", "+refs/jtac/*:refs/jtac/*"]);
    let (code, v) = json(&other, &["verify", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        (v["roots"].as_u64(), v["covered"].as_u64()),
        (Some(2), Some(3))
    );
    let ran = jtac(&other, &["replay", &h.three]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr
            .contains("verifies, but its nodes are not in this clone's store")
            && ran
                .stderr
                .contains(&format!("negotiation: Run `jtac verify {}`", h.three)),
        "{}",
        ran.stderr
    );
    let ran = jtac(&other, &["replay", &h.base]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.contains("has no root in this clone")
            && ran.stderr.contains(&format!("`jtac audit {}`", h.base)),
        "{}",
        ran.stderr
    );
    h.repo.git(&["update-ref", "-d", &root_ref(&h.one)]);
    let (code, v) = json(&h.repo, &["verify", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["invalid"].as_array().map(Vec::len), Some(0));
    let note = format!(
        "{}'s root links to a root for {} that this clone does not hold",
        h.three, h.one
    );
    assert_eq!(v["notes"], serde_json::json!([note]));
    let (_, v) = json(&h.repo, &["audit", "--json"]);
    assert_eq!(rows(&v)[2].1, "absent");
    let ran = jtac(&h.repo, &["audit"]);
    assert!(
        ran.stdout.contains(&format!("note    {note}")),
        "{}",
        ran.stdout
    );
}

#[test]
fn replay_recomputes_from_the_stored_policy_and_a_changed_manifest_is_a_mismatch() {
    let h = history();
    let (code, v) = json(&h.repo, &["replay", "HEAD", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        (
            v["policy_from"].as_str(),
            v["matches"].as_bool(),
            v["pass"].as_bool()
        ),
        (Some("store"), Some(true), Some(true))
    );
    assert_eq!(v["nodes"], 2);
    assert_eq!(v["signature"], "unknown");
    let ran = jtac(&h.repo, &["replay", "HEAD"]);
    assert!(
        ran.stdout.starts_with(&format!(
            "REPLAYED: {} replays to CLEARED HOT from 2 nodes",
            h.three
        )) && ran.stdout.contains("policy    from the store"),
        "{}",
        ran.stdout
    );
    let root = v["root"]
        .as_str()
        .unwrap()
        .trim_start_matches("sha256:")
        .to_owned();
    let store = h.repo.path().join(".git").join("jtac");
    let root_json: Value =
        serde_json::from_str(&blob(&h.repo, &format!("{}:root.json", root_ref(&h.three)))).unwrap();
    let policy = root_json["policy"]["hash"]
        .as_str()
        .unwrap()
        .trim_start_matches("sha256:")
        .to_owned();
    std::fs::remove_file(store.join("objects").join(&policy)).unwrap();
    let (code, v) = json(&h.repo, &["replay", "HEAD", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        (v["policy_from"].as_str(), v["matches"].as_bool()),
        (Some("tree"), Some(true))
    );
    let manifest = store.join("roots").join(&root).join("nodes.json");
    let m: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    let first = m["nodes"][0].as_str().unwrap();
    std::fs::write(&manifest, format!("{{\"nodes\":[\"{first}\"],\"v\":1}}")).unwrap();
    let ran = jtac(&h.repo, &["replay", "HEAD"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout.starts_with("MISMATCH: ")
            && ran.stdout.contains("evidence  DIFFERS")
            && ran.stdout.contains("counts    DIFFERS")
            && ran.stdout.contains("mismatch  the Merkle root differs"),
        "{}",
        ran.stdout
    );
    let node = first.trim_start_matches("sha256:");
    std::fs::write(store.join("objects").join(node), b"garbage").unwrap();
    let ran = jtac(&h.repo, &["replay", "HEAD"]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr.contains("do not hash to the name"),
        "{}",
        ran.stderr
    );
}

#[test]
fn show_prints_each_root_ref_byte_for_byte_and_defaults_to_the_unpushed_range() {
    let h = history();
    let (code, v) = json(&h.repo, &["show", "--json", "main"]);
    assert_eq!(code, 0);
    let shown = v["roots"].as_array().unwrap();
    assert_eq!(shown.len(), 2);
    for (s, commit) in shown.iter().zip([&h.three, &h.one]) {
        let name = root_ref(commit);
        assert_eq!(s["name"], name);
        assert_eq!(s["commit"], format!("sha1:{commit}"));
        assert_eq!(
            s["root_commit"],
            format!("sha1:{}", h.repo.git(&["rev-parse", &name]))
        );
        assert_eq!(s["root_json"], blob(&h.repo, &format!("{name}:root.json")));
        assert_eq!(s["root_sig"], blob(&h.repo, &format!("{name}:root.sig")));
    }
    assert_eq!(
        v["without_ref"],
        serde_json::json!([format!("sha1:{}", h.two), format!("sha1:{}", h.base)])
    );
    let ran = jtac(&h.repo, &["show"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(
        ran.stdout.starts_with(
            "SHOW refs/remotes/origin/main..HEAD: 2 roots would be sent; 1 commits have no root \
             ref of their own\n"
        ),
        "{}",
        ran.stdout
    );
    let name = root_ref(&h.three);
    let expect = format!(
        "ref {name} {}\n{}\n{}",
        h.repo.git(&["rev-parse", &name]),
        blob(&h.repo, &format!("{name}:root.json")),
        blob(&h.repo, &format!("{name}:root.sig"))
    );
    assert!(ran.stdout.contains(&expect), "{}", ran.stdout);
    assert!(
        ran.stdout.ends_with(&format!("no root ref {}\n", h.two)),
        "{}",
        ran.stdout
    );
    h.repo.git(&["branch", "--unset-upstream"]);
    let ran = jtac(&h.repo, &["show"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.contains("HEAD has no upstream")
            && ran.stderr.contains("`jtac show main..HEAD`"),
        "{}",
        ran.stderr
    );
}

fn trusts(repo: &Repo, args: &[&str]) -> (String, Vec<String>) {
    let (code, v) = json(repo, args);
    assert_eq!(code, 0, "{v}");
    let trust = v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["trust"].as_str().map(str::to_owned))
        .collect();
    (v["signers"].as_str().unwrap().to_owned(), trust)
}

#[test]
fn signers_are_read_from_head_not_the_work_tree_and_a_file_overrides_them() {
    let h = history();
    let pubkey = std::fs::read_to_string(h.key.with_extension("pub")).unwrap();
    let pubkey: Vec<&str> = pubkey.split_whitespace().take(2).collect();
    let line = format!("dev@example.com {}\n", pubkey.join(" "));
    assert_eq!(
        trusts(&h.repo, &["audit", "--json"]).1,
        ["unknown", "unknown", "unknown"]
    );
    h.repo.write(".jtac/signers", line.as_bytes());
    h.repo.commit("signers");
    let (source, trust) = trusts(&h.repo, &["audit", "--json", "HEAD~1"]);
    assert_eq!(source, "HEAD:.jtac/signers");
    assert_eq!(trust, ["trusted", "trusted", "trusted"]);
    h.repo
        .write(".jtac/signers", b"nobody@example.com ssh-ed25519 AAAA\n");
    assert_eq!(
        trusts(&h.repo, &["audit", "--json", "HEAD~1"]).1,
        ["trusted", "trusted", "trusted"]
    );
    #[cfg(unix)]
    {
        h.repo.remove(".jtac/signers");
        std::os::unix::fs::symlink("/nonexistent", h.repo.path().join(".jtac/signers")).unwrap();
        assert_eq!(
            trusts(&h.repo, &["audit", "--json", "HEAD~1"]).1,
            ["trusted", "trusted", "trusted"]
        );
    }
    let other = h
        .repo
        .write(".git/other-signers", b"dev@example.com ssh-ed25519 AAAA\n");
    let (source, trust) = trusts(
        &h.repo,
        &[
            "audit",
            "--json",
            "--signers",
            other.to_str().unwrap(),
            "HEAD~1",
        ],
    );
    assert_eq!(source, other.to_string_lossy());
    assert_eq!(trust, ["unknown", "unknown", "unknown"]);
    let ran = jtac(&h.repo, &["verify", "HEAD~1"]);
    assert_eq!(ran.code, 0);
    assert!(
        ran.stdout.contains("0 signed by unknown keys")
            && ran.stdout.contains("signers HEAD:.jtac/signers"),
        "{}",
        ran.stdout
    );
    #[cfg(unix)]
    {
        h.repo.git(&["rm", "-q", "--cached", ".jtac/signers"]);
        h.repo.git(&["add", ".jtac/signers"]);
        h.repo.commit("signers is a link");
        let ran = jtac(&h.repo, &["verify"]);
        assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
        assert!(
            ran.stderr
                .contains("`.jtac/signers` at HEAD is not a regular file")
                && ran.stderr.contains("--signers FILE"),
            "{}",
            ran.stderr
        );
    }
}

#[test]
fn a_range_is_what_rev_list_reads_and_an_option_or_a_bad_rev_is_refused() {
    let h = history();
    let span = format!("{}..HEAD", h.one);
    let (_, v) = json(&h.repo, &["audit", "--json", &span]);
    assert_eq!(
        rows(&v).iter().map(|r| r.0.clone()).collect::<Vec<_>>(),
        [h.three.clone(), h.two.clone()]
    );
    let not = format!("^{}", h.one);
    let (_, w) = json(&h.repo, &["audit", "--json", &not, "HEAD"]);
    assert_eq!(rows(&w), rows(&v));
    let ran = jtac(&h.repo, &["audit", "--all"]);
    assert_eq!(ran.code, 2);
    assert!(ran.stderr.contains("--all"), "{}", ran.stderr);
    let ran = jtac(&h.repo, &["audit", "nope"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.contains("`nope` is not a range git can list")
            && ran.stderr.contains("negotiation: Run `git rev-list nope`"),
        "{}",
        ran.stderr
    );
    let ran = jtac(&h.repo, &["replay", "nope"]);
    assert_eq!(ran.code, 2);
    assert!(
        ran.stderr.contains("`nope` does not name a commit")
            && ran.stderr.contains("`git rev-parse nope`"),
        "{}",
        ran.stderr
    );
    let (code, v) = json(
        &h.repo,
        &["audit", "--json", &format!("{}..{}", h.three, h.three)],
    );
    assert_eq!((code, v["totals"]["commits"].as_u64()), (0, Some(0)));
    assert_eq!(v["totals"]["percent_covered"], 0);
}
