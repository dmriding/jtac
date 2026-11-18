//! `jtac check --from <head> --base <old tip>` (DESIGN.md section 15, Merge roots): the
//! default-branch job's writer. HEAD is the commit the forge made for a pull request whose
//! head is `--from`; the rules diff from the branch's old tip under its trust, the head's
//! chain offers its trusted waivers, the head's root is compared as the pull request job
//! compared it, with the head's checks run again on its own tree, and the root, signed in the
//! CI namespace, records what the head had in `from`. A check the policy reads from CI
//! takes its result from `--results` (section 15.2). Only CI's own verdict fails the job;
//! what the head had never stops the merge root. A verdict that fails writes the failure
//! record in the root's place. A root on the tip, at the job's fetch or on the remote when
//! its push is refused, stands only as CI's own for this head, and so does a record.

mod observe;
mod out;
mod record;
mod tree;

use std::path::{Path, PathBuf};

use jtac_checks::Cache;
use jtac_checks::diff::Adoption;
use jtac_checks::diff::git::{git, quoted};
use jtac_core::{Compared, FromHead, GitOid, Signer, Ts};
use jtac_policy::Checks;
use jtac_sign::{Namespace, Tool, Verified};
use jtac_store::{Space, Store, StoreError, StoredRoot, ref_name};

use crate::check::Site;
use crate::ci::{self, head, levers};
use crate::cli::CheckArgs;
use crate::headof;
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
    from_rev: &'a str,
    base_rev: &'a str,
}

impl<'a> Job<'a> {
    /// Preflight as `check` has it, then the three commits: `--base` on HEAD's first-parent
    /// line, `--from` a held commit that is neither on that line, HEAD included, nor behind
    /// the old tip, and a merge base between the two (decisions 47 and 66).
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
        let old =
            repo::peel(&repo, &format, base_rev).map_err(|_| Refusal::rev("check", base_rev))?;
        let line = first_parent_line(&repo, &head);
        if !line.contains(&old.commit.hex()) {
            return Err(Refusal::from_base(base_rev, &head.commit.hex(), from_rev));
        }
        let merge_base = repo::probe(
            &repo,
            &["merge-base", &old.commit.hex(), &from.commit.hex()],
            1,
        )
        .map_err(|e| Refusal::repository(&e))?
        .ok_or_else(|| Refusal::from_unrelated(from_rev, base_rev))?;
        if merge_base == from.commit.hex() {
            return Err(Refusal::from_behind(from_rev, base_rev));
        }
        if line.contains(&from.commit.hex()) {
            return Err(Refusal::from_first_parent(from_rev, base_rev));
        }
        Ok(Self {
            repo,
            format,
            head,
            from,
            old,
            merge_base,
            from_rev,
            base_rev,
        })
    }
}

