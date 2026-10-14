//! The SSHSIG namespace a document is signed in (DESIGN.md section 11.8). A signature made
//! in one does not verify in the other.

/// One namespace per document kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    /// A root over its canonical bytes.
    Root,
    /// A lever node over its canonical bytes.
    Lever,
}

impl Namespace {
    /// The name ssh-keygen is given.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "jtac-root-v1",
            Self::Lever => "jtac-lever-v1",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_namespaces_are_versioned_and_distinct() {
        assert_eq!(Namespace::Root.as_str(), "jtac-root-v1");
        assert_eq!(Namespace::Lever.as_str(), "jtac-lever-v1");
    }
}
