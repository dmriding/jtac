//! `jtac init` (DESIGN.md section 11.7): key, store, policy, hook, refspec, lever node, in
//! that order. Every fact is gathered and every refusal raised before the first write, so a
//! refusal leaves nothing half done; `--dry-run` reports the same plan and writes nothing; a
//! second run finds nothing to do and writes no lever node.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use jtac_checks::git::{git_with, quoted};
use jtac_core::{Action, Body, GitOid, LeverBody, Node, Sha256Digest, Ts};
use jtac_policy::POLICY_PATH;
use jtac_sign::{Identity, KeyRef};
use jtac_store::Store;

use crate::cli::InitArgs;
use crate::detect::NewPolicy;
use crate::hookfile::Install;
use crate::initout::InitOutput;
use crate::keydir::Key;
use crate::refspec::REFSPEC;
use crate::refuse::Refusal;
use crate::repo::RepoError;
use crate::{Printed, detect, hookfile, keydir, refspec, render, repo};

/// Everything init found, before it writes anything.
struct Plan {
    repo: PathBuf,
    remote: Option<String>,
    refspec_there: bool,
    key: Key,
    store: Store,
    store_there: bool,
    policy: Option<NewPolicy>,
    hook: PathBuf,
    script: String,
    install: Install,
    lever: Option<(GitOid, Sha256Digest)>,
}

impl Plan {
    /// Whether any step writes, so a lever node is due. Mirrors the arms of [`apply`].
    fn writes(&self) -> bool {
        matches!(self.key, Key::New { .. })
            || !self.store_there
            || self.policy.is_some()
            || !matches!(self.install, Install::Keep | Install::Foreign)
            || (self.remote.is_some() && !self.refspec_there)
    }
}

/// Set the repository up, or say what would be.
pub fn init(a: &InitArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let plan = gather(a, cwd)?;
    let out = apply(&plan, a.dry_run)?;
    let text = if a.json {
        render::json(&out)?
    } else {
        out.text()
    };
    Ok(Printed::verdict(text, true))
}

fn gather(a: &InitArgs, cwd: &Path) -> Result<Plan, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let remote = refspec::choose(&refspec::remotes(&repo)?, a.remote.as_deref())?;
    let refspec_there = match &remote {
        Some(r) => refspec::present(&repo, r)?,
        None => false,
    };
    let key = keydir::plan(&repo)?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let policy_path = repo.join(POLICY_PATH);
    let jtac_dir = repo.join(".jtac");
    if jtac_dir.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
        return Err(Refusal::init_symlink(&jtac_dir));
    }
    let policy = match policy_path.symlink_metadata() {
        Ok(_) => None,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Some(detect::policy_text(&detect::detect(&repo)?)?)
        }
        Err(e) => return Err(Refusal::init_io("the policy", &policy_path, &e)),
    };
    let exe = hookfile::exe()?;
    let script = hookfile::script(&exe)?;
    let hook = hookfile::dir(&repo)?.join("pre-push");
    let install = hookfile::judge(&hookfile::read(&hook)?, &script, a.keep_hook)
        .ok_or_else(|| Refusal::init_foreign_hook(&hook, &exe))?;
    let mut plan = Plan {
        store_there: store.dir().is_dir(),
        repo,
        remote,
        refspec_there,
        key,
        store,
        policy,
        hook,
        script,
        install,
        lever: None,
    };
    if plan.writes() {
        let hash = match &plan.policy {
            Some(p) => p.hash,
            None => detect::kept_hash(&policy_path)?,
        };
        plan.lever = Some((tree(&plan.repo)?, hash));
    }
    Ok(plan)
}