/// HEAD's first-parent ancestors as hex, HEAD excluded: a merge, a squash and a rebase all
/// leave the branch's old tip there, and the pull request's own side never is, so a `--base`
/// off this line is a rewind, another line, or the head's own history.
fn first_parent_line(repo: &Path, head: &Head) -> Vec<String> {
    git(
        repo,
        &["rev-list", "--first-parent", &head.commit.hex(), "--"],
    )
    .map(|out| {
        String::from_utf8_lossy(&out)
            .lines()
            .skip(1)
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

impl Job<'_> {
    /// The head's root compared as the pull request job compared it, with the head's checks
    /// on its own tree: a detached worktree when the ledger holds a root for the head.
    fn compare(
        &self,
        chain: &levers::Chain,
        head_policy: Option<&policy::Loaded>,
        ci: &judge::Judged,
        cache: &dyn Cache,
    ) -> Result<Option<head::HeadRoot>, Refusal> {
        let repo = &self.repo;
        let rooted = chain
            .ledger
            .roots
            .iter()
            .any(|r| r.commit == self.from.commit);
        let checkout = match rooted {
            true => tree::HeadTree::checkout(repo, &self.head, &self.from)
                .map_err(|e| Refusal::worktree(&e, self.from_rev, self.base_rev))?,
            false => tree::HeadTree::here(repo),
        };
        let site = head::Site {
            repo,
            cwd: checkout.path(),
            format: &self.format,
            head: self.from,
            head_policy,
            ci,
            cache,
            keyed: true,
        };
        head::compare_head(&site, &chain.ledger)
    }
}

/// The rendered verdict; exit 1 on CI's own block alone.
pub fn check(a: &CheckArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let job = Job::resolve(a, cwd)?;
    let repo = &job.repo;
    let identity = identity::identity(repo)?;
    let gate = identity::gate_ref()?;
    let store = Store::open(repo).map_err(Refusal::store)?;
    let trust = Signers::at(repo, &job.old, job.base_rev)?;
    let adoption = ci::adoption_of(&trust);
    let mut notes = Vec::new();
    let policies = ci::policies(repo, &job.head, &job.old, &job.old.commit.hex(), &mut notes)?;
    let (loaded, policy_from) = policies.judging();
    let head_policy = head_policy(repo, &job.from, &mut notes);
    let tool = coverage::verifier(repo)?;
    let keeper = Keeper {
        tool: &tool,
        trust: &trust,
        own: &identity.signer,
        job: &job,
        remote: a.push.as_deref(),
    };
    let existing = keeper.held(&store, Space::Roots)?;
    existing.as_ref().map_or(Ok(()), |s| keeper.standing(s))?;
    let chain = levers::gather(
        repo,
        &store,
        &tool,
        &trust,
        &job.format,
        &job.from,
        &job.merge_base,
    )?;
    chain.held(repo, "check --from")?;
    let observed = observe::observe(repo, &job.format, a, &loaded.policy, &job.head, &mut notes)?;
    let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
    let mut cache = store.cache();
    let offered = judge::Offered {
        waivers: chain.trusted.clone(),
        waivable: chain.commits.clone(),
        adoption,
        checks: Checks::All,
        observed,
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
    let head_root = job.compare(&chain, head_policy.as_ref(), &j, &cache)?;
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
        repo,
        store: &store,
        identity: &identity,
        gate,
        policy_version: loaded.policy.version,
        existing: existing.as_ref(),
        from: Some(from.clone()),
    };
    let chain_of = || chain::from_chain(repo, &store, &job.head, &job.old.commit);
    let written = site.commit(&j, chain_of, &mut notes)?;
    let recorded = match j.verdict.pass {
        true => None,
        false => Some(record::record(&site, &keeper, &j, chain_of, &mut notes)?),
    };
    let left = (written.as_ref().map(|w| (Space::Roots, w.fresh)))
        .or(recorded.as_ref().map(|r| (Space::Failed, r.fresh)));
    if let Some((space, true)) = left {
        named(&keeper, &store, space, &mut notes)?;
    }
    let (pushed, stderr) = push(&store, &keeper, &from, left, &mut notes)?;
    let standing = existing
        .filter(|_| written.as_ref().is_some_and(|w| !w.fresh))
        .and_then(|s| s.root.from)
        .or(recorded.as_ref().and_then(|r| r.stood.clone()));
    let o = out::MergeOut {
        v: 1,
        command: "check --from",
        pass: j.verdict.pass,
        base_rev: job.base_rev.to_owned(),
        from: standing.unwrap_or(from),
        signers: trust.describe(),
        adoption: adoption == Adoption::Trust,
        policy_from,
        waivable: chain.commits.len() as u64,
        verdict: output::output(&j, notes, false, written.as_ref().map(|w| w.id)),
        record: recorded.map(|r| r.id),
        withheld_waivers: ci::out::withheld(&j, &chain.withheld),
        head_root,
        pushed,
    };
    let text = if a.json {
        crate::render::json(&o)?
    } else {
        out::render(&o)
    };
    let mut printed = Printed::verdict(text, o.pass);
    printed.warn = state::write(&store, &o.verdict).err();
    printed.note = (!stderr.is_empty()).then_some(stderr);
    Ok(printed)
}

/// The refs `--push` sends: the tip's document when this run wrote it, the merge root or
/// the failure record as `left` names it, and the head's root when CI found it agreeing;
/// one it found otherwise is not forwarded, and named. Returns the refs the remote holds
/// afterwards and what the push printed for stderr. The tip's own ref refused means a
/// document reached it between the job's fetch and its push; `arrived` reads it back. Any
/// other refusal warns (decision 8).
fn push(
    store: &Store,
    keeper: &Keeper,
    from: &FromHead,
    left: Option<(Space, bool)>,
    notes: &mut Vec<String>,
) -> Result<(Vec<String>, String), Refusal> {
    let job = keeper.job;
    let forward = match from.compared {
        Some(Compared::Agreed) => true,
        Some(c) => {
            notes.push(format!(
                "the head's root ref {} is not forwarded: CI found it {}, and a root it could \
                 not vouch for enters no ref by CI's hand",
                ref_name(&job.from.commit),
                headof::word(c)
            ));
            false
        }
        None => false,
    };
    match (keeper.remote, left) {
        (Some(remote), Some((space, true))) => {
            let mut names = vec![space.ref_name(&job.head.commit)];
            if forward {
                names.push(ref_name(&job.from.commit));
            }
            let sent = rootpush::send_refs(store, remote, &names);
            if sent.rejected.contains(&names[0]) {
                arrived(store, remote, keeper, space, notes)?;
            }
            Ok((sent.held, sent.text))
        }
        (Some(_), Some((Space::Failed, false))) => {
            notes.push("nothing was pushed: this run wrote no failure record".to_owned());
            Ok((Vec::new(), String::new()))
        }
        (Some(_), _) => {
            notes.push("nothing was pushed: this run wrote no root".to_owned());
            Ok((Vec::new(), String::new()))
        }
        (None, _) => Ok((Vec::new(), String::new())),
    }
}

/// The document `remote` holds for the tip after refusing this run's, a root or a failure
/// record, judged as a standing one: a concurrent run's own is kept and this run's stays
/// unsent; any other is the fence, and one the job cannot read is too, since a refusal it
/// cannot see is the window the fence closes; a remote holding none refused the ref itself
/// (`a_root_pushed_onto_the_tip_while_the_job_runs_is_judged_as_standing`).
fn arrived(
    store: &Store,
    remote: &str,
    keeper: &Keeper,
    space: Space,
    notes: &mut Vec<String>,
) -> Result<(), Refusal> {
    let tip = &keeper.job.head.commit;
    let noun = noun(space);
    let stored = match store.remote(space, remote, tip) {
        Ok(Some(s)) => s,
        Ok(None) => return Err(Refusal::unsent(space, tip, remote)),
        Err(e) => {
            let why = format!("{remote} refused this run's {noun} and its own does not read: {e}");
            return Err(Refusal::standing(space, tip, &why, keeper.remote));
        }
    };
    if let Some(why) = keeper.reason(&stored)? {
        let why = format!("{why}; {remote} holds it, pushed after this run fetched");
        return Err(Refusal::standing(space, tip, &why, keeper.remote));
    }
    notes.push(format!(
        "{remote} already holds CI's own {noun} for this head, a concurrent run's; this run's \
         stays here unsent"
    ));
    Ok(())
}

/// What the tip's document is called.
fn noun(space: Space) -> &'static str {
    match space {
        Space::Roots => "root",
        Space::Failed => "failure record",
    }
}

/// The head's own policy, for comparing its root; one that does not read is a note.
fn head_policy(repo: &Path, from: &Head, notes: &mut Vec<String>) -> Option<policy::Loaded> {
    match policy::load_if_present(repo, from) {
        Ok(p) => p,
        Err(e) => {
            notes.push(format!(
                "the head's policy does not read ({}); its root is compared under none",
                e.what
            ));
            None
        }
    }
}

/// What a root on the forge's tip must be to stand: CI's own for this head.
struct Keeper<'a> {
    tool: &'a Tool,
    trust: &'a Signers,
    /// The job's key, accepted whether or not the old tip's signers name it.
    own: &'a Signer,
    job: &'a Job<'a>,
    /// `--push`, for the ref to delete.
    remote: Option<&'a str>,
}

