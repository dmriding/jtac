//! `jtac init --ci github` (DESIGN.md section 15, Adoption): the workflow, the first
//! `.jtac/signers` with the operator's and the CI key's lines, and a CI key made under the
//! user config directory and never printed. Opt-in; a remote that is not GitHub is refused,
//! since the workflow is GitHub's. Every input is read in `plan`, so a refusal comes before
//! the first write (`the_ci_workflow_is_opt_in_pins_the_release_and_names_the_operators_steps`).

use std::path::{Path, PathBuf};

use jtac_checks::git::quoted;

use crate::ciwf::{self, WORKFLOW_PATH};
use crate::init::{ask, slashes, write_new};
use crate::initout::InitOutput;
use crate::keydir::{self, Key};
use crate::refuse::Refusal;
use crate::trust::TRUST_PATH;
use crate::{identity, repo};

/// The writes the flag asked for, judged before anything is written.
pub struct Planned {
    repo: PathBuf,
    owner: String,
    name: String,
    default_branch: String,
    workflow_there: bool,
    signers_there: bool,
    key: PathBuf,
    /// The CI key's signers line, read now when the key exists; `None` until it is made.
    ci_line: Option<String>,
    program: PathBuf,
}

/// Read the remote, the tree and the keys and decide the writes; `None` without the flag.
pub fn plan(
    repo: &Path,
    remote: Option<&str>,
    forge: Option<&str>,
    dev: &Key,
) -> Result<Option<Planned>, Refusal> {
    let Some(forge) = forge else {
        return Ok(None);
    };
    if forge != "github" {
        return Err(Refusal::init_no_forge(forge));
    }
    let Some(remote) = remote else {
        return Err(Refusal::init_ci_no_remote());
    };
    let url = repo::config(repo, &format!("remote.{remote}.url"))
        .map_err(|e| Refusal::repository(&e))?
        .unwrap_or_default();
    let (owner, name) =
        ciwf::github_slug(&url).ok_or_else(|| Refusal::init_ci_not_github(remote, &url))?;
    if let Key::Kept { id, .. } = dev
        && keydir::signers_line(id).is_none()
    {
        return Err(Refusal::init_ci_dev_line(id));
    }
    let dir = keydir::config_dir(std::env::consts::OS, |k| std::env::var_os(k))
        .ok_or_else(Refusal::init_no_config_dir)?;
    let key = dir
        .join("jtac")
        .join("ci")
        .join(ciwf::key_path(&owner, &name));
    let ci_line = match key.symlink_metadata() {
        Ok(_) => Some(ci_line(&key)?),
        Err(_) => None,
    };
    let program = identity::git_config(repo)?
        .gpg_ssh_program
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ssh-keygen".to_owned());
    Ok(Some(Planned {
        workflow_there: repo.join(WORKFLOW_PATH).symlink_metadata().is_ok(),
        signers_there: repo.join(TRUST_PATH).symlink_metadata().is_ok(),
        default_branch: default_branch(repo, remote)?,
        repo: repo.to_path_buf(),
        owner,
        name,
        key,
        ci_line,
        program: PathBuf::from(program),
    }))
}

/// The CI key's signers line from its `.pub`, or the refusal that names how to make one.
fn ci_line(key: &Path) -> Result<String, Refusal> {
    let public = ciwf::pub_path(key);
    std::fs::read_to_string(&public)
        .ok()
        .and_then(|text| ciwf::ci_signers_line(&text))
        .ok_or_else(|| Refusal::init_ci_pub(key))
}

/// The remote's default branch as `refs/remotes/<remote>/HEAD` names it; guessing from the
/// branch checked out would write a feature branch into the workflow and the fence.
fn default_branch(repo: &Path, remote: &str) -> Result<String, Refusal> {
    let prefix = format!("refs/remotes/{remote}/");
    let head = format!("{prefix}HEAD");
    repo::probe(repo, &["symbolic-ref", "-q", &head], 1)
        .map_err(|e| Refusal::repository(&e))?
        .and_then(|full| full.strip_prefix(&prefix).map(str::to_owned))
        .ok_or_else(|| Refusal::init_ci_no_default_branch(remote))
}

