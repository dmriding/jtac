//! What a pushed tip has (DESIGN.md sections 9.5, 11.3): its own root, else a root for the
//! same tree. A root counts when it names the tip's tree and its signature holds; who signed
//! is `verify`'s question, except for a root with `from`, whose waiver scope is wider than its
//! commit: it counts only under a signer the remote's default branch tip or the branch's old
//! tip names for the CI namespace. A root that exists and does not count is a note, never a
//! pass. `pre-push` and `status` both read it here, so the two never disagree.

use std::path::Path;

use jtac_checks::git::quoted;
use jtac_core::{GitOid, Root, Sha256Digest};
use jtac_gate::Coverage;
use jtac_sign::{Namespace, SignError, Tool, Trust, Verified};
use jtac_store::Store;

use crate::refuse::Refusal;
use crate::repo::{Head, config, peel, probe};
use crate::trust::Signers;

/// Where the CI namespace's signers are read, when a root with `from` is met and not before:
/// `refs/remotes/<remote>/HEAD` for a named remote, and the branch's old tip when held.
pub struct CiSite<'a> {
    /// The repository.
    pub repo: &'a Path,
    /// `sha1` or `sha256`.
    pub format: &'a str,
    /// The remote's name; `None` for a push that named a URL, which has no tracking ref.
    pub remote: Option<&'a str>,
    /// The remote's URL when known, for the next command.
    pub url: Option<&'a str>,
    /// The branch's old tip, when this clone holds it.
    pub old: Option<GitOid>,
}

impl CiSite<'_> {
    /// The signers read, and whether the default branch tip's were among them: without those
    /// an unknown signer is a retry through a named remote, not a developer's stale root.
    fn signers(&self) -> Result<(Vec<Signers>, bool), Refusal> {
        let mut out = Vec::new();
        if let Some(remote) = self.remote {
            let shown = format!("{remote}/HEAD");
            let rev = format!("refs/remotes/{shown}^{{commit}}");
            let args = ["rev-parse", "--verify", "--quiet", "--end-of-options", &rev];
            if let Some(hex) = probe(self.repo, &args, 1).map_err(|e| Refusal::repository(&e))? {
                let at = peel(self.repo, self.format, &hex).map_err(|e| Refusal::repository(&e))?;
                out.push(Signers::at(self.repo, &at, &shown)?);
            }
        }
        if let Some(old) = self.old {
            let at =
                peel(self.repo, self.format, &old.hex()).map_err(|e| Refusal::repository(&e))?;
            let shown = format!("the old tip {}", &old.hex()[..12]);
            out.push(Signers::at(self.repo, &at, &shown)?);
        }
        let default_read = out.len() > usize::from(self.old.is_some());
        Ok((out, default_read))
    }
}

/// The tip's coverage and the roots passed over.
pub struct Tip {
    /// For the decision.
    pub coverage: Coverage,
    /// The commit whose root counted, the tip's own or a tree match's, with that root's id.
    pub counted: Option<(GitOid, Sha256Digest)>,
    /// Each root that exists and does not count, with why.
    pub notes: Vec<String>,
    /// The tip's own root ref exists and did not count, so `jtac check` would keep it.
    pub own_stale: bool,
}

/// Nothing: what a deletion has.
pub fn none() -> Tip {
    Tip {
        coverage: Coverage::Absent,
        counted: None,
        notes: Vec::new(),
        own_stale: false,
    }
}

/// ssh-keygen as git config names it.
pub fn verifier(repo: &Path) -> Result<Tool, Refusal> {
    let program = config(repo, "gpg.ssh.program").map_err(|e| Refusal::repository(&e))?;
    jtac_sign::verifier(program.as_deref()).map_err(|e| Refusal::verifier(&e))
}

/// The tip's own root, else the first root in ref order that judged the same tree.
/// `ci` is where a root with `from` is judged; `None` asks no trust question, for a reader
/// that decides nothing (`show`).
pub fn tip(store: &Store, tool: &Tool, tip: &Head, ci: Option<&CiSite>) -> Result<Tip, Refusal> {
    let mut notes = Vec::new();
    let mut found =
        counted(store, tool, tip, &tip.commit, ci, &mut notes)?.map(|r| (tip.commit, r));
    let own_stale = found.is_none() && !notes.is_empty();
    let roots = store.roots().map_err(Refusal::store)?;
    notes.extend(roots.notes());
    if found.is_none() {
        for listed in roots.listed {
            if listed.commit == tip.commit {
                continue;
            }
            if let Some(root) = counted(store, tool, tip, &listed.commit, ci, &mut notes)? {
                found = Some((listed.commit, root));
                break;
            }
        }
    }
    let Some((commit, root)) = found else {
        return Ok(Tip {
            coverage: Coverage::Absent,
            counted: None,
            notes,
            own_stale,
        });
    };
    let coverage = if commit == tip.commit {
        Coverage::Root(root.state)
    } else {
        Coverage::ByTree {
            commit: commit.hex(),
        }
    };
    Ok(Tip {
        coverage,
        counted: Some((commit, root.id().map_err(Refusal::internal)?)),
        notes,
        own_stale,
    })
}

