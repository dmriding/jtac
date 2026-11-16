//! Where policy says a check's result is read from (DESIGN.md section 15.2).

use serde::{Deserialize, Serialize};

/// One CI definition file and, when named, one job in it: a policy check's `ci` value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    /// The definition file's path in the tree.
    pub workflow: String,
    /// The job, by the name the forge reports. Absent, the run's own conclusion is the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
}
