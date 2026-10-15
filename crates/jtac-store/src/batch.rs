//! Every root in the clone through one `cat-file --batch`. Reading a ref costs three git
//! children; an audit over a long history reads them all, so the namespace is read in one.

use jtac_core::{Root, from_canonical};

use crate::refs::Listed;
use crate::roots::StoredRoot;
use crate::{Store, StoreError};

/// One ref as read: the root, or why its content is not a root for its commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The ref.
    pub listed: Listed,
    /// The root, or what was wrong.
    pub stored: Result<StoredRoot, String>,
}

impl Store {
    /// Read every listed ref. A ref whose content is not a root is an `Err` in its slot,
    /// never a refusal of the rest.
    pub fn read_all(&self, listed: &[Listed]) -> Result<Vec<Read>, StoreError> {
        let mut stdin = String::new();
        for l in listed {
            let oid = l.root_commit.hex();
            stdin.push_str(&format!("{oid}\n{oid}:root.json\n{oid}:root.sig\n"));
        }
        if listed.is_empty() {
            return Ok(Vec::new());
        }
        let out = jtac_checks::git::git_with(
            self.repo(),
            &["cat-file", "--batch"],
            Some(stdin.as_bytes()),
            &[],
        )?;
        let records = records(&out)?;
        if records.len() != listed.len() * 3 {
            return Err(StoreError::Output {
                what: format!(
                    "cat-file --batch answered {} records for {} requests",
                    records.len(),
                    listed.len() * 3
                ),
            });
        }
        Ok(listed
            .iter()
            .zip(records.chunks(3))
            .map(|(l, chunk)| Read {
                listed: l.clone(),
                stored: stored(l, chunk),
            })
            .collect())
    }
}

/// A root from its three records: the ref's object, `root.json`, `root.sig`.
fn stored(l: &Listed, chunk: &[Record]) -> Result<StoredRoot, String> {
    match &chunk[0] {
        Record::Object { kind, .. } if kind == "commit" => {}
        Record::Object { kind, .. } => {
            return Err(format!("the ref points at a {kind}, not a root commit"));
        }
        Record::Missing => return Err("the ref points at nothing".to_owned()),
    }
    let blob = |r: &Record, name: &str| match r {
        Record::Object { kind, bytes } if kind == "blob" => Ok(bytes.clone()),
        Record::Object { kind, .. } => Err(format!("{name} is a {kind}, not a file")),
        Record::Missing => Err(format!("the root commit has no {name}")),
    };
    let json = blob(&chunk[1], "root.json")?;
    let root: Root = from_canonical(&json).map_err(|e| e.to_string())?;
    root.validate().map_err(|e| e.to_string())?;
    if root.commit != l.commit {
        return Err(format!("the root is for {}", root.commit));
    }
    let sig = blob(&chunk[2], "root.sig")?;
    Ok(StoredRoot { root, sig })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Record {
    Object { kind: String, bytes: Vec<u8> },
    Missing,
}

/// `<oid> <type> <size>\n<bytes>\n` per object, `<spec> missing\n` (or `ambiguous`) for
/// one git could not resolve.
fn records(out: &[u8]) -> Result<Vec<Record>, StoreError> {
    let bad = |what: &str| StoreError::Output {
        what: format!("cat-file --batch: {what}"),
    };
    let mut records = Vec::new();
    let mut rest = out;
    while !rest.is_empty() {
        let nl = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| bad("a header without a newline"))?;
        let header = String::from_utf8_lossy(&rest[..nl]).into_owned();
        rest = &rest[nl + 1..];
        let fields: Vec<&str> = header.split(' ').collect();
        match fields[..] {
            [_, "missing"] | [_, "ambiguous"] => records.push(Record::Missing),
            [_, kind, size] => {
                let size: usize = size
                    .parse()
                    .map_err(|_| bad("a size that is not a number"))?;
                if rest.len() < size + 1 || rest[size] != b'\n' {
                    return Err(bad("an object shorter than its header says"));
                }
                records.push(Record::Object {
                    kind: kind.to_owned(),
                    bytes: rest[..size].to_vec(),
                });
                rest = &rest[size + 1..];
            }
            _ => return Err(bad(&format!("header `{header}`"))),
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_records_are_split_by_header_size_and_a_missing_spec_is_a_slot() {
        let out = b"aaaa commit 5\nhello\nx:root.json missing\nbbbb blob 0\n\n";
        let got = records(out).unwrap();
        assert_eq!(
            got,
            [
                Record::Object {
                    kind: "commit".into(),
                    bytes: b"hello".to_vec()
                },
                Record::Missing,
                Record::Object {
                    kind: "blob".into(),
                    bytes: Vec::new()
                },
            ]
        );
        assert!(records(b"aaaa blob 9\nshort\n").is_err());
        assert!(records(b"aaaa blob x\n").is_err());
        assert!(records(b"no newline").is_err());
    }
}