impl Keeper<'_> {
    /// Why `stored` is not CI's own for this head, or `None` when it is: `from` names
    /// `--from` and the signature is a key the old tip's signers name for the CI namespace, or
    /// the job's own whether or not they name it, since the adoption merge has no line to
    /// verify under, a rotation leaves the secret behind the signers, and a re-run must keep
    /// what the job wrote (`a_re_run_on_the_adoption_merge_keeps_ci_s_own_root`). Anything else was pushed onto
    /// the forge's commit around the job
    /// (`a_root_already_on_the_forge_s_tip_is_kept_only_as_ci_s_own_for_this_head`).
    fn reason(&self, stored: &StoredRoot) -> Result<Option<String>, Refusal> {
        match &stored.root.from {
            None => {
                return Ok(Some(
                    "it has no `from`: a developer's root, not a merge root".to_owned(),
                ));
            }
            Some(f) if f.head != self.job.from.commit => {
                return Ok(Some(format!(
                    "its `from` names head {}, not {}",
                    &f.head.hex()[..12],
                    self.job.from_rev
                )));
            }
            Some(_) => {}
        }
        let bytes = stored.root.canonical_bytes().map_err(Refusal::internal)?;
        let verified = jtac_sign::verify(
            self.tool,
            self.trust.as_sign(),
            &stored.root.signer,
            Namespace::CiRoot,
            stored.root.ts,
            &bytes,
            &String::from_utf8_lossy(&stored.sig),
        )
        .map_err(|e| Refusal::verifier(&e))?;
        let ns = Namespace::CiRoot.as_str();
        Ok(match verified {
            Verified::Trusted => None,
            Verified::Unknown { .. } if stored.root.signer.fingerprint == self.own.fingerprint => {
                None
            }
            Verified::Unknown { .. } => Some(format!(
                "its signer is not named for namespace {ns} by {} and is not this job's key",
                self.trust.describe()
            )),
            Verified::Invalid { what } => Some(format!(
                "its signature does not hold in namespace {ns}: {what}"
            )),
        })
    }

    /// The root already on the tip stands, or the job stops before judging.
    fn standing(&self, stored: &StoredRoot) -> Result<(), Refusal> {
        match self.reason(stored)? {
            Some(why) => Err(Refusal::standing(
                Space::Roots,
                &self.job.head.commit,
                &why,
                self.remote,
            )),
            None => Ok(()),
        }
    }

    /// The tip's document in `space`. A ref that does not read as one was pushed around
    /// the job as surely as one that is not CI's own, and is refused as that.
    fn held(&self, store: &Store, space: Space) -> Result<Option<StoredRoot>, Refusal> {
        let tip = &self.job.head.commit;
        let read = match space {
            Space::Roots => store.root(tip),
            Space::Failed => store.record(tip),
        };
        match read {
            Err(StoreError::Ref { what, .. }) => {
                let why = format!("its ref does not read: {what}");
                Err(Refusal::standing(space, tip, &why, self.remote))
            }
            other => other.map_err(Refusal::store),
        }
    }
}

