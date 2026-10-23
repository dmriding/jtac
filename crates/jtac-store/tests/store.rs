//! The store against real repositories: the M7 done-when (a root written on one clone is
//! fetched and read on another), then every way a stored thing can be wrong.

use jtac_checks::{Cache, CacheKey, lookup, store};
use jtac_core::{
    Body, Capture, CheckBody, Counts, Covers, GateRef, GitOid, Node, Outcome, PolicyRef,
    PolicySource, Root, RootVersion, Sha256Digest, Signer, State, Ts,
};
use jtac_store::{PushOutcome, Store, StoreError, ref_name};
use jtac_testkit::Repo;

const EMPTY: &str = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const TS: &str = "2026-10-10T18:30:00Z";
const SIG: &[u8] = b"-----BEGIN SSH SIGNATURE-----\nexample\n-----END SSH SIGNATURE-----\n";
const REFSPEC: &str = "+refs/jtac/*:refs/jtac/*";

fn oid(repo: &Repo, spec: &str) -> GitOid {
    let format = repo.git(&["rev-parse", "--show-object-format"]);
    let hex = repo.git(&["rev-parse", "--verify", spec]);
    format!("{format}:{hex}").parse().unwrap()
}

fn root(repo: &Repo, ts: &str) -> Root {
    let commit = oid(repo, "HEAD^{commit}");
    Root {
        v: RootVersion::V1,
        base: None,
        from: None,
        commit,
        tree: oid(repo, "HEAD^{tree}"),
        ts: ts.parse().unwrap(),
        covers: Covers::List(vec![commit]),
        parents: vec![],
        policy: PolicyRef {
            hash: EMPTY.parse().unwrap(),
            version: 1,
            source: PolicySource::Repo,
        },
        gate: GateRef {
            version: "0.1.0".into(),
            binary: EMPTY.parse().unwrap(),
        },
        state: State::Covered,
        evidence: EMPTY.parse().unwrap(),
        levers: vec![],
        counts: Counts::default(),
        signer: Signer {
            principal: "dev@example.com".into(),
            fingerprint: "SHA256:AAAA".into(),
        },
    }
}

fn node(tool_version: &str) -> Node {
    let empty = Capture {
        sha: EMPTY.parse().unwrap(),
        bytes: 0,
        truncated: false,
    };
    Node {
        tree: GitOid::EMPTY_TREE_SHA1.parse().unwrap(),
        policy: EMPTY.parse().unwrap(),
        ts: TS.parse().unwrap(),
        body: Body::Check(CheckBody {
            id: "cargo-test".into(),
            argv: vec!["cargo".into(), "test".into()],
            tool_version: tool_version.into(),
            exit: 0,
            outcome: Outcome::Pass,
            stdout: empty.clone(),
            stderr: empty,
            duration_ms: 1234,
            cached_from: None,
        }),
    }
}

fn with_commit(options: &[&str]) -> Repo {
    let repo = Repo::init(options);
    repo.write("README.md", b"example\n");
    repo.commit("first");
    repo
}

fn corrupt(e: &StoreError) -> bool {
    matches!(e, StoreError::Corrupt { .. })
}

fn refused(e: &StoreError) -> bool {
    matches!(e, StoreError::Ref { .. })
}

#[test]
fn a_root_written_on_one_clone_is_fetched_and_read_on_another() {
    for options in [&[][..], &["--object-format=sha256"]] {
        let origin = Repo::bare(options);
        let a = Repo::clone_of(&origin);
        a.write("README.md", b"example\n");
        a.commit("first");
        a.git(&["push", "-q", "origin", "main"]);
        let store_a = Store::open(a.path()).unwrap();
        let root_a = root(&a, TS);
        let leaf = store_a.put_node(&node("1.98.0")).unwrap();
        let id = store_a.put_root(&root_a, SIG, &[leaf]).unwrap();
        assert_eq!(id, root_a.id().unwrap());
        let pushed = store_a.push("origin", &[root_a.commit], &[]).unwrap();
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].outcome, PushOutcome::Sent);
        assert_eq!(pushed[0].name, ref_name(&root_a.commit));

        let b = Repo::clone_of(&origin);
        b.git(&["fetch", "-q", "origin", REFSPEC]);
        let store_b = Store::open(b.path()).unwrap();
        let got = store_b.root(&root_a.commit).unwrap().unwrap();
        assert_eq!(got.root, root_a);
        assert_eq!(got.sig, SIG);
        assert_eq!(store_b.local_root(&id).unwrap(), None);
        assert_eq!(store_b.manifest(&id).unwrap(), None);
        assert_eq!(store_b.node(&leaf).unwrap(), None);
        let listed = store_b.roots().unwrap().listed;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].commit, root_a.commit);
        assert_eq!(
            listed[0].root_commit.hex(),
            b.git(&["rev-parse", &ref_name(&root_a.commit)])
        );
    }
}

