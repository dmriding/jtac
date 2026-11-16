//! The readers of root version 3 on a scripted history (DESIGN.md sections 9.5 and 15.2,
//! M20b, decisions 57, 58, 60, 62 and 63). No gate writes a version 3 root yet, so each is
//! built here and signed: a developer's root under a `ci` check reads partial and one that
//! leaves the check out of `unread` is invalid; the merge root covers the pull request and
//! the CI gate reads its first-parent line alone; a policy a blobless clone lacks is exit 2
//! with the fetch named, and `policy not held` in `audit`.

use std::path::{Path, PathBuf};
use std::process::Command;

use jtac_core::{
    Body, Covers, Forge, FromHead, GitOid, Node, Observed, ObservedCheck, Outcome, Parent, Root,
    RootVersion, Signer,
};
use jtac_testkit::Repo;
use serde_json::Value;

const CI_POLICY: &str = "version = 2\n[protected]\npaths = [\".github/**\", \".jtac/**\"]\n\
                         [checks.test]\nci = { workflow = \".github/workflows/ci.yml\", job = \"test\" }\n";

fn me() -> String {
    env!("CARGO_BIN_EXE_jtac").replace('\\', "/")
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

fn g(hex: &str) -> GitOid {
    format!("sha1:{hex}").parse().unwrap()
}

fn tree(repo: &Repo, commit: &str) -> GitOid {
    g(&repo.git(&["rev-parse", &format!("{commit}^{{tree}}")]))
}

fn root_of(repo: &Repo, commit: &str) -> Root {
    let spec = format!("refs/jtac/roots/{}/{commit}:root.json", &commit[..2]);
    let out = repo.command("git", &["cat-file", "blob", &spec]);
    assert!(out.status.success(), "{commit}");
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Write `r`, canonical and freshly signed, as `commit`'s root, in the namespace its shape
/// picks.
fn put(repo: &Repo, key: &Path, commit: &str, r: &Root) {
    let json = String::from_utf8(r.canonical_bytes().unwrap()).unwrap();
    let namespace = match r.from {
        Some(_) => "jtac-ci-root-v1",
        None => "jtac-root-v1",
    };
    repo.put_root(key, namespace, commit, &json, None);
}

/// Each row of `audit` as (state, via, ci), by commit.
fn row(v: &Value, commit: &str) -> (String, String, String) {
    let rows = v["rows"].as_array().unwrap();
    let r = (rows.iter())
        .find(|r| r["commit"] == format!("sha1:{commit}"))
        .unwrap_or_else(|| panic!("no row for {commit}: {v}"));
    let text = |k: &str| r[k].as_str().unwrap_or("-").to_owned();
    (text("state"), text("via"), text("ci"))
}

/// `zero` under a version 1 policy with a real root; `one` on a feature branch brings a
/// policy with a `ci` check; `merge` is the forge's merge of it; `r1` and `r2` land on
/// `main` after it as a rebase merge of `head`, which stays on its own branch.
struct History {
    repo: Repo,
    dev: PathBuf,
    ci: PathBuf,
    zero: String,
    one: String,
    merge: String,
    r1: String,
    r2: String,
    head: String,
}

impl History {
    /// `zero`'s root with every field a commit of its own changes left for the caller.
    fn template(&self, commit: &str, covers: &[&str], base: &str) -> Root {
        let zero = root_of(&self.repo, &self.zero);
        Root {
            commit: g(commit),
            tree: tree(&self.repo, commit),
            covers: Covers::List(covers.iter().map(|c| g(c)).collect()),
            base: Some(g(base)),
            parents: match base == self.zero {
                true => vec![Parent {
                    commit: g(&self.zero),
                    root: zero.id().unwrap(),
                }],
                false => vec![],
            },
            ..zero
        }
    }

    /// The developer's root on `commit`: version 3 with `test` unread.
    fn dev_root(&self, commit: &str, base: &str) -> Root {
        Root {
            v: RootVersion::V3,
            unread: vec!["test".into()],
            ..self.template(commit, &[commit], base)
        }
    }

    /// The merge job's root on `commit` for `head`, holding `test` as read from CI.
    fn ci_root(&self, commit: &str, covers: &[&str], base: &str, head: &str) -> Root {
        let r = self.template(commit, covers, base);
        let observed = Node {
            tree: r.tree,
            policy: r.policy.hash,
            ts: r.ts,
            body: Body::Observed(ObservedCheck {
                id: "test".into(),
                outcome: Outcome::Pass,
                observed: Observed {
                    forge: Forge::Github,
                    workflow: ".github/workflows/ci.yml".into(),
                    blob: g(&self
                        .repo
                        .git(&["rev-parse", &format!("{commit}:.github/workflows/ci.yml")])),
                    job: Some("test".into()),
                    commit: g(commit),
                    run: 1234,
                    attempt: 1,
                    conclusion: "success".into(),
                },
            }),
        };
        Root {
            v: RootVersion::V3,
            from: Some(FromHead {
                head: g(head),
                root: None,
                compared: None,
            }),
            observed: vec![observed],
            signer: Signer {
                principal: "ci@example.com".into(),
                fingerprint: fingerprint(&self.ci),
            },
            ..r
        }
    }
}

fn history() -> History {
    let repo = Repo::init(&[]);
    let policy = format!(
        "version = 1\n[checks.ok]\nrun = [\"{}\", \"--version\"]\n",
        me()
    );
    repo.write(".jtac/policy.toml", policy.as_bytes());
    repo.write(".gitignore", b"*.profraw\n");
    let dev = make_key(&repo, "dev", "dev@example.com");
    let ci = make_key(&repo, "ci", "ci@example.com");
    let signers = format!(
        "dev@example.com {}\nci@example.com namespaces=\"jtac-ci-root-v1,jtac-release-v1\" {}\n",
        pubkey(&dev),
        pubkey(&ci)
    );
    repo.write(".jtac/signers", signers.as_bytes());
    let path = dev.to_string_lossy().replace('\\', "/");
    repo.git(&["config", "jtac.signingkey", &path]);
    repo.git(&["config", "user.email", "dev@example.com"]);
    let zero = repo.commit("zero");
    let ran = jtac(&repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    repo.git(&["switch", "-q", "-c", "feature"]);
    repo.write(".jtac/policy.toml", CI_POLICY.as_bytes());
    repo.write(".github/workflows/ci.yml", b"on: push\n");
    let one = repo.commit("one");
    repo.git(&["switch", "-q", "main"]);
    let merge_args = ["merge", "-q", "--no-ff", "--no-gpg-sign", "-m", "merge"];
    repo.git(&[&merge_args[..], &["feature"]].concat());
    let merge = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["switch", "-q", "-c", "pr2"]);
    repo.write("h.txt", b"h\n");
    let head = repo.commit("head");
    repo.git(&["switch", "-q", "main"]);
    repo.write("r1.txt", b"r1\n");
    let r1 = repo.commit("r1");
    repo.write("r2.txt", b"r2\n");
    let r2 = repo.commit("r2");
    History {
        repo,
        dev,
        ci,
        zero,
        one,
        merge,
        r1,
        r2,
        head,
    }
}

#[test]
fn a_developers_root_under_a_ci_check_is_partial_and_one_that_leaves_it_out_is_invalid() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    let (code, v) = json(&h.repo, &["audit", &h.one, "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        row(&v, &h.one),
        ("partial".into(), "own".into(), "-".into())
    );
    assert_eq!(v["totals"]["partial"], 1, "{v}");
    assert_eq!(v["totals"]["covered"], 1, "{v}");
    let text = jtac(&h.repo, &["audit", &h.one]).stdout;
    assert!(text.contains("covered, ci unread own root"), "{text}");
    assert!(text.contains("1 covered with a ci check unread"), "{text}");

    let (code, v) = json(&h.repo, &["verify", &h.one, "--json"]);
    assert_eq!((code, &v["partial"]), (0, &Value::from(1)), "{v}");
    let (code, v) = json(&h.repo, &["verify", &h.one, "--require-coverage", "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["partial_commits"][0], format!("sha1:{}", h.one), "{v}");
    assert_eq!(v["absent_commits"], serde_json::json!([]), "{v}");

    // Signed as version 2 with nothing unread, as a 0.2 gate or a forger would write it.
    let silent = Root {
        v: RootVersion::V2,
        unread: vec![],
        ..h.dev_root(&h.one, &h.zero)
    };
    put(&h.repo, &h.dev, &h.one, &silent);
    let (code, v) = json(&h.repo, &["verify", &h.one, "--json"]);
    assert_eq!(code, 1, "{v}");
    let why = v["invalid"][0]["why"].as_str().unwrap();
    assert_eq!(
        why,
        "it lists no check unread, and the policy in the tree it names leaves `test` to CI"
    );
    let (_, v) = json(&h.repo, &["audit", &h.one, "--json"]);
    assert_eq!(row(&v, &h.one).0, "absent", "{v}");

    // A root on a tree with no `ci` check that claims one unread is invalid the same way.
    let loud = Root {
        v: RootVersion::V3,
        unread: vec!["test".into()],
        ..root_of(&h.repo, &h.zero)
    };
    put(&h.repo, &h.dev, &h.zero, &loud);
    let (code, v) = json(&h.repo, &["verify", &h.zero, "--json"]);
    assert_eq!(code, 1, "{v}");
    let why = v["invalid"][0]["why"].as_str().unwrap();
    assert!(why.starts_with("it lists `test` unread"), "{why}");
}

/// The pull request's own commit reads covered through the merge root beside the
/// developer's partial one, with no reading of the CI gate's; every commit of the rebase
/// is the CI gate's through the tip's root.
#[test]
fn the_merge_root_covers_the_pull_request_and_the_ci_gate_reads_its_first_parent_line() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    let merge = h.ci_root(&h.merge, &[&h.one, &h.merge], &h.zero, &h.one);
    put(&h.repo, &h.ci, &h.merge, &merge);
    let rebase = h.ci_root(&h.r2, &[&h.r1, &h.r2], &h.merge, &h.head);
    put(&h.repo, &h.ci, &h.r2, &rebase);

    let (code, v) = json(&h.repo, &["audit", "--json"]);
    assert_eq!(code, 0, "{v}");
    let read = |c: &str| row(&v, c);
    assert_eq!(
        read(&h.one),
        ("covered".into(), "covers".into(), "-".into())
    );
    assert_eq!(
        read(&h.merge),
        ("covered".into(), "own".into(), "covered".into())
    );
    assert_eq!(
        read(&h.r1),
        ("covered".into(), "covers".into(), "covered".into())
    );
    assert_eq!(
        read(&h.r2),
        ("covered".into(), "own".into(), "covered".into())
    );
    let (_, v) = json(&h.repo, &["verify", "--require-coverage", "--json"]);
    assert_eq!(v["pass"], true, "{v}");

    // A CI root whose base is not on its first-parent line reaches its own commit alone.
    let off = Root {
        base: Some(g(&h.one)),
        covers: Covers::List(vec![g(&h.merge), g(&h.r1), g(&h.r2)]),
        parents: vec![],
        ..rebase
    };
    put(&h.repo, &h.ci, &h.r2, &off);
    let (code, v) = json(&h.repo, &["audit", "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(row(&v, &h.r2).2, "covered", "{v}");
    assert_eq!(
        row(&v, &h.r1),
        ("covered".into(), "covers".into(), "-".into())
    );
    let notes = v["notes"].to_string();
    assert!(notes.contains("not on its first-parent line"), "{notes}");
}

/// Decision 60, in a clone made with `--filter=blob:none`: the roots are fetched whole, the
/// policy blobs are not.
#[test]
fn a_policy_a_blobless_clone_lacks_is_exit_two_with_the_fetch_named_and_a_row_in_audit() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    h.repo.git(&["config", "uploadpack.allowFilter", "true"]);
    h.repo
        .git(&["config", "uploadpack.allowAnySHA1InWant", "true"]);
    let url = format!(
        "file://{}",
        h.repo.path().to_string_lossy().replace('\\', "/")
    );
    let clone = Repo::empty();
    let args = [
        "clone",
        "-q",
        "--filter=blob:none",
        "--no-checkout",
        &url,
        ".",
    ];
    clone.git(&args);
    let refs = "+refs/jtac/*:refs/jtac/*";
    clone.git(&["fetch", "-q", "--filter=blob:limit=1g", "origin", refs]);
    let signers = clone.write(
        ".git/trusted",
        format!("dev@example.com {}\n", pubkey(&h.dev)).as_bytes(),
    );
    let signers = signers.to_str().unwrap();

    let (code, v) = json(&clone, &["audit", &h.one, "--signers", signers, "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(row(&v, &h.one).0, "policy_not_held", "{v}");
    assert_eq!(v["totals"]["policy_not_held"], 2, "{v}");
    assert!(v["notes"].to_string().contains("policy not held"), "{v}");

    // However the clone spells its promisor, or with none at all: the reading asks the
    // objects, not the configuration.
    for spelling in [Some("yes"), None] {
        match spelling {
            Some(value) => clone.git(&["config", "remote.origin.promisor", value]),
            None => clone.git(&["config", "--unset", "remote.origin.promisor"]),
        };
        let (_, v) = json(&clone, &["audit", &h.one, "--signers", signers, "--json"]);
        assert_eq!(row(&v, &h.one).0, "policy_not_held", "{spelling:?}: {v}");
    }
    clone.git(&["config", "remote.origin.promisor", "true"]);

    let ran = jtac(&clone, &["verify", &h.one, "--signers", signers]);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    let blob = h
        .repo
        .git(&["rev-parse", &format!("{}:.jtac/policy.toml", h.one)]);
    let named = ran.stderr.contains("git fetch origin ") && ran.stderr.contains(&blob);
    assert!(named, "{}", ran.stderr);

    let zero = h
        .repo
        .git(&["rev-parse", &format!("{}:.jtac/policy.toml", h.zero)]);
    clone.git(&["fetch", "-q", "origin", &blob, &zero]);
    let (code, v) = json(&clone, &["verify", &h.one, "--signers", signers, "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["partial"], 1, "{v}");
}

/// Section 16.3: a commit read partial is not covered at a release, with and without
/// `--require-ci`, and the merge root is what covers it.
#[test]
fn release_refuses_a_partial_commit_by_name_until_a_merge_root_covers_it() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    h.repo.git(&["tag", "-a", "-m", "t1", "t1", &h.one]);
    for flag in [None, Some("--require-ci")] {
        let args = ["release", "t1", "--from", &h.zero, "--json"];
        let args: Vec<&str> = args.into_iter().chain(flag).collect();
        let (code, v) = json(&h.repo, &args);
        assert_eq!(code, 1, "{v}");
        assert_eq!(v["partial"][0], format!("sha1:{}", h.one), "{v}");
        assert_eq!(v["absent"], serde_json::json!([]), "{v}");
        assert_eq!(v["not_ci"], serde_json::json!([]), "{v}");
    }
    let text = jtac(&h.repo, &["release", "t1", "--from", &h.zero]).stdout;
    assert!(text.contains("the local gate's root alone"), "{text}");
    assert!(
        text.contains("fix       partial: only a merge root"),
        "{text}"
    );

    let merge = h.ci_root(&h.merge, &[&h.one, &h.merge], &h.zero, &h.one);
    put(&h.repo, &h.ci, &h.merge, &merge);
    let (code, v) = json(&h.repo, &["release", "t1", "--from", &h.zero, "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["partial"], serde_json::json!([]), "{v}");
}
