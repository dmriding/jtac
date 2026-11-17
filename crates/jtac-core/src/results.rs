//! The results file: the runs a forge lists for one commit, as the forge adapter wrote them
//! (DESIGN.md section 15.2). The binary reads it and never calls a forge (decision 40).

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::canonical::MAX_SAFE_INTEGER;
use crate::observed::Forge;
use crate::oid::GitOid;
use crate::schema::V1;

/// The file, version 1: one entry for each run the forge lists for the commit, each at its
/// newest attempt. `from_json` is the one way in: the type has no `Deserialize` of its
/// own and no field another crate can set, so no caller reads or builds a file past the
/// reader's refusals.
///
/// ```compile_fail
/// fn runs(file: &mut jtac_core::Results) { file.runs.clear() }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Results {
    pub(crate) v: V1,
    pub(crate) forge: Forge,
    pub(crate) runs: Vec<ForgeRun>,
}

/// One run of one definition file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForgeRun {
    /// The definition file's path, as the forge reports it.
    pub workflow: String,
    /// The commit the run was for.
    pub commit: GitOid,
    /// The forge's word for what started the run.
    pub event: String,
    /// The ref the run was for, by the short name the forge reports.
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// The forge's id for the run; a re-run keeps it.
    pub id: u64,
    /// The newest attempt of the run.
    pub attempt: u64,
    /// The forge's word for how far the run has got.
    pub status: String,
    /// The forge's word for how the run ended; absent until it has.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub conclusion: Option<String>,
    /// Every job of that attempt.
    pub jobs: Vec<ForgeJob>,
}

/// One job of a run, named in full as the forge reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForgeJob {
    /// The job's name: a matrix leg and a called workflow's job carry theirs whole.
    pub name: String,
    /// The forge's word for how the job ended; absent until it has.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub conclusion: Option<String>,
}

// One spelling for no value: the key is left out. `Option` alone would also read `null`.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

/// Why a results file was not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResultsError {
    /// Not JSON, or not this version's shape.
    #[error("not a version 1 results file: {0}")]
    Shape(String),
    /// Two entries for one run: the adapter writes each run once, at its newest attempt.
    #[error("run {id} is listed more than once")]
    DuplicateRun {
        /// The run's id.
        id: u64,
    },
    /// A run id or attempt no node could carry (DESIGN.md section 9.1).
    #[error("run {id} holds an integer beyond 2^53-1")]
    UnsafeInteger {
        /// The run's id.
        id: u64,
    },
}

// What `from_json` parses. An unknown key, version or forge is refused, so a later format
// is never half-read as this one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    v: V1,
    forge: Forge,
    runs: Vec<ForgeRun>,
}

