//! Selection: which listed run answers a `ci` check, and what it says (DESIGN.md section
//! 15.2, decision 59). Pure over the results file.

use crate::node::Outcome;
use crate::observed::{Forge, Observed, ObservedCheck};
use crate::oid::GitOid;
use crate::pin::Pin;
use crate::results::{ForgeRun, Results};

/// The branch a merge job runs for, by the short name the forge reports. Never empty: a
/// run listed with no ref would match an empty name (`a_branch_name_is_never_empty`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch(String);

impl Branch {
    /// `None` for the empty name.
    pub fn new(name: &str) -> Option<Self> {
        (!name.is_empty()).then(|| Self(name.to_owned()))
    }
}

/// What the file holds for one `ci` check. Only `Read` is a result; the merge job writes
/// nothing on the other two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selected {
    /// No run matches, or one that does lacks the named job.
    Missing,
    /// A matching run has not concluded.
    Pending,
    /// Every matching run concluded; this is the worst of them.
    Read(Reading),
}

/// A version 3 body as selection read it. `Reading::check` is the one constructor, so a
/// gate that takes this type is offered no result selection did not read.
///
/// ```compile_fail
/// fn forged(c: jtac_core::ObservedCheck) -> jtac_core::ReadCheck { jtac_core::ReadCheck(c) }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadCheck(ObservedCheck);

impl ReadCheck {
    /// The body.
    pub fn body(&self) -> &ObservedCheck {
        &self.0
    }
}

/// One concluded result and the run it came from. Only `select` makes one, and `check` is
/// the one way a result becomes a node, so no node says more than selection read
/// (`a_reading_becomes_the_node_selection_read_and_no_other`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub(crate) outcome: Outcome,
    pub(crate) run: u64,
    pub(crate) attempt: u64,
    pub(crate) conclusion: String,
    pub(crate) forge: Forge,
    pub(crate) pin: Pin,
    pub(crate) commit: GitOid,
}

impl Reading {
    /// The version 3 body for policy check `id`, with `blob` the definition file's in the
    /// commit's tree, read from git.
    pub fn check(&self, id: &str, blob: GitOid) -> ReadCheck {
        ReadCheck(ObservedCheck {
            id: id.to_owned(),
            outcome: self.outcome,
            observed: Observed {
                forge: self.forge,
                workflow: self.pin.workflow.clone(),
                blob,
                job: self.pin.job.clone(),
                commit: self.commit,
                run: self.run,
                attempt: self.attempt,
                conclusion: self.conclusion.clone(),
            },
        })
    }
}

/// Read `pin` for `commit` from the runs a push to `branch` started. Every matching run is
/// read whole: pending while any is, then missing when any is, else the worst outcome from
/// the lowest run id that holds it. A run for another commit, event, ref or definition file
/// is never one of them
/// (`only_this_commits_push_to_this_branch_of_this_file_is_selected`,
/// `two_runs_for_one_push_take_the_worst`).
pub fn select(results: &Results, pin: &Pin, commit: &GitOid, branch: &Branch) -> Selected {
    let mut runs: Vec<&ForgeRun> = results
        .runs
        .iter()
        .filter(|r| {
            r.workflow == pin.workflow
                && r.commit == *commit
                && r.event == push(results.forge)
                && r.ref_name == branch.0
        })
        .collect();
    runs.sort_by_key(|r| r.id);
    let mut missing = runs.is_empty();
    let mut worst: Option<Reading> = None;
    for run in runs {
        match read(results.forge, run, pin, commit) {
            Selected::Pending => return Selected::Pending,
            Selected::Missing => missing = true,
            Selected::Read(r) => {
                if worst
                    .as_ref()
                    .is_none_or(|w| rank(r.outcome) < rank(w.outcome))
                {
                    worst = Some(r);
                }
            }
        }
    }
    match worst {
        Some(reading) if !missing => Selected::Read(reading),
        _ => Selected::Missing,
    }
}

