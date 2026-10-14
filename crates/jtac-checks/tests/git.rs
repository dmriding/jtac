//! The git helper against a real git: stdin, env, and a child that exits before reading.

use jtac_checks::git::{GitError, git_output, git_with};
use jtac_testkit::Repo;

#[test]
fn stdin_and_env_reach_git_and_an_early_exit_reports_git_not_a_pipe() {
    let repo = Repo::init(&[]);
    let oid = git_with(
        repo.path(),
        &["hash-object", "--stdin"],
        Some(b"example\n"),
        &[],
    )
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&oid).trim(),
        "33a9488b167e4391ad6297a1e43e56f7ec8a294e"
    );
    let out = git_output(
        repo.path(),
        &["var", "GIT_AUTHOR_IDENT"],
        None,
        &[
            ("GIT_AUTHOR_NAME", "probe"),
            ("GIT_AUTHOR_EMAIL", "probe@example.com"),
        ],
    )
    .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("probe <probe@example.com>"));

    repo.write("README.md", b"example\n");
    let head = repo.commit("first");
    let lines = format!("{head}\n").repeat(20_000);
    let out = git_with(
        repo.path(),
        &["cat-file", "--batch-check"],
        Some(lines.as_bytes()),
        &[],
    )
    .unwrap();
    assert_eq!(out.len(), 20_000 * (head.len() + " commit 000\n".len()));

    let not_a_repo = Repo::empty();
    let big = vec![b'x'; 4 << 20];
    let err = git_with(not_a_repo.path(), &["mktree"], Some(&big), &[]).unwrap_err();
    assert!(
        matches!(&err, GitError::Failed { stderr, .. } if stderr.contains("not a git repository")),
        "{err}"
    );
}
