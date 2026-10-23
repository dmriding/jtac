//! The SSHSIG namespace a document is signed in (DESIGN.md sections 11.8 and 15). A signature
//! made in one does not verify in another, and a root's shape picks its namespace: a root
//! with `from` is a CI root, so a key confined to the CI namespace signs no ordinary root and
//! no waiver, and an ordinary key's line, which names no namespace, trusts no CI root.

use jtac_core::Root;

/// One namespace per document kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    /// A root over its canonical bytes.
    Root,
    /// A root with `from` over its canonical bytes.
    CiRoot,
    /// A lever node over its canonical bytes.
    Lever,
}

impl Namespace {
    /// The name ssh-keygen is given.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "jtac-root-v1",
            Self::CiRoot => "jtac-ci-root-v1",
            Self::Lever => "jtac-lever-v1",
        }
    }

    /// The one namespace a root is signed and verified in, by its shape.
    pub fn of_root(root: &Root) -> Self {
        match root.from {
            Some(_) => Self::CiRoot,
            None => Self::Root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_namespaces_are_versioned_and_distinct() {
        assert_eq!(Namespace::Root.as_str(), "jtac-root-v1");
        assert_eq!(Namespace::CiRoot.as_str(), "jtac-ci-root-v1");
        assert_eq!(Namespace::Lever.as_str(), "jtac-lever-v1");
    }
}
