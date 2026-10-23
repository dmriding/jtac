//! Root version 2 on a scripted history (DESIGN.md section 15, M15a): `check` writes `base`
//! as the diff base and none on a parentless tip; `verify` refuses a root whose base is not
//! an ancestor, whose covers reach below its base, or whose version 1 form carries a version
//! 2 field; a version 1 root still reads.

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

fn root_json(repo: &Repo, commit: &str) -> String {
    let out = repo.command(
        "git",
        &[
            "cat-file",
            "blob",
            &format!("{}:root.json", root_ref(commit)),
        ],
    );
    assert!(out.status.success(), "{commit}");
    String::from_utf8(out.stdout).unwrap()
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

/// Replace `commit`'s root ref with `json`, freshly signed; `json` must stay canonical since
/// the reader verifies the signature over the bytes it recomputes from the parsed root.
fn replace_root(repo: &Repo, key: &Path, commit: &str, json: &str) {
    let json_path = repo.write(".git/forged/root.json", json.as_bytes());
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
    let sig = std::fs::read_to_string(sig_path).unwrap();
    let sig_path = repo.write(".git/forged/root.sig", sig.as_bytes());
    let j = repo.git(&["hash-object", "-w", "--", json_path.to_str().unwrap()]);
    let s = repo.git(&["hash-object", "-w", "--", sig_path.to_str().unwrap()]);
    let listing = format!("100644 blob {j}\troot.json\n100644 blob {s}\troot.sig\n");
    let tree = git_stdin(repo, &["mktree"], listing.as_bytes());
    let forged = repo.git(&["commit-tree", &tree, "-m", "forged"]);
    repo.git(&["update-ref", &root_ref(commit), &forged]);
}

/// `zero` parentless with a root; `base` on it with a root; `one` and `two` with one root at
/// `two`; `side` is `base`'s tree on `zero`, off the first-parent line.
struct History {
    repo: Repo,
    key: PathBuf,
    zero: String,
    base: String,
    one: String,
    two: String,
    side: String,
}

fn history() -> History {
    let repo = Repo::init(&[]);
    with_policy(&repo);
    let zero = repo.commit("zero");
    let key = keyed(&repo);
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    repo.write("src/lib.rs", b"fn a() {}\n");
    let base = repo.commit("base");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    repo.write("a.txt", b"a\n");
    let one = repo.commit("one");
    repo.write("b.txt", b"b\n");
    let two = repo.commit("two");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let side = repo.git(&[
        "commit-tree",
        &format!("{base}^{{tree}}"),
        "-p",
        &zero,
        "-m",
        "side",
    ]);
    repo.git(&["branch", "side", &side]);
    History {
        repo,
        key,
        zero,
        base,
        one,
        two,
        side,
    }
}

fn states(repo: &Repo) -> Vec<(String, String)> {
    let (_, v) = json(repo, &["audit", "--json"]);
    v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let commit = r["commit"].as_str().unwrap().trim_start_matches("sha1:");
            (commit.to_owned(), r["state"].as_str().unwrap().to_owned())
        })
        .collect()
}