fn apply(p: &Plan, dry: bool) -> Result<InitOutput, Refusal> {
    let mut out = InitOutput::new(dry);
    let id = key_step(&mut out, p, dry)?;
    let store = slashes(p.store.dir());
    if p.store_there {
        out.kept("store", format!("store {store}"));
    } else {
        if !dry {
            std::fs::create_dir_all(p.store.dir())
                .map_err(|e| Refusal::init_io("the store", p.store.dir(), &e))?;
        }
        out.wrote("store", format!("store {store}"));
    }
    match &p.policy {
        None => out.kept("policy", format!("policy {POLICY_PATH}")),
        Some(new) => {
            if !dry {
                write_new(&p.repo.join(POLICY_PATH), &new.text)?;
            }
            let found = &new.languages;
            out.wrote(
                "policy",
                format!("policy {POLICY_PATH} (detected: {found})"),
            );
            out.policy = Some(new.text.clone());
            if new.checks == 0 {
                out.notes.push(format!(
                    "no checks were detected: {POLICY_PATH} holds only the integrity rules; add \
                     a `[checks.<id>]` table with `run = [...]` for each command that must pass"
                ));
            }
        }
    }
    let hook = slashes(&p.hook);
    match p.install {
        Install::Keep => out.kept("hook", format!("pre-push hook {hook}")),
        Install::Foreign => {
            out.kept("hook", format!("pre-push hook {hook} (yours, unread)"));
            out.notes.push(
                "the pre-push hook is not jtac's and was not read: the exec line you added \
                 is the gate only where the script reaches it; `git push` of a commit with \
                 no root must be refused"
                    .to_owned(),
            );
        }
        Install::Chmod => {
            if !dry {
                hookfile::chmod(&p.hook)?;
            }
            out.wrote("hook", format!("pre-push hook {hook} (made executable)"));
        }
        Install::Write | Install::Rewrite => {
            if !dry {
                hookfile::write(&p.hook, &p.script)?;
            }
            let moved = match p.install {
                Install::Rewrite => " (it named a jtac binary that moved)",
                _ => "",
            };
            out.wrote("hook", format!("pre-push hook {hook}{moved}"));
        }
    }
    match (&p.remote, p.refspec_there) {
        (None, _) => out.notes.push(
            "no remote: run `jtac init` again after `git remote add origin <url>` to add the \
             evidence refspec"
                .to_owned(),
        ),
        (Some(r), true) => out.kept("refspec", format!("fetch refspec {REFSPEC} on {r}")),
        (Some(r), false) => {
            if !dry {
                refspec::add(&p.repo, r)?;
            }
            out.wrote("refspec", format!("fetch refspec {REFSPEC} on {r}"));
        }
    }
    lever_step(&mut out, p, id.as_ref(), dry)?;
    Ok(out)
}

/// Keep the named key, or make one and name it; the identity, once the key exists.
fn key_step(out: &mut InitOutput, p: &Plan, dry: bool) -> Result<Option<Identity>, Refusal> {
    let id = match &p.key {
        Key::Kept { id, from } => {
            let key = match &id.key {
                KeyRef::File(path) => slashes(path),
                KeyRef::Literal(_) => "in the agent".to_owned(),
            };
            let fp = &id.signer.fingerprint;
            out.kept("key", format!("signing key {key} ({fp}, named by {from})"));
            Some(id.clone())
        }
        Key::New {
            program,
            email,
            path,
            exists,
        } => {
            let id = if dry {
                None
            } else {
                Some(keydir::create(&p.repo, program, email, path, *exists)?)
            };
            let made = if *exists {
                "reused the file there"
            } else {
                "generated"
            };
            let fp = id
                .as_ref()
                .map_or_else(String::new, |i| format!(", {}", i.signer.fingerprint));
            out.wrote(
                "key",
                format!(
                    "signing key {} ({made}{fp}), named by jtac.signingkey in the global git \
                     config",
                    slashes(path)
                ),
            );
            id
        }
    };
    out.signers = id.as_ref().and_then(keydir::signers_line);
    let unread = match (&out.signers, id.as_ref().map(|i| &i.key)) {
        (None, Some(KeyRef::File(path))) => format!(
            " (`ssh-keygen -y -f {}` prints the public key; prefix it with the email)",
            quoted(&slashes(path), "<key path>")
        ),
        _ => String::new(),
    };
    if id.is_some() {
        out.notes.push(format!(
            "the allowed signer line{unread} is for `.jtac/signers`, where `jtac verify` reads \
             trusted keys: add it there and commit it to trust this key; init does not write \
             that file"
        ));
    }
    Ok(id)
}

/// Record that init changed the gate, once, when this run wrote something.
fn lever_step(
    out: &mut InitOutput,
    p: &Plan,
    id: Option<&Identity>,
    dry: bool,
) -> Result<(), Refusal> {
    let Some((tree, policy)) = p.lever else {
        out.kept(
            "lever",
            "lever node: nothing changed, so none written".to_owned(),
        );
        return Ok(());
    };
    if dry {
        out.wrote("lever", "lever node (action init)".to_owned());
        return Ok(());
    }
    let signer = id
        .map(|i| i.signer.clone())
        .ok_or_else(|| Refusal::internal("no signer for the init lever node"))?;
    let node = Node {
        tree,
        policy,
        ts: Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?,
        body: Body::Lever(LeverBody {
            action: Action::Init,
            signer,
            reason: format!("jtac init wrote {}", out.written().join(", ")),
            scope: None,
            expires: None,
        }),
    };
    let node_id = p.store.put_node(&node).map_err(Refusal::store)?;
    out.wrote("lever", format!("lever node {node_id} (action init)"));
    Ok(())
}