impl Results {
    /// Read the file's bytes. Refuses a shape this version does not write, a run listed
    /// twice and an integer a node could not carry. serde's derive reads spellings no
    /// writer emits, a struct as an array or an enum as a one-key map, so the file must
    /// also say, value for value, what this type writes back; whitespace and key order
    /// are free (`the_reader_refuses_what_version_1_does_not_write`).
    pub fn from_json(bytes: &[u8]) -> Result<Self, ResultsError> {
        let shape = |e: serde_json::Error| ResultsError::Shape(e.to_string());
        let Wire { v, forge, runs } = serde_json::from_slice(bytes).map_err(shape)?;
        let file = Self { v, forge, runs };
        let said: Value = serde_json::from_slice(bytes).map_err(shape)?;
        if serde_json::to_value(&file).map_err(shape)? != said {
            return Err(ResultsError::Shape(
                "a value is not spelled as version 1 writes it".into(),
            ));
        }
        for (i, run) in file.runs.iter().enumerate() {
            let id = run.id;
            if id > MAX_SAFE_INTEGER || run.attempt > MAX_SAFE_INTEGER {
                return Err(ResultsError::UnsafeInteger { id });
            }
            if file.runs[..i].iter().any(|r| r.id == id) {
                return Err(ResultsError::DuplicateRun { id });
            }
        }
        Ok(file)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::node::tests::COMMIT_ONE;

    pub(crate) const WORKFLOW: &str = ".github/workflows/ci.yml";

    pub(crate) fn job(name: &str, conclusion: Option<&str>) -> ForgeJob {
        ForgeJob {
            name: name.into(),
            conclusion: conclusion.map(Into::into),
        }
    }

    /// A completed push run of `WORKFLOW` for `COMMIT_ONE` on `main`, one job `test`.
    pub(crate) fn run(id: u64, conclusion: &str) -> ForgeRun {
        ForgeRun {
            workflow: WORKFLOW.into(),
            commit: COMMIT_ONE.parse().unwrap(),
            event: "push".into(),
            ref_name: "main".into(),
            id,
            attempt: 1,
            status: "completed".into(),
            conclusion: Some(conclusion.into()),
            jobs: vec![job("test", Some(conclusion))],
        }
    }

    pub(crate) fn file(runs: Vec<ForgeRun>) -> Results {
        Results {
            v: V1,
            forge: Forge::Github,
            runs,
        }
    }

    const DOC: &str = concat!(
        r#"{"v":1,"forge":"github","runs":[{"workflow":".github/workflows/ci.yml","#,
        r#""commit":"sha1:0000000000000000000000000000000000000001","event":"push","ref":"main","#,
        r#""id":1234,"attempt":1,"status":"completed","conclusion":"success","#,
        r#""jobs":[{"name":"test","conclusion":"success"}]}]}"#
    );

    #[test]
    fn the_design_example_reads_and_writes_back() {
        // DESIGN.md section 15.2. The adapter writes this shape; changing it is a new version.
        let want = file(vec![run(1234, "success")]);
        assert_eq!(Results::from_json(DOC.as_bytes()).unwrap(), want);
        assert_eq!(serde_json::to_string(&want).unwrap(), DOC);
        let spaced = serde_json::to_string_pretty(&want).unwrap();
        assert_eq!(Results::from_json(spaced.as_bytes()).unwrap(), want);
    }

    #[test]
    fn a_run_or_job_not_yet_concluded_leaves_the_key_out() {
        let mut pending = run(1, "success");
        pending.status = "in_progress".into();
        pending.conclusion = None;
        pending.jobs = vec![job("test", None)];
        let text = serde_json::to_string(&file(vec![pending.clone()])).unwrap();
        assert!(!text.contains("conclusion"), "{text}");
        assert_eq!(
            Results::from_json(text.as_bytes()).unwrap().runs,
            vec![pending]
        );
        assert_eq!(
            Results::from_json(br#"{"v":1,"forge":"github","runs":[]}"#)
                .unwrap()
                .runs,
            vec![]
        );
    }

    #[test]
    fn the_reader_refuses_what_version_1_does_not_write() {
        let edit = |from: &str, to: &str| {
            assert!(DOC.contains(from), "{from}");
            DOC.replacen(from, to, 1)
        };
        let twice = serde_json::to_string(&file(vec![run(7, "success"), run(7, "failure")]));
        let rows = [
            ("not json", "{".to_string(), "Shape"),
            ("version 2", edit(r#""v":1"#, r#""v":2"#), "Shape"),
            ("no version", edit(r#""v":1,"#, ""), "Shape"),
            ("unknown forge", edit("github\"", "example\""), "Shape"),
            (
                "extra top-level key",
                edit(r#""v":1"#, r#""v":1,"x":1"#),
                "Shape",
            ),
            (
                "extra run key",
                edit(r#""id":1234"#, r#""id":1234,"x":1"#),
                "Shape",
            ),
            (
                "extra job key",
                edit(r#""name":"test""#, r#""name":"test","x":1"#),
                "Shape",
            ),
            (
                "a key twice",
                edit(r#""id":1234"#, r#""id":1,"id":1234"#),
                "Shape",
            ),
            ("a bare commit id", edit("sha1:", ""), "Shape"),
            (
                "a null run conclusion",
                edit(
                    r#""conclusion":"success","jobs""#,
                    r#""conclusion":null,"jobs""#,
                ),
                "Shape",
            ),
            (
                "a null job conclusion",
                edit(r#""conclusion":"success"}]"#, r#""conclusion":null}]"#),
                "Shape",
            ),
            (
                "a float id",
                edit(r#""id":1234"#, r#""id":1234.0"#),
                "Shape",
            ),
            (
                "a string id",
                edit(r#""id":1234"#, r#""id":"1234""#),
                "Shape",
            ),
            (
                "a negative attempt",
                edit(r#""attempt":1"#, r#""attempt":-1"#),
                "Shape",
            ),
            (
                "no jobs key",
                edit(r#","jobs":[{"name":"test","conclusion":"success"}]"#, ""),
                "Shape",
            ),
            ("no status", edit(r#""status":"completed","#, ""), "Shape"),
            ("trailing bytes", format!("{DOC}{{}}"), "Shape"),
            (
                "an id past 2^53-1",
                edit(r#""id":1234"#, r#""id":9007199254740992"#),
                "UnsafeInteger",
            ),
            (
                "an attempt past 2^53-1",
                edit(r#""attempt":1"#, r#""attempt":9007199254740992"#),
                "UnsafeInteger",
            ),
            ("one run twice", twice.unwrap(), "DuplicateRun"),
            (
                "the file as an array",
                r#"[1,"github",[]]"#.to_string(),
                "Shape",
            ),
            (
                "the forge as a one-key map",
                edit(r#""forge":"github""#, r#""forge":{"github":null}"#),
                "Shape",
            ),
            (
                "a run as an array",
                format!(
                    r#"{{"v":1,"forge":"github","runs":[["{WORKFLOW}","{COMMIT_ONE}","push","main",1,1,"completed","success",[]]]}}"#
                ),
                "Shape",
            ),
            (
                "a job as an array",
                edit(
                    r#"{"name":"test","conclusion":"success"}"#,
                    r#"["test","success"]"#,
                ),
                "Shape",
            ),
            (
                "a job as a short array",
                edit(r#"{"name":"test","conclusion":"success"}"#, r#"["test"]"#),
                "Shape",
            ),
        ];
        for (name, doc, want) in rows {
            let err = Results::from_json(doc.as_bytes()).expect_err(name);
            let got = format!("{err:?}");
            assert!(got.starts_with(want), "{name}: {got}");
            assert!(!err.to_string().is_empty(), "{name}");
        }
        let top = edit(r#""id":1234"#, r#""id":9007199254740991"#);
        assert!(Results::from_json(top.as_bytes()).is_ok());
    }
}
