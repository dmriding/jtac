//! What `jtac release` gathers (DESIGN.md section 16.3): the tag as an object and a commit,
//! the range's start, the first-parent line, the sums file, and one fact per commit from
//! the survey's rows. Decides nothing: `jtac_gate::release` does.

use std::collections::BTreeSet;
use std::path::Path;

use jtac_checks::git::{GitError, git};
use jtac_core::{Artifact, From, GitOid, Sha256Digest};
use jtac_gate::{ReleaseFact, Trust, Via};
use jtac_store::Store;

use crate::range;
use crate::refuse::Refusal;
use crate::repo::{self, Head, RepoError};
use crate::survey::Survey;
use crate::trust::Signers;

use super::attest;

/// A tag resolved once: the object the ref points at and the commit it peels to.
pub struct Tag {
    /// The short name as given.
    pub name: String,
    /// The tag object when annotated, else the commit.
    pub object: GitOid,
    /// The commit and its tree.
    pub head: Head,
}

/// `refs/tags/<name>`, refused when there is no such tag.
pub fn tag(repo: &Path, format: &str, name: &str) -> Result<Tag, Refusal> {
    let full = format!("refs/tags/{name}");
    if name.is_empty() || git(repo, &["check-ref-format", &full]).is_err() {
        return Err(Refusal::not_a_tag(name));
    }
    let args = [
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &full,
    ];
    let out = match git(repo, &args) {
        Ok(out) => out,
        Err(GitError::Failed {
            status: Some(1), ..
        }) => return Err(Refusal::not_a_tag(name)),
        Err(e) => return Err(Refusal::repository(&RepoError::Git(e))),
    };
    let object = repo::oid(format, String::from_utf8_lossy(&out).trim())
        .map_err(|e| Refusal::repository(&e))?;
    let head = repo::peel(repo, format, &full).map_err(|e| Refusal::repository(&e))?;
    Ok(Tag {
        name: name.to_owned(),
        object,
        head,
    })
}

/// The range's start: `rev` when given, an ancestor of the tag or refused, recorded as a
/// commit alone; else the nearest attested release that is an ancestor and verifies
/// trusted under the tag's own trust, so releases chain through documents a reader can
/// hold them to. A tag with neither is refused, as is a start that is the tag's own
/// commit, which would judge nothing.
pub fn from(
    repo: &Path,
    store: &Store,
    format: &str,
    tag: &Tag,
    rev: Option<&str>,
    signers: &Signers,
    notes: &mut Vec<String>,
) -> Result<From, Refusal> {
    let from = match rev {
        Some(rev) => {
            let head = repo::peel(repo, format, rev).map_err(|_| Refusal::rev("release", rev))?;
            if !range::ancestor(repo, &head.commit, &tag.head.commit)? {
                return Err(Refusal::release_from(&tag.name, rev));
            }
            From {
                commit: head.commit,
                tag: None,
            }
        }
        None => previous(repo, store, tag, signers, notes)?
            .ok_or_else(|| Refusal::no_from(&tag.name))?,
    };
    if from.commit == tag.head.commit {
        return Err(Refusal::empty_range(&tag.name, &from.commit.hex()));
    }
    Ok(from)
}

/// The trusted attestation nearest below the tag: the ancestor with the fewest commits up
/// to the tag's. A ref under the namespace that does not read, does not verify trusted, or
/// names a commit this clone lacks is not a start; it is noted, since anyone with push
/// access could have put it there, and a release must not be shortened or stopped by it.
fn previous(
    repo: &Path,
    store: &Store,
    tag: &Tag,
    signers: &Signers,
    notes: &mut Vec<String>,
) -> Result<Option<From>, Refusal> {
    let mut nearest: Option<(u64, From)> = None;
    for name in store.release_tags().map_err(Refusal::store)? {
        let skipped = |why: String| {
            format!(
                "skipped {}: {why}; not a start",
                jtac_store::release_ref(&name)
            )
        };
        let stored = match store.release(&name) {
            Ok(Some(stored)) => stored,
            Ok(None) => continue,
            Err(e) => {
                notes.push(skipped(e.to_string()));
                continue;
            }
        };
        let trust = attest::trust_of(repo, signers, &stored)?;
        if trust != "trusted" {
            notes.push(skipped(format!("its signature reads {trust}")));
            continue;
        }
        let r = stored.release;
        if r.commit == tag.head.commit
            || range::kinds(repo, &[r.commit])?[0].as_deref() != Some("commit")
            || !range::ancestor(repo, &r.commit, &tag.head.commit)?
        {
            continue;
        }
        let span = format!("{}..{}", r.commit.hex(), tag.head.commit.hex());
        let args = ["rev-list", "--count", "--end-of-options", &span];
        let out = git(repo, &args).map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
        let count: u64 = String::from_utf8_lossy(&out)
            .trim()
            .parse()
            .map_err(|_| Refusal::internal("rev-list --count printed no number"))?;
        if nearest.as_ref().is_none_or(|(n, _)| count < *n) {
            nearest = Some((
                count,
                From {
                    commit: r.commit,
                    tag: Some(r.tag),
                },
            ));
        }
    }
    Ok(nearest.map(|(_, from)| from))
}

