//! The SSHSIG namespace a document is signed in (DESIGN.md sections 11.8, 15 and 16.3). A
//! signature made in one does not verify in another, and a root's shape picks its namespace:
//! a root with `from` is a CI root, so a key confined to the CI namespace signs no ordinary
//! root and no waiver, and an ordinary key's line, which names no namespace, trusts no CI
//! root and no release attestation.

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
    /// A release attestation over its canonical bytes.
    Release,
}

impl Namespace {
    /// The name ssh-keygen is given.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "jtac-root-v1",
            Self::CiRoot => "jtac-ci-root-v1",
            Self::Lever => "jtac-lever-v1",
            Self::Release => "jtac-release-v1",
        }
    }

    /// The document a signers line must name this namespace to trust; `None` where any line
    /// ssh-keygen trusts will do.
    pub const fn confined(self) -> Option<&'static str> {
        match self {
            Self::CiRoot => Some("a root with `from`"),
            Self::Release => Some("a release attestation"),
            Self::Root | Self::Lever => None,
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
        assert_eq!(Namespace::Release.as_str(), "jtac-release-v1");
        assert!(Namespace::Root.confined().is_none());
        assert!(Namespace::Lever.confined().is_none());
        assert!(Namespace::CiRoot.confined().is_some());
        assert!(Namespace::Release.confined().is_some());
    }
}