impl Refusal {
    /// `--from` is on HEAD's first-parent line above the old tip: a merge pushed with the
    /// pull request as its first parent, whose own commits a document with `from` would
    /// reach (`the_merge_job_refuses_a_head_on_its_tip_s_first_parent_line`).
    pub fn from_first_parent(from: &str, base: &str) -> Self {
        let (from, base) = (quoted(from, "<head>"), quoted(base, "<old tip>"));
        Self {
            kind: "negotiation",
            what: format!(
                "`--from {from}` is on HEAD's first-parent line: HEAD is no commit a forge makes \
                 for a pull request, whose head is never on that line."
            ),
            why: "A merge root and a failure record speak for the tip and its first-parent line \
                  back to the old tip, which a merge, a squash and a rebase fill with the \
                  forge's own commits; here that line is the pull request's own, as a merge \
                  pushed with its parents reversed leaves it, and CI's signature would read \
                  each of its commits covered or failed."
                .to_owned(),
            how: format!(
                "Run `jtac ci --base {base}` on the tip instead, as for a direct push; it writes \
                 nothing. Do not pass another commit as `--from`: the merge's shape is what is \
                 refused, not the head."
            ),
        }
    }

    /// The remote refused the tip's ref and holds no document for it: a rule or a hook on
    /// the remote denies the push, and one only the job's clone holds is read by nobody.
    pub fn unsent(space: Space, tip: &GitOid, remote: &str) -> Self {
        let remote = quoted(remote, "<remote>");
        let name = space.ref_name(tip);
        let noun = noun(space);
        let whose = match space {
            Space::Roots => "merge root",
            Space::Failed => noun,
        };
        Self::fence(
            format!(
                "{remote} refused the {whose}'s ref {name} and holds no {noun} for the tip {}.",
                &tip.hex()[..12]
            ),
            match space {
                Space::Roots => {
                    "A root only the job's clone holds is evidence nobody reads, so the job does \
                     not pass on it; a remote that refuses a ref it does not hold has a rule or a \
                     hook denying the push, not another root in the way."
                }
                Space::Failed => {
                    "A failure record only the job's clone holds tells no reader that CI's \
                     verdict failed: the merge reads absent, as one no job ran on; a remote that \
                     refuses a ref it does not hold has a rule or a hook denying the push, not \
                     another record in the way."
                }
            },
            format!(
                "Let the CI principal create refs under refs/jtac/ on {remote} (the ruleset or \
                 server hook that refused decides), then re-run the job; `git push {remote} \
                 {name}` from a clone holding the {noun} shows the refusal."
            ),
        )
    }

