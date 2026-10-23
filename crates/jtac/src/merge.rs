//! `jtac check --from <head> --base <old tip>` (DESIGN.md section 15, Merge roots): the
//! default-branch job's writer. HEAD is the commit the forge made for a pull request whose
//! head is `--from`; the rules diff from the branch's old tip under its trust, the head's
//! chain offers its trusted waivers, the head's root is compared as the pull request job
//! compared it, with the head's checks run again on its own tree, and the root, signed in the
//! CI namespace, records what the head had in `from`. Only CI's own verdict fails the job;
//! what the head had never stops the merge root.

mod out;
mod tree;

use std::path::{Path, PathBuf};

use jtac_checks::diff::Adoption;
use jtac_core::{Compared, FromHead, GitOid, Ts};
use jtac_sign::{Namespace, Tool, Verified};
use jtac_store::{Store, ref_name};

use crate::check::{Site, Written};
use crate::ci::{self, head, levers};
use crate::cli::CheckArgs;
use crate::refuse::Refusal;
use crate::repo::{self, Base, Head};
use crate::trust::Signers;
use crate::{Printed, chain, coverage, identity, judge, output, policy, rootpush, state, write};

/// What the flags name, resolved and checked before any work.
struct Job<'a> {
    repo: PathBuf,
    format: String,
    /// The forge's commit.
    head: Head,
    /// The pull request head.
    from: Head,
    /// The branch's old tip.
    old: Head,
    /// Where the head's chain stops, as hex.
    merge_base: String,
    base_rev: &'a str,
}

impl<'a> Job<'a> {
    /// Preflight as `check` has it, then the three commits: `--from` a held commit other
    /// than HEAD, `--base` a proper ancestor of HEAD, and a merge base between the two.
    fn resolve(a: &'a CheckArgs, cwd: &Path) -> Result<Self, Refusal> {
        let (Some(from_rev), Some(base_rev)) = (&a.from, &a.base) else {
            unreachable!("clap: `--from` requires `--base`")
        };
        let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
        let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
        let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
        if let Some(why) = repo::dirty(&repo).map_err(|e| Refusal::repository(&e))? {
            return Err(Refusal::dirty(&why));
        }
        if let Some(path) = repo::grafted(&repo).map_err(|e| Refusal::repository(&e))? {
            return Err(Refusal::grafted(&path));
        }
        if repo::shallow(&repo).map_err(|e| Refusal::repository(&e))? {
            return Err(Refusal::shallow());
        }
        let from =
            repo::peel(&repo, &format, from_rev).map_err(|_| Refusal::rev("check", from_rev))?;
        if from.commit == head.commit {
            return Err(Refusal::from_self(from_rev, base_rev));
        }
        let old = repo::peel(&repo, &format, base_rev).map_err(|e| Refusal::base(base_rev, &e))?;
        if old.commit == head.commit || !crate::range::ancestor(&repo, &old.commit, &head.commit)? {
            return Err(Refusal::from_base(base_rev, &head.commit.hex()));
        }
        let merge_base = repo::probe(
            &repo,
            &["merge-base", &old.commit.hex(), &from.commit.hex()],
            1,
        )
        .map_err(|e| Refusal::repository(&e))?
        .ok_or_else(|| Refusal::from_unrelated(from_rev, base_rev))?;
        Ok(Self {
            repo,
            format,
            head,
            from,
            old,
            merge_base,
            base_rev,
        })
    }
}

