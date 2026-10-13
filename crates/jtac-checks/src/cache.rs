//! The check cache: key, hit construction, and the one rule for what is stored
//! (DESIGN.md section 11.1). Backends implement `Cache`; `.git/jtac/cache/` lands in M7.

use std::collections::BTreeMap;

use jtac_core::{
    Body, CanonicalError, CheckBody, GitOid, Node, Outcome, Sha256Digest, Ts, canonical_digest,
};
use serde::Serialize;

/// What a cached check result is keyed on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CacheKey {
    /// Git tree the check ran on.
    pub tree: GitOid,
    /// Hash of the resolved policy.
    pub policy: Sha256Digest,
    /// The check id.
    pub id: String,
    /// What the tool's version probe printed. Never empty here.
    pub tool_version: String,
}

impl CacheKey {
    /// The key for a fresh, passing check node with a known tool version; `None` otherwise.
    pub fn of(node: &Node) -> Option<Self> {
        let Body::Check(body) = &node.body else {
            return None;
        };
        let fresh_pass = body.outcome == Outcome::Pass && body.cached_from.is_none();
        (fresh_pass && !body.tool_version.is_empty()).then(|| Self {
            tree: node.tree,
            policy: node.policy,
            id: body.id.clone(),
            tool_version: body.tool_version.clone(),
        })
    }

    /// One name per key, usable as a file name.
    pub fn name(&self) -> Result<Sha256Digest, CanonicalError> {
        canonical_digest(self)
    }
}

/// Where cached check nodes live. `store` and `lookup` are the only callers that decide.
pub trait Cache {
    /// The node stored under `name`, if any.
    fn get(&self, name: &Sha256Digest) -> Option<Node>;
    /// Keep `node` under `name`.
    fn put(&mut self, name: Sha256Digest, node: Node);
}

/// A cache that forgets at exit. Tests and `--preview` use it.
#[derive(Debug, Default)]
pub struct MemoryCache(BTreeMap<Sha256Digest, Node>);

impl Cache for MemoryCache {
    fn get(&self, name: &Sha256Digest) -> Option<Node> {
        self.0.get(name).cloned()
    }

    fn put(&mut self, name: Sha256Digest, node: Node) {
        self.0.insert(name, node);
    }
}

/// Keep `node` when it is a fresh pass with a known tool version. Returns whether it was kept.
pub fn store(cache: &mut dyn Cache, node: &Node) -> Result<bool, CanonicalError> {
    let Some(key) = CacheKey::of(node) else {
        return Ok(false);
    };
    cache.put(key.name()?, node.clone());
    Ok(true)
}

/// The cached node for `key`, re-issued at `now` as a hit. A stored node that is not the fresh
/// pass `key` names is ignored, so a wrong file under a cache name yields a rerun, not a pass.
pub fn lookup(cache: &dyn Cache, key: &CacheKey, now: Ts) -> Result<Option<Node>, CanonicalError> {
    match cache.get(&key.name()?) {
        Some(original) if CacheKey::of(&original).as_ref() == Some(key) => {
            hit(&original, now).map(Some)
        }
        _ => Ok(None),
    }
}

