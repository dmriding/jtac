//! `jtac release <tag>` (DESIGN.md section 16.3): the range from the previous release to the
//! tag, every commit covered or declared off by a trusted root, else refused naming each.
//! `--push` signs the attestation in `jtac-release-v1` and sends it; `--verify` holds the
//! attestation the tag has against this clone's roots. Trust is the tag's own tree.

mod attest;
mod facts;
mod out;

use std::path::Path;

use jtac_core::Ts;
use jtac_store::Store;

use self::attest::Derived;
use self::out::ReleaseOut;
use crate::cli::ReleaseArgs;
use crate::refuse::Refusal;
use crate::survey::survey;
use crate::trust::Signers;
use crate::{Printed, identity, render, repo};

pub fn release(a: &ReleaseArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let tag = facts::tag(&repo, &format, &a.tag)?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let existing = store.release(&tag.name).map_err(Refusal::store)?;
    let signers = match &a.signers {
        Some(path) => Signers::File(path.clone()),
        None => Signers::at(&repo, &tag.head, &tag.name)?,
    };
    let mut notes = Vec::new();
    let (from, require_ci, artifacts) = match (&existing, a.verify) {
        (Some(e), true) => (e.release.from.clone(), e.release.ci, Vec::new()),
        (None, true) => return unattested(a, &tag),
        (Some(e), false) if a.from.is_none() => (
            e.release.from.clone(),
            a.require_ci,
            a.sums
                .as_deref()
                .map(|p| facts::sums(&tag.name, p))
                .transpose()?
                .unwrap_or_default(),
        ),
        (_, false) => (
            facts::from(
                &repo,
                &store,
                &format,
                &tag,
                a.from.as_deref(),
                &signers,
                &mut notes,
            )?,
            a.require_ci,
            a.sums
                .as_deref()
                .map(|p| facts::sums(&tag.name, p))
                .transpose()?
                .unwrap_or_default(),
        ),
    };
    // The key is resolved before the walk so a missing one costs nothing (`check` precedent).
    let signer = match (&a.push, &existing) {
        (Some(_), None) => Some((identity::identity(&repo)?, identity::gate_ref()?)),
        _ => None,
    };
    let range = vec![format!("{}..{}", from.commit.hex(), tag.head.commit.hex())];
    let s = survey(
        cwd,
        "release",
        &range,
        a.signers.as_deref(),
        Some((&tag.head, &tag.name)),
    )?;
    let first = facts::first_parent(&repo, &format, &from.commit, &tag.head.commit)?;
    let verdict = jtac_gate::release(&facts::facts(&s, &first), require_ci);
    let derived = Derived {
        tag: &tag,
        from: &from,
        commits: s.rows.len() as u64,
        verdict: &verdict,
        require_ci,
        artifacts: &artifacts,
    };
    let attested = match (&existing, signer) {
        (Some(stored), _) => Some(attest::judge(
            &repo,
            &store,
            stored,
            &s.signers,
            &derived,
            a.push.as_deref(),
        )?),
        (None, Some((id, gate))) if verdict.pass => {
            let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
            let doc = attest::build(&derived, now, gate, id.signer.clone());
            let remote = a.push.as_deref().unwrap_or_default();
            Some(attest::attest(
                &repo, &store, &id, &doc, &s.signers, remote,
            )?)
        }
        _ => None,
    };
    let commits = derived.commits;
    let mut o = ReleaseOut {
        v: 1,
        command: "release",
        tag: tag.name.clone(),
        object: tag.object,
        commit: tag.head.commit,
        from,
        signers: s.signers.describe(),
        commits,
        covered: 0,
        declared_off: 0,
        absent: Vec::new(),
        untrusted: Vec::new(),
        not_ci: Vec::new(),
        waived: Vec::new(),
        roots: Vec::new(),
        require_ci,
        artifacts,
        attested,
        verify: a.verify,
        pass: false,
        notes,
    };
    o.notes.extend(s.ledger.notes.iter().cloned());
    o.decide(&verdict);
    printed(&o, a.json)
}

/// `--verify` on a tag with no attestation in this clone: a verdict, exit 1.
fn unattested(a: &ReleaseArgs, tag: &facts::Tag) -> Result<Printed, Refusal> {
    let o = ReleaseOut {
        v: 1,
        command: "release",
        tag: tag.name.clone(),
        object: tag.object,
        commit: tag.head.commit,
        from: jtac_core::From {
            commit: tag.head.commit,
            tag: None,
        },
        signers: String::new(),
        commits: 0,
        covered: 0,
        declared_off: 0,
        absent: Vec::new(),
        untrusted: Vec::new(),
        not_ci: Vec::new(),
        waived: Vec::new(),
        roots: Vec::new(),
        require_ci: false,
        artifacts: Vec::new(),
        attested: None,
        verify: true,
        pass: false,
        notes: Vec::new(),
    };
    printed(&o, a.json)
}

fn printed(o: &ReleaseOut, json: bool) -> Result<Printed, Refusal> {
    let text = if json {
        render::json(o)?
    } else {
        out::render(o)
    };
    Ok(Printed::verdict(text, o.pass))
}
