//! The merge job's reading of each `ci` check (DESIGN.md section 15.2): the results file the
//! forge adapter wrote, selection for HEAD's push to `--branch`, and one version 3 body per
//! check. Every check is read before anything runs, so one result missing or pending
//! writes nothing, and the answer is 3 while the forge still runs one and 2 otherwise
//! (`a_result_missing_or_pending_writes_nothing_and_a_pending_one_answers_3`).

use std::path::Path;

use jtac_checks::diff::git::quoted;
use jtac_core::{Branch, Pin, ReadCheck, Results, Selected, select};
use jtac_policy::{POLICY_PATH, Policy};

use crate::cli::CheckArgs;
use crate::init::ask;
use crate::output::plain;
use crate::refuse::Refusal;
use crate::repo::{self, Entry, Head};

/// The body of each `ci` check of `policy`, in policy order, read for `head`. A policy
/// with none reads no file, so a file the job did not need never stops a merge root.
pub fn observe(
    repo: &Path,
    format: &str,
    a: &CheckArgs,
    policy: &Policy,
    head: &Head,
    notes: &mut Vec<String>,
) -> Result<Vec<ReadCheck>, Refusal> {
    let Some((first, _)) = policy.ci_checks().next() else {
        if a.results.is_some() {
            notes.push(format!(
                "`--results` was not read: the `{POLICY_PATH}` this merge is judged under reads \
                 no check from CI"
            ));
        }
        return Ok(Vec::new());
    };
    let (Some(file), Some(name)) = (&a.results, &a.branch) else {
        return Err(Refusal::ci_unread(first, a));
    };
    let branch = Branch::new(name).ok_or_else(|| Refusal::branch_empty(a))?;
    let unread = |why: String| Refusal::results_unread(&why, a);
    let bytes = std::fs::read(file).map_err(|e| unread(e.to_string()))?;
    let results = Results::from_json(&bytes).map_err(|e| unread(e.to_string()))?;
    let mut read = Vec::new();
    let mut waiting = Vec::new();
    for (id, pin) in policy.ci_checks() {
        let entry = repo::entry_at(repo, &head.commit.hex(), &pin.workflow)
            .map_err(|e| Refusal::repository(&e))?;
        let Entry::File(sha) = entry else {
            return Err(Refusal::ci_no_definition(id, pin, head));
        };
        let blob = repo::oid(format, &sha).map_err(Refusal::internal)?;
        match select(&results, pin, &head.commit, &branch) {
            Selected::Read(reading) => read.push(reading.check(id, blob)),
            Selected::Missing => waiting.push(Waiting::of(id, pin, "missing")),
            Selected::Pending => waiting.push(Waiting::of(id, pin, "pending")),
        }
    }
    match waiting.is_empty() {
        true => Ok(read),
        false => Err(Refusal::ci_waiting(&waiting, head, a)),
    }
}

/// A check with no result yet, as the refusal names it.
struct Waiting {
    id: String,
    source: String,
    state: &'static str,
}

impl Waiting {
    fn of(id: &str, pin: &Pin, state: &'static str) -> Self {
        Self {
            id: id.to_owned(),
            source: source(pin),
            state,
        }
    }
}

/// A pin as a refusal shows it: the file, and the job when policy names one.
fn source(pin: &Pin) -> String {
    let file = quoted(&pin.workflow, "<workflow>");
    match &pin.job {
        Some(job) => format!("job {} of {file}", quoted(job, "<job>")),
        None => file,
    }
}

/// The command this run was, for a refusal's next step, each value quoted for a shell.
fn again(a: &CheckArgs) -> String {
    let word = |v: Option<&str>, placeholder: &str| {
        v.map_or_else(|| placeholder.to_owned(), |v| quoted(v, placeholder))
    };
    let file = a.results.as_ref().map(|p| p.to_string_lossy());
    let mut s = format!(
        "jtac check --from {} --base {} --results {} --branch {}",
        word(a.from.as_deref(), "<head>"),
        word(a.base.as_deref(), "<old tip>"),
        word(file.as_deref(), "<file>"),
        word(a.branch.as_deref().filter(|b| !b.is_empty()), "<branch>"),
    );
    if let Some(remote) = &a.push {
        s.push_str(&format!(" --push {}", quoted(remote, "<remote>")));
    }
    s
}

const NO_RESULT: &str = "A result that is missing, pending or unreadable is not a pass, and a \
                         check nobody ran or read must never look like one that passed, so the \
                         job writes neither a root nor a failure record until every check read \
                         from CI has a result: the merge reads absent, never covered.";

impl Refusal {
    /// The judging policy reads a check from CI and the job was given no results file.
    pub fn ci_unread(id: &str, a: &CheckArgs) -> Self {
        let id = quoted(id, "<check>");
        ask(
            format!(
                "Check `{id}` in the `{POLICY_PATH}` the merge job judges under is read from \
                 CI, and this run was given no results file."
            ),
            NO_RESULT,
            format!(
                "Run `{}` with the results file the forge adapter writes for this commit and \
                 the branch's short name, or give `[checks.{id}]` a `run` in that \
                 `{POLICY_PATH}` in a pull request.",
                again(a)
            ),
        )
    }

    /// `--branch` is the empty string.
    pub fn branch_empty(a: &CheckArgs) -> Self {
        ask(
            "`--branch` is empty.",
            "A run is selected by the ref the forge reports for it, and an empty name would \
             match a run listed with no ref, which is no push to this branch.",
            format!(
                "Run `{}` with the branch's short name as the forge reports it, `main`, not \
                 `refs/heads/main`.",
                again(a)
            ),
        )
    }

