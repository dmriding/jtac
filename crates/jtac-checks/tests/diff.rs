//! `diff::git` against a throwaway repository: paths, renames, blobs, binaries, and refusals.
//! No real git config is read.

use std::path::{Path, PathBuf};
use std::process::Command;

use jtac_checks::diff::git::{GitError, changes, merge_base};
use jtac_checks::diff::{Blob, Change, report};
use jtac_policy::Integrity;

struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn init(options: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = Self { dir };
        let mut args = vec!["init", "-q", "-b", "main"];
        args.extend(options);
        repo.git(&args);
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(self.path())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "example")
            .env("GIT_AUTHOR_EMAIL", "example@example.com")
            .env("GIT_COMMITTER_NAME", "example")
            .env("GIT_COMMITTER_EMAIL", "example@example.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn write(&self, path: &str, bytes: &[u8]) -> PathBuf {
        let full = self.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, bytes).unwrap();
        full
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-gpg-sign", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }
}

#[test]
fn changes_pair_renames_read_both_blobs_and_mark_binaries() {
    let repo = Repo::init(&[]);
    repo.write("src/a.rs", b"fn a() {}\n");
    repo.write("src/gone.rs", b"fn gone() {}\n");
    repo.write(
        "docs/old.md",
        b"# title\n\nbody that is long enough to survive a rename\n",
    );
    repo.write("img.bin", b"\x00\x01\x02");
    let base = repo.commit("base");
    repo.write("src/a.rs", b"fn a() {}\nfn b() {}\n");
    std::fs::remove_file(repo.path().join("src/gone.rs")).unwrap();
    std::fs::rename(
        repo.path().join("docs/old.md"),
        repo.path().join("docs/new.md"),
    )
    .unwrap();
    repo.write("img.bin", b"\x00\x01\x03");
    repo.write("new.txt", b"hello\n");
    let tip = repo.commit("tip");
    let got = changes(repo.path(), &base, &tip).unwrap();
    let text = |s: &str| Some(Blob::Text(s.to_owned()));
    assert_eq!(
        got,
        [
            Change {
                path: "docs/new.md".into(),
                from: Some("docs/old.md".into()),
                old: text("# title\n\nbody that is long enough to survive a rename\n"),
                new: text("# title\n\nbody that is long enough to survive a rename\n"),
            },
            Change {
                path: "img.bin".into(),
                from: None,
                old: Some(Blob::Binary),
                new: Some(Blob::Binary),
            },
            Change {
                path: "new.txt".into(),
                from: None,
                old: None,
                new: text("hello\n"),
            },
            Change {
                path: "src/a.rs".into(),
                from: None,
                old: text("fn a() {}\n"),
                new: text("fn a() {}\nfn b() {}\n"),
            },
            Change {
                path: "src/gone.rs".into(),
                from: None,
                old: text("fn gone() {}\n"),
                new: None,
            },
        ]
    );
    assert_eq!(merge_base(repo.path(), &base, &tip).unwrap(), base);
    assert!(changes(repo.path(), &tip, &tip).unwrap().is_empty());
    let policy = jtac_policy::parse("version = 1\n").unwrap();
    let report = report(&Integrity::default(), &policy.matchers().unwrap(), &got);
    assert_eq!(report.added_lines, 2);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

#[test]
fn a_sha256_repository_and_a_gitlink_read_as_absent_sides() {
    let repo = Repo::init(&["--object-format=sha256"]);
    repo.write("alpha.txt", b"alpha\n");
    repo.write("lib", b"x\n");
    let base = repo.commit("base");
    assert_eq!(base.len(), 64);
    std::fs::remove_file(repo.path().join("alpha.txt")).unwrap();
    std::fs::remove_file(repo.path().join("lib")).unwrap();
    repo.git(&["rm", "-q", "--cached", "alpha.txt", "lib"]);
    let gitlink = format!("160000,{base},lib");
    repo.git(&["update-index", "--add", "--cacheinfo", &gitlink]);
    repo.write("beta.txt", b"beta\n");
    repo.git(&["add", "beta.txt"]);
    repo.git(&["commit", "-q", "--no-gpg-sign", "-m", "tip"]);
    let tip = repo.git(&["rev-parse", "HEAD"]);
    let text = |s: &str| Some(Blob::Text(s.to_owned()));
    assert_eq!(
        changes(repo.path(), &base, &tip).unwrap(),
        [
            Change {
                path: "alpha.txt".into(),
                from: None,
                old: text("alpha\n"),
                new: None,
            },
            Change {
                path: "beta.txt".into(),
                from: None,
                old: None,
                new: text("beta\n"),
            },
            Change {
                path: "lib".into(),
                from: None,
                old: text("x\n"),
                new: None,
            },
        ]
    );
}

#[test]
fn a_bad_revision_and_a_missing_repository_refuse() {
    let repo = Repo::init(&[]);
    repo.write("a.txt", b"a\n");
    let tip = repo.commit("one");
    match changes(repo.path(), "nope", &tip) {
        Err(GitError::Failed {
            argv,
            status,
            stderr,
        }) => {
            assert_eq!(argv[..3], ["git", "diff", "--raw"]);
            assert_eq!(status, Some(128));
            assert!(stderr.contains("nope"), "{stderr}");
        }
        other => panic!("{other:?}"),
    }
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        merge_base(empty.path(), "a", "b"),
        Err(GitError::Failed {
            status: Some(128),
            ..
        })
    ));
    let gone = empty.path().join("gone");
    assert!(matches!(
        changes(&gone, "a", "b"),
        Err(GitError::Spawn { .. })
    ));
}