#[test]
fn check_writes_version_two_with_the_diff_base_and_none_on_a_parentless_tip() {
    let h = history();
    let first: Value = serde_json::from_str(&root_json(&h.repo, &h.zero)).unwrap();
    assert_eq!(first["v"], 2);
    assert!(first.get("base").is_none(), "{first}");
    assert!(first.get("from").is_none(), "{first}");
    assert_eq!(
        first["covers"],
        serde_json::json!([format!("sha1:{}", h.zero)])
    );
    let tip: Value = serde_json::from_str(&root_json(&h.repo, &h.two)).unwrap();
    assert_eq!(tip["v"], 2);
    assert_eq!(tip["base"], format!("sha1:{}", h.base));
    assert_eq!(
        tip["covers"],
        serde_json::json!([format!("sha1:{}", h.one), format!("sha1:{}", h.two)])
    );
    assert_eq!(tip["parents"][0]["commit"], format!("sha1:{}", h.base));
    let ran = jtac(&h.repo, &["verify"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let got = states(&h.repo);
    assert!(got.iter().all(|(_, s)| s == "covered"), "{got:?}");
    // The oldest ancestor named by --base below the standing chain is a declared fact.
    h.repo.write("c.txt", b"c\n");
    let three = h.repo.commit("three");
    let ran = jtac(&h.repo, &["check", "--base", &h.zero]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let chosen: Value = serde_json::from_str(&root_json(&h.repo, &three)).unwrap();
    assert_eq!(chosen["base"], format!("sha1:{}", h.zero));
    assert_eq!(
        chosen["covers"],
        serde_json::json!([format!("sha1:{three}")])
    );
    assert_eq!(chosen["parents"][0]["commit"], format!("sha1:{}", h.two));
    assert_eq!(jtac(&h.repo, &["verify"]).code, 0);
}

#[test]
fn a_version_two_root_breaking_its_shape_or_chain_is_invalid_and_version_one_still_reads() {
    let h = history();
    let name = root_ref(&h.two);
    let kept = h.repo.git(&["rev-parse", &name]);
    let original = root_json(&h.repo, &h.two);
    let base_field = format!("{{\"base\":\"sha1:{}\",", h.base);
    assert!(original.starts_with(&base_field), "{original}");
    let covers_open = format!("\"covers\":[\"sha1:{}\"", h.one);
    let cases: [(&str, String, &str); 5] = [
        (
            "base not an ancestor",
            original.replace(&h.base, &h.side),
            "is not an ancestor of its tip",
        ),
        (
            "no base, a parent",
            format!(
                "{}\"parents\":[]{}",
                &original[..original.find("\"parents\":[").unwrap()],
                &original[original.find(",\"policy\"").unwrap()..]
            )
            .replace(&base_field, "{")
            .replace(&format!("{covers_open},"), "\"covers\":["),
            "it names no base and its tip has a parent",
        ),
        (
            "covers lists base",
            original.replace(
                &covers_open,
                &format!("\"covers\":[\"sha1:{}\",\"sha1:{}\"", h.base, h.one),
            ),
            "covers reaches base",
        ),
        (
            "covers reaches below base",
            original.replace(
                &covers_open,
                &format!("\"covers\":[\"sha1:{}\",\"sha1:{}\"", h.zero, h.one),
            ),
            "which is not between its parents or base and its tip",
        ),
        (
            "version 1 with a base",
            original.replace("\"v\":2}", "\"v\":1}"),
            "a version 1 root has no `base` field",
        ),
    ];
    for (name_of, forged, want) in &cases {
        assert_ne!(*forged, original, "{name_of}: the edit did not apply");
        replace_root(&h.repo, &h.key, &h.two, forged);
        let (code, v) = json(&h.repo, &["verify", "--json"]);
        assert_eq!(code, 1, "{name_of}: {v}");
        let why = v["invalid"][0]["why"].as_str().unwrap_or_default();
        assert!(why.contains(want), "{name_of}: {v}");
        let got = states(&h.repo);
        let state = |c: &str| got.iter().find(|r| r.0 == c).map(|r| r.1.as_str());
        assert_eq!(state(&h.two), Some("absent"), "{name_of}: {got:?}");
        assert_eq!(state(&h.base), Some("covered"), "{name_of}: {got:?}");
        h.repo.git(&["update-ref", &name, &kept]);
    }
    let v1 = original
        .replace(&base_field, "{")
        .replace("\"v\":2}", "\"v\":1}");
    replace_root(&h.repo, &h.key, &h.two, &v1);
    let (code, v) = json(&h.repo, &["verify", "--json"]);
    assert_eq!(code, 0, "{v}");
    let got = states(&h.repo);
    assert!(got.iter().all(|(_, s)| s == "covered"), "{got:?}");
}