/// HEAD's tree, or the empty tree before the first commit.
fn tree(repo: &Path) -> Result<GitOid, Refusal> {
    let fail = |e: RepoError| Refusal::repository(&e);
    let head = ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"];
    if repo::probe(repo, &head, 1).map_err(fail)?.is_some() {
        return repo::head(repo).map(|h| h.tree).map_err(fail);
    }
    let empty = ["hash-object", "-t", "tree", "--stdin"];
    let hex = git_with(repo, &empty, Some(b""), &[]).map_err(|e| fail(e.into()))?;
    let format = repo::format(repo).map_err(fail)?;
    repo::oid(&format, String::from_utf8_lossy(&hex).trim()).map_err(fail)
}

/// Write a file that must not exist yet, creating its directory.
fn write_new(path: &Path, text: &str) -> Result<(), Refusal> {
    let io = |e: std::io::Error| Refusal::init_io("the policy", path, &e);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    file.write_all(text.as_bytes()).map_err(io)
}

/// A negotiation built outside `refuse`, whose constructor is private to it.
pub(crate) fn ask(
    what: impl Into<String>,
    why: impl Into<String>,
    how: impl Into<String>,
) -> Refusal {
    Refusal {
        kind: "negotiation",
        ..Refusal::fence(what, why, how)
    }
}

/// A path as git config and `sh` spell it on every platform.
pub(crate) fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl Refusal {
    /// `.jtac` is a symlink, so a write into it would land wherever it points.
    pub fn init_symlink(path: &Path) -> Self {
        let path = slashes(path);
        ask(
            format!("`{path}` is a symlink."),
            "`.jtac/` is a protected path: jtac writes into it only as a directory of this \
             repository, never through a link that can point anywhere on the machine.",
            format!(
                "Replace it with a directory, `rm {q} && mkdir {q}`, commit, then run `jtac \
                 init` again.",
                q = quoted(&path, "<path>")
            ),
        )
    }

    /// A file or directory init needed could not be read or written.
    pub fn init_io(what: &str, path: &Path, err: &std::io::Error) -> Self {
        let path = slashes(path);
        ask(
            format!("could not read or write {what} at `{path}`: {err}."),
            "init reports a step as done only when it landed; a hook or policy that did not \
             write is a gate that is not there.",
            format!(
                "Check permissions and free space for `{path}`, then run `jtac init` again; \
                 the steps that landed are kept."
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_init_refusal_has_three_parts_and_names_the_next_command() {
        let io = std::io::Error::other("denied");
        let parse = jtac_policy::parse("version = 2\n").unwrap_err();
        let git = jtac_checks::git::GitError::Failed {
            argv: vec!["git".into()],
            status: Some(255),
            stderr: "locked".into(),
        };
        let sign = jtac_sign::SignError::Key {
            path: "/workspace/key".into(),
            what: "bad".into(),
        };
        let path = Path::new("/workspace/.git/hooks/pre-push");
        let rows = [
            Refusal::init_io("the store", path, &io),
            Refusal::init_symlink(path),
            Refusal::init_no_remote("up", &["origin".into()]),
            Refusal::init_no_remote("up", &[]),
            Refusal::init_which_remote(&["up".into(), "down".into()]),
            Refusal::init_foreign_hook(path, "/workspace/bin/jtac"),
            Refusal::init_hook_path("/workspace/it's/jtac"),
            Refusal::init_no_email(),
            Refusal::init_no_config_dir(),
            Refusal::init_keygen(Path::new("ssh-keygen"), path, "no"),
            Refusal::init_key(&sign),
            Refusal::init_key_file(path, &sign),
            Refusal::init_config_write("/workspace/key", &git),
            Refusal::init_package_json("expected value"),
            Refusal::init_policy(Some(&parse)),
            Refusal::init_policy(None),
        ];
        for r in &rows {
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(
                ["jtac ", "git ", "ssh-keygen ", "export "]
                    .iter()
                    .any(|c| r.how.contains(c)),
                "{r:?}"
            );
            assert!(r.text().contains("\nnegotiation: "), "{r:?}");
            assert!(r.json().contains("\"error\""), "{r:?}");
        }
        let foreign = Refusal::init_foreign_hook(path, "/workspace/bin/jtac");
        assert!(
            foreign
                .how
                .contains("exec '/workspace/bin/jtac' hook pre-push \"$@\"")
        );
        assert!(
            Refusal::init_no_email()
                .how
                .contains("git config --global user.email")
        );
    }
}
