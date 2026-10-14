//! `state.json`: the last verdict as `jtac check` printed it, root id included, so `why` and
//! `status` render the same structure (DESIGN.md section 9.6). The store keeps the bytes.

use jtac_store::Store;

use crate::output::Output;
use crate::refuse::Refusal;

/// Replace the last verdict.
pub fn write(store: &Store, out: &Output) -> Result<(), Refusal> {
    let bytes = serde_json::to_vec(out).map_err(Refusal::internal)?;
    store.put_state(&bytes).map_err(Refusal::store)
}

/// The last verdict, `None` before the first check. A file this build cannot read is a
/// refusal that names the check to run.
pub fn read(store: &Store) -> Result<Option<Output>, Refusal> {
    let Some(bytes) = store.state().map_err(Refusal::store)? else {
        return Ok(None);
    };
    match serde_json::from_slice::<Output>(&bytes) {
        Ok(out) if out.v == 1 => Ok(Some(out)),
        _ => Err(Refusal::stale_state()),
    }
}
