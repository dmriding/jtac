//! What `jtac ci` prints (DESIGN.md sections 7.1 and 15): CI's own verdict as `check` shows
//! one, then HEAD's root with both sides of the comparison, the waivers offered nowhere, and
//! each failure that is not the verdict's in three parts. Versioned with `v`.

use std::fmt::Write;

use jtac_core::{Compared, Counts, GitOid, SignedLever, gate};
use jtac_store::ref_name;
use serde::Serialize;

use super::head::HeadRoot;
use crate::judge::Judged;
use crate::output::{Output, WaiverOut};
use crate::reason::explain;
use crate::render;

/// What `jtac ci` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CiOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// `--base` as given.
    pub base_rev: String,
    /// Where trust came from.
    pub signers: String,
    /// The base holds no signers file: every protected-path finding is minor (decision 45).
    pub adoption: bool,
    /// `head`, or `base` when the diff touches `.jtac/**` and the base holds a policy.
    pub policy_from: &'static str,
    /// Commits a commit waiver may name beside HEAD.
    pub waivable: u64,
    /// CI's own verdict.
    pub verdict: Output,
    /// Waivers on the chain whose signer the base does not trust.
    pub untrusted_waivers: Vec<Untrusted>,
    /// HEAD's root, when it has one.
    pub head_root: Option<HeadRoot>,
    /// Whether a missing or unverifiable root fails.
    pub require_local_root: bool,
    /// Failures beside the verdict's, in three parts.
    pub reasons: Vec<Part>,
    /// Exit 0 when true.
    pub pass: bool,
}

/// A waiver named and offered nowhere.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Untrusted {
    /// The commit whose root carries it.
    pub commit: GitOid,
    /// The waiver.
    #[serde(flatten)]
    pub waiver: WaiverOut,
    /// The verdict's reasons it would have covered, or the verdict itself.
    pub covers: Vec<String>,
}

/// One failure in three parts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Part {
    /// What happened.
    pub what: String,
    /// Why the rule exists.
    pub why: String,
    /// The next step.
    pub how: String,
}

/// Each untrusted lever with what offering it would have changed, found by asking the verdict
/// again with that lever alone added.
pub fn untrusted(j: &Judged, levers: &[(GitOid, SignedLever)]) -> Vec<Untrusted> {
    levers
        .iter()
        .filter_map(|(commit, lever)| {
            let waiver = WaiverOut::of(lever)?;
            let mut nodes = j.nodes.clone();
            nodes.push(lever.node.clone());
            let with = gate(&j.facts, &nodes, j.now);
            let covers = if with.state != j.verdict.state {
                vec!["the verdict: a commit waiver in scope declares it off".to_owned()]
            } else {
                j.verdict
                    .reasons
                    .iter()
                    .filter(|r| !with.reasons.contains(r))
                    .map(|r| explain(r, j).what)
                    .collect()
            };
            Some(Untrusted {
                commit: *commit,
                waiver,
                covers,
            })
        })
        .collect()
}

impl CiOut {
    /// Fill `reasons`, `pass`, and the verdict's banner from what was gathered.
    pub fn decide(&mut self) {
        let hex = self.verdict.commit.hex();
        let name = ref_name(&self.verdict.commit);
        let again = format!(
            "delete the ref with `git update-ref -d {name}`, run `jtac check` on a clean tree at \
             {hex}, and push the root again"
        );
        let push = format!(
            "run `jtac check` at {hex} and push the root (`git push <remote> {name}`); the pre-push hook `jtac init` installs does both"
        );
        let part = |what: String, why: &str, how: String| Part {
            what,
            why: why.to_owned(),
            how,
        };
        match &self.head_root {
            None if self.require_local_root => self.reasons.push(part(
                "HEAD has no root, and `--require-local-root` is set.".to_owned(),
                "The workflow asked for the developer's own evidence beside CI's run; with no \
                 root the push was never judged locally, or its root was not pushed.",
                push,
            )),
            Some(h) => {
                let why = h.why.clone().unwrap_or_default();
                match h.compared {
                    Compared::Invalid => self.reasons.push(part(
                        format!("HEAD's root does not count: {why}."),
                        "A root is evidence only when it parses, names HEAD's tree, its signature \
                         holds, and its shape and chain are what its version requires; one that \
                         fails any of these vouches for nothing, and CI fails rather than pass \
                         beside it.",
                        format!("Run `jtac verify {hex}` locally for the same reason, then {again}."),
                    )),
                    Compared::Lied => self.reasons.push(part(
                        format!("HEAD's root lied about its policy: {why}."),
                        "A root from this gate version naming another policy hash was judged under \
                         rules HEAD does not carry; it fails the check so a forged root gains \
                         nothing that skipping jtac does not.",
                        format!("Commit the policy the root was judged under, or {again}."),
                    )),
                    Compared::Disagreed => self.reasons.push(part(
                        format!(
                            "HEAD's root records {}; this gate's run of the same diff reaches {}.",
                            side(h.recorded.as_ref()),
                            side(h.run.as_ref())
                        ),
                        "The root's verdict is checked by running the same rules on the same diff \
                         under the same policy and gate version; a root that says otherwise was \
                         not computed from this history.",
                        format!("Run `jtac replay {hex}` where the root was written to see its nodes, then {again}."),
                    )),
                    Compared::Unverifiable if self.require_local_root => self.reasons.push(part(
                        format!("HEAD's root cannot be compared ({why}), and `--require-local-root` is set."),
                        "A root this gate cannot compare is treated as a missing one: a rule table \
                         can change counts under one hash, and history the clone lacks leaves the \
                         scope unchecked.",
                        format!("With jtac {} installed, {again}.", env!("CARGO_PKG_VERSION")),
                    )),
                    Compared::Agreed | Compared::Unverifiable => {}
                }
            }
            None => {}
        }
        self.pass = self.verdict.pass && self.reasons.is_empty();
        self.verdict.command = "ci".to_owned();
        self.verdict.banner = if self.pass { "CLEARED HOT" } else { "ABORT" }.to_owned();
    }
}

