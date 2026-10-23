//! Throwaway git repositories for integration tests (DESIGN.md section 13): a temp directory,
//! an environment that reads no real git config, and the few git calls a scenario needs.
//! Dev-only: every crate that spawns git in a test depends on this one, so there is one
//! implementation of the safe environment.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A repository under a temp directory, deleted on drop.
pub struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    /// `git init -b main` plus `options`, for example `--object-format=sha256`.
    pub fn init(options: &[&str]) -> Self {
        let repo = Self::empty();
        let mut args = vec!["init", "-q", "-b", "main"];
        args.extend(options);
        repo.git(&args);
        repo
    }

    /// A bare repository plus `options`, a stand-in for a remote.
    pub fn bare(options: &[&str]) -> Self {
        let repo = Self::empty();
        let mut args = vec!["init", "-q", "--bare", "-b", "main"];
        args.extend(options);
        repo.git(&args);
        repo
    }

    /// A clone of `origin`, with it as the remote `origin`.
    pub fn clone_of(origin: &Repo) -> Self {
        let repo = Self::empty();
        let src = origin.path().to_string_lossy().into_owned();
        repo.git(&["clone", "-q", "--", &src, "."]);
        repo
    }

    /// A temp directory with no repository in it.
    pub fn empty() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The environment a child gets: `PATH`, a `HOME` inside the temp directory, git told to
    /// read no global or system config, the coverage profile path when one is set, so an
    /// instrumented binary writes its profile there and not into the repository under test,
    /// and the Windows system variables without which OpenSSH does not start.
    pub fn env(&self) -> Vec<(String, String)> {
        let inherit = |name: &str| {
            std::env::var_os(name).map(|v| (name.to_owned(), v.to_string_lossy().into_owned()))
        };
        [
            ("HOME", self.path().to_string_lossy().into_owned()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".to_owned()),
            ("GIT_CONFIG_NOSYSTEM", "1".to_owned()),
            ("GIT_AUTHOR_NAME", "example".to_owned()),
            ("GIT_AUTHOR_EMAIL", "example@example.com".to_owned()),
            ("GIT_COMMITTER_NAME", "example".to_owned()),
            ("GIT_COMMITTER_EMAIL", "example@example.com".to_owned()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .chain(inherit("PATH"))
        .chain(inherit("LLVM_PROFILE_FILE"))
        .chain(inherit("SYSTEMROOT"))
        .chain(inherit("SYSTEMDRIVE"))
        .chain(inherit("PROGRAMDATA"))
        .chain(inherit("TEMP"))
        .chain(inherit("TMP"))
        .chain(inherit("PATHEXT"))
        .chain(inherit("COMSPEC"))
        .collect()
    }

    /// Run `program` with `args` in the repository under [`Repo::env`].
    pub fn command(&self, program: &str, args: &[&str]) -> std::process::Output {
        self.command_with(program, args, &[])
    }

    /// [`Repo::command`] with `extra` variables on top of [`Repo::env`].
    pub fn command_with(
        &self,
        program: &str,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> std::process::Output {
        Command::new(program)
            .args(args)
            .current_dir(self.path())
            .env_clear()
            .envs(self.env())
            .envs(extra.iter().copied())
            .output()
            .unwrap()
    }

    /// Run git and return its trimmed stdout. Panics on a non-zero exit.
    pub fn git(&self, args: &[&str]) -> String {
        let out = self.command("git", args);
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// Write `bytes` at `path`, creating directories.
    pub fn write(&self, path: &str, bytes: &[u8]) -> PathBuf {
        let full = self.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, bytes).unwrap();
        full
    }

    /// Delete the file at `path`.
    pub fn remove(&self, path: &str) {
        std::fs::remove_file(self.path().join(path)).unwrap();
    }

    /// Stage everything and commit, unsigned. Returns the commit id.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-gpg-sign", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Replace `commit`'s root ref with `json` and `sig`, or with a fresh signature by `key`
    /// in `namespace` when `sig` is `None`. `json` must be canonical: the reader verifies the
    /// signature over the bytes it recomputes from the parsed root.
    pub fn put_root(
        &self,
        key: &Path,
        namespace: &str,
        commit: &str,
        json: &str,
        sig: Option<&str>,
    ) {
        let json_path = self.write(".git/forged/root.json", json.as_bytes());
        let sig = match sig {
            Some(s) => s.to_owned(),
            None => {
                let sig_path = json_path.with_extension("json.sig");
                let _ = std::fs::remove_file(&sig_path);
                let out = Command::new("ssh-keygen")
                    .args(["-Y", "sign", "-n", namespace, "-P", "", "-f"])
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
        let sig_path = self.write(".git/forged/root.sig", sig.as_bytes());
        let j = self.git(&["hash-object", "-w", "--", json_path.to_str().unwrap()]);
        let s = self.git(&["hash-object", "-w", "--", sig_path.to_str().unwrap()]);
        let listing = format!("100644 blob {j}\troot.json\n100644 blob {s}\troot.sig\n");
        let mut child = Command::new("git")
            .args(["mktree"])
            .current_dir(self.path())
            .env_clear()
            .envs(self.env())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), listing.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "git mktree");
        let tree = String::from_utf8(out.stdout).unwrap().trim().to_owned();
        let forged = self.git(&["commit-tree", &tree, "-m", "forged"]);
        let root_ref = format!("refs/jtac/roots/{}/{commit}", &commit[..2]);
        self.git(&["update-ref", &root_ref, &forged]);
    }
}
