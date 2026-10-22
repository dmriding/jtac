//! The lab `jtac bench` works in: a temp directory holding a home, a global git config, the
//! key `jtac init` generates, and one throwaway repository per control with a bare origin.
//! Every child gets an emptied environment, so no real config or key is read.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::init::ask;
use crate::refuse::Refusal;

/// The temp directory and the binary under test.
pub struct Lab {
    dir: PathBuf,
    home: PathBuf,
    exe: PathBuf,
    made: std::cell::Cell<usize>,
}

/// One throwaway repository with its bare `origin`, gated by `jtac init`.
pub struct Repo<'a> {
    lab: &'a Lab,
    /// The clone.
    pub path: PathBuf,
    /// Its bare origin.
    pub bare: PathBuf,
}

impl Lab {
    /// A fresh lab; refuses when git or ssh-keygen cannot start.
    pub fn new(exe: &Path) -> Result<Self, Refusal> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("jtac-bench-{}-{nanos}", std::process::id()));
        let home = dir.join("home");
        let io =
            |e: std::io::Error| Refusal::bench_setup("create the lab directory", &e.to_string());
        std::fs::create_dir_all(home.join("xdg")).map_err(io)?;
        std::fs::create_dir_all(dir.join("tmp")).map_err(io)?;
        std::fs::write(
            home.join("gitconfig"),
            "[user]\n\tname = bench\n\temail = bench@example.com\n",
        )
        .map_err(io)?;
        let lab = Self {
            dir,
            home,
            exe: exe.to_path_buf(),
            made: std::cell::Cell::new(0),
        };
        for (tool, probe) in [("git", "--version"), ("ssh-keygen", "-?")] {
            lab.command(tool, &[probe], &lab.dir, None)
                .map_err(|e| Refusal::bench_tool(tool, &e))?;
        }
        Ok(lab)
    }

    /// The binary, with forward slashes so it reads the same inside TOML on every platform.
    pub fn exe(&self) -> String {
        self.exe.to_string_lossy().replace('\\', "/")
    }

    /// The environment a child gets: PATH, a home and a temp directory inside the lab (the
    /// signer's scratch files land there, not in the system temp), git told to read no real
    /// config, and the Windows variables without which OpenSSH does not start.
    fn env(&self) -> Vec<(String, String)> {
        let inherit = |name: &str| {
            std::env::var_os(name).map(|v| (name.to_owned(), v.to_string_lossy().into_owned()))
        };
        let home = self.home.to_string_lossy().into_owned();
        let xdg = self.home.join("xdg").to_string_lossy().into_owned();
        let tmp = self.dir.join("tmp").to_string_lossy().into_owned();
        [
            ("HOME", home.clone()),
            ("USERPROFILE", home),
            ("TMPDIR", tmp.clone()),
            ("TEMP", tmp.clone()),
            ("TMP", tmp),
            (
                "GIT_CONFIG_GLOBAL",
                self.home.join("gitconfig").to_string_lossy().into_owned(),
            ),
            ("GIT_CONFIG_NOSYSTEM", "1".to_owned()),
            ("XDG_CONFIG_HOME", xdg.clone()),
            ("APPDATA", xdg),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .chain(
            [
                "PATH",
                "LLVM_PROFILE_FILE",
                "SYSTEMROOT",
                "SYSTEMDRIVE",
                "PROGRAMDATA",
                "PATHEXT",
                "COMSPEC",
            ]
            .into_iter()
            .filter_map(inherit),
        )
        .collect()
    }

    /// Run `program` in `cwd` with the lab's environment, `stdin` written when given.
    pub fn command(
        &self,
        program: &str,
        args: &[&str],
        cwd: &Path,
        stdin: Option<&str>,
    ) -> std::io::Result<Output> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(self.env())
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(text.as_bytes())?;
        }
        child.wait_with_output()
    }

    /// A gated repository: policy, `.gitignore` and `src/lib.rs` in an adoption commit pushed
    /// to a bare origin, then `jtac init` for the key, the hook and the refspec.
    pub fn repo(&self, policy: &str, lib_rs: &str) -> Result<Repo<'_>, Refusal> {
        let n = self.made.get() + 1;
        self.made.set(n);
        let path = self.dir.join(format!("repo{n}"));
        let bare = self.dir.join(format!("origin{n}.git"));
        let io = |e: std::io::Error| {
            Refusal::bench_setup("write the scratch repository", &e.to_string())
        };
        std::fs::create_dir_all(path.join(".jtac")).map_err(io)?;
        std::fs::create_dir_all(path.join("src")).map_err(io)?;
        std::fs::create_dir_all(&bare).map_err(io)?;
        let repo = Repo {
            lab: self,
            path,
            bare,
        };
        let bare = repo.bare.to_string_lossy().replace('\\', "/");
        repo.git_in(
            &repo.bare.clone(),
            &["init", "-q", "--bare", "-b", "main"],
            None,
        )?;
        repo.git(&["init", "-q", "-b", "main"])?;
        repo.git(&["remote", "add", "origin", &bare])?;
        repo.write(".jtac/policy.toml", policy)?;
        repo.write(".gitignore", "*.profraw\n")?;
        repo.write("src/lib.rs", lib_rs)?;
        repo.commit("adopt")?;
        repo.git(&["push", "-q", "-u", "origin", "main"])?;
        let out = repo.jtac(&["init"])?;
        if !out.status.success() {
            return Err(Refusal::bench_setup(
                "run `jtac init` in the scratch repository",
                String::from_utf8_lossy(&out.stderr).trim(),
            ));
        }
        Ok(repo)
    }
}

