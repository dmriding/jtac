//! `jtac waive` (DESIGN.md section 11.6): one signed lever node, scoped to a commit or to a
//! rule under a path with an expiry, kept locally until a verdict applies it. Under an agent
//! session the node records the marker and the command says so; it never refuses for it.

use std::fmt::Write;
use std::path::Path;

use jtac_core::{
    Action, Body, Gate, GitOid, LeverBody, Node, Scope, Sha256Digest, Signer, Ts, clean_prefix,
};
use jtac_policy::facts;
use jtac_sign::{Namespace, sign};
use jtac_store::Store;
use serde::Serialize;

use crate::cli::WaiveArgs;
use crate::refuse::Refusal;
use crate::{Printed, agent, identity, policy, repo};

/// What `jtac waive` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WaiveOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// The lever node id.
    pub lever: Sha256Digest,
    /// What it covers.
    pub scope: Scope,
    /// When it was written.
    pub ts: Ts,
    /// When it stops applying; absent for a commit waiver.
    pub expires: Option<Ts>,
    /// Who signed.
    pub signer: Signer,
    /// The agent-session marker it was written under, when one was set.
    pub agent: Option<String>,
    /// Verbatim.
    pub reason: String,
}

/// Write, sign, keep, and say what was kept.
pub fn waive(a: &WaiveArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let format = repo::format(&repo).map_err(|e| Refusal::repository(&e))?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    let now = Ts::from_timestamp(jiff::Timestamp::now()).map_err(Refusal::internal)?;
    for (flag, value) in [
        ("--reason", Some(&a.reason)),
        ("--rule", a.rule.as_ref()),
        ("--path", a.path.as_ref()),
    ] {
        if value.is_some_and(|v| v.trim().is_empty() || v.chars().any(char::is_control)) {
            return Err(Refusal::waive_text(flag));
        }
    }
    if a.ci && (a.rule.is_some() || a.path.is_some() || a.expires.is_some()) {
        return Err(Refusal::ci_flags(a.rev.as_deref(), &a.reason));
    }
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let (scope, tree, expires) = match (&a.rev, &a.rule, &a.path) {
        (Some(rev), None, None) => {
            if a.expires.is_some() {
                return Err(Refusal::commit_expires(rev, &a.reason));
            }
            let target = repo::peel(&repo, &format, rev).map_err(|_| Refusal::rev("waive", rev))?;
            if a.ci
                && store
                    .root(&target.commit)
                    .map_err(Refusal::store)?
                    .is_some()
            {
                return Err(Refusal::ci_root_stands(rev, &a.reason, &target.commit));
            }
            let scope = Scope::Commit {
                commit: target.commit,
                gate: a.ci.then_some(Gate::Ci),
            };
            (scope, target.tree, None)
        }
        (None, Some(rule), Some(path)) => {
            if !clean_prefix(path) {
                return Err(Refusal::waive_path(rule, path, &a.reason));
            }
            let scope = Scope::Rule {
                rule: rule.clone(),
                path: path.clone(),
            };
            let expires = expiry(now, a.expires.as_deref()).ok_or_else(|| {
                Refusal::waive_expires(rule, path, &a.reason, a.expires.as_deref())
            })?;
            (scope, head.tree, Some(expires?))
        }
        _ => return Err(Refusal::waive_scope()),
    };
    let loaded = policy::load(&repo, &head)?;
    let id = identity::identity(&repo)?;
    let marker = agent::here();
    let node = Node {
        tree,
        policy: facts(&loaded.policy, head.commit, head.tree)
            .map_err(Refusal::internal)?
            .policy,
        ts: now,
        body: Body::Lever(LeverBody {
            action: Action::Waive,
            signer: id.signer.clone(),
            reason: a.reason.clone(),
            scope: Some(scope.clone()),
            expires,
            agent: marker.map(str::to_owned),
        }),
    };
    let bytes = node.canonical_bytes().map_err(Refusal::internal)?;
    let sig = sign(&id, Namespace::Lever, &bytes).map_err(|e| Refusal::sign(&e))?;
    let lever = store.put_lever(&node, &sig).map_err(Refusal::store)?;
    let out = WaiveOut {
        v: 1,
        command: "waive",
        lever,
        scope,
        ts: now,
        expires,
        signer: id.signer,
        agent: marker.map(str::to_owned),
        reason: a.reason.clone(),
    };
    let text = if a.json {
        crate::render::json(&out)?
    } else {
        render(&out)
    };
    let mut printed = Printed::verdict(text, true);
    printed.note = marker.map(|m| under_agent(m, &store.lever_path(&lever)));
    Ok(printed)
}

/// `now` plus `--expires`; `None` unless it is `Nd` with N in 1..=30.
fn expiry(now: Ts, given: Option<&str>) -> Option<Result<Ts, Refusal>> {
    let days: i64 = given
        .and_then(|g| g.strip_suffix('d'))
        .and_then(|n| n.parse().ok())
        .filter(|n| (1..=30).contains(n))?;
    let at = jiff::Timestamp::from_second(now.timestamp().as_second() + days * 86_400)
        .map_err(Refusal::internal);
    Some(at.and_then(|at| Ts::from_timestamp(at).map_err(Refusal::internal)))
}

fn render(o: &WaiveOut) -> String {
    let mut s = String::new();
    let (what, next) = match &o.scope {
        Scope::Commit { commit, gate: None } => (
            format!("commit {}", commit.hex()),
            format!(
                "run `jtac check` with {} checked out; its root reads declared off",
                short(commit)
            ),
        ),
        Scope::Commit {
            commit,
            gate: Some(Gate::Ci),
        } => (
            format!("ci gate on commit {}", commit.hex()),
            format!(
                "run `jtac check --push <remote>` with {} checked out; its root carries this, \
                 stays covered, and the CI gate reads declared off",
                short(commit)
            ),
        ),
        Scope::Rule { rule, path } => (
            format!("{rule} under {path}"),
            format!("run `jtac check`; a {rule} finding under `{path}` is covered until then"),
        ),
    };
    let _ = writeln!(s, "WAIVED {what}: lever {}", o.lever);
    let _ = writeln!(
        s,
        "  signer  {} {}",
        o.signer.principal, o.signer.fingerprint
    );
    let _ = writeln!(s, "  reason  {}", o.reason);
    match o.expires {
        Some(at) => {
            let _ = writeln!(s, "  expires {at}");
        }
        None => {
            let _ = writeln!(s, "  expires none (covers one commit)");
        }
    }
    if let Some(m) = &o.agent {
        let _ = writeln!(
            s,
            "  agent   written under an agent session ({m}); every reader flags it"
        );
    }
    let _ = writeln!(s, "  next    {next}");
    s
}

fn short(commit: &GitOid) -> String {
    commit.hex()[..12].to_owned()
}

/// The stderr note: what the marker means and how a person withdraws the waiver before a
/// verdict applies it. Three parts, as a refusal has, since a model may be the reader.
fn under_agent(marker: &str, sig: &Path) -> String {
    format!(
        "jtac: this waiver was written under an agent session ({marker}).\n\
         A waiver is a human lever; an agent pulling one is recorded in the lever node and \
         flagged by `status`, `check`, the push hook, `verify` and `audit`, not forbidden: one \
         account runs the person and the agent, so a lock here would be theatre.\n\
         If no person asked for it, withdraw it before a verdict applies it: `rm {}`.\n",
        jtac_checks::git::quoted(&sig.to_string_lossy(), "FILE")
    )
}