#[test]
fn the_local_copy_and_manifest_serve_replay() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let r = root(&repo, TS);
    let leaves = [
        s.put_node(&node("1.98.0")).unwrap(),
        s.put_node(&node("1.99.0")).unwrap(),
    ];
    let id = s.put_root(&r, SIG, &leaves).unwrap();
    let local = s.local_root(&id).unwrap().unwrap();
    assert_eq!((local.root, local.sig.as_slice()), (r.clone(), SIG));
    assert_eq!(s.manifest(&id).unwrap().unwrap(), leaves);
    assert_eq!(s.node(&leaves[1]).unwrap().unwrap(), node("1.99.0"));
    assert_eq!(s.root(&r.commit).unwrap().unwrap().root, r);
}

#[test]
fn objects_round_trip_and_a_file_that_does_not_hash_to_its_name_is_refused() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let output = b"test result: ok\n";
    let sha = s.put_blob(output).unwrap();
    assert_eq!(sha, Sha256Digest::of(output));
    assert_eq!(s.blob(&sha).unwrap().unwrap(), output);
    assert_eq!(s.blob(&EMPTY.parse().unwrap()).unwrap(), None);
    let id = s.put_node(&node("1.98.0")).unwrap();
    let hex = id.to_string()[Sha256Digest::PREFIX.len()..].to_owned();
    let path = s.dir().join("objects").join(hex);
    std::fs::write(&path, b"{}").unwrap();
    assert!(corrupt(&s.node(&id).unwrap_err()));
    assert!(corrupt(&s.blob(&id).unwrap_err()));
    assert_eq!(s.put_node(&node("1.98.0")).unwrap(), id);
    assert_eq!(s.node(&id).unwrap().unwrap(), node("1.98.0"));
}

#[test]
fn a_local_root_with_edited_json_a_wrong_manifest_version_or_no_sig_is_refused() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let r = root(&repo, TS);
    let id = s.put_root(&r, SIG, &[]).unwrap();
    let dir = s.dir().join("roots").join(&id.to_string()[7..]);
    std::fs::write(dir.join("nodes.json"), b"{\"nodes\":[],\"v\":2}").unwrap();
    assert!(corrupt(&s.manifest(&id).unwrap_err()));
    std::fs::remove_file(dir.join("root.sig")).unwrap();
    assert!(corrupt(&s.local_root(&id).unwrap_err()));
    std::fs::write(dir.join("root.json"), b"{}").unwrap();
    assert!(corrupt(&s.local_root(&id).unwrap_err()));
}

#[test]
fn the_root_commit_is_parentless_unsigned_and_the_same_for_the_same_root() {
    let repo = with_commit(&[]);
    repo.git(&["config", "commit.gpgsign", "true"]);
    repo.git(&["config", "gpg.program", "/nonexistent/gpg"]);
    let s = Store::open(repo.path()).unwrap();
    let r = root(&repo, TS);
    s.put_root(&r, SIG, &[]).unwrap();
    let name = ref_name(&r.commit);
    let first = repo.git(&["rev-parse", &name]);
    let object = repo.git(&["cat-file", "-p", &first]);
    assert!(!object.contains("parent "), "{object}");
    assert!(!object.contains("gpgsig"), "{object}");
    assert!(
        object.contains("author jtac <dev@example.com> 1791657000 +0000"),
        "{object}"
    );
    assert!(object.ends_with(&r.id().unwrap().to_string()), "{object}");
    assert_eq!(
        repo.git(&["ls-tree", "--name-only", &first]),
        "root.json\nroot.sig"
    );
    let other = Repo::clone_of(&repo);
    other.git(&["config", "i18n.commitEncoding", "ISO-8859-1"]);
    Store::open(other.path())
        .unwrap()
        .put_root(&r, SIG, &[])
        .unwrap();
    assert_eq!(other.git(&["rev-parse", &name]), first);
}