/// `commit`'s root when it names the tip's tree and its signature holds. A tree that differs
/// is a note only for the tip's own root; for any other it is just not a match.
fn counted(
    store: &Store,
    tool: &Tool,
    tip: &Head,
    commit: &GitOid,
    ci: Option<&CiSite>,
    notes: &mut Vec<String>,
) -> Result<Option<Root>, Refusal> {
    let mut not =
        |why: String| notes.push(format!("{}'s root is not counted: {why}", commit.hex()));
    let stored = match store.root(commit) {
        Ok(Some(stored)) => stored,
        Ok(None) => return Ok(None),
        Err(e) => {
            not(e.to_string());
            return Ok(None);
        }
    };
    let root = stored.root;
    if root.tree != tip.tree {
        if *commit == tip.commit {
            not(format!("it names tree {}, not {}", root.tree, tip.tree));
        }
        return Ok(None);
    }
    let bytes = root.canonical_bytes().map_err(Refusal::internal)?;
    let sig = String::from_utf8_lossy(&stored.sig);
    let ns = Namespace::of_root(&root);
    let verify = |trust: Trust| {
        jtac_sign::verify(tool, trust, &root.signer, ns, root.ts, &bytes, &sig)
            .map_err(|e| Refusal::verifier(&e))
    };
    let (Some(ci), Namespace::CiRoot) = (ci, ns) else {
        return Ok(match verify(Trust::None)? {
            Verified::Trusted | Verified::Unknown { .. } => Some(root),
            Verified::Invalid { what } => {
                not(format!("its signature does not hold: {what}"));
                None
            }
        });
    };
    let own = *commit == tip.commit;
    let (signers, default_read) = ci.signers()?;
    if signers.is_empty() && own {
        return Err(Refusal::ci_root(ci, &tip.commit.hex(), false));
    }
    for s in &signers {
        match verify(s.as_sign())? {
            Verified::Trusted => return Ok(Some(root)),
            Verified::Unknown { .. } => {}
            Verified::Invalid { what } => {
                not(format!("its signature does not hold: {what}"));
                return Ok(None);
            }
        }
    }
    if own && !default_read {
        return Err(Refusal::ci_root(ci, &tip.commit.hex(), true));
    }
    let read: Vec<String> = signers.iter().map(Signers::describe).collect();
    not(match read.is_empty() {
        true => format!(
            "it has `from`, a CI root, and this clone holds no tip to read namespace {}'s \
             signers from",
            ns.as_str()
        ),
        false => format!(
            "it has `from`, a CI root, and its signer is not named for namespace {} by {}",
            ns.as_str(),
            read.join(" or ")
        ),
    });
    Ok(None)
}

impl Refusal {
    /// A root with `from` at the tip that the default branch tip's signers could not judge:
    /// this clone holds no such tip, or the old tip alone, whose signers do not name the key.
    pub fn ci_root(ci: &CiSite, tip: &str, old_only: bool) -> Self {
        let remote = ci.remote.map(|r| quoted(r, "<remote>"));
        let url = ci
            .url
            .map_or_else(|| "<url>".to_owned(), |u| quoted(u, "<url>"));
        let how = match &remote {
            Some(remote) => format!(
                "Run `git fetch {remote}`, then `git remote set-head {remote} --auto`, then \
                 push again. Do not add `--no-verify`: the root would land unjudged, and a \
                 reader trusts it only under a signers line for the CI namespace."
            ),
            None => format!(
                "Push through a named remote, so this clone can hold its default branch: `git \
                 remote add <name> {url}`, `git fetch <name>`, `git remote set-head <name> \
                 --auto`, then `git push <name> <branch>`. A push that names a URL has no \
                 tracking ref to read."
            ),
        };
        let held = match old_only {
            true => "the old tip alone, whose signers do not name the root's signer",
            false => "neither",
        };
        Self {
            kind: "negotiation",
            what: format!(
                "{tip}'s root has `from`: a CI root, whose signer must be named for namespace \
                 {} by the `.jtac/signers` of {}'s default branch tip or of the branch's old \
                 tip, and this clone holds {held}.",
                Namespace::CiRoot.as_str(),
                remote.as_deref().unwrap_or("the remote")
            ),
            why: "A root with `from` declares a pull request's commits as its waiver scope, \
                  wider than its own commit, so only a key the repository names for CI may sign \
                  one; the hook reads whom it names from the remote's own tips, never from the \
                  pushed tip, which could carry its own key, and clears a root only when the \
                  default branch tip says it is a developer's."
                .to_owned(),
            how,
        }
    }

    pub fn lever_verifier(err: &SignError) -> Self {
        Self {
            kind: "negotiation",
            what: format!("cannot check a stored waiver's signature: {err}."),
            why: "A waiver is applied only when its signature holds; one nobody could check must \
                  not be applied, and silently skipping it would judge without a waiver the \
                  person wrote."
                .to_owned(),
            how: "Install OpenSSH's ssh-keygen, or name it with `git config gpg.ssh.program \
                  <path to ssh-keygen>`, then run `jtac check` again."
                .to_owned(),
        }
    }

    /// ssh-keygen could not run to check a root's signature.
    pub fn verifier(err: &SignError) -> Self {
        Self {
            kind: "negotiation",
            what: format!("cannot check a root's signature: {err}."),
            why: "A root counts only when its signature holds; a signature nobody could check \
                  must never read as one that held, so the push is refused rather than passed."
                .to_owned(),
            how: "Install OpenSSH's ssh-keygen, or name it with `git config gpg.ssh.program \
                  <path to ssh-keygen>`, then run the command again."
                .to_owned(),
        }
    }
}