// One run. A run not completed is pending whatever it lists; so is a completed one whose
// word is not there yet. A job name held twice is `error` under the run's own word.
fn read(forge: Forge, run: &ForgeRun, pin: &Pin, commit: &GitOid) -> Selected {
    if run.status != completed(forge) {
        return Selected::Pending;
    }
    let (word, twice) = match pin.job.as_deref() {
        None => (run.conclusion.as_deref(), false),
        Some(name) => {
            let mut held = run.jobs.iter().filter(|j| j.name == name);
            match (held.next(), held.next()) {
                (None, _) => return Selected::Missing,
                (Some(j), None) => (j.conclusion.as_deref(), false),
                (Some(_), Some(_)) => (run.conclusion.as_deref(), true),
            }
        }
    };
    let Some(word) = word else {
        return Selected::Pending;
    };
    Selected::Read(Reading {
        outcome: if twice {
            Outcome::Error
        } else {
            forge.outcome(word)
        },
        run: run.id,
        attempt: run.attempt,
        conclusion: word.into(),
        forge,
        pin: pin.clone(),
        commit: *commit,
    })
}

fn push(forge: Forge) -> &'static str {
    match forge {
        Forge::Github => "push",
    }
}

fn completed(forge: Forge) -> &'static str {
    match forge {
        Forge::Github => "completed",
    }
}

