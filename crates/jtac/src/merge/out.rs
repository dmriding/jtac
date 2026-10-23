//! What `jtac check --from` prints (DESIGN.md sections 7.1 and 15): the verdict as `check`
//! shows one, the root's `from`, the head's root with both sides of the comparison, the
//! waivers withheld, and the root refs pushed. Versioned with `v`.

use std::fmt::Write;

use jtac_core::{Compared, FromHead};
use serde::Serialize;

use crate::ci::head::HeadRoot;
use crate::ci::out::Withheld;
use crate::output::Output;
use crate::render;

/// What the merge job prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MergeOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// `--base` as given.
    pub base_rev: String,
    /// What the root records about the head.
    pub from: FromHead,
    /// Where trust came from: the old tip's signers.
    pub signers: String,
    /// The old tip holds no signers file: the adoption merge (decision 45).
    pub adoption: bool,
    /// `head`, or `base` when the diff touches `.jtac/**` and the old tip holds a policy.
    pub policy_from: &'static str,
    /// Commits a commit waiver may name beside HEAD: the pull request's.
    pub waivable: u64,
    /// CI's own verdict, with the root written.
    pub verdict: Output,
    /// Waivers on the head's chain that were offered nowhere, with why.
    pub withheld_waivers: Vec<Withheld>,
    /// The head's root, when it has one.
    pub head_root: Option<HeadRoot>,
    /// Root refs sent to `--push`'s remote; the head's only when it agreed.
    pub pushed: Vec<String>,
}

/// The text: the verdict as `check` prints it, then the merge job's own lines.
pub fn render(o: &MergeOut) -> String {
    let mut s = render::text(&o.verdict);
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
    s.push_str(&crate::ci::out::withheld_lines(&o.withheld_waivers));
    let _ = writeln!(
        s,
        "  from    {} head {}",
        &o.from.head.hex()[..12],
        o.from.compared.map_or("absent", compared)
    );
    if let Some(h) = &o.head_root {
        s.push_str(&crate::ci::out::head_lines(h));
    }
    for r in &o.pushed {
        let _ = writeln!(s, "  pushed  {r}");
    }
    s
}

/// The word `from.compared` prints as, `audit`'s label for every outcome but `agreed`.
pub fn compared(c: Compared) -> &'static str {
    match c {
        Compared::Agreed => "agreed",
        Compared::Disagreed => "disagreed",
        Compared::Lied => "lied",
        Compared::Invalid => "invalid",
        Compared::Unverifiable => "unverified",
    }
}
