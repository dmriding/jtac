//! What `jtac release` prints (DESIGN.md sections 7.1 and 16.3): the verdict on the range,
//! each commit it names with its fix, and the attestation's standing. Versioned with `v`.

use std::fmt::Write;

use jtac_checks::git::quoted;
use jtac_core::{Artifact, From, GitOid, Sha256Digest};
use jtac_gate::ReleaseVerdict;
use serde::Serialize;

use super::attest::Attested;
use crate::output::plain;
use crate::survey::short;

/// One run, as `--json` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReleaseOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The tag.
    pub tag: String,
    /// The tag object.
    pub object: GitOid,
    /// Its commit.
    pub commit: GitOid,
    /// Where the range starts.
    pub from: From,
    /// Where trust came from.
    pub signers: String,
    /// Commits in the range.
    pub commits: u64,
    /// Covered, by either gate.
    pub covered: u64,
    /// Declared off, by either gate; listed in `waived`.
    pub declared_off: u64,
    /// No root.
    pub absent: Vec<GitOid>,
    /// A root the trust file does not vouch for.
    pub untrusted: Vec<GitOid>,
    /// First-parent commits the CI gate never judged, under `--require-ci`.
    pub not_ci: Vec<GitOid>,
    /// Declared off.
    pub waived: Vec<GitOid>,
    /// Every root the range stands on.
    pub roots: Vec<Sha256Digest>,
    /// `--require-ci`.
    pub require_ci: bool,
    /// From `--sums`.
    pub artifacts: Vec<Artifact>,
    /// The attestation, written or found.
    pub attested: Option<Attested>,
    /// `--verify`.
    pub verify: bool,
    /// The range passes and the attestation, where there is one, holds.
    pub pass: bool,
    /// Skipped refs, parent roots this clone lacks.
    pub notes: Vec<String>,
}

impl ReleaseOut {
    /// Fold the verdict in and decide.
    pub fn decide(&mut self, v: &ReleaseVerdict) {
        self.absent.clone_from(&v.absent);
        self.untrusted.clone_from(&v.untrusted);
        self.not_ci.clone_from(&v.not_ci);
        self.waived.clone_from(&v.waived);
        self.roots.clone_from(&v.roots);
        self.declared_off = v.waived.len() as u64;
        let named = (v.absent.len() + v.untrusted.len() + v.not_ci.len()) as u64;
        self.covered = self
            .commits
            .saturating_sub(self.declared_off)
            .saturating_sub(named);
        self.pass = v.pass
            && match &self.attested {
                Some(a) => a.holds(),
                None => !self.verify,
            };
    }

    /// The first word after the tag.
    fn word(&self) -> &'static str {
        match (&self.attested, self.pass, self.verify) {
            (_, false, true) if self.attested.is_none() => "UNATTESTED",
            (Some(a), _, _) if !a.holds() => "INVALID",
            (_, false, _) => "REFUSED",
            (Some(_), true, _) => "ATTESTED",
            (None, true, _) => "CLEAR",
        }
    }
}

/// Human form.
pub fn render(o: &ReleaseOut) -> String {
    let mut s = String::new();
    let since = match &o.from.tag {
        Some(tag) => format!("since {}", plain(tag)),
        None => format!("after {}", short(&o.from.commit)),
    };
    let _ = writeln!(
        s,
        "RELEASE {} {}: {} commits {since}, {} covered, {} declared off{}",
        o.tag,
        o.word(),
        o.commits,
        o.covered,
        o.declared_off,
        if o.require_ci {
            "; first-parent line held to CI"
        } else {
            ""
        }
    );
    let _ = writeln!(s, "  signers   {}", o.signers);
    for c in &o.waived {
        let _ = writeln!(s, "  waived    {}", short(c));
    }
    for c in &o.absent {
        let _ = writeln!(s, "  absent    {} no root", c.hex());
    }
    for c in &o.untrusted {
        let _ = writeln!(
            s,
            "  untrusted {} its root's signer, or an applied waiver's, is not in the trust file",
            c.hex()
        );
    }
    for c in &o.not_ci {
        let _ = writeln!(
            s,
            "  not ci    {} on the first-parent line without CI's root",
            c.hex()
        );
    }
    let tag = quoted(&o.tag, "<tag>");
    let waive = "`jtac waive <commit> --ci --reason <why>` with that commit checked out, then \
                 `jtac check` there, so a root carries the waiver";
    if !o.absent.is_empty() {
        let _ = writeln!(
            s,
            "  fix       absent: `git fetch <remote> '+refs/jtac/*:refs/jtac/*'` brings a root \
             that was pushed; a commit merged around the gate takes {waive}; then tag the next \
             patch, since a tag is never moved"
        );
    }
    if !o.untrusted.is_empty() {
        let _ = writeln!(
            s,
            "  fix       untrusted: add the signer's key to .jtac/signers in a reviewed commit \
             the next tag descends from, or {waive}"
        );
    }
    if !o.not_ci.is_empty() {
        let _ = writeln!(
            s,
            "  fix       not ci: tag the merge commit the forge made, not the pull request's \
             own; a merge made around CI takes {waive}"
        );
    }
    if !o.artifacts.is_empty() {
        let _ = writeln!(s, "  artifacts {}", o.artifacts.len());
    }
    match &o.attested {
        Some(a) => {
            let _ = writeln!(
                s,
                "  attested  {} by {} ({}) at {}, {}{}",
                a.id,
                plain(&a.signer.principal),
                a.trust,
                a.reference,
                a.origin,
                a.pushed
                    .as_deref()
                    .map_or_else(String::new, |p| format!(", push {p}"))
            );
            if let Some(why) = &a.disagrees {
                let _ = writeln!(s, "  disagrees {}", plain(why));
            }
            if a.pushed
                .as_deref()
                .is_some_and(|p| p.starts_with("rejected"))
            {
                let _ = writeln!(
                    s,
                    "  fix       the remote already holds an attestation for {tag}: `git fetch \
                     <remote> '+refs/jtac/*:refs/jtac/*'` and run again; the one that landed \
                     first stands"
                );
            } else if a.disagrees.is_some() && !o.absent.is_empty() {
                let _ = writeln!(
                    s,
                    "  fix       this clone lacks roots the attestation stands on: `git fetch \
                     <remote> '+refs/jtac/*:refs/jtac/*'` and run again"
                );
            } else if !a.holds() {
                let _ = writeln!(
                    s,
                    "  fix       an attestation that does not hold is not replaced: a tag is \
                     attested once; `jtac audit {}..{}` shows the roots, and the next patch \
                     gets its own",
                    quoted(&o.from.commit.hex(), "<from>"),
                    tag
                );
            }
        }
        None if o.verify => {
            let _ = writeln!(
                s,
                "  fix       no attestation at refs/jtac/releases/{}: `git fetch <remote> \
                 '+refs/jtac/*:refs/jtac/*'` brings one that was pushed; the release job \
                 writes it with `jtac release {tag} --push <remote>`",
                o.tag
            );
        }
        None if o.pass => {
            let _ = writeln!(
                s,
                "  next      `jtac release {tag} --push <remote>` from the release job signs \
                 and sends the attestation"
            );
        }
        None => {}
    }
    for note in &o.notes {
        let _ = writeln!(s, "  note      {}", plain(note));
    }
    s
}
