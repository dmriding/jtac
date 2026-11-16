//! Selection: which listed run answers a `ci` check, and what it says (DESIGN.md section
//! 15.2, decision 59). Pure over the results file.

use crate::node::Outcome;
use crate::observed::Forge;
use crate::oid::GitOid;
use crate::pin::Pin;
use crate::results::{ForgeRun, Results};

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

/// One concluded result and the run it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    /// What the forge's word maps to.
    pub outcome: Outcome,
    /// The run's id.
    pub run: u64,
    /// The attempt read.
    pub attempt: u64,
    /// The forge's word, kept for the node.
    pub conclusion: String,
}

/// Read `pin` for `commit` from the runs a push to `branch` started. Every matching run is
/// read whole: pending while any is, then missing when any is, else the worst outcome from
/// the lowest run id that holds it. A run for another commit, event, ref or definition file
/// is never one of them (`selection_table`, `two_runs_for_one_push_take_the_worst`).
pub fn select(results: &Results, pin: &Pin, commit: &GitOid, branch: &str) -> Selected {
    let mut runs: Vec<&ForgeRun> = results
        .runs
        .iter()
        .filter(|r| {
            r.workflow == pin.workflow
                && r.commit == *commit
                && r.event == push(results.forge)
                && r.ref_name == branch
        })
        .collect();
    runs.sort_by_key(|r| r.id);
    let mut missing = runs.is_empty();
    let mut worst: Option<Reading> = None;
    for run in runs {
        match read(results.forge, run, pin.job.as_deref()) {
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
fn read(forge: Forge, run: &ForgeRun, job: Option<&str>) -> Selected {
    if run.status != completed(forge) {
        return Selected::Pending;
    }
    let (word, twice) = match job {
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
            outcome(forge, word)
        },
        run: run.id,
        attempt: run.attempt,
        conclusion: word.into(),
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

// Every word but these four is `error`: a check that did not run must not look like a pass.
fn outcome(forge: Forge, word: &str) -> Outcome {
    match (forge, word) {
        (Forge::Github, "success") => Outcome::Pass,
        (Forge::Github, "failure" | "timed_out") => Outcome::Fail,
        (Forge::Github, "skipped") => Outcome::Skipped,
        (Forge::Github, _) => Outcome::Error,
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

    fn pick(runs: Vec<ForgeRun>, job: Option<&str>) -> Selected {
        select(&file(runs), &pin(job), &COMMIT_ONE.parse().unwrap(), "main")
    }

    fn got(outcome: Outcome, run: u64, conclusion: &str) -> Selected {
        Selected::Read(Reading {
            outcome,
            run,
            attempt: 1,
            conclusion: conclusion.into(),
        })
    }

    fn with(edit: impl FnOnce(&mut ForgeRun)) -> Vec<ForgeRun> {
        let mut r = run(1, "success");
        edit(&mut r);
        vec![r]
    }

    /// DESIGN.md section 15.2, M20's done-when: a run for anything but this commit's push
    /// to this branch of this file is not selected.
    #[test]
    fn selection_table() {
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
            ("no run at all", vec![], test, Selected::Missing),
            (
                "another commit",
                with(|r| {
                    r.commit = "sha1:0000000000000000000000000000000000000002"
                        .parse()
                        .unwrap()
                }),
                test,
                Selected::Missing,
            ),
            (
                "a pull request's run",
                with(|r| r.event = "pull_request".into()),
                test,
                Selected::Missing,
            ),
            (
                "a manual run",
                with(|r| r.event = "workflow_dispatch".into()),
                test,
                Selected::Missing,
            ),
            (
                "a scheduled run",
                with(|r| r.event = "schedule".into()),
                test,
                Selected::Missing,
            ),
            (
                "another ref",
                with(|r| r.ref_name = "release".into()),
                test,
                Selected::Missing,
            ),
            (
                "the full ref name",
                with(|r| r.ref_name = "refs/heads/main".into()),
                test,
                Selected::Missing,
            ),
            (
                "another definition file",
                with(|r| r.workflow = ".github/workflows/other.yml".into()),
                test,
                Selected::Missing,
            ),
            (
                "the file under another spelling",
                with(|r| r.workflow = format!("./{WORKFLOW}")),
                test,
                Selected::Missing,
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
                Selected::Pending,
            ),
            (
                "a queued run",
                with(|r| r.status = "queued".into()),
                None,
                Selected::Pending,
            ),
            (
                "a run in progress that lacks the job so far",
                with(|r| {
                    r.status = "in_progress".into();
                    r.jobs.clear();
                }),
                test,
                Selected::Pending,
            ),
            (
                "a completed run with no word yet",
                with(|r| r.conclusion = None),
                None,
                Selected::Pending,
            ),
            (
                "a completed run whose job has no word yet",
                with(|r| r.jobs = vec![job("test", None)]),
                test,
                Selected::Pending,
            ),
            (
                "a missing job",
                with(|r| r.jobs = vec![job("lint", Some("success"))]),
                test,
                Selected::Missing,
            ),
            (
                "a job name that differs by case",
                with(|r| r.jobs = vec![job("Test", Some("success"))]),
                test,
                Selected::Missing,
            ),
            (
                "a called workflow's job is named in full",
                with(|r| r.jobs = vec![job("call / test", Some("success"))]),
                test,
                Selected::Missing,
            ),
            (
                "a job name held twice",
                with(|r| r.jobs = vec![job("test", Some("success")), job("test", Some("success"))]),
                test,
                got(Outcome::Error, 1, "success"),
            ),
            (
                "a job name held twice in a run with no word yet",
                with(|r| {
                    r.jobs = vec![job("test", Some("success")), job("test", Some("success"))];
                    r.conclusion = None;
                }),
                test,
                Selected::Pending,
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
                Selected::Read(Reading {
                    outcome: Outcome::Pass,
                    run: 1,
                    attempt: 3,
                    conclusion: "success".into(),
                }),
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
                let want = got(outcome(Forge::Github, worse), id, worse);
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
                Selected::Pending,
            ),
            (
                "pending beside a failure",
                vec![pending(1), run(2, "failure")],
                Selected::Pending,
            ),
            (
                "pending beside a missing job",
                vec![jobless(1), pending(2)],
                Selected::Pending,
            ),
            (
                "a missing job beside a pass",
                vec![run(1, "success"), jobless(2)],
                Selected::Missing,
            ),
            (
                "a missing job beside a failure",
                vec![jobless(1), run(2, "failure")],
                Selected::Missing,
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