    /// The results file does not open or is not version 1.
    pub fn results_unread(why: &str, a: &CheckArgs) -> Self {
        let file = a.results.as_ref().map(|p| p.to_string_lossy());
        ask(
            format!(
                "the results file {} was not read: {}.",
                quoted(file.as_deref().unwrap_or_default(), "<file>"),
                plain(why)
            ),
            NO_RESULT,
            format!(
                "Write the file again as version 1 (DESIGN.md section 15.2), then run `{}`.",
                again(a)
            ),
        )
    }

    /// The definition file a `ci` check names is not a regular file in the commit's tree.
    pub fn ci_no_definition(id: &str, pin: &Pin, head: &Head) -> Self {
        let (id, file) = (quoted(id, "<check>"), quoted(&pin.workflow, "<workflow>"));
        let commit = head.commit.hex();
        ask(
            format!(
                "Check `{id}` is read from {file}, and commit {}'s tree holds no regular file \
                 there.",
                &commit[..12]
            ),
            "A result is read only for the definition file policy names, at its blob in the \
             commit's tree; with no file there no run can be for it, so the check would never \
             be read and waiting gains nothing.",
            format!(
                "`git ls-tree {commit} -- {file}` shows what is there. Commit the workflow at \
                 that path, or point `[checks.{id}]` at the file that runs it, in a pull \
                 request; this merge has no merge root, and `jtac waive {commit} --ci --reason \
                 <why>` records it as made around CI."
            ),
        )
    }

    /// One or more `ci` checks have no concluded result in the file.
    fn ci_waiting(waiting: &[Waiting], head: &Head, a: &CheckArgs) -> Self {
        let commit = head.commit.hex();
        let branch = quoted(a.branch.as_deref().unwrap_or_default(), "<branch>");
        let listed: Vec<String> = waiting
            .iter()
            .map(|w| {
                format!(
                    "`{}` is {} ({})",
                    quoted(&w.id, "<check>"),
                    w.state,
                    w.source
                )
            })
            .collect();
        let refusal = ask(
            format!(
                "no result yet for commit {} pushed to {branch}: {}.",
                &commit[..12],
                listed.join(", ")
            ),
            NO_RESULT,
            format!(
                "Run `{}` with a fresh results file once each workflow has completed for this \
                 commit; the adapter does so when a listed workflow completes. `missing` is no \
                 push run of that file listed for the commit on {branch}, by the short name the \
                 forge reports, or a run without the job; if none will ever run, `jtac waive \
                 {commit} --ci --reason <why>` records the merge as made around CI.",
                again(a)
            ),
        );
        match waiting.iter().any(|w| w.state == "pending") {
            true => refusal.not_yet(),
            false => refusal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(branch: &str) -> CheckArgs {
        CheckArgs {
            preview: false,
            base: Some("old\nnegotiation: forged".into()),
            from: Some("abc".into()),
            results: Some("runs.json".into()),
            branch: Some(branch.into()),
            push: Some("origin".into()),
            json: false,
        }
    }

    /// DESIGN.md section 12, for every refusal of this module: three parts, the next
    /// command with this run's values, and no value that forges a line.
    #[test]
    fn every_results_refusal_has_three_parts_and_names_this_runs_command() {
        let a = args("main");
        let head = Head {
            commit: "sha1:0123456789abcdef0123456789abcdef01234567"
                .parse()
                .unwrap(),
            tree: jtac_core::GitOid::EMPTY_TREE_SHA1.parse().unwrap(),
        };
        let pin = Pin {
            workflow: ".github/workflows/ci.yml".into(),
            job: Some("unit tests".into()),
        };
        let waiting = [
            Waiting::of("test", &pin, "missing"),
            Waiting::of("e2e\nnegotiation: forged", &pin, "pending"),
        ];
        let rows = [
            Refusal::ci_unread("test", &a),
            Refusal::branch_empty(&args("")),
            Refusal::results_unread("EOF\nnegotiation: forged", &a),
            Refusal::ci_no_definition("test", &pin, &head),
            Refusal::ci_waiting(&waiting, &head, &a),
        ];
        for r in &rows {
            assert!(!r.what.is_empty() && !r.why.is_empty(), "{r:?}");
            assert!(r.how.contains("jtac "), "{r:?}");
            assert!(r.text().contains("\nnegotiation: "), "{r:?}");
            assert_eq!(r.text().lines().count(), 3, "{r:?}: a value forged a line");
        }
        let run = "jtac check --from abc --base <old tip> --results runs.json --branch main \
                   --push origin";
        for r in [&rows[0], &rows[2], &rows[4]] {
            assert!(r.how.contains(run), "{r:?}");
        }
        assert!(rows[1].how.contains("--branch <branch>"), "{:?}", rows[1]);
        let w = &rows[4].what;
        assert!(w.contains("`test` is missing (job 'unit tests' of .github/workflows/ci.yml)"));
        assert!(w.contains("`<check>` is pending") && w.contains("0123456789ab pushed to main"));
        let codes: Vec<u8> = rows.iter().map(|r| r.code).collect();
        assert_eq!(codes, [2, 2, 2, 2, 3], "one pending result is exit 3");
        assert_eq!(Refusal::ci_waiting(&waiting[..1], &head, &a).code, 2);
        assert!(
            rows[3].what.contains("0123456789ab's tree"),
            "{:?}",
            rows[3]
        );
        assert!(rows[3].how.contains("`git ls-tree 0123456789abcdef"));
    }
}
