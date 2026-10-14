//! `jtac why` (DESIGN.md section 11.9): the last verdict as `check` printed it, with a note
//! when HEAD has moved since.

use std::path::Path;

use jtac_store::Store;

use crate::refuse::Refusal;
use crate::{Printed, render, repo, state};

/// Render the last verdict. Exit 0 when it passed, 1 when it blocked.
pub fn why(cwd: &Path, json: bool) -> Result<Printed, Refusal> {
    let repo = repo::root(cwd).map_err(|e| Refusal::repository(&e))?;
    let store = Store::open(&repo).map_err(Refusal::store)?;
    let mut out = state::read(&store)?.ok_or_else(Refusal::no_state)?;
    let head = repo::head(&repo).map_err(|e| Refusal::no_head(&e))?;
    if head.commit != out.commit {
        out.notes.push(format!(
            "HEAD is now {}; this verdict is for {}; run `jtac check`",
            head.commit.hex(),
            out.commit.hex()
        ));
    }
    Ok(Printed::verdict(render::render(&out, json)?, out.pass))
}
