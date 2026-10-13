//! RFC 6962 tree over raw node-id digests, with inclusion proofs (DESIGN.md section 9.3).

use sha2::{Digest as _, Sha256};

use crate::digest::Sha256Digest;

/// Merkle root of `leaves`. Input order does not matter; the tree sorts.
pub fn merkle_root(leaves: &[Sha256Digest]) -> Result<Sha256Digest, MerkleError> {
    sorted(leaves).map(|s| mth(&s))
}

/// Proof that `leaf` is in the tree over `leaves`.
pub fn prove(leaves: &[Sha256Digest], leaf: &Sha256Digest) -> Result<InclusionProof, MerkleError> {
    let s = sorted(leaves)?;
    let index = s
        .binary_search(leaf)
        .map_err(|_| MerkleError::NotALeaf(*leaf))?;
    Ok(InclusionProof {
        leaf: *leaf,
        index: index as u64,
        size: s.len() as u64,
        path: audit_path(&s, index),
    })
}

/// An RFC 6962 audit path. No wire form yet: a reveal format gets its own versioned schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InclusionProof {
    /// The leaf digest.
    pub leaf: Sha256Digest,
    /// Position of the leaf in sorted order.
    pub index: u64,
    /// Number of leaves in the tree. Not bound by the root: a size that keeps the path shape
    /// also verifies, so `verify` proves the leaf, never the count.
    pub size: u64,
    /// Sibling hashes from the leaf up to the root.
    pub path: Vec<Sha256Digest>,
}

impl InclusionProof {
    /// True when the path leads from `leaf` to `root`. RFC 9162 section 2.1.3.2, with the
    /// final `sn == 0` check that stops a short or padded path from verifying.
    pub fn verify(&self, root: &Sha256Digest) -> bool {
        if self.index >= self.size {
            return false;
        }
        let mut fnode = self.index;
        let mut snode = self.size - 1;
        let mut r = leaf_hash(&self.leaf);
        for p in &self.path {
            if snode == 0 {
                return false;
            }
            if fnode & 1 == 1 || fnode == snode {
                r = node_hash(p, &r);
                if fnode & 1 == 0 {
                    loop {
                        fnode >>= 1;
                        snode >>= 1;
                        if fnode & 1 == 1 || fnode == 0 {
                            break;
                        }
                    }
                }
            } else {
                r = node_hash(&r, p);
            }
            fnode >>= 1;
            snode >>= 1;
        }
        snode == 0 && r == *root
    }
}

/// Why a tree or proof could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MerkleError {
    /// The same digest twice. Evidence never repeats a node, so this is a corrupt input.
    #[error("duplicate leaf {0}")]
    Duplicate(Sha256Digest),
    /// Asked to prove a digest the tree does not hold.
    #[error("{0} is not a leaf of this tree")]
    NotALeaf(Sha256Digest),
}

fn sorted(leaves: &[Sha256Digest]) -> Result<Vec<Sha256Digest>, MerkleError> {
    let mut s = leaves.to_vec();
    s.sort_unstable();
    match s.windows(2).find(|w| w[0] == w[1]) {
        Some(w) => Err(MerkleError::Duplicate(w[0])),
        None => Ok(s),
    }
}

// Largest power of two strictly below n, for n >= 2.
fn split(n: usize) -> usize {
    1 << (usize::BITS - 1 - (n - 1).leading_zeros())
}

fn mth(s: &[Sha256Digest]) -> Sha256Digest {
    match s {
        [] => Sha256Digest::of(b""),
        [leaf] => leaf_hash(leaf),
        _ => {
            let k = split(s.len());
            node_hash(&mth(&s[..k]), &mth(&s[k..]))
        }
    }
}

fn audit_path(s: &[Sha256Digest], m: usize) -> Vec<Sha256Digest> {
    if s.len() <= 1 {
        return Vec::new();
    }
    let k = split(s.len());
    let (mut path, sibling) = if m < k {
        (audit_path(&s[..k], m), mth(&s[k..]))
    } else {
        (audit_path(&s[k..], m - k), mth(&s[..k]))
    };
    path.push(sibling);
    path
}

fn leaf_hash(d: &Sha256Digest) -> Sha256Digest {
    let mut h = Sha256::new();
    h.update([0u8]);
    h.update(d.as_bytes());
    Sha256Digest::from_bytes(h.finalize().into())
}

