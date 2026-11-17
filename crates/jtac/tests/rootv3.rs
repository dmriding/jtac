//! Root version 3 on a scripted history (DESIGN.md sections 9.5 and 15.2, M20b, decisions
//! 57, 58, 60, 62 and 63). The local gate writes the developer's root; a merge root with a
//! version 3 node has no writer yet, so each is built here and signed, as is any root a
//! real gate would not write. A developer's root under a `ci` check reads partial and one
//! that leaves the check out of `unread` is invalid; the merge root covers the pull request
//! and the CI gate reads its first-parent line alone; a policy a partial clone lacks is
//! exit 2 with the fetch named, and `policy not held` in `audit`; at `release` a CI waiver
//! on a merge made around CI declares off the partial commits it brought in (decision 64).

use std::path::{Path, PathBuf};
use std::process::Command;

use jtac_core::{
    Body, Covers, Forge, FromHead, GitOid, Node, Observed, ObservedCheck, Outcome, Parent, Root,
    RootVersion, Sha256Digest, SignedLever, Signer, State, merkle_root,
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
    jtac_with(repo, args, &[])
}

fn jtac_with(repo: &Repo, args: &[&str], env: &[(&str, &str)]) -> Ran {
    let out = repo.command_with(env!("CARGO_BIN_EXE_jtac"), args, env);
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

fn hex(id: &Sha256Digest) -> String {
    id.to_string().trim_start_matches("sha256:").to_owned()
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

    /// The developer's root on a merge made around CI: partial, carrying the CI waiver
    /// `jtac waive --ci` signs for it with that commit checked out.
    fn waived_root(&self, commit: &str, covers: &[&str], base: &str) -> Root {
        self.repo.git(&["switch", "-q", "--detach", commit]);
        let args = ["waive", commit, "--ci", "--reason", "runners out", "--json"];
        let (code, v) = json(&self.repo, &args);
        assert_eq!(code, 0, "{v}");
        self.repo.git(&["switch", "-q", "main"]);
        let id = v["lever"].as_str().unwrap().trim_start_matches("sha256:");
        let store = self.repo.path().join(".git").join("jtac");
        let node = std::fs::read(store.join("objects").join(id)).unwrap();
        let node: Node = serde_json::from_slice(&node).unwrap();
        let sig = std::fs::read_to_string(store.join("levers").join(format!("{id}.sig")));
        let mut r = Root {
            v: RootVersion::V3,
            unread: vec!["test".into()],
            ts: node.ts,
            levers: vec![SignedLever {
                node,
                sig: sig.unwrap(),
            }],
            ..self.template(commit, covers, base)
        };
        r.counts.waivers = 1;
        r
    }

    /// `test` as the merge job reads it from CI for `r`'s commit, a leaf of `r`.
    fn observed(&self, r: &Root) -> Node {
        let workflow = format!("{}:.github/workflows/ci.yml", r.commit.hex());
        Node {
            tree: r.tree,
            policy: r.policy.hash,
            ts: r.ts,
            body: Body::Observed(ObservedCheck {
                id: "test".into(),
                outcome: Outcome::Pass,
                observed: Observed {
                    forge: Forge::Github,
                    workflow: ".github/workflows/ci.yml".into(),
                    blob: g(&self.repo.git(&["rev-parse", &workflow])),
                    job: Some("test".into()),
                    commit: r.commit,
                    run: 1234,
                    attempt: 1,
                    conclusion: "success".into(),
                },
            }),
        }
    }

    /// The local gate's own root on `head`, a commit under the `ci` policy.
    fn checked(&self) -> Root {
        self.repo.git(&["switch", "-q", "--detach", &self.head]);
        let (code, v) = json(&self.repo, &["check", "--base", &self.merge, "--json"]);
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["unread"], serde_json::json!(["test"]), "{v}");
        root_of(&self.repo, &self.head)
    }

    /// Replace `head`'s root with `r` and the local manifest `replay` reads its leaves by.
    fn plant(&self, r: &Root, leaves: &[Sha256Digest]) {
        put(&self.repo, &self.dev, &self.head, r);
        let dir = self.store().join("roots").join(hex(&r.id().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = serde_json::json!({ "v": 1, "nodes": leaves });
        let bytes = jtac_core::canonical_bytes(&manifest).unwrap();
        std::fs::write(dir.join("nodes.json"), bytes).unwrap();
    }

    fn store(&self) -> PathBuf {
        self.repo.path().join(".git").join("jtac")
    }

    /// A blobless clone of the repository with its roots fetched whole, and the trust file.
    fn partial_clone(&self, filter: &str) -> (Repo, String) {
        self.repo.git(&["config", "uploadpack.allowFilter", "true"]);
        self.repo
            .git(&["config", "uploadpack.allowAnySHA1InWant", "true"]);
        let url = format!(
            "file://{}",
            self.repo.path().to_string_lossy().replace('\\', "/")
        );
        let clone = Repo::empty();
        clone.git(&["clone", "-q", filter, "--no-checkout", &url, "."]);
        let refs = "+refs/jtac/*:refs/jtac/*";
        clone.git(&["fetch", "-q", "--filter=blob:limit=1g", "origin", refs]);
        let signers = clone.write(
            ".git/trusted",
            format!("dev@example.com {}\n", pubkey(&self.dev)).as_bytes(),
        );
        (clone, signers.to_str().unwrap().to_owned())
    }

    /// The merge job's root on `commit` for `head`, holding `test` as read from CI.
    fn ci_root(&self, commit: &str, covers: &[&str], base: &str, head: &str) -> Root {
        let r = self.template(commit, covers, base);
        let observed = self.observed(&r);
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
    let (clone, signers) = h.partial_clone("--filter=blob:none");
    let signers = signers.as_str();

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
    let policy = |commit: &str| {
        let file = format!("{commit}:.jtac/policy.toml");
        h.repo.git(&["rev-parse", &file])
    };
    let (blob, zero) = (policy(&h.one), policy(&h.zero));
    let mut ids = [blob.as_str(), zero.as_str()];
    ids.sort_unstable();
    let fetch = format!("Run `git fetch origin {}`", ids.join(" "));
    assert!(ran.stderr.contains(&fetch), "{}", ran.stderr);

    clone.git(&["fetch", "-q", "origin", &blob, &zero]);
    let (code, v) = json(&clone, &["verify", &h.one, "--signers", signers, "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["partial"], 1, "{v}");
}

/// The writer (M20b): under a policy with a `ci` check the local gate's root is version 3
/// and lists the check unread; it replays, and a second clone reads it `covered, ci
/// unread`. A root under a policy with no `ci` check stays version 2.
#[test]
fn the_local_gate_writes_a_partial_root_that_replays_and_a_second_clone_reads_ci_unread() {
    let h = history();
    let written = h.checked();
    assert_eq!(
        (written.v, &written.unread[..], written.state),
        (RootVersion::V3, &["test".to_owned()][..], State::Covered)
    );
    assert!(written.from.is_none() && written.observed.is_empty());
    let plain = root_of(&h.repo, &h.zero);
    assert_eq!((plain.v, plain.unread.len()), (RootVersion::V2, 0));

    let (code, v) = json(&h.repo, &["replay", &h.head, "--json"]);
    assert_eq!((code, &v["matches"]), (0, &Value::from(true)), "{v}");
    assert!(v["notes"].to_string().contains("partial"), "{v}");

    let second = Repo::clone_of(&h.repo);
    second.git(&["fetch", "-q", "origin", "+refs/jtac/*:refs/jtac/*"]);
    let (code, v) = json(&second, &["audit", &h.head, "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        row(&v, &h.head),
        ("partial".into(), "own".into(), "-".into())
    );
    let text = jtac(&second, &["audit", &h.head]).stdout;
    assert!(text.contains("covered, ci unread own root"), "{text}");
    let (code, v) = json(
        &second,
        &["verify", &h.head, "--require-coverage", "--json"],
    );
    assert_eq!(code, 1, "{v}");
    let partial = v["partial_commits"].to_string();
    assert!(partial.contains(&h.head), "{v}");
}

/// Issue 161 note 1: `replay` builds its facts by the root's shape and holds a root
/// without `from` to its tree's policy. A CI result planted among the leaves of a
/// developer's root is a node for no check, though the Merkle root and the counts were
/// forged to match; and a root that lists nothing unread is invalid before it is replayed.
#[test]
fn replay_counts_no_ci_result_in_a_root_without_from_and_holds_it_to_its_trees_policy() {
    let h = history();
    let real = h.checked();
    let manifest = h.store().join("roots").join(hex(&real.id().unwrap()));
    let manifest = std::fs::read(manifest.join("nodes.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    let leaves: Vec<Sha256Digest> = serde_json::from_value(manifest["nodes"].clone()).unwrap();

    let planted = h.observed(&real);
    let id = planted.id().unwrap();
    let object = h.store().join("objects").join(hex(&id));
    std::fs::write(object, planted.canonical_bytes().unwrap()).unwrap();
    let with: Vec<Sha256Digest> = leaves.iter().copied().chain([id]).collect();
    let mut forged = Root {
        evidence: merkle_root(&with).unwrap(),
        ..real.clone()
    };
    forged.counts.checks += 1;
    forged.counts.pass += 1;
    h.plant(&forged, &with);
    let (code, v) = json(&h.repo, &["replay", &h.head, "--json"]);
    assert_eq!(code, 1, "{v}");
    for part in ["evidence", "counts", "state"] {
        assert_eq!(v[part]["recorded"], v[part]["recomputed"], "{part}: {v}");
    }
    assert_eq!(
        v["mismatches"],
        serde_json::json!(["the verdict is ABORT: check `test` is not in the policy"]),
        "{v}"
    );

    let silent = Root {
        v: RootVersion::V2,
        unread: vec![],
        ..real
    };
    h.plant(&silent, &leaves);
    let (code, v) = json(&h.repo, &["replay", &h.head, "--json"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(
        v["mismatches"],
        serde_json::json!([
            "the root is invalid: it lists no check unread, and the policy in the tree it \
             names leaves `test` to CI"
        ]),
        "{v}"
    );
}

/// Decision 60 for every command that judges, and issue 168: `p1`'s root names a tree whose
/// policy file a blobless clone lacks, and `p0`'s own differs too but no reading turns on
/// it, since the root that covers `p0` is partial whatever `p0` holds. `verify`, `replay`,
/// `jtac ci` and `check --from` each answer 2 and name `p1`'s file alone.
#[test]
fn every_command_that_judges_names_the_one_policy_object_its_answer_turns_on() {
    let h = history();
    h.repo.git(&["switch", "-q", "-c", "pr3", "main"]);
    let edit = |tag: &str| {
        let own = format!("{CI_POLICY}# {tag}\n");
        h.repo.write(".jtac/policy.toml", own.as_bytes());
        h.repo.write(&format!("{tag}.txt"), b"x\n");
        h.repo.commit(tag)
    };
    let (p0, p1) = (edit("p0"), edit("p1"));
    h.repo.write(".jtac/policy.toml", CI_POLICY.as_bytes());
    h.repo.write("p2.txt", b"x\n");
    let p2 = h.repo.commit("p2");
    put(&h.repo, &h.dev, &p1, &h.dev_root(&p1, &p0));
    let wide = Root {
        covers: Covers::List(vec![g(&p0), g(&p1), g(&p2)]),
        ..h.dev_root(&p2, &h.r2)
    };
    put(&h.repo, &h.dev, &p2, &wide);
    h.repo.git(&["switch", "-q", "main"]);
    let merge_args = ["merge", "-q", "--no-ff", "--no-gpg-sign", "-m", "merge pr3"];
    h.repo.git(&[&merge_args[..], &["pr3"]].concat());
    let policy = |commit: &str| {
        let file = format!("{commit}:.jtac/policy.toml");
        h.repo.git(&["rev-parse", &file])
    };
    let (lacked, unasked) = (policy(&p1), policy(&p0));

    let (clone, signers) = h.partial_clone("--filter=blob:none");
    clone.git(&["reset", "-q", "--hard"]);
    let key = h.dev.to_string_lossy().replace('\\', "/");
    clone.git(&["config", "jtac.signingkey", &key]);
    clone.git(&["config", "user.email", "dev@example.com"]);
    let range = format!("{}..{p2}", h.r2);
    let (code, v) = json(&clone, &["audit", &range, "--signers", &signers, "--json"]);
    assert_eq!(code, 0, "{v}");
    let states = [&p0, &p1, &p2].map(|c| row(&v, c).0);
    assert_eq!(states, ["partial", "policy_not_held", "partial"], "{v}");

    let commands: [(&str, Vec<&str>); 4] = [
        ("verify", vec!["verify", &range, "--signers", &signers]),
        ("replay", vec!["replay", &p1, "--signers", &signers]),
        ("ci", vec!["ci", "--base", &h.r2]),
        (
            "check --from",
            vec!["check", "--from", &p2, "--base", &h.r2],
        ),
    ];
    for (command, args) in commands {
        let ran = jtac(&clone, &args);
        assert_eq!(
            (ran.code, ran.stdout.as_str()),
            (2, ""),
            "{command}: {}",
            ran.stderr
        );
        let fetch = format!("Run `git fetch origin {lacked}`, then `jtac {command}` again");
        assert!(ran.stderr.contains(&fetch), "{command}: {}", ran.stderr);
        assert!(!ran.stderr.contains(&unasked), "{command}: {}", ran.stderr);
    }
}

/// Issue 168: in a `--filter=tree:1` clone a commit's tree is here and the directory that
/// holds the policy is not. The fetch names that directory, the object git stopped at, and
/// never the commit's tree, which the clone holds.
#[test]
fn a_clone_that_lacks_the_policy_directory_is_told_to_fetch_that_tree_and_no_other() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    let (clone, signers) = h.partial_clone("--filter=tree:1");
    let range = format!("{}..{}", h.zero, h.one);
    let args = ["verify", range.as_str(), "--signers", signers.as_str()];
    let dir = h.repo.git(&["rev-parse", &format!("{}:.jtac", h.one)]);
    let tree = h.repo.git(&["rev-parse", &format!("{}^{{tree}}", h.one)]);
    let ran = jtac(&clone, &args);
    assert_eq!(ran.code, 2, "{}{}", ran.stdout, ran.stderr);
    let fetch = format!("Run `git fetch origin {dir}`");
    assert!(ran.stderr.contains(&fetch), "{}", ran.stderr);
    assert!(!ran.stderr.contains(&tree), "{}", ran.stderr);
    clone.git(&["fetch", "-q", "origin", &dir]);
    let (code, v) = json(&clone, &[&args[..], &["--json"]].concat());
    assert_eq!((code, &v["partial"]), (0, &Value::from(1)), "{v}");
}

/// A root may name any id as its tree, and every root is read before one is judged. One
/// that names a blob is invalid by its tree, whoever signed it, and stops no reader.
#[test]
fn a_root_that_names_a_blob_as_its_tree_is_invalid_and_stops_no_reader() {
    let h = history();
    let blob = h.repo.git(&["rev-parse", &format!("{}:.gitignore", h.one)]);
    let forged = Root {
        tree: g(&blob),
        ..h.dev_root(&h.one, &h.zero)
    };
    let stranger = make_key(&h.repo, "stranger", "stranger@example.com");
    put(&h.repo, &stranger, &h.one, &forged);
    let (code, v) = json(&h.repo, &["audit", &h.one, "--json"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(row(&v, &h.one).0, "absent", "{v}");
    let (code, v) = json(&h.repo, &["verify", &h.one, "--json"]);
    assert_eq!(code, 1, "{v}");
    let why = v["invalid"][0]["why"].as_str().unwrap();
    assert!(
        why.starts_with(&format!("it names tree sha1:{blob}, not")),
        "{why}"
    );
}

/// Issue 168: git takes pathspec magic from the environment, where `icase` matches another
/// file and `ls-tree` refuses it and `glob`. No git child inherits either, so the gate
/// loads the policy and the reader reads the tree's as without them.
#[test]
fn a_pathspec_variable_in_the_environment_changes_no_reading() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    let audit = ["audit", &h.one, "--json"];
    let check = ["check", "--preview", "--base", &h.merge];
    let plain = jtac(&h.repo, &audit);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    for var in ["GIT_ICASE_PATHSPECS", "GIT_GLOB_PATHSPECS"] {
        let ran = jtac_with(&h.repo, &audit, &[(var, "1")]);
        assert_eq!(
            (ran.code, &ran.stdout),
            (0, &plain.stdout),
            "{var}: {}",
            ran.stderr
        );
        let ran = jtac_with(&h.repo, &check, &[(var, "1")]);
        assert_eq!(ran.code, 0, "{var}: {}{}", ran.stdout, ran.stderr);
    }
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

/// Decision 64, issue 169: a two-parent merge made around CI. The waiver in the merge's own
/// root declares off the partial commit the merge brought in, whether that commit holds a
/// root of its own or is read through the merge's `covers`, as a fork's is; a tag on that
/// commit, its merge outside the range, is refused as before.
#[test]
fn a_ci_waiver_on_a_merge_made_around_ci_recovers_the_pull_request_at_release() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    h.repo.git(&["tag", "-a", "-m", "t1", "t1", &h.one]);
    h.repo.git(&["tag", "-a", "-m", "t2", "t2", &h.merge]);
    let release = |tag: &str, flag: Option<&str>| {
        let args = ["release", tag, "--from", &h.zero, "--json"];
        let args: Vec<&str> = args.into_iter().chain(flag).collect();
        json(&h.repo, &args)
    };
    let listed = |v: &Value| -> Vec<String> {
        let mut hexes: Vec<String> = (v.as_array().unwrap().iter())
            .map(|c| c.as_str().unwrap().trim_start_matches("sha1:").to_owned())
            .collect();
        hexes.sort();
        hexes
    };
    let mut both = vec![h.one.clone(), h.merge.clone()];
    both.sort();

    let (code, v) = release("t2", None);
    assert_eq!(code, 1, "{v}");
    assert_eq!(listed(&v["partial"]), [h.one.as_str()], "{v}");
    assert_eq!(listed(&v["absent"]), [h.merge.as_str()], "{v}");

    let own = h.waived_root(&h.merge, &[&h.merge], &h.zero);
    let fork = Root {
        covers: Covers::List(vec![g(&h.one), g(&h.merge)]),
        ..own.clone()
    };
    put(&h.repo, &h.dev, &h.merge, &own);
    for flag in [None, Some("--require-ci")] {
        let (code, v) = release("t2", flag);
        assert_eq!(code, 0, "{flag:?}: {v}");
        assert_eq!(listed(&v["waived"]), both, "{flag:?}: {v}");
        let through = serde_json::json!([{
            "commit": format!("sha1:{}", h.one),
            "merge": format!("sha1:{}", h.merge),
        }]);
        assert_eq!(v["through"], through, "{flag:?}: {v}");
        assert_eq!(v["roots"].as_array().unwrap().len(), 2, "{flag:?}: {v}");
        let (code, v) = release("t1", flag);
        assert_eq!(code, 1, "{flag:?}: {v}");
        assert_eq!(listed(&v["partial"]), [h.one.as_str()], "{flag:?}: {v}");
    }
    let (_, v) = json(&h.repo, &["audit", &h.merge, "--json"]);
    assert_eq!(
        row(&v, &h.one),
        ("partial".into(), "own".into(), "-".into()),
        "the CI gate's reading of the pull request's commit does not move"
    );
    assert_eq!(row(&v, &h.merge).2, "declared_off", "{v}");

    let one = format!("refs/jtac/roots/{}/{}", &h.one[..2], h.one);
    h.repo.git(&["update-ref", "-d", &one]);
    put(&h.repo, &h.dev, &h.merge, &fork);
    let (code, v) = release("t2", None);
    assert_eq!(code, 0, "{v}");
    assert_eq!(listed(&v["waived"]), both, "{v}");
    assert_eq!(v["roots"].as_array().unwrap().len(), 1, "{v}");

    let text = jtac(&h.repo, &["release", "t2", "--from", &h.zero]).stdout;
    let line = format!(
        "  waived    {} through the CI waiver on {}\n",
        &h.one[..12],
        &h.merge[..12]
    );
    assert!(text.contains(&line), "{text}");
    let text = jtac(&h.repo, &["release", "t1", "--from", &h.zero]).stdout;
    assert!(
        text.contains("jtac waive <merge> --ci --reason <why>")
            && text.contains("do not waive those commits one by one"),
        "{text}"
    );
}

/// Decision 64's reach is what the waived merge brought in and no more: behind a later
/// merge that carries the waiver, an earlier pull request whose own merge holds no root
/// stays partial, since it is reachable from the waived merge's first parent.
#[test]
fn a_ci_waiver_on_a_merge_recovers_no_pull_request_an_earlier_merge_brought_in() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    put(&h.repo, &h.dev, &h.head, &h.dev_root(&h.head, &h.merge));
    let merge_args = ["merge", "-q", "--no-ff", "--no-gpg-sign", "-m", "later"];
    h.repo.git(&[&merge_args[..], &["pr2"]].concat());
    let later = h.repo.git(&["rev-parse", "HEAD"]);
    put(
        &h.repo,
        &h.dev,
        &later,
        &h.waived_root(&later, &[&later], &h.r2),
    );
    h.repo.git(&["tag", "-a", "-m", "t3", "t3", &later]);

    let (code, v) = json(&h.repo, &["release", "t3", "--from", &h.zero, "--json"]);
    assert_eq!(code, 1, "{v}");
    let listed = |key: &str| -> Vec<String> {
        let mut hexes: Vec<String> = (v[key].as_array().unwrap().iter())
            .map(|c| c.as_str().unwrap().trim_start_matches("sha1:").to_owned())
            .collect();
        hexes.sort();
        hexes
    };
    let sorted = |hexes: &[&String]| {
        let mut hexes: Vec<String> = hexes.iter().map(|h| (*h).clone()).collect();
        hexes.sort();
        hexes
    };
    assert_eq!(listed("waived"), sorted(&[&h.head, &later]), "{v}");
    assert_eq!(listed("partial"), sorted(&[&h.one]), "{v}");
    assert_eq!(listed("absent"), sorted(&[&h.merge, &h.r1, &h.r2]), "{v}");
}

/// Decision 64's stated limit: the first-parent line is all a reader has of where the
/// branch was. `one` stands for the branch's earlier commit, its root standing; a merge
/// made by hand on `side` with `side` as first parent reads `one` as what it brought in,
/// and its waiver reaches it, with the merge named. A direct push or a forge that
/// fast-forwards lands this shape; the fence is the forge making every commit on the
/// branch (DESIGN.md section 16.3).
#[test]
fn a_merge_with_its_parents_reversed_reads_the_old_line_as_the_side_it_brought_in() {
    let h = history();
    put(&h.repo, &h.dev, &h.one, &h.dev_root(&h.one, &h.zero));
    h.repo.git(&["switch", "-q", "-c", "side", &h.zero]);
    h.repo.write("x.txt", b"x\n");
    let side = h.repo.commit("side");
    let ran = jtac(&h.repo, &["check"]);
    assert_eq!(ran.code, 0, "{}{}", ran.stdout, ran.stderr);
    let merge_args = ["merge", "-q", "--no-ff", "--no-gpg-sign", "-m", "reversed"];
    h.repo.git(&[&merge_args[..], &["feature"]].concat());
    let reversed = h.repo.git(&["rev-parse", "HEAD"]);
    let parents = h.repo.git(&["rev-parse", "HEAD^1", "HEAD^2"]);
    assert_eq!(parents, format!("{side}\n{}", h.one));
    put(
        &h.repo,
        &h.dev,
        &reversed,
        &h.waived_root(&reversed, &[&reversed], &side),
    );
    h.repo.git(&["tag", "-a", "-m", "tr", "tr", &reversed]);

    let (code, v) = json(&h.repo, &["release", "tr", "--from", &h.zero, "--json"]);
    assert_eq!(code, 0, "{v}");
    let through = serde_json::json!([{
        "commit": format!("sha1:{}", h.one),
        "merge": format!("sha1:{reversed}"),
    }]);
    assert_eq!(v["through"], through, "{v}");
    assert_eq!(
        (&v["covered"], &v["declared_off"]),
        (&1.into(), &2.into()),
        "{v}"
    );
}