// Wall time is zero because nothing ran. Only `lookup` calls this, with a check node.
fn hit(original: &Node, now: Ts) -> Result<Node, CanonicalError> {
    let body = match &original.body {
        Body::Check(body) => CheckBody {
            duration_ms: 0,
            cached_from: Some(original.id()?),
            ..body.clone()
        },
        _ => unreachable!("lookup hands hit a check node"),
    };
    Ok(Node {
        tree: original.tree,
        policy: original.policy,
        ts: now,
        body: Body::Check(body),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jtac_core::Capture;

    const EMPTY: &str = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const POLICY: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const TS: &str = "2026-10-10T18:30:00Z";
    const LATER: &str = "2026-10-11T18:30:00Z";

    fn check(outcome: Outcome, tool_version: &str) -> Node {
        let empty = Capture {
            sha: EMPTY.parse().unwrap(),
            bytes: 0,
            truncated: false,
        };
        Node {
            tree: GitOid::EMPTY_TREE_SHA1.parse().unwrap(),
            policy: POLICY.parse().unwrap(),
            ts: TS.parse().unwrap(),
            body: Body::Check(CheckBody {
                id: "test".into(),
                argv: vec!["cargo".into(), "test".into()],
                tool_version: tool_version.into(),
                exit: 0,
                outcome,
                stdout: empty.clone(),
                stderr: empty,
                duration_ms: 1234,
                cached_from: None,
            }),
        }
    }

    fn key(node: &Node) -> CacheKey {
        let Body::Check(b) = &node.body else {
            unreachable!()
        };
        CacheKey {
            tree: node.tree,
            policy: node.policy,
            id: b.id.clone(),
            tool_version: b.tool_version.clone(),
        }
    }

    #[test]
    fn only_a_fresh_pass_with_a_tool_version_is_stored() {
        let hit_node = hit(&check(Outcome::Pass, "1.98.0"), TS.parse().unwrap()).unwrap();
        let rows = [
            ("pass", check(Outcome::Pass, "1.98.0"), true),
            ("fail", check(Outcome::Fail, "1.98.0"), false),
            ("error", check(Outcome::Error, "1.98.0"), false),
            ("skipped", check(Outcome::Skipped, "1.98.0"), false),
            ("no tool version", check(Outcome::Pass, ""), false),
            ("already a hit", hit_node, false),
        ];
        for (name, node, want) in rows {
            let mut cache = MemoryCache::default();
            assert_eq!(store(&mut cache, &node).unwrap(), want, "{name}");
            assert_eq!(cache.0.len(), usize::from(want), "{name}");
        }
    }

    #[test]
    fn lookup_reissues_the_stored_pass_at_now_pointing_back() {
        let original = check(Outcome::Pass, "1.98.0");
        let mut cache = MemoryCache::default();
        store(&mut cache, &original).unwrap();
        let now: Ts = LATER.parse().unwrap();
        let got = lookup(&cache, &key(&original), now).unwrap().unwrap();
        assert_eq!(got.ts, now);
        assert_eq!((got.tree, got.policy), (original.tree, original.policy));
        let (Body::Check(g), Body::Check(o)) = (&got.body, &original.body) else {
            unreachable!()
        };
        assert_eq!(g.cached_from, Some(original.id().unwrap()));
        assert_eq!(g.duration_ms, 0);
        assert_eq!(
            (&g.id, &g.argv, &g.tool_version, g.exit),
            (&o.id, &o.argv, &o.tool_version, o.exit)
        );
        assert_ne!(got.id().unwrap(), original.id().unwrap());
    }

    #[test]
    fn lookup_misses_on_any_key_change_or_a_wrong_node_under_the_name() {
        let original = check(Outcome::Pass, "1.98.0");
        let mut cache = MemoryCache::default();
        store(&mut cache, &original).unwrap();
        let now: Ts = LATER.parse().unwrap();
        let base = key(&original);
        let rows = [
            (
                "other tool version",
                CacheKey {
                    tool_version: "1.99.0".into(),
                    ..base.clone()
                },
            ),
            (
                "other id",
                CacheKey {
                    id: "fmt".into(),
                    ..base.clone()
                },
            ),
            (
                "other tree",
                CacheKey {
                    tree: "sha1:0000000000000000000000000000000000000001"
                        .parse()
                        .unwrap(),
                    ..base.clone()
                },
            ),
            (
                "other policy",
                CacheKey {
                    policy: Sha256Digest::of(b"other"),
                    ..base.clone()
                },
            ),
        ];
        for (name, key) in rows {
            assert!(lookup(&cache, &key, now).unwrap().is_none(), "{name}");
        }
        cache.put(base.name().unwrap(), check(Outcome::Fail, "1.98.0"));
        assert!(
            lookup(&cache, &base, now).unwrap().is_none(),
            "fail under the name"
        );
        cache.put(base.name().unwrap(), check(Outcome::Pass, "1.99.0"));
        assert!(
            lookup(&cache, &base, now).unwrap().is_none(),
            "other version under the name"
        );
    }
}