/// Issue 27: two checks on one tip raced and the second `update-ref` replaced the first,
/// so one run's printed root was not the ref's. The ref is created with an empty old value.
#[test]
fn a_root_ref_is_created_never_replaced() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let r = root(&repo, TS);
    let id = s.put_root(&r, SIG, &[]).unwrap();
    let name = ref_name(&r.commit);
    let first = repo.git(&["rev-parse", &name]);
    let later = Root {
        ts: "2026-10-11T18:30:00Z".parse().unwrap(),
        ..r.clone()
    };
    for again in [&r, &later] {
        let err = s.put_root(again, SIG, &[]).unwrap_err();
        assert!(
            matches!(&err, StoreError::Exists { name: n } if *n == name),
            "{err}"
        );
        assert_eq!(repo.git(&["rev-parse", &name]), first);
        let again = again.id().unwrap();
        assert_eq!(s.local_root(&again).unwrap().is_some(), again == id);
    }
}

#[test]
fn put_root_refuses_what_would_not_read_back_and_writes_nothing() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let r = root(&repo, TS);
    let foreign = Repo::init(&[]);
    foreign.write("foreign.md", b"a commit this repository never had\n");
    foreign.commit("foreign");
    repo.git(&["tag", "-a", "-m", "tagged", "v0"]);
    let tag = oid(&repo, "v0");
    let rows = [
        (
            "counts.waivers without a lever",
            Root {
                counts: Counts {
                    waivers: 1,
                    ..Counts::default()
                },
                ..r.clone()
            },
        ),
        ("a commit this repository does not have", root(&foreign, TS)),
        (
            "an annotated tag where the commit goes",
            Root {
                commit: tag,
                covers: Covers::List(vec![tag]),
                ..r.clone()
            },
        ),
        (
            "a ts before 1970",
            Root {
                ts: "1969-12-31T23:59:00Z".parse().unwrap(),
                ..r.clone()
            },
        ),
    ];
    for (what, bad) in rows {
        let err = s.put_root(&bad, SIG, &[]).expect_err(what);
        assert!(matches!(err, StoreError::Invalid { .. }), "{what}: {err}");
        assert_eq!(s.local_root(&bad.id().unwrap()).unwrap(), None, "{what}");
        assert_eq!(s.root(&bad.commit).unwrap(), None, "{what}");
    }
    assert!(!s.dir().exists());
}

#[test]
fn a_ref_whose_content_is_not_the_root_for_its_commit_is_refused() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let r = root(&repo, TS);
    s.put_root(&r, SIG, &[]).unwrap();
    let root_commit = repo.git(&["rev-parse", &ref_name(&r.commit)]);
    repo.write("other.md", b"example\n");
    repo.commit("second");
    let other = oid(&repo, "HEAD^{commit}");
    assert_eq!(s.root(&other).unwrap(), None);
    repo.git(&["update-ref", &ref_name(&other), &root_commit]);
    assert!(refused(&s.root(&other).unwrap_err()));
    for not_a_commit in ["HEAD:README.md", "HEAD^{tree}"] {
        repo.git(&["update-ref", &ref_name(&other), not_a_commit]);
        assert!(refused(&s.root(&other).unwrap_err()), "{not_a_commit}");
    }
}

/// The ref is pointed at a commit whose tree holds `bytes` as `root.json`.
fn plant(repo: &Repo, name: &str, bytes: &[u8]) {
    repo.write("root.json", bytes);
    repo.write("root.sig", SIG);
    repo.git(&["add", "root.json", "root.sig"]);
    let tree = repo.git(&["write-tree"]);
    let commit = repo.git(&["commit-tree", "--no-gpg-sign", "-m", "planted", &tree]);
    repo.git(&["update-ref", name, &commit]);
}

#[test]
fn a_root_blob_that_is_not_canonical_or_does_not_validate_is_refused() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let r = root(&repo, TS);
    let name = ref_name(&r.commit);
    let bad_counts = Root {
        counts: Counts {
            waivers: 1,
            ..Counts::default()
        },
        ..r.clone()
    };
    let rows = [
        ("pretty printed", serde_json::to_vec_pretty(&r).unwrap()),
        (
            "counts.waivers without a lever",
            bad_counts.canonical_bytes().unwrap(),
        ),
        ("not json", b"nope".to_vec()),
    ];
    for (what, bytes) in rows {
        plant(&repo, &name, &bytes);
        assert!(refused(&s.root(&r.commit).unwrap_err()), "{what}");
    }
    plant(&repo, &name, &r.canonical_bytes().unwrap());
    assert_eq!(s.root(&r.commit).unwrap().unwrap().root, r);
}

