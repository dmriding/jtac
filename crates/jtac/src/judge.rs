//! Gather the evidence for one verdict and hand it to `gate` (DESIGN.md section 11.1 steps 3
//! to 5): the `integrity` check node first, its findings, then every policy check in order.
//! With a cache, a check's node may be a hit; without one (`--preview`) every check runs and
//! nothing is kept, because a pass computed on a dirty tree would be cached under HEAD's
//! tree id.

use std::path::Path;
use std::time::Instant;

use jtac_checks::diff::git::changes;
use jtac_checks::diff::{Adoption, Report, report};
use jtac_checks::{CAPTURE_CAP, Cache, Checked, Context, run, run_cached, specs};
use jtac_core::{
    Body, Capture, CheckBody, Facts, FindingBody, GitOid, Node, Outcome, Sha256Digest, SignedLever,
    Ts, Verdict, gate,
};
use jtac_policy::{INTEGRITY_CHECK_ID, facts};

use crate::policy::Loaded;
use crate::refuse::Refusal;
use crate::repo::{self, Base, Head, RepoError};

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

/// What the verdict is offered beside the evidence (DESIGN.md sections 11.6 and 15): the
/// waivers whose signatures hold, the commits a commit waiver may name beside HEAD, and
/// whether anything was in force to protect. The local gate offers every stored waiver, HEAD
/// alone, and decision 38's history test; CI offers the chain's trusted waivers, the pull
/// request's commits, and the base branch's trust.
pub struct Offered {
    /// Waivers the verdict may apply.
    pub waivers: Vec<SignedLever>,
    /// Beside HEAD, the commits a commit waiver may name.
    pub waivable: Vec<GitOid>,
    /// The adoption fact for `JT-PROT-001`.
    pub adoption: Adoption,
}

/// Decision 38's fact: no parent of `head`, and nothing behind any of them, ever held
/// `.jtac/policy.toml`. Read from history, never from the diff's base, which `--base` lets
/// the pusher choose; every parent, since a merge whose first parent never held a policy
/// would otherwise weaken the one its second parent holds; through `rev-list`, since `log`
/// reads `log.follow` and `log.showRoot` from the pusher's own config and either hides the hit
/// (`adopting_jtac_passes_the_first_check_and_the_next_policy_edit_is_major`). A root commit
/// has no parent and no diff to judge.
pub fn adoption(repo: &Path, head: &Head) -> Result<Adoption, Refusal> {
    let git = |e: RepoError| Refusal::repository(&e);
    let parents = repo::probe(
        repo,
        &["rev-parse", &format!("{}^@", head.commit.hex())],
        128,
    )
    .map_err(git)?
    .unwrap_or_default();
    if parents.is_empty() {
        return Ok(Adoption::None);
    }
    let mut args = vec!["rev-list", "-1", "--full-history"];
    args.extend(parents.lines());
    args.extend(["--", jtac_policy::POLICY_PATH]);
    let touched = repo::probe(repo, &args, 128).map_err(git)?;
    Ok(if touched.is_some_and(|t| t.is_empty()) {
        Adoption::Policy
    } else {
        Adoption::None
    })
}

/// Run the rules and the checks on `head` and decide. Every offered waiver is given to the
/// verdict; it keeps the ones in scope and only those become leaves (DESIGN.md section 9.4).
pub fn judge(
    repo: &Path,
    loaded: &Loaded,
    head: Head,
    base: Base,
    now: Ts,
    cache: Option<&dyn Cache>,
    offered: &Offered,
) -> Result<Judged, Refusal> {
    let mut facts = facts(&loaded.policy, head.commit, head.tree).map_err(Refusal::internal)?;
    facts.waivable = offered.waivable.clone();
    let node = |body: Body| Node {
        tree: head.tree,
        policy: facts.policy,
        ts: now,
        body,
    };
    let started = Instant::now();
    let read = changes(repo, &base.commit, &head.commit.hex()).map_err(|e| e.to_string());
    let report = read.as_ref().ok().map(|c| {
        report(
            &loaded.policy.integrity,
            &loaded.matchers,
            c,
            offered.adoption,
        )
    });
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
    nodes.extend(offered.waivers.iter().map(|w| w.node.clone()));
    let verdict = gate(&facts, &nodes, now);
    let levers: Vec<SignedLever> = verdict
        .applied
        .iter()
        .filter_map(|&i| i.checked_sub(evidence))
        .map(|i| offered.waivers[i].clone())
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
pub fn integrity_body(error: Option<&str>, millis: u128) -> CheckBody {
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