fn rank(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Fail => 0,
        Outcome::Error => 1,
        Outcome::Skipped => 2,
        Outcome::Pass => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::tests::COMMIT_ONE;
    use crate::results::tests::{WORKFLOW, file, job, run};

    fn pin(job: Option<&str>) -> Pin {
        Pin {
            workflow: WORKFLOW.into(),
            job: job.map(Into::into),
        }
    }

    /// What selection read, without the pin and commit the caller gave it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Seen {
        Missing,
        Pending,
        Read(Outcome, u64, u64, String),
    }

    fn main() -> Branch {
        Branch::new("main").unwrap()
    }

    fn pick(runs: Vec<ForgeRun>, job: Option<&str>) -> Seen {
        match select(
            &file(runs),
            &pin(job),
            &COMMIT_ONE.parse().unwrap(),
            &main(),
        ) {
            Selected::Missing => Seen::Missing,
            Selected::Pending => Seen::Pending,
            Selected::Read(r) => Seen::Read(r.outcome, r.run, r.attempt, r.conclusion),
        }
    }

    fn got(outcome: Outcome, run: u64, conclusion: &str) -> Seen {
        Seen::Read(outcome, run, 1, conclusion.into())
    }

    /// Issue 161 note 6: the reader accepts a run with no ref, so the name it is matched
    /// against can never be empty.
    #[test]
    fn a_branch_name_is_never_empty() {
        assert_eq!(Branch::new(""), None);
        let mut refless = run(1, "success");
        refless.ref_name = String::new();
        let commit = COMMIT_ONE.parse().unwrap();
        let read = |b: &str| {
            select(
                &file(vec![refless.clone()]),
                &pin(None),
                &commit,
                &Branch::new(b).unwrap(),
            )
        };
        assert_eq!(read("main"), Selected::Missing);
        assert_eq!(read(" "), Selected::Missing);
    }

    /// Issue 161 note 2: the node's outcome and source are selection's, for every word
    /// and for a job name held twice, so it is coherent by construction.
    #[test]
    fn a_reading_becomes_the_node_selection_read_and_no_other() {
        let commit: GitOid = COMMIT_ONE.parse().unwrap();
        let blob: GitOid = "sha1:e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
            .parse()
            .unwrap();
        let twice = |word: &str| {
            let mut r = run(7, word);
            r.attempt = 3;
            r.jobs.push(job("test", Some("success")));
            r
        };
        let words = [
            "success",
            "failure",
            "timed_out",
            "skipped",
            "cancelled",
            "",
        ];
        for word in words {
            for (job, runs) in [
                (Some("test"), vec![run(7, word)]),
                (None, vec![run(7, word)]),
                (Some("test"), vec![twice(word)]),
            ] {
                let held_twice = runs[0].jobs.len() == 2;
                let pin = pin(job);
                let Selected::Read(reading) = select(&file(runs), &pin, &commit, &main()) else {
                    panic!("{word:?} {job:?}: no reading");
                };
                let read = reading.check("unit", blob);
                let node = read.body();
                assert!(node.coherent(), "{word:?} {job:?}");
                assert!(node.observed.is_for(&pin, &commit), "{word:?} {job:?}");
                let want = match held_twice {
                    true => Outcome::Error,
                    false => Forge::Github.outcome(word),
                };
                assert_eq!((node.id.as_str(), node.outcome), ("unit", want));
                let o = &node.observed;
                assert_eq!(
                    (o.forge, o.blob, o.conclusion.as_str()),
                    (Forge::Github, blob, word)
                );
                assert_eq!((o.run, o.attempt), (7, if held_twice { 3 } else { 1 }));
            }
        }
    }

    fn with(edit: impl FnOnce(&mut ForgeRun)) -> Vec<ForgeRun> {
        let mut r = run(1, "success");
        edit(&mut r);
        vec![r]
    }

    /// DESIGN.md section 15.2, M20's done-when: a run for anything but this commit's push
    /// to this branch of this file is not selected.
    #[test]
    fn only_this_commits_push_to_this_branch_of_this_file_is_selected() {
        let test = Some("test");
        let pass = got(Outcome::Pass, 1, "success");
        let rows = vec![
            (
                "a lone run under the branch's name",
                with(|_| {}),
                test,
                pass.clone(),
            ),
            (
                "no job named: the run's word",
                with(|r| r.jobs.clear()),
                None,
                pass.clone(),
            ),
            ("no run at all", vec![], test, Seen::Missing),
            (
                "another commit",
                with(|r| {
                    r.commit = "sha1:0000000000000000000000000000000000000002"
                        .parse()
                        .unwrap()
                }),
                test,
                Seen::Missing,
            ),
            (
                "a pull request's run",
                with(|r| r.event = "pull_request".into()),
                test,
                Seen::Missing,
            ),
            (
                "a manual run",
                with(|r| r.event = "workflow_dispatch".into()),
                test,
                Seen::Missing,
            ),
            (
                "a scheduled run",
                with(|r| r.event = "schedule".into()),
                test,
                Seen::Missing,
            ),
            (
                "another ref",
                with(|r| r.ref_name = "release".into()),
                test,
                Seen::Missing,
            ),
            (
                "the full ref name",
                with(|r| r.ref_name = "refs/heads/main".into()),
                test,
                Seen::Missing,
            ),
            (
                "another definition file",
                with(|r| r.workflow = ".github/workflows/other.yml".into()),
                test,
                Seen::Missing,
            ),
            (
                "the file under another spelling",
                with(|r| r.workflow = format!("./{WORKFLOW}")),
                test,
                Seen::Missing,
            ),
        ];
        for (name, runs, job, want) in rows {
            assert_eq!(pick(runs, job), want, "{name}");
        }
    }

    /// What is not one concluded result is never a pass: pending, missing, or `error`.
    #[test]
    fn a_run_is_read_only_once_it_has_concluded_and_holds_the_job_once() {
        let test = Some("test");
        let pass = got(Outcome::Pass, 1, "success");
        let rows = vec![
            (
                "a run in progress",
                with(|r| r.status = "in_progress".into()),
                test,
                Seen::Pending,
            ),
            (
                "a queued run",
                with(|r| r.status = "queued".into()),
                None,
                Seen::Pending,
            ),
            (
                "a run in progress that lacks the job so far",
                with(|r| {
                    r.status = "in_progress".into();
                    r.jobs.clear();
                }),
                test,
                Seen::Pending,
            ),
            (
                "a completed run with no word yet",
                with(|r| r.conclusion = None),
                None,
                Seen::Pending,
            ),
            (
                "a completed run whose job has no word yet",
                with(|r| r.jobs = vec![job("test", None)]),
                test,
                Seen::Pending,
            ),
            (
                "a missing job",
                with(|r| r.jobs = vec![job("lint", Some("success"))]),
                test,
                Seen::Missing,
            ),
            (
                "a job name that differs by case",
                with(|r| r.jobs = vec![job("Test", Some("success"))]),
                test,
                Seen::Missing,
            ),
            (
                "a called workflow's job is named in full",
                with(|r| r.jobs = vec![job("call / test", Some("success"))]),
                test,
                Seen::Missing,
            ),
            (
                "a job name held twice",
                with(|r| r.jobs = vec![job("test", Some("success")), job("test", Some("success"))]),
                test,
                got(Outcome::Error, 1, "success"),
            ),
            (
                "a job name held twice keeps the run's word, not a job's",
                with(|r| {
                    r.jobs = vec![job("test", Some("success")), job("test", Some("success"))];
                    r.conclusion = Some("failure".into());
                }),
                test,
                got(Outcome::Error, 1, "failure"),
            ),
            (
                "a job name held twice in a run with no word yet",
                with(|r| {
                    r.jobs = vec![job("test", Some("success")), job("test", Some("success"))];
                    r.conclusion = None;
                }),
                test,
                Seen::Pending,
            ),
            (
                "the job's word, not the run's",
                with(|r| r.conclusion = Some("failure".into())),
                test,
                pass.clone(),
            ),
            (
                "the run's word, not a job's",
                with(|r| r.jobs = vec![job("test", Some("failure"))]),
                None,
                pass,
            ),
            (
                "the newest attempt is the one the file holds",
                with(|r| r.attempt = 3),
                test,
                Seen::Read(Outcome::Pass, 1, 3, "success".into()),
            ),
        ];
        for (name, runs, job, want) in rows {
            assert_eq!(pick(runs, job), want, "{name}");
        }
    }

    #[test]
    fn every_conclusion_but_four_is_an_error() {
        let rows = [
            ("success", Outcome::Pass),
            ("failure", Outcome::Fail),
            ("timed_out", Outcome::Fail),
            ("skipped", Outcome::Skipped),
            ("cancelled", Outcome::Error),
            ("neutral", Outcome::Error),
            ("action_required", Outcome::Error),
            ("stale", Outcome::Error),
            ("startup_failure", Outcome::Error),
            ("Success", Outcome::Error),
            ("", Outcome::Error),
        ];
        for (word, want) in rows {
            for job in [Some("test"), None] {
                assert_eq!(
                    pick(vec![run(1, word)], job),
                    got(want, 1, word),
                    "{word:?} {job:?}"
                );
            }
        }
    }

    /// Decision 59: each pair of outcomes, in both file orders, reads the worse one from
    /// the lowest run id that holds it; pending beats missing, and either beats a result.
    #[test]
    fn two_runs_for_one_push_take_the_worst() {
        let words = ["failure", "cancelled", "skipped", "success"];
        for (i, a) in words.iter().enumerate() {
            for (j, b) in words.iter().enumerate() {
                let worse = if i <= j { a } else { b };
                let id = if i <= j { 5 } else { 9 };
                let want = got(Forge::Github.outcome(worse), id, worse);
                assert_eq!(
                    pick(vec![run(5, a), run(9, b)], Some("test")),
                    want,
                    "{a} {b}"
                );
                assert_eq!(
                    pick(vec![run(9, b), run(5, a)], Some("test")),
                    want,
                    "{b} {a}"
                );
            }
        }
        let pending = |id| {
            let mut r = run(id, "success");
            r.status = "in_progress".into();
            r
        };
        let jobless = |id| {
            let mut r = run(id, "success");
            r.jobs.clear();
            r
        };
        let other_ref = |id| {
            let mut r = run(id, "failure");
            r.ref_name = "release".into();
            r
        };
        let rows = vec![
            (
                "pending beside a pass",
                vec![run(1, "success"), pending(2)],
                Seen::Pending,
            ),
            (
                "pending beside a failure",
                vec![pending(1), run(2, "failure")],
                Seen::Pending,
            ),
            (
                "pending beside a missing job",
                vec![jobless(1), pending(2)],
                Seen::Pending,
            ),
            (
                "a missing job beside a pass",
                vec![run(1, "success"), jobless(2)],
                Seen::Missing,
            ),
            (
                "a missing job beside a failure",
                vec![jobless(1), run(2, "failure")],
                Seen::Missing,
            ),
            (
                "a failing run for another ref is not beside it",
                vec![run(1, "success"), other_ref(2)],
                got(Outcome::Pass, 1, "success"),
            ),
        ];
        for (name, runs, want) in rows {
            assert_eq!(pick(runs, Some("test")), want, "{name}");
        }
    }
}
