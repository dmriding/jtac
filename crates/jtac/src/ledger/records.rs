//! The failure records that reach a range (DESIGN.md section 15.2), read from
//! `refs/jtac/failed/` and judged by the ledger's one judge, as the CI root each would have
//! been. No record stops a command: a ref that holds none, or one that does not hold as a
//! document, is a note, and one not trusted whole is kept and counts for nothing
//! (`a_record_that_is_not_ci_s_own_stops_the_job_on_a_failed_verdict_and_no_other_command`).

use std::collections::BTreeSet;

use jtac_core::GitOid;
use jtac_gate::{RecordFact, Trust};
use jtac_store::Space;

use super::{Judge, Ledger, Site, covered};
use crate::range::{self, Listed};
use crate::refuse::Refusal;
use crate::repo;

impl Ledger {
    /// Keep each record whose commit or whose `covers` is in `commits`, with its reach. A
    /// record for a commit this clone lacks reaches nothing it holds and is not read.
    pub fn read_records(&mut self, site: &Site, commits: &[Listed]) -> Result<(), Refusal> {
        let Site { repo, format, .. } = *site;
        let listed = site.store.records().map_err(Refusal::store)?;
        self.notes.extend(listed.notes());
        let in_range: BTreeSet<GitOid> = commits.iter().map(|c| c.commit).collect();
        let mut touched = Vec::new();
        for read in site.store.read_all(&listed).map_err(Refusal::store)? {
            let commit = read.listed.commit;
            match read.stored {
                Ok(s) => {
                    let covers = covered(repo, format, &s.root)?;
                    if in_range.contains(&commit) || covers.iter().any(|c| in_range.contains(c)) {
                        touched.push((s, covers));
                    }
                }
                Err(why) if in_range.contains(&commit) => self.notes.push(format!(
                    "{} holds no failure record and is not read: {why}",
                    Space::Failed.ref_name(&commit)
                )),
                Err(_) => {}
            }
        }
        let tips: Vec<GitOid> = touched.iter().map(|(s, _)| s.root.commit).collect();
        let trees = range::trees(repo, format, &tips)?;
        let kinds = range::kinds(repo, &tips)?;
        let shallow = repo::shallow(repo).map_err(|e| Refusal::repository(&e))?;
        for (k, (s, covers)) in touched.into_iter().enumerate() {
            if trees[k].is_none() {
                continue;
            }
            let judge = Judge {
                repo,
                tool: site.tool,
                signers: site.signers,
                command: site.command,
                format,
                shallow,
                based: false,
                noun: "failure record",
                ids: &self.ids,
                policies: &self.policies,
                notes: &mut self.notes,
            };
            let fact = judge.root(&s, &covers, trees[k], kinds[k].as_deref())?;
            let commit = s.root.commit;
            if let Some(why) = &fact.invalid {
                self.notes.push(format!(
                    "{} is not read: {why}",
                    Space::Failed.ref_name(&commit)
                ));
                continue;
            }
            if !fact.whole() {
                self.notes.push(format!(
                    "{}'s failure record counts for nothing: {}",
                    commit.hex(),
                    match &fact.trust {
                        Trust::Unknown { why } => why,
                        _ =>
                            "a waiver it carries is signed by a key the trust in force does \
                              not name",
                    }
                ));
            }
            self.records.push(RecordFact {
                commit,
                reach: fact.reach.clone(),
                trusted: fact.whole(),
            });
            self.failed.push(s);
        }
        Ok(())
    }
}
