//! The rule table of DESIGN.md section 10.3: id, the reason the rule exists, and the fix. The
//! CLI's three-part message (section 12) is built from these; a finding's message is the detail.

/// One diff-integrity rule, in table order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// A change to a protected path.
    Prot001,
    /// A test function or file removed.
    Test001,
    /// A test newly marked skip, ignore, or focused so the others are skipped.
    Test002,
    /// Assertions removed from a test that still exists.
    Test003,
    /// A new lint or type suppression.
    Sup001,
    /// A CI step removed or made non-blocking.
    Ci001,
    /// A string shaped like a credential added.
    Sec001,
    /// A changed file past the production-line cap with no exemption marker.
    Size001,
    /// A changed function past the line cap.
    Size002,
    /// A changed file with as many comment lines as code, or a comment inside a block.
    Cmt001,
    /// Lines added past `max_added_lines`.
    Add001,
}

impl Rule {
    /// Every rule in table order.
    pub const ALL: [Rule; 11] = [
        Rule::Prot001,
        Rule::Test001,
        Rule::Test002,
        Rule::Test003,
        Rule::Sup001,
        Rule::Ci001,
        Rule::Sec001,
        Rule::Size001,
        Rule::Size002,
        Rule::Cmt001,
        Rule::Add001,
    ];

    /// The rule an evidence id names, if any.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id() == id)
    }

    /// The id as evidence spells it.
    pub fn id(self) -> &'static str {
        match self {
            Self::Prot001 => "JT-PROT-001",
            Self::Test001 => "JT-TEST-001",
            Self::Test002 => "JT-TEST-002",
            Self::Test003 => "JT-TEST-003",
            Self::Sup001 => "JT-SUP-001",
            Self::Ci001 => "JT-CI-001",
            Self::Sec001 => "JT-SEC-001",
            Self::Size001 => "JT-SIZE-001",
            Self::Size002 => "JT-SIZE-002",
            Self::Cmt001 => "JT-CMT-001",
            Self::Add001 => "JT-ADD-001",
        }
    }

    /// Why the rule exists.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Prot001 => {
                "Protected paths hold the policy, CI, and tool configuration that every other \
                 check depends on. A change there can silently weaken the gate, so it is \
                 recorded as a finding and raises the change's tier."
            }
            Self::Test001 => {
                "A removed test is the most common way a failing change becomes a passing one. \
                 The test may be obsolete, but that is a decision a person records, not a side \
                 effect of a fix."
            }
            Self::Test002 => {
                "A skipped or ignored test passes nothing and looks green. Focusing one test \
                 skips every other, which is the same thing wearing a different spelling."
            }
            Self::Test003 => {
                "A test that still runs but asserts less proves less. Fewer assertions in a \
                 test that kept its name is the shape of a test weakened to make a change pass."
            }
            Self::Sup001 => {
                "A suppression turns a tool's finding off at one line. Each one is a decision \
                 that a person should see; past the budget it blocks."
            }
            Self::Ci001 => {
                "A CI step removed or allowed to fail stops a check from blocking a merge. \
                 jtac's own evidence is compared in CI from v0.2, so CI configuration is a gate \
                 input, not an ordinary file."
            }
            Self::Sec001 => {
                "A string shaped like a credential is an immediate leak once pushed, whatever \
                 it was for. History is not rewritten, so it is caught before the commit leaves \
                 the machine."
            }
            Self::Size001 => {
                "Model-written code arrives correct at several times the necessary length, and \
                 length is what review cannot keep up with. A file past the cap holds more than \
                 one concern."
            }
            Self::Size002 => {
                "A function past the cap cannot be read in one sitting, and a reviewer who \
                 skims it approves what they did not read."
            }
            Self::Cmt001 => {
                "A comment that narrates code, or sits inside a block, is text a reviewer reads \
                 instead of the code; the WHY belongs at a block's head or tail and the rest in \
                 the pull request."
            }
            Self::Add001 => {
                "Lines added is the cost a reviewer pays. The repository set a budget for one \
                 change, and this one is over it."
            }
        }
    }

    /// What to do about it.
    pub fn fix(self) -> &'static str {
        match self {
            Self::Prot001 => {
                "leave the protected path as it was, or have a maintainer review the change under the raised tier"
            }
            Self::Test001 => {
                "restore the test; if it is obsolete, remove it with a rule waiver that says why: jtac waive --rule JT-TEST-001 --path <path> --reason <why>"
            }
            Self::Test002 => "remove the skip, ignore, or focus marker and fix the test it hides",
            Self::Test003 => {
                "restore the assertions; if the behavior changed, assert the new behavior instead of asserting less"
            }
            Self::Sup001 => {
                "fix what the tool reported instead of suppressing it, or raise integrity.max_new_suppressions in the policy with a reviewer's approval"
            }
            Self::Ci001 => {
                "restore the CI step and let it block; a flaky step is fixed, not allowed to fail"
            }
            Self::Sec001 => {
                "remove the string, rotate the credential it came from, and read it from the environment or a secret store"
            }
            Self::Size001 => {
                "split the file by concern; a file that cannot be split carries jtac-loc-exempt: <reason> on line 1 and a defense in the pull request"
            }
            Self::Size002 => "split the function; extract the piece with its own name and test",
            Self::Cmt001 => {
                "cut the comment lines below the code lines, move a comment to the head or tail of its block, and put the explanation in the pull request"
            }
            Self::Add001 => {
                "split the change into smaller pull requests, or raise integrity.max_added_lines in the policy with a reviewer's approval"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Rule; 11] = Rule::ALL;

    #[test]
    fn every_rule_has_a_distinct_id_a_reason_and_a_fix_in_table_order() {
        let ids: Vec<&str> = ALL.iter().map(|r| r.id()).collect();
        let mut sorted = ids.clone();
        sorted.dedup();
        assert_eq!(ids, sorted);
        for pair in ALL.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        for rule in ALL {
            let (head, number) = rule.id().rsplit_once('-').unwrap();
            assert!(head.starts_with("JT-") && number.len() == 3, "{rule:?}");
            assert!(number.chars().all(|c| c.is_ascii_digit()), "{rule:?}");
            assert!(
                rule.reason().len() > 40 && rule.fix().len() > 20,
                "{rule:?}"
            );
            assert!(!rule.fix().contains('{'), "{rule:?}: unfilled value");
            assert_eq!(Rule::from_id(rule.id()), Some(rule));
        }
        assert_eq!(Rule::from_id("JT-TEST-004"), None);
    }
}