fn side(s: Option<&super::head::Side>) -> String {
    match s {
        Some(s) => format!(
            "{} with {}",
            crate::survey::state_name(Some(s.state)),
            counts(&s.counts)
        ),
        None => "nothing".to_owned(),
    }
}

fn counts(c: &Counts) -> String {
    format!(
        "checks {}/{} pass, findings {} blocker {} major {} minor, waivers {}",
        c.pass, c.checks, c.findings.blocker, c.findings.major, c.findings.minor, c.waivers
    )
}

/// The text: the verdict as `check` prints it under CI's banner, then CI's own lines.
pub fn render(o: &CiOut) -> String {
    let body = render::text(&o.verdict);
    let mut s = String::new();
    let head = match &o.head_root {
        None => "no head root".to_owned(),
        Some(h) => format!("head root {}", compared(h.compared)),
    };
    let _ = writeln!(
        s,
        "{}: {}; {head}",
        o.verdict.banner,
        render::summary(&o.verdict)
    );
    let _ = writeln!(
        s,
        "  signers {}{}",
        o.signers,
        if o.adoption {
            " (adoption: every protected-path finding is minor)"
        } else {
            ""
        }
    );
    let _ = writeln!(
        s,
        "  scope   {} commits a commit waiver may name; policy from {}",
        o.waivable, o.policy_from
    );
    for line in body
        .lines()
        .skip(1)
        .filter(|l| *l != "  root    none written")
    {
        let _ = writeln!(s, "{line}");
    }
    for u in &o.untrusted_waivers {
        let _ = writeln!(
            s,
            "  waiver  not applied, signer not in signers: {} {}",
            &u.commit.hex()[..12],
            u.waiver.line()
        );
        for c in &u.covers {
            let _ = writeln!(s, "          would cover {c}");
        }
    }
    if let Some(h) = &o.head_root {
        let signer = h.signer.as_ref().map_or(String::new(), |s| {
            format!(" by {} ({})", s.principal, h.trust)
        });
        let _ = writeln!(
            s,
            "  head    root {}{signer}: {}{}",
            h.root.map_or("unreadable".to_owned(), |r| r.to_string()),
            compared(h.compared),
            h.why.as_ref().map_or(String::new(), |w| format!(", {w}"))
        );
        if let (Some(r), Some(run)) = (&h.recorded, &h.run) {
            let _ = writeln!(s, "          recorded {}", side(Some(r)));
            let _ = writeln!(s, "          run      {}", side(Some(run)));
        }
    }
    let first = o.verdict.reasons.len();
    for (i, r) in o.reasons.iter().enumerate() {
        let _ = writeln!(s, "  {}. {}", first + i + 1, r.what);
        let _ = writeln!(s, "     why  {}", r.why);
        let _ = writeln!(s, "     fix  {}", r.how);
    }
    s
}

fn compared(c: Compared) -> &'static str {
    match c {
        Compared::Agreed => "agreed",
        Compared::Disagreed => "disagreed",
        Compared::Lied => "lied",
        Compared::Invalid => "invalid",
        Compared::Unverifiable => "unverifiable",
    }
}