fn node_hash(left: &Sha256Digest, right: &Sha256Digest) -> Sha256Digest {
    let mut h = Sha256::new();
    h.update([1u8]);
    h.update(left.as_bytes());
    h.update(right.as_bytes());
    Sha256Digest::from_bytes(h.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letters() -> Vec<Sha256Digest> {
        "abcde".bytes().map(|b| Sha256Digest::of([b])).collect()
    }

    fn digests(n: usize) -> Vec<Sha256Digest> {
        (0..n).map(|i| Sha256Digest::of(i.to_string())).collect()
    }

    fn hex(s: &str) -> Sha256Digest {
        format!("sha256:{s}").parse().unwrap()
    }

    #[test]
    fn merkle_known_answers() {
        // DESIGN.md section 9.3. Leaves are sha256 of the bytes a..e; input is unsorted.
        let want = [
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "de22f76c222682c331f7dda7349654b6a9f4f710077025e9b29130023712780f",
            "19abe2ce03b25e3d5616841e998bf9a6d82385a4aa54420cb4543fde06731b47",
            "5a5a959d18f4993fbdb0337315966dd7d3555b530ab5d095e47490150806e868",
            "f777efb8147617bc84463a6e7954eccad236ca12e1e87c22e98c2dcbbc39ac50",
            "006b72c2b127f9e2011db36182468cb6c3a92836fc3e39eb4fdbc3ea604f62b8",
        ];
        let mut sorted_leaves = letters();
        sorted_leaves.sort();
        for (n, root) in want.iter().enumerate() {
            assert_eq!(
                merkle_root(&sorted_leaves[..n]).unwrap(),
                hex(root),
                "n={n}"
            );
        }
        assert_eq!(merkle_root(&letters()).unwrap(), hex(want[5]));

        let p = prove(&sorted_leaves[..3], &sorted_leaves[1]).unwrap();
        assert_eq!(
            p.path,
            [
                hex(want[1]),
                hex("a0d9f0a50b35b9f7d7edc57fb64f4771ddef0fefeaca4e6f949a1514db5b136d")
            ]
        );
        assert!(p.verify(&hex(want[3])));
        let p = prove(&letters(), &sorted_leaves[4]).unwrap();
        assert_eq!(
            (p.index, p.size, p.path.clone()),
            (4, 5, vec![hex(want[4])])
        );
        assert!(p.verify(&hex(want[5])));
    }

    #[test]
    fn every_leaf_of_every_small_tree_proves_inclusion() {
        for n in 1..=9 {
            let leaves = digests(n);
            let root = merkle_root(&leaves).unwrap();
            for leaf in &leaves {
                let p = prove(&leaves, leaf).unwrap();
                assert!(p.verify(&root), "n={n} leaf={leaf}");
            }
        }
    }

    #[test]
    fn a_proof_proves_only_its_leaf_under_its_root() {
        let leaves = digests(7);
        let root = merkle_root(&leaves).unwrap();
        let good = prove(&leaves, &leaves[3]).unwrap();
        let other = Sha256Digest::of("not a leaf");
        let mut wrong_leaf = good.clone();
        wrong_leaf.leaf = other;
        let mut wrong_index = good.clone();
        wrong_index.index ^= 1;
        let mut index_past_size = good.clone();
        index_past_size.index = 7;
        // Size only shapes the path, so only a size that changes the shape can fail.
        let mut size_too_small = good.clone();
        size_too_small.size = 4;
        let mut size_too_large = good.clone();
        size_too_large.size = 100;
        let mut same_shape = good.clone();
        same_shape.size = 8;
        assert!(
            same_shape.verify(&root),
            "size is not authenticated; see InclusionProof::size"
        );
        for (index, size) in [
            (u64::MAX - 1, u64::MAX),
            (0, u64::MAX),
            (u64::MAX, u64::MAX),
            (3, 1 << 63),
        ] {
            let mut huge = good.clone();
            huge.index = index;
            huge.size = size;
            assert!(!huge.verify(&root), "index={index} size={size} verified");
            huge.path = vec![other; 64];
            assert!(
                !huge.verify(&root),
                "index={index} size={size} with a 64-entry path verified"
            );
        }
        let mut short_path = good.clone();
        short_path.path.pop();
        let mut long_path = good.clone();
        long_path.path.push(other);
        let mut swapped = good.clone();
        swapped.path.swap(0, 1);
        let mut empty_path = good.clone();
        empty_path.path.clear();
        for (name, p) in [
            ("wrong leaf", wrong_leaf),
            ("wrong index", wrong_index),
            ("index past size", index_past_size),
            ("size too small", size_too_small),
            ("size too large", size_too_large),
            ("short path", short_path),
            ("long path", long_path),
            ("swapped siblings", swapped),
            ("empty path", empty_path),
        ] {
            assert!(!p.verify(&root), "{name} verified");
        }
        assert!(!good.verify(&other), "wrong root verified");
        assert!(good.verify(&root));
    }

    #[test]
    fn duplicates_and_missing_leaves_are_errors() {
        let mut leaves = digests(3);
        let other = Sha256Digest::of("other");
        assert_eq!(
            prove(&leaves, &other).unwrap_err(),
            MerkleError::NotALeaf(other)
        );
        assert_eq!(
            prove(&[], &other).unwrap_err(),
            MerkleError::NotALeaf(other)
        );
        leaves.push(leaves[1]);
        assert_eq!(
            merkle_root(&leaves).unwrap_err(),
            MerkleError::Duplicate(leaves[1])
        );
        assert_eq!(
            prove(&leaves, &leaves[0]).unwrap_err(),
            MerkleError::Duplicate(leaves[1])
        );
        assert!(
            MerkleError::Duplicate(other)
                .to_string()
                .contains("duplicate leaf sha256:")
        );
    }

    #[test]
    fn split_is_the_largest_power_of_two_below_n() {
        for (n, k) in [(2, 1), (3, 2), (4, 2), (5, 4), (8, 4), (9, 8), (1025, 1024)] {
            assert_eq!(split(n), k, "n={n}");
        }
    }
}
