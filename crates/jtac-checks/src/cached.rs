//! One check through the cache (DESIGN.md section 11.1 step 4): probe the tool, look the key
//! up, run only on a miss. Keeping a pass is the caller's, after the node and its captures are
//! in the store, so a hit never names evidence that was not kept. `--preview` never comes here.

use jtac_core::{Body, GitOid, Node, Sha256Digest, Ts};

use crate::cache::{Cache, CacheError, CacheKey, lookup};
use crate::run::{Context, probe, run_probed};
use crate::spec::Spec;

/// A check's node, and the capture bytes when it ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checked {
    /// A cache hit re-issued at `now`; its captures are already in the store.
    Hit(Node),
    /// A fresh run.
    Ran {
        /// The node.
        node: Node,
        /// Captured stdout, cut at the cap.
        stdout: Vec<u8>,
        /// Captured stderr, cut at the cap.
        stderr: Vec<u8>,
    },
}

impl Checked {
    /// The node either way.
    pub fn node(&self) -> &Node {
        match self {
            Self::Hit(node) | Self::Ran { node, .. } => node,
        }
    }
}

/// `spec` for `tree` under `policy`, from the cache when its key is there.
pub fn run_cached(
    spec: &Spec,
    ctx: &Context,
    cache: &dyn Cache,
    tree: GitOid,
    policy: Sha256Digest,
    now: Ts,
) -> Result<Checked, CacheError> {
    let probed = probe(spec, ctx);
    if !probed.tool_version.is_empty() {
        let key = CacheKey {
            tree,
            policy,
            id: spec.id.clone(),
            tool_version: probed.tool_version.clone(),
        };
        if let Some(hit) = lookup(cache, &key, now)? {
            return Ok(Checked::Hit(hit));
        }
    }
    let ran = run_probed(spec, ctx, probed);
    let node = Node {
        tree,
        policy,
        ts: now,
        body: Body::Check(ran.body),
    };
    Ok(Checked::Ran {
        node,
        stdout: ran.stdout,
        stderr: ran.stderr,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jtac_core::{CheckBody, Outcome};

    use super::*;
    use crate::cache::MemoryCache;

    const POLICY: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    fn spec(argv: &[&str]) -> Spec {
        Spec {
            id: "probe".into(),
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
            timeout: Duration::from_secs(60),
        }
    }

    fn body(c: &Checked) -> &CheckBody {
        match &c.node().body {
            Body::Check(b) => b,
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_known_tool_hits_on_the_second_run_and_an_unknown_one_never_does() {
        let ctx = Context {
            cwd: std::env::temp_dir(),
            capture_cap: 4096,
        };
        let tree: GitOid = GitOid::EMPTY_TREE_SHA1.parse().unwrap();
        let policy: Sha256Digest = POLICY.parse().unwrap();
        let now: Ts = "2026-10-10T18:30:00Z".parse().unwrap();
        let mut cache = MemoryCache::default();
        let known = spec(&["cargo", "--version"]);
        let first = run_cached(&known, &ctx, &cache, tree, policy, now).unwrap();
        assert!(matches!(first, Checked::Ran { .. }));
        assert_eq!(body(&first).outcome, Outcome::Pass);
        let again = run_cached(&known, &ctx, &cache, tree, policy, now).unwrap();
        assert!(matches!(again, Checked::Ran { .. }), "nothing kept yet");
        assert!(crate::cache::store(&mut cache, first.node()).unwrap());
        let second = run_cached(&known, &ctx, &cache, tree, policy, now).unwrap();
        assert!(matches!(second, Checked::Hit(_)), "{second:?}");
        assert_eq!(body(&second).cached_from, Some(first.node().id().unwrap()));
        let unknown = spec(&["git", "--version"]);
        for _ in 0..2 {
            let got = run_cached(&unknown, &ctx, &cache, tree, policy, now).unwrap();
            assert!(matches!(got, Checked::Ran { .. }));
            assert_eq!(body(&got).tool_version, "");
        }
    }
}
