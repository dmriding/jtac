//! One change as every rule sees it: language and test convention on each side, the line
//! edit, and both versions scanned once.

use super::change::{Change, Edit};
use super::lang::Lang;
use super::scan::{Line, scan};

/// A change with the facts the rules share.
pub(crate) struct View<'a> {
    /// The change itself.
    pub change: &'a Change,
    /// By the old path, so a rename away from a language still judges what left.
    pub old_lang: Option<Lang>,
    /// By the new path.
    pub new_lang: Option<Lang>,
    /// Whether the old path held tests by convention.
    pub old_test: bool,
    /// Whether the new path holds tests by convention.
    pub new_test: bool,
    /// Lines added and removed.
    pub edit: Edit<'a>,
    /// The old version; empty when absent, binary, or of no language.
    pub old: Vec<Line<'a>>,
    /// The new version; empty when absent, binary, or of no language.
    pub new: Vec<Line<'a>>,
}

impl<'a> View<'a> {
    pub(crate) fn of(change: &'a Change) -> Self {
        let from = change.from.as_deref().unwrap_or(&change.path);
        let (old_lang, new_lang) = (Lang::of(from), Lang::of(&change.path));
        let scanned = |lang: Option<Lang>, text: Option<&'a str>| {
            lang.zip(text)
                .map(|(lang, text)| scan(lang, text))
                .unwrap_or_default()
        };
        Self {
            change,
            old_lang,
            new_lang,
            old_test: old_lang.is_some_and(|l| l.is_test_path(from)),
            new_test: new_lang.is_some_and(|l| l.is_test_path(&change.path)),
            edit: change.edit(),
            old: scanned(old_lang, change.old_text()),
            new: scanned(new_lang, change.new_text()),
        }
    }

    /// The path a finding names.
    pub(crate) fn path(&self) -> &str {
        &self.change.path
    }

    /// Added lines as (number, as written, with strings blanked), so a marker quoted in a test
    /// table is not one; a file with no language table matches as written. `comments` keeps
    /// comment text in the blanked form.
    pub(crate) fn added(&self, comments: bool) -> Vec<(u64, &str, String)> {
        self.edit
            .added
            .iter()
            .map(|(n, text)| {
                let line = usize::try_from(*n).ok().and_then(|i| self.new.get(i - 1));
                let shown = match (line, comments) {
                    (Some(l), true) => l.visible(),
                    (Some(l), false) => l.code.clone(),
                    (None, _) => (*text).to_owned(),
                };
                (*n, *text, shown)
            })
            .collect()
    }
}