impl Planned {
    /// Whether [`apply`] writes.
    pub fn writes(&self) -> bool {
        !(self.workflow_there && self.signers_there && self.ci_line.is_some())
    }
}

/// The step: the CI key, the workflow, the trust file, then what the operator does next.
pub fn apply(out: &mut InitOutput, p: Option<&Planned>, dry: bool) -> Result<(), Refusal> {
    let Some(p) = p else {
        return Ok(());
    };
    let key = slashes(&p.key);
    let ci_line = match &p.ci_line {
        Some(line) => {
            out.kept("ci-key", format!("CI signing key {key}"));
            Some(line.clone())
        }
        None => {
            out.wrote(
                "ci-key",
                format!("CI signing key {key} (generated; the secret takes it, a commit never)"),
            );
            if dry {
                None
            } else {
                keydir::generate(&p.program, ciwf::CI_PRINCIPAL, &p.key)?;
                Some(ci_line(&p.key)?)
            }
        }
    };
    let version = env!("CARGO_PKG_VERSION");
    if p.workflow_there {
        out.kept("workflow", format!("workflow {WORKFLOW_PATH}"));
    } else {
        if !dry {
            let text = ciwf::workflow(version, &p.default_branch);
            write_new("the workflow", &p.repo.join(WORKFLOW_PATH), &text)?;
        }
        out.wrote(
            "workflow",
            format!("workflow {WORKFLOW_PATH} (pinned to v{version})"),
        );
    }
    let both = format!("trust file {TRUST_PATH} (your line and the CI key's)");
    match (p.signers_there, &out.signers, &ci_line) {
        (true, _, Some(line)) => {
            let trust = std::fs::read_to_string(p.repo.join(TRUST_PATH)).unwrap_or_default();
            if ciwf::names_key(&trust, line) {
                out.kept(
                    "signers",
                    format!("trust file {TRUST_PATH} (names the CI key)"),
                );
            } else {
                out.kept("signers", format!("trust file {TRUST_PATH} (yours)"));
                out.notes.push(format!(
                    "add the CI key's line to `{TRUST_PATH}` and commit it, or its roots read \
                     as an unknown signer's: {}",
                    line.trim_end()
                ));
            }
        }
        (true, _, None) => out.kept("signers", format!("trust file {TRUST_PATH} (yours)")),
        (false, Some(dev), Some(line)) => {
            if !dry {
                let text = ciwf::signers_file(dev, line);
                write_new("the trust file", &p.repo.join(TRUST_PATH), &text)?;
            }
            out.wrote("signers", both);
        }
        (false, _, _) if dry => out.wrote("signers", both),
        (false, _, _) => return Err(Refusal::internal("no signer line for the trust file")),
    }
    let steps = ciwf::next_steps(&p.owner, &p.name, &key, &p.default_branch);
    out.notes.extend(steps);
    Ok(())
}

impl Refusal {
    /// `--ci` names a forge with no workflow.
    pub fn init_no_forge(name: &str) -> Self {
        ask(
            format!("`--ci {name}` names a forge jtac has no workflow for."),
            "The CI gate is one binary that knows no forge; a workflow file per forge feeds it \
             the event and turns the exit code into a check, and this release ships GitHub's.",
            "Run `jtac init --ci github`, or write the workflow for your forge from the \
             GitHub one (docs/CI.md in the jtac repository).",
        )
    }

    /// No remote, so no forge.
    pub fn init_ci_no_remote() -> Self {
        ask(
            "no remote names the forge.",
            "The workflow names the default branch the merge root job watches and the \
             repository the CODEOWNERS line is for, both read from the remote.",
            "Run `git remote add origin <url>`, then `jtac init --ci github` again.",
        )
    }