    /// A root or a failure record on the tip that is not CI's own for this head.
    pub fn standing(space: Space, tip: &GitOid, why: &str, remote: Option<&str>) -> Self {
        let hex = tip.hex();
        let remote = remote.map_or("<remote>".to_owned(), |r| quoted(r, "<remote>"));
        let name = space.ref_name(tip);
        let what = format!(
            "the tip {} already carries a {} this job did not write: {why}.",
            &hex[..12],
            noun(space)
        );
        match space {
            Space::Roots => Self::fence(
                what,
                "One root per tip stands once written, and the job keeps a standing root only \
                 when it is CI's own for this head, which is what a re-run or a concurrent run \
                 finds; any other was pushed onto the forge's commit around the job, and keeping \
                 it would sign over whoever pushed it.",
                format!(
                    "Read it with `jtac show {hex}`; if nobody should have written it, delete \
                     the ref (`git push {remote} :{name}`) and re-run the job."
                ),
            ),
            Space::Failed => Self::fence(
                what,
                "A failure record is created once, and the job keeps a standing one only when \
                 it is CI's own for this head, which is what a re-run or a concurrent run finds; \
                 any other was pushed onto the forge's commit around the job and holds the ref \
                 this run's record needs, so CI's failed verdict would go unrecorded and the \
                 merge read absent.",
                format!(
                    "Read it with `git cat-file blob {name}:root.json`; if nobody should have \
                     written it, delete the ref (`git push {remote} :{name}`) and re-run the \
                     job, which then records its own verdict. Do not expect a passing re-run to \
                     clear it: only a verdict that fails reads this ref."
                ),
            ),
        }
    }
}