/// The rendered verdict; exit 1 on CI's own block alone.
pub fn check(a: &CheckArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let job = Job::resolve(a, cwd)?;
    let repo = &job.repo;
    let identity = identity::identity(repo)?;
    let gate = identity::gate_ref()?;
    let store = Store::open(repo).map_err(Refusal::store)?;
    let existing = store.root(&job.head.commit).map_err(Refusal::store)?;
    let trust = Signers::at(repo, &job.old, job.base_rev)?;
    let adoption = ci::adoption_of(&trust);
    let mut notes = Vec::new();
    let policies = ci::policies(repo, &job.head, &job.old, &job.old.commit.hex(), &mut notes)?;
    let (loaded, policy_from) = policies.judging();
    let head_policy = match policy::load_if_present(repo, &job.from) {
        Ok(p) => p,
        Err(e) => {
            notes.push(format!(
                "the head's policy does not read ({}); its root is compared under none",
                e.what
            ));
            None
        }
    };
    let tool = coverage::verifier(repo)?;
    let chain = levers::gather(
        repo,
        &store,
        &tool,
        &trust,
        &job.format,
        &job.from,
        &job.merge_base,
    )?;
    let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
    let mut cache = store.cache();
    let offered = judge::Offered {
        waivers: chain.trusted.clone(),
        waivable: chain.commits.clone(),
        adoption,
    };
    let base = Base {
        commit: job.old.commit.hex(),
        from: format!("--base {}, the branch's old tip", job.base_rev),
    };
    let j = judge::judge(repo, loaded, job.head, base, now, Some(&cache), &offered)?;
    if repo::head(repo).map_err(|e| Refusal::no_head(&e))? != job.head {
        return Err(Refusal::moved(None));
    }
    if let Some(why) = repo::dirty(repo).map_err(|e| Refusal::repository(&e))? {
        return Err(Refusal::moved(Some(&why)));
    }
    let bytes = jtac_core::canonical_bytes(&loaded.policy).map_err(Refusal::internal)?;
    write::evidence(&store, &bytes, &j, &mut cache)?;
    let head_root = {
        let checkout = tree::HeadTree::checkout(repo, &store, &job.head, &job.from)?;
        let site = head::Site {
            repo,
            cwd: checkout.path(),
            format: &job.format,
            head: job.from,
            head_policy: head_policy.as_ref(),
            ci: &j,
            cache: &cache,
        };
        head::compare_head(&site, &chain.ledger)?
    };
    notes.extend(chain.notes);
    notes.extend(chain.ledger.notes.iter().cloned());
    let from = FromHead {
        head: job.from.commit,
        root: head_root.as_ref().and_then(|h| h.root),
        compared: head_root.as_ref().and_then(|h| h.root.map(|_| h.compared)),
    };
    if head_root.as_ref().is_some_and(|h| h.root.is_none()) {
        notes.push(format!(
            "the head's root ref holds no readable root; `from` records none, as it would \
             for a head with no root ({})",
            ref_name(&job.from.commit)
        ));
    }
    let site = Site {
        store: &store,
        identity: &identity,
        gate,
        policy_version: loaded.policy.version,
        existing: existing.as_ref(),
        from: Some(from.clone()),
    };
    let chain_of = || chain::from_chain(repo, &store, &job.head, &job.old.commit);
    let written = site.commit(&j, chain_of, &mut notes)?;
    if written.as_ref().is_some_and(|w| w.fresh) {
        named(&store, &tool, &trust, &job.head, &mut notes)?;
    }
    let (pushed, stderr) = push(
        &store,
        a.push.as_deref(),
        &job,
        &from,
        written.as_ref(),
        &mut notes,
    );
    let o = out::MergeOut {
        v: 1,
        command: "check",
        base_rev: job.base_rev.to_owned(),
        from,
        signers: trust.describe(),
        adoption: adoption == Adoption::Trust,
        policy_from,
        waivable: chain.commits.len() as u64,
        verdict: output::output(&j, notes, false, written.as_ref().map(|w| w.id)),
        withheld_waivers: ci::out::withheld(&j, &chain.withheld),
        head_root,
        pushed,
    };
    let text = if a.json {
        crate::render::json(&o)?
    } else {
        out::render(&o)
    };
    let mut printed = Printed::verdict(text, o.verdict.pass);
    printed.warn = state::write(&store, &o.verdict).err();
    printed.note = (!stderr.is_empty()).then_some(stderr);
    Ok(printed)
}

/// The refs `--push` sends: the merge root's when this run wrote it, and the head's when CI
/// found it agreeing; one it found otherwise is withheld and named. Returns the refs sent and
/// what the push printed for stderr.
fn push(
    store: &Store,
    remote: Option<&str>,
    job: &Job,
    from: &FromHead,
    written: Option<&Written>,
    notes: &mut Vec<String>,
) -> (Vec<String>, String) {
    let forward = match from.compared {
        Some(Compared::Agreed) => true,
        Some(c) => {
            notes.push(format!(
                "the head's root ref {} is withheld: CI found it {}, and a root it could not \
                 vouch for enters no ref every clone fetches",
                ref_name(&job.from.commit),
                out::compared(c)
            ));
            false
        }
        None => false,
    };
    match (remote, written) {
        (Some(remote), Some(w)) if w.fresh => {
            let mut commits: Vec<GitOid> = vec![job.head.commit];
            if forward {
                commits.push(job.from.commit);
            }
            let stderr = rootpush::send(store, remote, &commits);
            (commits.iter().map(ref_name).collect(), stderr)
        }
        (Some(_), _) => {
            notes.push("nothing was pushed: this run wrote no root".to_owned());
            (Vec::new(), String::new())
        }
        (None, _) => (Vec::new(), String::new()),
    }
}

/// A note when the key that signed is not one the old tip's signers name for the CI
/// namespace: every reader will read the root as an unknown signer's. The adoption merge,
/// whose old tip holds no signers file, is the expected case.
fn named(
    store: &Store,
    tool: &Tool,
    trust: &Signers,
    head: &Head,
    notes: &mut Vec<String>,
) -> Result<(), Refusal> {
    let Some(stored) = store.root(&head.commit).map_err(Refusal::store)? else {
        return Ok(());
    };
    let bytes = stored.root.canonical_bytes().map_err(Refusal::internal)?;
    let verified = jtac_sign::verify(
        tool,
        trust.as_sign(),
        &stored.root.signer,
        Namespace::CiRoot,
        stored.root.ts,
        &bytes,
        &String::from_utf8_lossy(&stored.sig),
    )
    .map_err(|e| Refusal::verifier(&e))?;
    if !matches!(verified, Verified::Trusted) {
        notes.push(format!(
            "the signing key is not named for namespace {} by {}: readers will see an unknown \
             signer's root; a `.jtac/signers` line with `namespaces=\"{}\"` names it",
            Namespace::CiRoot.as_str(),
            trust.describe(),
            Namespace::CiRoot.as_str()
        ));
    }
    Ok(())
}
