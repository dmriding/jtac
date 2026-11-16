//! The READMEs crates.io renders: the root one for `jtac`, `crates/README.md` for the libraries.

use std::{fs, path::Path};

/// Every link target in `text`, inline or in an HTML attribute.
fn targets(text: &str) -> impl Iterator<Item = &str> {
    [("](", ')'), ("href=\"", '"'), ("src=\"", '"')]
        .into_iter()
        .flat_map(move |(open, close)| {
            text.split(open)
                .skip(1)
                .filter_map(move |rest| rest.split(close).next())
        })
}

/// crates.io resolves a relative link against the crate's directory, not the README's, and a
/// published page is never replaced: a path that works on the forge is a dead link there.
/// Inline only: a reference definition a paragraph absorbs leaves every label undefined, and
/// no scan short of a Markdown parser sees it.
#[test]
fn a_published_readme_links_inline_to_absolute_urls_and_its_own_anchors() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for readme in [crates.join("../README.md"), crates.join("README.md")] {
        let text = fs::read_to_string(&readme).unwrap();
        let refused: Vec<&str> = targets(&text)
            .filter(|target| !target.starts_with("https://") && !target.starts_with('#'))
            .chain(
                text.lines()
                    .filter(|line| line.starts_with('[') && line.contains("]: ")),
            )
            .collect();
        assert!(refused.is_empty(), "{}: {refused:?}", readme.display());
    }
}
