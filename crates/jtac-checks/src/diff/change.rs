//! One changed file as the rules see it, and the line edit between its versions. The diff is
//! ours (Myers over lines), not git's, so no `diff.algorithm` setting can move a finding.

use similar::{Algorithm, DiffOp, capture_diff_slices};

/// Leading bytes inspected for a NUL, git's rule for a binary blob.
const BINARY_PROBE: usize = 8000;

/// A file's content on one side of the change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blob {
    /// Lossy UTF-8.
    Text(String),
    /// No rule reads it; the limit is named in DESIGN.md section 10.3.
    Binary,
}

impl Blob {
    /// `bytes` as the rules see it.
    pub fn of(bytes: &[u8]) -> Self {
        if bytes.iter().take(BINARY_PROBE).any(|&b| b == 0) {
            Self::Binary
        } else {
            Self::Text(String::from_utf8_lossy(bytes).into_owned())
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Binary => None,
        }
    }
}

/// One path the change touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Repository-relative, `/` between directories.
    pub path: String,
    /// The path before a rename.
    pub from: Option<String>,
    /// Content before; absent when the change added the file.
    pub old: Option<Blob>,
    /// Content after; absent when the change deleted the file.
    pub new: Option<Blob>,
}

/// The lines a change added and removed, numbered from 1 in their own version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit<'a> {
    /// Lines only the new version has.
    pub added: Vec<(u64, &'a str)>,
    /// Lines only the old version had.
    pub removed: Vec<(u64, &'a str)>,
}

/// Lines split at `\r\n`, `\n`, or a lone `\r`, which JavaScript and Python also end a line at;
/// `str::lines` would hide code after a lone `\r` inside a comment.
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b'\n' | b'\r') {
            lines.push(&text[start..i]);
            if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

impl Change {
    /// The old text; `None` when absent or binary.
    pub fn old_text(&self) -> Option<&str> {
        self.old.as_ref().and_then(Blob::text)
    }

    /// The new text; `None` when absent or binary.
    pub fn new_text(&self) -> Option<&str> {
        self.new.as_ref().and_then(Blob::text)
    }

    /// The line edit. A side that is absent or binary has no lines.
    pub fn edit(&self) -> Edit<'_> {
        let old = self.old_text().map(split_lines).unwrap_or_default();
        let new = self.new_text().map(split_lines).unwrap_or_default();
        let mut edit = Edit::default();
        let number = |i: usize| u64::try_from(i + 1).unwrap_or(u64::MAX);
        for op in capture_diff_slices(Algorithm::Myers, &old, &new) {
            let (old_range, new_range) = match op {
                DiffOp::Equal { .. } => continue,
                DiffOp::Delete {
                    old_index, old_len, ..
                } => (old_index..old_index + old_len, 0..0),
                DiffOp::Insert {
                    new_index, new_len, ..
                } => (0..0, new_index..new_index + new_len),
                DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => (
                    old_index..old_index + old_len,
                    new_index..new_index + new_len,
                ),
            };
            edit.removed.extend(old_range.map(|i| (number(i), old[i])));
            edit.added.extend(new_range.map(|i| (number(i), new[i])));
        }
        edit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(old: Option<&str>, new: Option<&str>) -> Change {
        Change {
            path: "a.txt".into(),
            from: None,
            old: old.map(|s| Blob::Text(s.into())),
            new: new.map(|s| Blob::Text(s.into())),
        }
    }

    #[test]
    fn edit_numbers_lines_in_their_own_version_and_normalizes_crlf() {
        let c = change(Some("a\nb\nc\n"), Some("a\r\nx\r\nc\r\nd"));
        let got = c.edit();
        assert_eq!(got.added, vec![(2, "x"), (4, "d")]);
        assert_eq!(got.removed, vec![(2, "b")]);
    }

    #[test]
    fn a_lone_cr_ends_a_line_too() {
        assert_eq!(split_lines("a\rb\r\nc\n\rd"), ["a", "b", "c", "", "d"]);
        assert_eq!(split_lines(""), [] as [&str; 0]);
        assert_eq!(split_lines("\n"), [""]);
        assert_eq!(split_lines("x"), ["x"]);
    }

    #[test]
    fn an_added_or_deleted_file_is_all_one_side() {
        assert_eq!(
            change(None, Some("a\nb")).edit(),
            Edit {
                added: vec![(1, "a"), (2, "b")],
                removed: vec![]
            }
        );
        assert_eq!(
            change(Some("a"), None).edit(),
            Edit {
                added: vec![],
                removed: vec![(1, "a")]
            }
        );
        assert_eq!(change(None, None).edit(), Edit::default());
    }

    #[test]
    fn a_binary_side_has_no_lines() {
        let probe = Blob::of(b"ab\0cd");
        assert_eq!(probe, Blob::Binary);
        assert_eq!(Blob::of(b"ab\xffcd"), Blob::Text("ab\u{fffd}cd".into()));
        let mut far = vec![b'a'; BINARY_PROBE];
        far.push(0);
        assert!(matches!(Blob::of(&far), Blob::Text(_)));
        let c = Change {
            path: "a.bin".into(),
            from: None,
            old: Some(Blob::Binary),
            new: Some(Blob::Text("x\n".into())),
        };
        assert_eq!(c.edit().added, vec![(1, "x")]);
        assert_eq!(c.edit().removed, vec![]);
    }
}
