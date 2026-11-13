//! `jtac backtest <range>` (DESIGN.md section 11.10, decision 51): the diff-integrity rules over
//! merged history, one unit per first-parent commit against its first parent, under one
//! policy, with no waiver, no adoption and no check run. A report: it writes nothing to the
//! repository or the store, needs no `jtac init`, and exits 0 whenever it ran.

mod policy;
mod refuse;
mod render;
mod row;
mod summary;

use std::io::Write;
use std::path::Path;

use jtac_checks::diff::git::changes;
use jtac_checks::diff::{Adoption, report};

use crate::Printed;
use crate::cli::BacktestArgs;
use crate::range;
use crate::refuse::Refusal;
use crate::repo::{self, RepoError};

use row::Row;
use summary::Under;

/// Judge every unit of the range and print the summary.
pub fn backtest(a: &BacktestArgs, cwd: &Path) -> Result<Printed, Refusal> {
    let git = |e: RepoError| Refusal::backtest_repository(&e, &a.range);
    let repo = repo::root(cwd).map_err(git)?;
    if let Some(path) = repo::grafted(&repo).map_err(git)? {
        return Err(Refusal::grafted(&path));
    }
    if let Some(path) = &a.rows
        && path.symlink_metadata().is_ok()
    {
        return Err(Refusal::backtest_rows_exist(path, &a.range));
    }
    let format = repo::format(&repo).map_err(git)?;
    let line = range::first_parent_line(&repo, &format, "backtest", &a.range)?;
    let chosen = policy::choose(&repo, &format, a, line.first())?;
    let (policy, matchers) = (&chosen.policy, &chosen.matchers);
    let block_at = policy.integrity.block_at;
    let mut rows = Vec::new();
    let mut skipped = 0;
    for unit in &line {
        let Some(parent) = unit.parents.first() else {
            if range::has_parent(&repo, &unit.commit).map_err(git)? {
                return Err(Refusal::backtest_shallow(&unit.commit, &a.range));
            }
            skipped += 1;
            continue;
        };
        let changed = changes(&repo, &parent.hex(), &unit.commit.hex())
            .map_err(|e| Refusal::backtest_diff(&unit.commit, parent, &e, &a.range))?;
        let said = report(&policy.integrity, matchers, &changed, Adoption::None);
        rows.push(Row::of(unit, changed.len(), said, block_at));
    }
    let under = Under {
        block_at,
        policy: chosen.hash,
        from: chosen.from,
    };
    let mut summary = summary::summarize(&rows, skipped, under);
    if let Some(path) = &a.rows {
        write_rows(path, &rows).map_err(|e| Refusal::backtest_rows_write(path, &e, &a.range))?;
        summary.rows_written = Some(rows.len() as u64);
    }
    let text = if a.json {
        crate::render::json(&summary)?
    } else {
        render::text(&summary)
    };
    Ok(Printed::verdict(text, true))
}

/// One JSON line per row, into a file created here: an existing one is never replaced.
fn write_rows(path: &Path, rows: &[Row]) -> std::io::Result<()> {
    let mut text = String::new();
    for row in rows {
        text.push_str(&serde_json::to_string(row).map_err(std::io::Error::other)?);
        text.push('\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_rows_creates_its_file_and_never_writes_over_or_through_one() {
        let dir = jtac_testkit::Repo::empty();
        let path = dir.path().join("rows.jsonl");
        write_rows(&path, &[]).unwrap();
        std::fs::write(&path, "an earlier run\n").unwrap();
        let err = write_rows(&path, &[]).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "an earlier run\n");
        #[cfg(unix)]
        {
            let target = dir.path().join("elsewhere");
            let link = dir.path().join("link.jsonl");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let err = write_rows(&link, &[]).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
            assert!(!target.exists(), "a dangling link was written through");
        }
    }
}
