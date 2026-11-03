//! `jtac release` on a scripted history (DESIGN.md section 16.3, M18): a clean range
//! clears and attests, an absent commit is refused naming it, `--require-ci` names a tag
//! on a pull request commit, a CI waiver counts, the start is the previous attestation or
//! `--from` and never a bare tag, only the CI key's line may attest, and an attested tag
//! verifies in a fresh clone while a forged or edited attestation does not.

use std::path::{Path, PathBuf};
use std::process::Command;

use jtac_core::{Compared, Covers, FromHead, GitOid, Parent, Root, Signer};
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

/// An ed25519 key under `.git/<name>` with `comment` as its principal.
fn make_key(repo: &Repo, name: &str, comment: &str) -> PathBuf {
    let key = repo.path().join(".git").join(name);
    let out = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
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

fn use_key(repo: &Repo, key: &Path, principal: &str) {
    let path = key.to_string_lossy().replace('\\', "/");
    repo.git(&["config", "jtac.signingkey", &path]);
    repo.git(&["config", "user.email", principal]);
}

fn pubkey(key: &Path) -> String {
    let text = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    text.split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

fn fingerprint(key: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .args(["-l", "-f"])
        .arg(key.with_extension("pub"))
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    text.split_whitespace().nth(1).unwrap().to_owned()
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

fn check(repo: &Repo) {
    let ran = jtac(repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
}

fn g(hex: &str) -> GitOid {
    format!("sha1:{hex}").parse().unwrap()
}

fn root_of(repo: &Repo, commit: &str) -> Root {
    let spec = format!("refs/jtac/roots/{}/{commit}:root.json", &commit[..2]);
    let out = repo.command("git", &["cat-file", "blob", &spec]);
    assert!(out.status.success(), "{commit}");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn hexes(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().trim_start_matches("sha1:").to_owned())
        .collect()
}

/// `zero` (adoption, tagged `v0.1.0`) on `main`; `one` and `two` on a feature branch with
/// the developer's roots; `merge` of the branch with a CI root (tagged `v0.2.0`); `three`
/// after it with no root. The developer's line trusts every namespace it may; the CI key's
/// line is confined to the CI and release namespaces. Nothing is attested yet.
struct History {
    origin: Repo,
    repo: Repo,
    dev: PathBuf,
    ci: PathBuf,
    zero: String,
    one: String,
    two: String,
    merge: String,
    three: String,
}

fn history() -> History {
    let origin = Repo::bare(&[]);
    let repo = Repo::clone_of(&origin);
    with_policy(&repo);
    let dev = make_key(&repo, "dev", "dev@example.com");
    let ci = make_key(&repo, "ci", "ci@example.com");
    let signers = format!(
        "dev@example.com {}\nci@example.com namespaces=\"jtac-ci-root-v1,jtac-release-v1\" {}\n",
        pubkey(&dev),
        pubkey(&ci)
    );
    repo.write(".jtac/signers", signers.as_bytes());
    use_key(&repo, &dev, "dev@example.com");
    repo.write("src/lib.rs", b"fn a() {}\n");
    let zero = repo.commit("zero");
    check(&repo);
    repo.git(&["tag", "-a", "-m", "v0.1.0", "v0.1.0"]);
    repo.git(&["switch", "-q", "-c", "feature"]);
    repo.write("a.txt", b"a\n");
    let one = repo.commit("one");
    check(&repo);
    repo.write("b.txt", b"b\n");
    let two = repo.commit("two");
    check(&repo);
    repo.git(&["switch", "-q", "main"]);
    repo.git(&[
        "merge",
        "-q",
        "--no-ff",
        "--no-gpg-sign",
        "-m",
        "merge",
        "feature",
    ]);
    let merge = repo.git(&["rev-parse", "HEAD"]);
    let mut r = root_of(&repo, &two);
    r.commit = g(&merge);
    r.tree = g(&repo.git(&["rev-parse", "HEAD^{tree}"]));
    r.covers = Covers::List(vec![g(&one), g(&two), g(&merge)]);
    r.base = Some(g(&zero));
    r.parents = vec![Parent {
        commit: g(&zero),
        root: root_of(&repo, &zero).id().unwrap(),
    }];
    r.from = Some(FromHead {
        head: g(&two),
        root: Some(root_of(&repo, &two).id().unwrap()),
        compared: Some(Compared::Agreed),
    });
    r.signer = Signer {
        principal: "ci@example.com".into(),
        fingerprint: fingerprint(&ci),
    };
    let json = String::from_utf8(r.canonical_bytes().unwrap()).unwrap();
    repo.put_root(&ci, "jtac-ci-root-v1", &merge, &json, None);
    repo.git(&["tag", "-a", "-m", "v0.2.0", "v0.2.0"]);
    repo.write("c.txt", b"c\n");
    let three = repo.commit("three");
    repo.git(&["push", "-q", "origin", "main", "feature", "--tags"]);
    repo.git(&["push", "-q", "origin", "+refs/jtac/*:refs/jtac/*"]);
    History {
        origin,
        repo,
        dev,
        ci,
        zero,
        one,
        two,
        merge,
        three,
    }
}

/// `v0.2.0` attested by the CI key from `v0.1.0`, the developer's key back in config.
fn attest_v020(h: &History) {
    use_key(&h.repo, &h.ci, "ci@example.com");
    let args = [
        "release", "v0.2.0", "--from", "v0.1.0", "--push", "origin", "--json",
    ];
    let (code, v) = json(&h.repo, &args);
    assert_eq!(code, 0, "{v}");
    use_key(&h.repo, &h.dev, "dev@example.com");
}

#[test]
fn a_clean_range_clears_from_the_named_start_and_names_each_root_once() {
    let h = history();
    let (code, v) = json(
        &h.repo,
        &["release", "v0.2.0", "--from", "v0.1.0", "--json"],
    );
    assert_eq!(code, 0, "{v}");
    assert!(v["from"]["tag"].is_null(), "{v}");
    assert_eq!(v["from"]["commit"], format!("sha1:{}", h.zero), "{v}");
    assert_eq!(v["commits"], 3, "{v}");
    assert_eq!(v["covered"], 3, "{v}");
    assert_eq!(v["roots"].as_array().unwrap().len(), 3, "{v}");
    assert_eq!(v["signers"], "v0.2.0:.jtac/signers", "{v}");
    assert!(v["attested"].is_null(), "{v}");
    let ran = jtac(&h.repo, &["release", "v0.2.0", "--from", "v0.1.0"]);
    assert!(
        ran.stdout.starts_with(&format!(
            "RELEASE v0.2.0 CLEAR: 3 commits after {}",
            &h.zero[..12]
        )),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("--push <remote>"), "{}", ran.stdout);
    let args = [
        "release",
        "v0.2.0",
        "--from",
        "v0.1.0",
        "--require-ci",
        "--json",
    ];
    let (code, v) = json(&h.repo, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["not_ci"], serde_json::json!([]), "{v}");
}

#[test]
fn an_absent_commit_is_refused_naming_it_and_a_ci_waiver_then_counts() {
    let h = history();
    h.repo.git(&["tag", "-a", "-m", "v0.3.0", "v0.3.0"]);
    let ran = jtac(&h.repo, &["release", "v0.3.0"]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr
            .contains("No attested release is an ancestor of v0.3.0"),
        "{}",
        ran.stderr
    );
    attest_v020(&h);
    // A lightweight `v*` tag between the releases is not a start: nothing attested it.
    h.repo.git(&["tag", "v0.2.5", &h.merge]);
    let (code, v) = json(&h.repo, &["release", "v0.3.0", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(hexes(&v["absent"]), [h.three.as_str()], "{v}");
    assert_eq!(v["from"]["tag"], "v0.2.0", "{v}");
    assert_eq!(v["commits"], 1, "{v}");
    assert_eq!(v["covered"], 0, "{v}");
    let ran = jtac(&h.repo, &["release", "v0.3.0", "--push", "origin"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout
            .starts_with("RELEASE v0.3.0 REFUSED: 1 commits since v0.2.0"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout
            .contains(&format!("absent    {} no root", h.three)),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("jtac waive <commit> --ci"),
        "{}",
        ran.stdout
    );
    assert!(
        h.origin
            .command("git", &["rev-parse", "refs/jtac/releases/v0.3.0"])
            .status
            .code()
            != Some(0),
        "a refused release pushes nothing"
    );
    let ran = jtac(
        &h.repo,
        &["waive", &h.three, "--ci", "--reason", "merged around CI"],
    );
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    check(&h.repo);
    let (code, v) = json(&h.repo, &["release", "v0.3.0", "--require-ci", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(hexes(&v["waived"]), [h.three.as_str()], "{v}");
    assert_eq!(v["declared_off"], 1, "{v}");
    assert_eq!(v["covered"], 0, "{v}");
}

/// The first v0.2 tag's case: a tag on the pull request's own commit, whose root is the
/// developer's; CI's root sits on the merge commit, outside the range.
#[test]
fn require_ci_names_a_tag_on_a_pull_request_commit_and_an_untrusted_root() {
    let h = history();
    h.repo.git(&["tag", "-a", "-m", "v0.1.1", "v0.1.1", &h.two]);
    let (code, v) = json(
        &h.repo,
        &["release", "v0.1.1", "--from", "v0.1.0", "--json"],
    );
    assert_eq!(code, 0, "{v}");
    let args = [
        "release",
        "v0.1.1",
        "--from",
        "v0.1.0",
        "--require-ci",
        "--json",
    ];
    let (code, v) = json(&h.repo, &args);
    assert_eq!(code, 1, "{v}");
    assert_eq!(hexes(&v["not_ci"]), [h.two.clone(), h.one.clone()], "{v}");
    let ran = jtac(&h.repo, &args[..5]);
    assert!(
        ran.stdout.contains("tag the merge commit the forge made"),
        "{}",
        ran.stdout
    );
    let ci_only = h.repo.write(
        ".git/ci-only",
        format!(
            "ci@example.com namespaces=\"jtac-ci-root-v1,jtac-release-v1\" {}\n",
            pubkey(&h.ci)
        )
        .as_bytes(),
    );
    let (code, v) = json(
        &h.repo,
        &[
            "release",
            "v0.2.0",
            "--from",
            "v0.1.0",
            "--signers",
            ci_only.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(code, 1, "{v}");
    assert_eq!(
        hexes(&v["untrusted"]),
        [h.two.clone(), h.one.clone()],
        "{v}"
    );
    assert_eq!(v["absent"], serde_json::json!([]), "{v}");
}

#[test]
fn the_start_is_the_previous_attestation_or_from_and_a_bad_start_or_tag_is_refused() {
    let h = history();
    let (code, v) = json(&h.repo, &["release", "v0.2.0", "--from", &h.one, "--json"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["from"]["tag"].is_null(), "{v}");
    assert_eq!(v["commits"], 2, "{v}");
    // A ref outside refs/tags/ that shadows the tag's name is a commit, not the tag.
    h.repo.git(&["update-ref", "refs/v0.1.0", &h.one]);
    let (code, v) = json(
        &h.repo,
        &["release", "v0.2.0", "--from", "v0.1.0", "--json"],
    );
    assert_eq!(code, 0, "{v}");
    assert!(v["from"]["tag"].is_null(), "{v}");
    assert_eq!(v["from"]["commit"], format!("sha1:{}", h.one), "{v}");
    h.repo.git(&["update-ref", "-d", "refs/v0.1.0"]);
    let (code, v) = json(
        &h.repo,
        &["release", "v0.2.0", "--from", "v0.1.0^0", "--json"],
    );
    assert_eq!(code, 0, "{v}");
    assert!(v["from"]["tag"].is_null(), "{v}");
    let rows = [
        (
            vec!["release", "v0.2.0", "--from", &h.three],
            "is not an ancestor of v0.2.0",
        ),
        (
            vec!["release", "v0.2.0", "--from", &h.merge],
            "is v0.2.0's own commit, so the range holds no commit",
        ),
        (
            vec!["release", "v0.2.0"],
            "No attested release is an ancestor",
        ),
        (
            vec!["release", "v0.1.0"],
            "No attested release is an ancestor",
        ),
        (vec!["release", "main"], "`main` is not a tag"),
        (vec!["release", "v9"], "`v9` is not a tag"),
        (vec!["release", "v0.2.0", "--from", "nope"], "nope"),
    ];
    for (args, want) in rows {
        let ran = jtac(&h.repo, &args);
        assert_eq!(ran.code, 2, "{args:?}: {}{}", ran.stdout, ran.stderr);
        assert!(ran.stderr.contains(want), "{args:?}: {}", ran.stderr);
        assert!(
            ran.stderr.contains("negotiation: "),
            "{args:?}: {}",
            ran.stderr
        );
    }
    // Once attested, a tag's own start is its default, and the next tag chains to it.
    attest_v020(&h);
    let (code, v) = json(&h.repo, &["release", "v0.2.0", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["from"]["tag"].is_null(), "{v}");
    assert_eq!(v["attested"]["origin"], "stands", "{v}");
    h.repo.git(&["tag", "-a", "-m", "v0.2.1", "v0.2.1"]);
    let (code, v) = json(&h.repo, &["release", "v0.2.1", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["from"]["tag"], "v0.2.0", "{v}");
    assert_eq!(v["commits"], 1, "{v}");
    assert_eq!(v["notes"], serde_json::json!([]), "{v}");

    // A planted document under the namespace, signed by a key no line trusts for the
    // namespace, is not a start: the chain still reaches v0.2.0. Junk there is noted, not
    // a refusal. The nearest of two trusted starts wins.
    let doc = h
        .repo
        .git(&["cat-file", "blob", "refs/jtac/releases/v0.2.0:release.json"]);
    let mut planted: Value = serde_json::from_str(&doc).unwrap();
    planted["tag"] = serde_json::json!("evil");
    planted["commit"] = serde_json::json!(format!("sha1:{}", h.three));
    planted["tree"] = serde_json::json!(format!(
        "sha1:{}",
        h.repo.git(&["rev-parse", &format!("{}^{{tree}}", h.three)])
    ));
    planted["signer"] = serde_json::json!({
        "principal": "dev@example.com",
        "fingerprint": fingerprint(&h.dev),
    });
    let planted = jtac_core::canonical_bytes(&planted).unwrap();
    h.repo.put_release(
        &h.dev,
        "jtac-release-v1",
        "evil",
        std::str::from_utf8(&planted).unwrap(),
        None,
    );
    h.repo
        .put_release(&h.dev, "jtac-release-v1", "junk", "{", Some("x"));
    h.repo.write("d.txt", b"d\n");
    let four = h.repo.commit("four");
    h.repo.git(&["tag", "-a", "-m", "v0.2.2", "v0.2.2"]);
    let (code, v) = json(&h.repo, &["release", "v0.2.2", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["from"]["tag"], "v0.2.0", "{v}");
    assert_eq!(
        hexes(&v["absent"]),
        [four.as_str(), h.three.as_str()],
        "{v}"
    );
    let notes: Vec<&str> = v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert!(
        notes
            .iter()
            .any(|n| n.contains("releases/evil") && n.contains("unknown")),
        "{v}"
    );
    assert!(notes.iter().any(|n| n.contains("releases/junk")), "{v}");
    h.repo.git(&["update-ref", "-d", "refs/jtac/releases/evil"]);
    h.repo.git(&["update-ref", "-d", "refs/jtac/releases/junk"]);
    let ran = jtac(&h.repo, &["waive", &h.three, "--ci", "--reason", "x"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    h.repo.git(&["switch", "-q", "--detach", &h.three]);
    check(&h.repo);
    h.repo.git(&["switch", "-q", "main"]);
    h.repo
        .git(&["tag", "-a", "-m", "v0.2.1b", "v0.2.1b", &h.three]);
    use_key(&h.repo, &h.ci, "ci@example.com");
    let (code, v) = json(
        &h.repo,
        &["release", "v0.2.1b", "--push", "origin", "--json"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["from"]["tag"], "v0.2.0", "{v}");
    use_key(&h.repo, &h.dev, "dev@example.com");
    let (code, v) = json(&h.repo, &["release", "v0.2.2", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["from"]["tag"], "v0.2.1b", "{v}");
    assert_eq!(hexes(&v["absent"]), [four.as_str()], "{v}");
}

#[test]
fn sums_name_the_artifacts_and_a_malformed_file_is_refused() {
    let h = history();
    let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let sums = h.repo.write(
        ".git/SHA256SUMS",
        format!("{hex}  jtac-v0.2.0-x.tar.gz\n").as_bytes(),
    );
    let (code, v) = json(
        &h.repo,
        &[
            "release",
            "v0.2.0",
            "--from",
            "v0.1.0",
            "--sums",
            sums.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["artifacts"][0]["name"], "jtac-v0.2.0-x.tar.gz", "{v}");
    assert_eq!(v["artifacts"][0]["sha256"], format!("sha256:{hex}"), "{v}");
    let bad = h.repo.write(".git/SHA256SUMS", b"not a sum\n");
    let ran = jtac(
        &h.repo,
        &[
            "release",
            "v0.2.0",
            "--from",
            "v0.1.0",
            "--sums",
            bad.to_str().unwrap(),
        ],
    );
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr.contains("is not a SHA256SUMS file: line 1"),
        "{}",
        ran.stderr
    );
    assert!(ran.stderr.contains("sha256sum -- *"), "{}", ran.stderr);
}

#[test]
fn only_a_key_whose_line_names_the_namespace_attests() {
    let h = history();
    let ran = jtac(
        &h.repo,
        &["release", "v0.2.0", "--from", "v0.1.0", "--push", "origin"],
    );
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr
            .contains("is not trusted for jtac-release-v1 by v0.2.0:.jtac/signers"),
        "{}",
        ran.stderr
    );
    assert!(
        ran.stderr.contains("from the release job"),
        "{}",
        ran.stderr
    );
    assert!(
        h.repo
            .command("git", &["rev-parse", "refs/jtac/releases/v0.2.0"])
            .status
            .code()
            != Some(0),
        "an untrusted key writes nothing"
    );

    use_key(&h.repo, &h.ci, "ci@example.com");
    let (code, v) = json(
        &h.repo,
        &[
            "release",
            "v0.2.0",
            "--from",
            "v0.1.0",
            "--require-ci",
            "--push",
            "origin",
            "--json",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["attested"]["origin"], "written", "{v}");
    assert_eq!(v["attested"]["trust"], "trusted", "{v}");
    assert_eq!(v["attested"]["pushed"], "sent", "{v}");
    assert_eq!(
        v["attested"]["signer"]["principal"], "ci@example.com",
        "{v}"
    );
    assert_eq!(
        v["attested"]["reference"], "refs/jtac/releases/v0.2.0",
        "{v}"
    );
    let id = v["attested"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        h.origin.git(&["rev-parse", "refs/jtac/releases/v0.2.0"]),
        h.repo.git(&["rev-parse", "refs/jtac/releases/v0.2.0"])
    );
    let object = h.repo.git(&["cat-file", "-p", "refs/jtac/releases/v0.2.0"]);
    assert!(object.ends_with(&id), "{object}");
    let doc: Value = serde_json::from_str(&h.repo.git(&[
        "cat-file",
        "blob",
        "refs/jtac/releases/v0.2.0:release.json",
    ]))
    .unwrap();
    assert_eq!(
        doc["object"],
        format!("sha1:{}", h.repo.git(&["rev-parse", "refs/tags/v0.2.0"])),
        "{doc}"
    );
    assert_eq!(doc["commit"], format!("sha1:{}", h.merge), "{doc}");
    assert_eq!(doc["ci"], true, "{doc}");
}

#[test]
fn a_rerun_finds_the_attestation_standing_and_a_changed_input_disagrees() {
    let h = history();
    use_key(&h.repo, &h.ci, "ci@example.com");
    let args = [
        "release",
        "v0.2.0",
        "--from",
        "v0.1.0",
        "--require-ci",
        "--push",
        "origin",
        "--json",
    ];
    let (code, v) = json(&h.repo, &args);
    assert_eq!(code, 0, "{v}");
    let id = v["attested"]["id"].as_str().unwrap().to_owned();
    // A rerun of the release job: the attestation stands, its own start is the default,
    // and nothing is rewritten.
    let (code, v) = json(
        &h.repo,
        &[
            "release",
            "v0.2.0",
            "--require-ci",
            "--push",
            "origin",
            "--json",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["attested"]["origin"], "stands", "{v}");
    assert_eq!(v["attested"]["pushed"], "up to date", "{v}");
    assert_eq!(v["attested"]["id"], id, "{v}");
    assert!(v["from"]["tag"].is_null(), "{v}");
    let (code, v) = json(
        &h.repo,
        &[
            "release",
            "v0.2.0",
            "--from",
            &h.zero,
            "--require-ci",
            "--json",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["attested"]["origin"], "stands", "{v}");
    let ran = jtac(&h.repo, &["release", "v0.2.0", "--require-ci"]);
    assert!(
        ran.stdout.starts_with("RELEASE v0.2.0 ATTESTED"),
        "{}",
        ran.stdout
    );
    // A rerun whose build differs from the attested artifacts is refused.
    let other = h.repo.write(
        ".git/SHA256SUMS",
        b"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  rebuilt.tar.gz\n",
    );
    let (code, v) = json(
        &h.repo,
        &[
            "release",
            "v0.2.0",
            "--require-ci",
            "--sums",
            other.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(code, 1, "{v}");
    assert!(
        v["attested"]["disagrees"]
            .as_str()
            .unwrap()
            .contains("artifacts"),
        "{v}"
    );
    // Another start, or no `--require-ci`, disagrees with what was attested.
    let (code, v) = json(
        &h.repo,
        &[
            "release",
            "v0.2.0",
            "--from",
            &h.one,
            "--require-ci",
            "--json",
        ],
    );
    assert_eq!(code, 1, "{v}");
    assert!(
        v["attested"]["disagrees"]
            .as_str()
            .unwrap()
            .contains("range starts after"),
        "{v}"
    );
    let (code, v) = json(&h.repo, &["release", "v0.2.0", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["attested"]["disagrees"]
            .as_str()
            .unwrap()
            .contains("with `--require-ci`"),
        "{v}"
    );
}

/// `v0.2.0` attested by the CI key from `v0.1.0` under `--require-ci`; the key stays.
fn attest_ci(h: &History) {
    use_key(&h.repo, &h.ci, "ci@example.com");
    let args = [
        "release",
        "v0.2.0",
        "--from",
        "v0.1.0",
        "--require-ci",
        "--push",
        "origin",
        "--json",
    ];
    let (code, v) = json(&h.repo, &args);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn an_attested_tag_verifies_in_a_fresh_clone_once_the_refs_are_fetched() {
    let h = history();
    attest_ci(&h);
    let clone = Repo::clone_of(&h.origin);
    let ran = jtac(&clone, &["release", "v0.2.0", "--verify"]);
    assert_eq!(ran.code, 1, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stdout.starts_with("RELEASE v0.2.0 UNATTESTED"),
        "{}",
        ran.stdout
    );
    assert!(
        ran.stdout.contains("+refs/jtac/*:refs/jtac/*"),
        "{}",
        ran.stdout
    );
    clone.git(&["fetch", "-q", "origin", "+refs/jtac/*:refs/jtac/*"]);
    let (code, v) = json(&clone, &["release", "v0.2.0", "--verify", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["verify"], true, "{v}");
    assert_eq!(v["require_ci"], true, "{v}");
    assert_eq!(v["attested"]["trust"], "trusted", "{v}");
    assert!(v["attested"]["disagrees"].is_null(), "{v}");
    assert!(v["attested"]["pushed"].is_null(), "{v}");
    let ran = jtac(&clone, &["release", "v0.2.0", "--verify"]);
    assert!(
        ran.stdout.starts_with("RELEASE v0.2.0 ATTESTED"),
        "{}",
        ran.stdout
    );
}

/// A fresh clone of an attested `v0.2.0` with the evidence refs fetched.
fn attested_clone() -> (History, Repo) {
    let h = history();
    attest_ci(&h);
    let clone = Repo::clone_of(&h.origin);
    clone.git(&["fetch", "-q", "origin", "+refs/jtac/*:refs/jtac/*"]);
    let (code, v) = json(&clone, &["release", "v0.2.0", "--verify", "--json"]);
    assert_eq!(code, 0, "{v}");
    (h, clone)
}

#[test]
fn a_forged_or_edited_attestation_does_not_verify() {
    let (h, clone) = attested_clone();
    // Forged: the CI key's document signed by the developer's key (another key: invalid);
    // a document naming the developer, signed by the developer, whose line names no
    // namespace (unknown); the CI key over edited fields (trusted, disagrees); the real
    // signature over edited bytes (invalid).
    let json_text = clone.git(&["cat-file", "blob", "refs/jtac/releases/v0.2.0:release.json"]);
    clone.put_release(&h.dev, "jtac-release-v1", "v0.2.0", &json_text, None);
    let (code, v) = json(&clone, &["release", "v0.2.0", "--verify", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["attested"]["trust"], "invalid", "{v}");
    let mut as_dev: Value = serde_json::from_str(&json_text).unwrap();
    as_dev["signer"] = serde_json::json!({
        "principal": "dev@example.com",
        "fingerprint": fingerprint(&h.dev),
    });
    let as_dev = jtac_core::canonical_bytes(&as_dev).unwrap();
    clone.put_release(
        &h.dev,
        "jtac-release-v1",
        "v0.2.0",
        std::str::from_utf8(&as_dev).unwrap(),
        None,
    );
    let (code, v) = json(&clone, &["release", "v0.2.0", "--verify", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["attested"]["trust"], "unknown", "{v}");
    let first_root = v["roots"][0].clone();
    let edits: [(&[(&str, Value)], &str); 7] = [
        (
            &[("roots", serde_json::json!([first_root]))],
            "roots it names",
        ),
        (
            &[
                ("declared_off", serde_json::json!(1)),
                ("covered", serde_json::json!(2)),
                ("waived", serde_json::json!([format!("sha1:{}", h.one)])),
            ],
            "counts",
        ),
        (
            &[("object", serde_json::json!(format!("sha1:{}", h.merge)))],
            "not this tag",
        ),
        (
            &[(
                "from",
                serde_json::json!({"commit": format!("sha1:{}", h.zero), "tag": "v0.0.9"}),
            )],
            "its start v0.0.9 is not an attested release",
        ),
        (
            &[(
                "from",
                serde_json::json!({"commit": format!("sha1:{}", h.one), "tag": "v0.2.0"}),
            )],
            "its start v0.2.0 is attested at",
        ),
        (
            &[
                ("commits", serde_json::json!(7)),
                ("covered", serde_json::json!(7)),
            ],
            "counts 7 commits",
        ),
        (
            &[("tree", serde_json::json!(format!("sha1:{}", h.one)))],
            "not this tag",
        ),
    ];
    for (fields, want) in edits {
        let mut edited: Value = serde_json::from_str(&json_text).unwrap();
        for (field, value) in fields {
            edited[*field] = value.clone();
        }
        let field = fields[0].0;
        let edited = jtac_core::canonical_bytes(&edited).unwrap();
        clone.put_release(
            &h.ci,
            "jtac-release-v1",
            "v0.2.0",
            std::str::from_utf8(&edited).unwrap(),
            None,
        );
        let (code, v) = json(&clone, &["release", "v0.2.0", "--verify", "--json"]);
        assert_eq!(code, 1, "{field}: {v}");
        assert_eq!(v["attested"]["trust"], "trusted", "{field}: {v}");
        assert!(
            v["attested"]["disagrees"].as_str().unwrap().contains(want),
            "{field}: {v}"
        );
    }
    let ran = jtac(&clone, &["release", "v0.2.0", "--verify"]);
    assert!(
        ran.stdout.starts_with("RELEASE v0.2.0 INVALID"),
        "{}",
        ran.stdout
    );
    assert!(ran.stdout.contains("attested once"), "{}", ran.stdout);
    let mut lying: Value = serde_json::from_str(&json_text).unwrap();
    lying["covered"] = serde_json::json!(9);
    let lying = jtac_core::canonical_bytes(&lying).unwrap();
    clone.put_release(
        &h.ci,
        "jtac-release-v1",
        "v0.2.0",
        std::str::from_utf8(&lying).unwrap(),
        None,
    );
    let ran = jtac(&clone, &["release", "v0.2.0", "--verify"]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    assert!(
        ran.stderr.contains("not the commit count"),
        "{}",
        ran.stderr
    );
    let sig = clone.git(&["cat-file", "blob", "refs/jtac/releases/v0.2.0:release.sig"]);
    clone.put_release(&h.ci, "jtac-release-v1", "v0.2.0", &json_text, Some(&sig));
    let (code, v) = json(&clone, &["release", "v0.2.0", "--verify", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["attested"]["trust"], "invalid", "{v}");
}