#[test]
fn push_reports_each_ref_and_a_second_root_for_the_same_tip_is_rejected() {
    let origin = Repo::bare(&[]);
    let a = Repo::clone_of(&origin);
    a.write("README.md", b"example\n");
    a.commit("first");
    a.git(&["push", "-q", "origin", "main"]);
    let store_a = Store::open(a.path()).unwrap();
    let r = root(&a, TS);
    store_a.put_root(&r, SIG, &[]).unwrap();
    assert_eq!(
        store_a.push("origin", &[r.commit], &[]).unwrap()[0].outcome,
        PushOutcome::Sent
    );
    assert_eq!(
        store_a.push("origin", &[r.commit], &[]).unwrap()[0].outcome,
        PushOutcome::UpToDate
    );

    let b = Repo::clone_of(&origin);
    let store_b = Store::open(b.path()).unwrap();
    let later = Root {
        ts: "2026-10-11T18:30:00Z".parse().unwrap(),
        ..root(&b, TS)
    };
    store_b.put_root(&later, SIG, &[]).unwrap();
    let pushed = store_b.push("origin", &[later.commit], &[]).unwrap();
    assert!(
        matches!(&pushed[0].outcome, PushOutcome::Rejected { summary } if summary.contains("rejected")),
        "{pushed:?}"
    );
    assert_eq!(
        origin.git(&["rev-parse", &ref_name(&r.commit)]),
        a.git(&["rev-parse", &ref_name(&r.commit)]),
        "a rejected push changes nothing on the remote"
    );

    let err = store_b.push("nowhere", &[later.commit], &[]).unwrap_err();
    assert!(matches!(err, StoreError::Git(_)), "{err}");

    b.write("ahead.md", b"example\n");
    b.commit("ahead of origin");
    let before = origin.git(&["rev-parse", "main"]);
    assert_eq!(store_b.push("origin", &[], &[]).unwrap(), vec![]);
    assert_eq!(
        origin.git(&["rev-parse", "main"]),
        before,
        "an empty push sends nothing"
    );
    let twice = store_a.push("origin", &[r.commit, r.commit], &[]).unwrap();
    assert_eq!(twice.len(), 1);
    assert_eq!(twice[0].outcome, PushOutcome::UpToDate);
}

#[test]
fn push_hands_its_env_to_the_child_git_and_so_to_its_hooks() {
    let origin = Repo::bare(&[]);
    let a = Repo::clone_of(&origin);
    a.write("README.md", b"example\n");
    a.commit("first");
    let store = Store::open(a.path()).unwrap();
    let r = root(&a, TS);
    store.put_root(&r, SIG, &[]).unwrap();
    let hooks = a.path().join(a.git(&["rev-parse", "--git-path", "hooks"]));
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-push");
    std::fs::write(&hook, "#!/bin/sh\nprintf '%s' \"$JTAC_HOOK\" > hook-env\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let pushed = store
        .push("origin", &[r.commit], &[("JTAC_HOOK", "1")])
        .unwrap();
    assert_eq!(pushed[0].outcome, PushOutcome::Sent);
    let seen = std::fs::read_to_string(a.path().join("hook-env")).unwrap();
    assert_eq!(seen, "1");
}

#[test]
fn the_store_lives_in_the_common_dir_so_worktrees_share_it() {
    let repo = with_commit(&[]);
    let wt = repo.path().join("wt");
    repo.git(&["worktree", "add", "-q", wt.to_str().unwrap()]);
    let main = Store::open(repo.path()).unwrap();
    let linked = Store::open(&wt).unwrap();
    assert_eq!(main.dir(), linked.dir());
    let canonical = |p: &std::path::Path| std::fs::canonicalize(p).unwrap();
    assert_eq!(
        canonical(main.dir().parent().unwrap()),
        canonical(&repo.path().join(".git")),
        "git prints the long path with forward slashes; the temp dir may be the short form"
    );
    assert_eq!(main.dir().file_name().unwrap(), "jtac");
    assert!(!main.dir().exists(), "open creates nothing");
    let r = root(&repo, TS);
    linked.put_root(&r, SIG, &[]).unwrap();
    assert_eq!(main.root(&r.commit).unwrap().unwrap().root, r);
    assert!(Store::open(&repo.path().join("missing")).is_err());
}

#[test]
fn the_file_cache_round_trips_and_a_bad_file_is_a_miss() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let mut cache = s.cache();
    let original = node("1.98.0");
    assert!(store(&mut cache, &original).unwrap());
    let key = CacheKey::of(&original).unwrap();
    let now: Ts = "2026-10-11T18:30:00Z".parse().unwrap();
    let hit = lookup(&cache, &key, now).unwrap().unwrap();
    let Body::Check(body) = &hit.body else {
        unreachable!()
    };
    assert_eq!(body.cached_from, Some(original.id().unwrap()));
    let name = key.name().unwrap();
    let path = s.dir().join("cache").join(&name.to_string()[7..]);
    assert!(path.is_file());
    std::fs::write(&path, b"not a node").unwrap();
    assert_eq!(cache.get(&name), None);
    assert_eq!(lookup(&cache, &key, now).unwrap(), None);
    std::fs::remove_dir_all(s.dir().join("cache")).unwrap();
    std::fs::write(s.dir().join("cache"), b"a file where the dir goes").unwrap();
    assert!(cache.put(name, original).is_err());
}

