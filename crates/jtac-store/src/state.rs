//! `state.json`: the last verdict, written by `jtac check` and read by `status` and `why`
//! (DESIGN.md section 9.6). The shape is the binary's; the store keeps the bytes.

use crate::{Store, StoreError, fs};

const FILE: &str = "state.json";

impl Store {
    /// Replace the last verdict.
    pub fn put_state(&self, bytes: &[u8]) -> Result<(), StoreError> {
        fs::write(&self.dir().join(FILE), bytes)
    }

    /// The last verdict, or `None` when no check has written one.
    pub fn state(&self) -> Result<Option<Vec<u8>>, StoreError> {
        fs::read(&self.dir().join(FILE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jtac_testkit::Repo;

    #[test]
    fn state_is_absent_until_written_and_the_last_write_wins() {
        let repo = Repo::init(&[]);
        let store = Store::open(repo.path()).unwrap();
        assert_eq!(store.state().unwrap(), None);
        store.put_state(b"one").unwrap();
        store.put_state(b"two").unwrap();
        assert_eq!(store.state().unwrap().as_deref(), Some(&b"two"[..]));
    }
}
