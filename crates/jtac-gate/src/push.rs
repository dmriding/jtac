//! One ref update at a time, from the facts the `pre-push` hook or `status` gathered: delete
//! and tag pass, a rewound protected branch is the fence, then the tip needs a root. The CLI
//! renders; nothing here knows a ref's name beyond its namespace.

use jtac_core::State;

/// What the remote would do with the ref, from the old oid and the new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum History {
    /// The remote has no such ref.
    Creates,
    /// The old oid is an ancestor of the new one.
    FastForward,
    /// It is not: only a force would land this.
    Rewinds,
    /// The old oid is not in this clone, so neither answer is known.
    Unknown,
}

/// What the tip commit has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// A valid root of its own, or one that covers it.
    Root(State),
    /// A valid root for another commit with the same tree (DESIGN.md section 9.5).
    ByTree {
        /// That commit's hex.
        commit: String,
    },
    /// Nothing.
    Absent,
}

/// One line of what git hands `pre-push`, with the facts around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefUpdate {
    /// The ref on the remote, full name.
    pub remote_ref: String,
    /// The local side is the null oid.
    pub delete: bool,
    /// The remote branch matches `[fences].protected_branches`.
    pub protected: bool,
    /// `[fences].force_push_protected` is `deny`.
    pub deny_force: bool,
    /// Old oid against new.
    pub history: History,
    /// What the new tip has.
    pub tip: Coverage,
}

/// Why a ref passes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pass {
    /// Deleting a ref needs no evidence.
    Delete,
    /// A tag names a commit a branch already carried.
    Tag,
    /// A ref under `refs/jtac/` carries evidence, not code.
    Evidence,
    /// The tip has a root in this state.
    Root(State),
    /// Another commit's root judged the same tree.
    Tree {
        /// That commit's hex.
        commit: String,
    },
}

/// Why a ref is refused. `Rewind` is the fence; the others name a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refuse {
    /// A non-fast-forward update of a protected branch.
    Rewind,
    /// The old oid is not in this clone, so a rewind cannot be ruled out.
    Unknown,
    /// The tip has no root.
    Absent,
}

impl Refuse {
    /// A fence has no path to yes; a negotiation names the command.
    pub fn is_fence(&self) -> bool {
        matches!(self, Self::Rewind)
    }
}

/// The decision for one ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The push may proceed.
    Pass(Pass),
    /// It may not.
    Refuse(Refuse),
}

/// The refs that pass on their name alone: a deletion, a tag, evidence. `decide` asks this
/// first, so a gatherer that reads no tip for these agrees with it
/// (`exempt_refs_pass_before_any_fact_about_the_tip`).
pub fn exempt(remote_ref: &str, delete: bool) -> Option<Pass> {
    if delete {
        Some(Pass::Delete)
    } else if remote_ref.starts_with("refs/tags/") {
        Some(Pass::Tag)
    } else if remote_ref.starts_with("refs/jtac/") {
        Some(Pass::Evidence)
    } else {
        None
    }
}