/// A ref nobody wrote through jtac is noise: listing it as skipped keeps every reader
/// judging the tip, and the writer's own refs still list (issue 30). Its name is the
/// pusher's choice, so the command the note names must not run it through `sh`, and a
/// name that is not UTF-8 is skipped like any other.
#[test]
fn roots_skips_a_ref_the_writer_would_not_write_with_a_note() {
    let repo = with_commit(&[]);
    let s = Store::open(repo.path()).unwrap();
    let head = repo.git(&["rev-parse", "HEAD"]);
    let good = ref_name(&oid(&repo, "HEAD"));
    let wrong_shard = if head.starts_with("ff") { "00" } else { "ff" };
    let rows = [
        "refs/jtac/roots/zz/not-a-commit",
        &format!("refs/jtac/roots/zz/q/{head}"),
        &format!("refs/jtac/roots/{wrong_shard}/{head}"),
        &format!("refs/jtac/roots/{}/{head}", &head[..1]),
        &format!("refs/jtac/roots/{}/{head}", &head[..3]),
        &format!("refs/jtac/roots/{head}"),
        &format!(
            "refs/jtac/roots/{}/{}",
            &head[..2],
            "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321"
        ),
    ];
    repo.git(&["update-ref", &good, &head]);
    for name in rows {
        repo.git(&["update-ref", name, &head]);
        let roots = s.roots().unwrap();
        assert_eq!(roots.skipped, vec![name.as_bytes().to_vec()]);
        assert_eq!(roots.listed.len(), 1, "{name}");
        let note = roots.notes().next().unwrap();
        assert!(
            note.contains(name) && note.contains("git update-ref -d"),
            "{note}"
        );
        repo.git(&["update-ref", "-d", name]);
    }
    let roots = s.roots().unwrap();
    assert!(roots.skipped.is_empty() && roots.listed.len() == 1);
    let plain = "refs/jtac/roots/zz/junk";
    repo.git(&["update-ref", plain, &head]);
    let note = s.roots().unwrap().notes().next().unwrap();
    assert_eq!(
        note.split('`').nth(1).unwrap(),
        "git update-ref -d refs/jtac/roots/zz/junk",
        "{note}"
    );
    repo.git(&["update-ref", "-d", plain]);
    for hostile in [
        "refs/jtac/roots/zz/$(touch${IFS}PWNED)/it's",
        "refs/jtac/roots/zz/x`touch${IFS}PWNED`y",
    ] {
        repo.git(&["update-ref", hostile, &head]);
        let note = s.roots().unwrap().notes().next().unwrap();
        assert!(
            !note.contains("touch") && !note.contains("zz/") && note.contains("for-each-ref"),
            "{note}"
        );
        repo.git(&["update-ref", "-d", hostile]);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let raw = std::ffi::OsStr::from_bytes(b"refs/jtac/roots/zz/\xff");
        let ok = std::process::Command::new("git")
            .arg("update-ref")
            .arg(raw)
            .arg(&head)
            .current_dir(repo.path())
            .env_clear()
            .envs(repo.env())
            .status()
            .unwrap();
        assert!(ok.success());
        let roots = s.roots().unwrap();
        assert_eq!(roots.skipped, vec![b"refs/jtac/roots/zz/\xff".to_vec()]);
        assert_eq!(roots.listed.len(), 1);
        let note = roots.notes().next().unwrap();
        assert!(
            note.contains("git for-each-ref refs/jtac/roots/") && !note.contains("zz/"),
            "{note}"
        );
    }
}