/// The commits on the first-parent line from `tip` back to, and excluding, `from`.
pub fn first_parent(
    repo: &Path,
    format: &str,
    from: &GitOid,
    tip: &GitOid,
) -> Result<BTreeSet<GitOid>, Refusal> {
    let (tip, not) = (tip.hex(), format!("^{}", from.hex()));
    let args = ["rev-list", "--first-parent", "--end-of-options", &tip, &not];
    let out = git(repo, &args).map_err(|e| Refusal::repository(&RepoError::Git(e)))?;
    String::from_utf8_lossy(&out)
        .lines()
        .map(|hex| repo::oid(format, hex).map_err(Refusal::internal))
        .collect()
}

/// One fact per row. Trusted means the root's signer and every applied waiver's signer are
/// named: a covered verdict that leaned on an unknown key's waiver is an unknown key's.
pub fn facts(s: &Survey, first: &BTreeSet<GitOid>) -> Vec<ReleaseFact> {
    s.rows
        .iter()
        .map(|r| {
            let fact = r.root.map(|i| &s.ledger.roots[i]);
            let stored = r.root.and_then(|i| s.ledger.stored[i].as_ref());
            let trusted = fact.is_some_and(|f| {
                f.trust == Trust::Trusted && f.waiver_trust.iter().all(|t| *t == Trust::Trusted)
            });
            let ci_waived = matches!((&r.via, r.root), (Some(Via::Own), Some(i))
                if s.ci_state(i, &r.commit).is_some());
            ReleaseFact {
                commit: r.commit,
                first_parent: first.contains(&r.commit),
                state: r.state,
                trusted,
                ci: ci_waived || stored.is_some_and(|st| st.root.from.is_some()),
                ci_waived,
                root: fact.and_then(|f| f.id),
            }
        })
        .collect()
}

/// `sha256sum` output: `<hex>  <name>` or `<hex> *<name>` per line, each name once. An
/// escaped line (a name with a newline, which `sha256sum` prefixes with `\`) is refused.
pub fn sums(tag: &str, path: &Path) -> Result<Vec<Artifact>, Refusal> {
    let refuse = |why: String| Refusal::sums(tag, path, &why);
    let text = std::fs::read_to_string(path).map_err(|e| refuse(e.to_string()))?;
    let mut out: Vec<Artifact> = Vec::new();
    for (i, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let n = i + 1;
        if line.starts_with('\\') {
            return Err(refuse(format!(
                "line {n} is escaped, so its name is not plain"
            )));
        }
        let (hex, name) = line
            .split_once("  ")
            .or_else(|| line.split_once(" *"))
            .ok_or_else(|| refuse(format!("line {n} has no digest and name")))?;
        let sha256: Sha256Digest = format!("sha256:{hex}")
            .parse()
            .map_err(|_| refuse(format!("line {n} has no SHA-256 digest")))?;
        if name.is_empty() {
            return Err(refuse(format!("line {n} names no file")));
        }
        if out.iter().any(|a| a.name == name) {
            return Err(refuse(format!("line {n} names `{name}` twice")));
        }
        out.push(Artifact {
            name: name.to_owned(),
            sha256,
        });
    }
    if out.is_empty() {
        return Err(refuse("it lists no file".to_owned()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn sums_of(text: &str) -> Result<Vec<Artifact>, Refusal> {
        let dir = jtac_testkit::Repo::empty();
        let file = dir.write("SHA256SUMS", text.as_bytes());
        sums("v1", &file)
    }

    #[test]
    fn sums_reads_both_sha256sum_spellings_and_refuses_each_malformed_line() {
        let ok = sums_of(&format!("{HEX}  a.tar.gz\n\n{HEX} *b.zip\n")).unwrap();
        assert_eq!(
            ok.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["a.tar.gz", "b.zip"]
        );
        let rows = [
            ("", "lists no file"),
            (&format!("{HEX}\n"), "line 1 has no digest"),
            ("abc  a\n", "line 1 has no SHA-256"),
            (&format!("{HEX}  \n"), "line 1 names no file"),
            (&format!("{HEX}  a\n{HEX}  a\n"), "line 2 names `a` twice"),
            (&format!("\\{HEX}  a\\nb\n"), "line 1 is escaped"),
        ];
        for (text, want) in rows {
            let err = sums_of(text).unwrap_err();
            assert!(err.what.contains(want), "{text:?}: {}", err.what);
        }
    }
}