impl Repo<'_> {
    /// Git in the clone; a non-zero exit is a setup refusal.
    pub fn git(&self, args: &[&str]) -> Result<String, Refusal> {
        self.git_in(&self.path.clone(), args, None)
    }

    /// Git in `cwd` with `stdin`; a non-zero exit is a setup refusal.
    pub fn git_in(
        &self,
        cwd: &Path,
        args: &[&str],
        stdin: Option<&str>,
    ) -> Result<String, Refusal> {
        let out = self
            .lab
            .command("git", args, cwd, stdin)
            .map_err(|e| Refusal::bench_tool("git", &e))?;
        if !out.status.success() {
            return Err(Refusal::bench_setup(
                &format!("run `git {}`", args.join(" ")),
                String::from_utf8_lossy(&out.stderr).trim(),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    /// Git in the clone, whatever it exits: the attack's own push.
    pub fn try_git(&self, args: &[&str]) -> Result<Output, Refusal> {
        self.try_git_in(&self.path.clone(), args)
    }

    /// Git in `cwd`, whatever it exits.
    pub fn try_git_in(&self, cwd: &Path, args: &[&str]) -> Result<Output, Refusal> {
        self.lab
            .command("git", args, cwd, None)
            .map_err(|e| Refusal::bench_tool("git", &e))
    }

    /// The binary under test, in the clone, whatever it exits.
    pub fn jtac(&self, args: &[&str]) -> Result<Output, Refusal> {
        let exe = self.lab.exe.to_string_lossy().into_owned();
        self.lab
            .command(&exe, args, &self.path, None)
            .map_err(|e| Refusal::exe(&e))
    }

    /// Write `text` at `path` under the clone.
    pub fn write(&self, path: &str, text: &str) -> Result<(), Refusal> {
        std::fs::write(self.path.join(path), text)
            .map_err(|e| Refusal::bench_setup(&format!("write `{path}`"), &e.to_string()))
    }

    /// Stage everything and commit, unsigned; the new commit id.
    pub fn commit(&self, message: &str) -> Result<String, Refusal> {
        self.git(&["add", "-A"])?;
        self.git(&["commit", "-q", "--no-gpg-sign", "-m", message])?;
        self.git(&["rev-parse", "HEAD"])
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Refusal {
    pub(crate) fn bench_tool(tool: &str, err: &std::io::Error) -> Self {
        ask(
            format!("`jtac bench` cannot run: `{tool}` did not start ({err})."),
            "bench attacks the gate through its real path, which runs git and signs with \
             ssh-keygen; a proof that skipped a step would prove nothing.",
            format!(
                "Install {tool} (git 2.31 or later, OpenSSH 8.9 or later) so it is on PATH, then \
                 run `jtac bench` again."
            ),
        )
    }

    pub(crate) fn bench_setup(step: &str, detail: &str) -> Self {
        ask(
            format!("`jtac bench` could not {step}: {detail}"),
            "The scratch repository is the proof's ground; a step that failed is reported, \
             never skipped, so a half-built lab cannot pass as a gate that held.",
            "Fix what the tool reports (disk space, a temp directory you can write, git on \
             PATH), then run `jtac bench` again.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bench_refusal_has_three_parts_and_names_the_command_to_rerun() {
        let io = std::io::Error::other("no such file");
        for r in [
            Refusal::bench_tool("git", &io),
            Refusal::bench_setup("write `x`", "denied"),
        ] {
            assert_eq!(r.kind, "negotiation");
            assert!(!r.what.is_empty() && !r.why.is_empty());
            assert!(r.how.contains("run `jtac bench` again"), "{}", r.how);
        }
    }
}