    /// The remote is not on github.com.
    pub fn init_ci_not_github(remote: &str, url: &str) -> Self {
        ask(
            format!("remote `{remote}` is `{url}`, not a github.com repository."),
            "The workflow is GitHub's: its events, its environment rule and its secret; on \
             another forge it would never run.",
            "Run `jtac init --ci github --remote <name>` naming the GitHub remote, or adapt \
             the workflow by hand from docs/CI.md in the jtac repository.",
        )
    }

    /// The clone does not know the remote's default branch.
    pub fn init_ci_no_default_branch(remote: &str) -> Self {
        ask(
            format!("`refs/remotes/{remote}/HEAD` is not set, so the default branch is unknown."),
            "The workflow's push trigger, the environment's deployment rule and the ruleset \
             all name that branch; a guess from the branch checked out would fence a feature \
             branch and leave the real one unsigned.",
            format!(
                "Run `git remote set-head {} --auto`, then `jtac init --ci github` again.",
                quoted(remote, "<remote>")
            ),
        )
    }

    /// The CI key exists but its `.pub` does not read.
    pub fn init_ci_pub(key: &Path) -> Self {
        let key = quoted(&slashes(key), "<key path>");
        ask(
            format!("the CI key `{key}` has no readable `.pub` beside it."),
            "The trust file names the CI key by its public half; without it the key could \
             sign roots no reader trusts.",
            format!(
                "Run `ssh-keygen -y -f {key} > {key}.pub`, then `jtac init --ci github` again."
            ),
        )
    }

    /// The operator's public key is not readable, so the first trust file cannot name it.
    pub fn init_ci_dev_line(id: &jtac_sign::Identity) -> Self {
        let path = match &id.key {
            jtac_sign::KeyRef::File(p) => quoted(&slashes(p), "<key path>"),
            jtac_sign::KeyRef::Literal(_) => "<key path>".to_owned(),
        };
        ask(
            format!("the public half of your signing key is not readable beside `{path}`."),
            "The first trust file names your key for every namespace, so a waiver of yours \
             can clear the next protected change; the CI key alone could clear nothing.",
            format!(
                "Run `ssh-keygen -y -f {path} > {path}.pub`, then `jtac init --ci github` \
                 again."
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ci_refusal_has_three_parts_and_names_the_next_command() {
        let id = jtac_sign::Identity {
            key: jtac_sign::KeyRef::File(PathBuf::from("/work space/key")),
            signer: jtac_core::Signer {
                principal: "dev@example.com".to_owned(),
                fingerprint: "SHA256:AAAA".to_owned(),
            },
            tool: jtac_sign::Tool::new("ssh-keygen"),
        };
        let rows = [
            Refusal::init_no_forge("gitlab"),
            Refusal::init_ci_no_remote(),
            Refusal::init_ci_not_github("origin", "https://example.com/a/b.git"),
            Refusal::init_ci_no_default_branch("origin"),
            Refusal::init_ci_pub(Path::new("/workspace/ci/my.app_ed25519")),
            Refusal::init_ci_dev_line(&id),
        ];
        for r in &rows {
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(
                ["jtac init --ci github", "ssh-keygen ", "git remote "]
                    .iter()
                    .any(|c| r.how.contains(c)),
                "{r:?}"
            );
            assert!(r.text().contains("\nnegotiation: "), "{r:?}");
        }
        assert!(rows[3].how.contains("git remote set-head origin --auto"));
        assert!(
            rows[4]
                .how
                .contains("-f /workspace/ci/my.app_ed25519 > /workspace/ci/my.app_ed25519.pub")
        );
        assert!(
            rows[5]
                .how
                .contains("ssh-keygen -y -f '/work space/key' > '/work space/key'.pub")
        );
    }
}