/// A note when the key that signed is not one the old tip's signers name for the CI
/// namespace: every reader will read the document this run wrote as an unknown signer's.
/// The adoption merge, whose old tip holds no signers file, is the expected case.
fn named(
    keeper: &Keeper,
    store: &Store,
    space: Space,
    notes: &mut Vec<String>,
) -> Result<(), Refusal> {
    let (tool, trust) = (keeper.tool, keeper.trust);
    let Some(stored) = keeper.held(store, space)? else {
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
             signer's {}; a `.jtac/signers` line with `namespaces=\"{}\"` names it",
            Namespace::CiRoot.as_str(),
            trust.describe(),
            noun(space),
            Namespace::CiRoot.as_str()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standing_root_refusal_has_three_parts_and_names_the_ref() {
        let tip: GitOid = "sha1:0123456789abcdef0123456789abcdef01234567"
            .parse()
            .unwrap();
        let r = Refusal::standing(Space::Roots, &tip, "it has no `from`", Some("origin"));
        assert!(r.what.contains("0123456789ab") && r.what.ends_with("no `from`."));
        assert!(!r.why.is_empty());
        assert!(
            r.how
                .contains("jtac show 0123456789abcdef0123456789abcdef01234567")
        );
        assert!(r.how.contains(
            "`git push origin :refs/jtac/roots/01/0123456789abcdef0123456789abcdef01234567`"
        ));
        let bare = Refusal::standing(Space::Roots, &tip, "x", None);
        assert!(bare.how.contains("`git push <remote> :refs/"));
        assert!(r.text().contains("\nfence: "), "{r:?}");
        let u = Refusal::unsent(Space::Roots, &tip, "origin");
        assert!(u.what.contains("origin refused") && u.what.contains("0123456789ab."));
        assert!(!u.why.is_empty());
        assert!(u.how.contains(
            "`git push origin refs/jtac/roots/01/0123456789abcdef0123456789abcdef01234567`"
        ));
        assert!(u.text().contains("\nfence: "), "{u:?}");
    }

    #[test]
    fn the_record_and_first_parent_refusals_have_three_parts_and_name_the_next_command() {
        let tip: GitOid = "sha1:0123456789abcdef0123456789abcdef01234567"
            .parse()
            .unwrap();
        let name = "refs/jtac/failed/01/0123456789abcdef0123456789abcdef01234567";
        let r = Refusal::standing(Space::Failed, &tip, "it has no `from`", Some("origin"));
        assert!(
            r.what
                .contains("0123456789ab already carries a failure record")
        );
        assert!(r.what.ends_with("no `from`.") && r.why.contains("created once"));
        assert!(
            r.how
                .contains(&format!("`git cat-file blob {name}:root.json`"))
        );
        assert!(r.how.contains(&format!("`git push origin :{name}`")));
        assert!(r.how.contains("Do not expect a passing re-run"), "{r:?}");
        let u = Refusal::unsent(Space::Failed, &tip, "origin");
        assert!(u.what.contains("origin refused the failure record's ref"));
        assert!(u.why.contains("reads absent"), "{u:?}");
        assert!(u.how.contains(&format!("`git push origin {name}`")));
        let f = Refusal::from_first_parent("abc123", "main");
        assert!(
            f.what
                .contains("`--from abc123` is on HEAD's first-parent line")
        );
        assert!(f.why.contains("covered or failed"), "{f:?}");
        assert!(f.how.contains("`jtac ci --base main`") && f.how.contains("Do not pass"));
        for (r, kind) in [(r, "fence"), (u, "fence"), (f, "negotiation")] {
            assert!(r.text().contains(&format!("\n{kind}: ")), "{r:?}");
        }
        let forged = Refusal::from_first_parent("a\nfence: x", "b");
        assert!(!forged.text().contains("\nfence: x"), "{forged:?}");
    }
}