/// Decide one ref. Order: delete, tag, evidence, fence, coverage. The fence is judged before the root,
/// because a root on a rewound tip would otherwise read as a pass.
pub fn decide(u: &RefUpdate) -> Decision {
    if let Some(pass) = exempt(&u.remote_ref, u.delete) {
        return Decision::Pass(pass);
    }
    if u.protected && u.deny_force {
        match u.history {
            History::Rewinds => return Decision::Refuse(Refuse::Rewind),
            History::Unknown => return Decision::Refuse(Refuse::Unknown),
            History::Creates | History::FastForward => {}
        }
    }
    match &u.tip {
        Coverage::Root(state) => Decision::Pass(Pass::Root(*state)),
        Coverage::ByTree { commit } => Decision::Pass(Pass::Tree {
            commit: commit.clone(),
        }),
        Coverage::Absent => Decision::Refuse(Refuse::Absent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(remote_ref: &str, history: History, tip: Coverage) -> RefUpdate {
        RefUpdate {
            remote_ref: remote_ref.into(),
            delete: false,
            protected: remote_ref == "refs/heads/main",
            deny_force: true,
            history,
            tip,
        }
    }

    #[test]
    fn every_row_of_the_push_table_decides_as_section_11_3_says() {
        let tree = Coverage::ByTree {
            commit: "abc".into(),
        };
        let rows: Vec<(&str, RefUpdate, Decision)> = vec![
            (
                "a deletion passes without evidence, even of a protected branch",
                RefUpdate {
                    delete: true,
                    ..update("refs/heads/main", History::Rewinds, Coverage::Absent)
                },
                Decision::Pass(Pass::Delete),
            ),
            (
                "a tag passes without evidence",
                update("refs/tags/v1", History::Creates, Coverage::Absent),
                Decision::Pass(Pass::Tag),
            ),
            (
                "a root ref pushed by hand is evidence, not code",
                update("refs/jtac/roots/ab/abc", History::Creates, Coverage::Absent),
                Decision::Pass(Pass::Evidence),
            ),
            (
                "a rewound protected branch is the fence, root or not",
                update(
                    "refs/heads/main",
                    History::Rewinds,
                    Coverage::Root(State::Covered),
                ),
                Decision::Refuse(Refuse::Rewind),
            ),
            (
                "an old oid this clone lacks cannot clear the fence",
                update(
                    "refs/heads/main",
                    History::Unknown,
                    Coverage::Root(State::Covered),
                ),
                Decision::Refuse(Refuse::Unknown),
            ),
            (
                "a rewound unprotected branch is judged on its root",
                update("refs/heads/topic", History::Rewinds, Coverage::Absent),
                Decision::Refuse(Refuse::Absent),
            ),
            (
                "an unknown old oid on an unprotected branch is judged on its root",
                update("refs/heads/topic", History::Unknown, tree.clone()),
                Decision::Pass(Pass::Tree {
                    commit: "abc".into(),
                }),
            ),
            (
                "the fence is policy: allow lets a rewind through to the root",
                RefUpdate {
                    deny_force: false,
                    ..update(
                        "refs/heads/main",
                        History::Rewinds,
                        Coverage::Root(State::DeclaredOff),
                    )
                },
                Decision::Pass(Pass::Root(State::DeclaredOff)),
            ),
            (
                "a fast-forward with a covered tip passes",
                update(
                    "refs/heads/main",
                    History::FastForward,
                    Coverage::Root(State::Covered),
                ),
                Decision::Pass(Pass::Root(State::Covered)),
            ),
            (
                "a new branch with a tree match passes",
                update("refs/heads/topic", History::Creates, tree),
                Decision::Pass(Pass::Tree {
                    commit: "abc".into(),
                }),
            ),
            (
                "a tip with nothing is refused and named the command",
                update("refs/heads/main", History::FastForward, Coverage::Absent),
                Decision::Refuse(Refuse::Absent),
            ),
        ];
        for (claim, u, want) in rows {
            assert_eq!(decide(&u), want, "{claim}");
        }
        assert!(Refuse::Rewind.is_fence());
        assert!(!Refuse::Unknown.is_fence() && !Refuse::Absent.is_fence());
    }

    #[test]
    fn exempt_refs_pass_before_any_fact_about_the_tip() {
        let rows = [
            ("refs/heads/main", true, Some(Pass::Delete)),
            ("refs/tags/v1", true, Some(Pass::Delete)),
            ("refs/tags/v1", false, Some(Pass::Tag)),
            ("refs/jtac/roots/ab/abc", false, Some(Pass::Evidence)),
            ("refs/heads/main", false, None),
            ("refs/heads/refs/tags/v1", false, None),
            ("refs/tagsv1", false, None),
            ("refs/jtacx/roots", false, None),
            ("HEAD", false, None),
        ];
        for (remote_ref, delete, want) in rows {
            assert_eq!(exempt(remote_ref, delete), want, "{remote_ref} {delete}");
            let u = RefUpdate {
                delete,
                ..update(remote_ref, History::Rewinds, Coverage::Absent)
            };
            if let Some(pass) = want {
                assert_eq!(decide(&u), Decision::Pass(pass), "{remote_ref}");
            }
        }
    }
}
