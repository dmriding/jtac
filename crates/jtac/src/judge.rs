//! Gather the evidence for one verdict and hand it to `gate` (DESIGN.md section 11.1 steps 3
//! to 5): the `integrity` check node first, its findings, then every policy check in order.
//! With a cache, a check's node may be a hit; without one (`--preview`) every check runs and
//! nothing is kept, because a pass computed on a dirty tree would be cached under HEAD's
//! tree id.

use std::path::Path;
use std::time::Instant;

use jtac_checks::diff::git::changes;
use jtac_checks::diff::{Report, report};
use jtac_checks::{CAPTURE_CAP, Cache, Checked, Context, run, run_cached, specs};
use jtac_core::{
    Body, Capture, CheckBody, Facts, FindingBody, Node, Outcome, Sha256Digest, SignedLever, Ts,
    Verdict, gate,
};
use jtac_policy::{INTEGRITY_CHECK_ID, facts};

use crate::policy::Loaded;
use crate::refuse::Refusal;
use crate::repo::{Base, Head};

/// Everything one verdict was computed from.
pub struct Judged {
    /// HEAD.
    pub head: Head,
    /// The base the rules read from.
    pub base: Base,
    /// What the verdict judged.
    pub facts: Facts,
    /// The instant every node carries and `gate` was given.
    pub now: Ts,
    /// Check and finding nodes, `integrity` first, then the waivers the verdict applied: the
    /// root's leaves, in order.
    pub nodes: Vec<Node>,
    /// The applied waivers with their signatures, the root's `levers`.
    pub levers: Vec<SignedLever>,
    /// Every captured stream of a fresh run, for the store; a hit's were kept before its
    /// entry was (`write::evidence`).
    pub captures: Vec<Vec<u8>>,
    /// Why the rules did not run: git could not read the diff.
    pub integrity_error: Option<String>,
    /// What the rules counted, when they ran.
    pub report: Option<Report>,
    /// The decision.
    pub verdict: Verdict,
}

/// Run the rules and the checks on `head` and decide. Every stored waiver is offered; the
/// verdict keeps the ones in scope and only those become leaves (DESIGN.md section 9.4).
pub fn judge(
    repo: &Path,
    loaded: &Loaded,
    head: Head,
    base: Base,
    now: Ts,
    cache: Option<&dyn Cache>,
    waivers: &[SignedLever],
) -> Result<Judged, Refusal> {
    let facts = facts(&loaded.policy, head.commit, head.tree).map_err(Refusal::internal)?;
    let node = |body: Body| Node {
        tree: head.tree,
        policy: facts.policy,
        ts: now,
        body,
    };
    let started = Instant::now();
    let read = changes(repo, &base.commit, &head.commit.hex()).map_err(|e| e.to_string());
    let report = read
        .as_ref()
        .ok()
        .map(|c| report(&loaded.policy.integrity, &loaded.matchers, c));
    let integrity_error = read.err();
    let integrity = node(Body::Check(integrity_body(
        integrity_error.as_deref(),
        started.elapsed().as_millis(),
    )));
    let check_id = integrity.id().map_err(Refusal::internal)?;
    let mut captures = vec![integrity_error.clone().unwrap_or_default().into_bytes()];
    let mut nodes = vec![integrity];
    for f in report.iter().flat_map(|r| &r.findings) {
        nodes.push(node(Body::Finding(FindingBody {
            check: check_id,
            rule: f.rule.id().to_owned(),
            severity: f.severity,
            path: f.path.clone(),
            line: f.line,
            message: f.message.clone(),
        })));
    }
    let ctx = Context {
        cwd: repo.to_path_buf(),
        capture_cap: CAPTURE_CAP,
    };
    for spec in specs(&loaded.policy) {
        let Some(cache) = cache else {
            nodes.push(node(Body::Check(run(&spec, &ctx).body)));
            continue;
        };
        match run_cached(&spec, &ctx, cache, head.tree, facts.policy, now)
            .map_err(Refusal::store)?
        {
            Checked::Hit(n) => nodes.push(n),
            Checked::Ran {
                node,
                stdout,
                stderr,
            } => {
                captures.extend([stdout, stderr]);
                nodes.push(node);
            }
        }
    }
    let evidence = nodes.len();
    nodes.extend(waivers.iter().map(|w| w.node.clone()));
    let verdict = gate(&facts, &nodes, now);
    let levers: Vec<SignedLever> = verdict
        .applied
        .iter()
        .filter_map(|&i| i.checked_sub(evidence))
        .map(|i| waivers[i].clone())
        .collect();
    nodes.truncate(evidence);
    nodes.extend(levers.iter().map(|l| l.node.clone()));
    Ok(Judged {
        head,
        base,
        facts,
        now,
        nodes,
        levers,
        captures,
        integrity_error,
        report,
        verdict,
    })
}

/// The reserved check (DESIGN.md section 9.2): empty argv, jtac's version, `pass` when the
/// rules ran, `error` with the reason as its stderr when they could not.
fn integrity_body(error: Option<&str>, millis: u128) -> CheckBody {
    let capture = |bytes: &[u8]| Capture {
        sha: Sha256Digest::of(bytes),
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        truncated: false,
    };
    CheckBody {
        id: INTEGRITY_CHECK_ID.to_owned(),
        argv: Vec::new(),
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        exit: if error.is_some() { -1 } else { 0 },
        outcome: if error.is_some() {
            Outcome::Error
        } else {
            Outcome::Pass
        },
        stdout: capture(b""),
        stderr: capture(error.unwrap_or_default().as_bytes()),
        duration_ms: u64::try_from(millis).unwrap_or(u64::MAX),
        cached_from: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity_body_is_pass_with_empty_captures_or_error_with_the_reason() {
        let pass = integrity_body(None, 7);
        assert_eq!(
            (pass.outcome, pass.exit, pass.duration_ms),
            (Outcome::Pass, 0, 7)
        );
        assert_eq!(pass.stderr.bytes, 0);
        assert!(pass.argv.is_empty());
        assert_eq!(pass.tool_version, env!("CARGO_PKG_VERSION"));
        let err = integrity_body(Some("no base"), u128::MAX);
        assert_eq!(
            (err.outcome, err.exit, err.duration_ms),
            (Outcome::Error, -1, u64::MAX)
        );
        assert_eq!(err.stderr.bytes, 7);
        assert_eq!(err.stderr.sha, Sha256Digest::of(b"no base"));
    }
}
