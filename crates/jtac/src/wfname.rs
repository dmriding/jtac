//! A GitHub workflow's name, read from its file for `on.workflow_run.workflows`, whose
//! entries are glob patterns over names. Pure and not a YAML parser: one column-0 `name:`
//! line in a spelling that needs no escape as YAML or as a pattern is a name, and every
//! other file is unreadable, so a name init cannot be sure of is never written
//! (`only_a_plainly_spelled_name_is_read_and_every_other_file_is_unreadable`).

/// Why a workflow file gave no name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// No column-0 `name:` line: GitHub then shows the path, which no page says the trigger
    /// matches.
    None,
    /// More than one.
    Several,
    /// The value as written, outside the spelling, escaped to printable ASCII.
    Spelling(String),
}

/// The punctuation a name may hold beside letters, digits and spaces: none is special in
/// GitHub's filter patterns or inside a double-quoted YAML scalar.
pub const PUNCTUATION: &str = "-_./()&,:";

/// YAML reads these bare words as something other than a string.
const NOT_STRINGS: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];

fn plain(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(' ')
        && !name.ends_with(' ')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || PUNCTUATION.contains(c))
}

/// The value of a `name:` line: quoted whole, or bare from a letter to the comment.
fn value(rest: &str) -> Option<&str> {
    let rest = rest.trim();
    if let Some(quote) = rest.chars().next().filter(|c| "'\"".contains(*c)) {
        let (name, tail) = rest[1..].split_once(quote)?;
        let comment = tail.trim_start();
        let ends = comment.is_empty() || (comment.starts_with('#') && comment.len() < tail.len());
        return (ends && plain(name)).then_some(name);
    }
    let name = rest.split(" #").next()?.trim_end();
    let bare = name.starts_with(|c: char| c.is_ascii_alphabetic())
        && !name.contains(": ")
        && !NOT_STRINGS.iter().any(|w| w.eq_ignore_ascii_case(name));
    (bare && plain(name)).then_some(name)
}

/// The workflow's name from its file's text.
pub fn name(text: &str) -> Result<String, Unreadable> {
    let mut lines = text.lines().filter_map(|l| {
        let rest = l.strip_prefix("name:")?;
        (rest.is_empty() || rest.starts_with(' ')).then_some(rest)
    });
    let rest = lines.next().ok_or(Unreadable::None)?;
    if lines.next().is_some() {
        return Err(Unreadable::Several);
    }
    value(rest)
        .map(str::to_owned)
        .ok_or_else(|| Unreadable::Spelling(jtac_policy::shown(rest.trim())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plainly_spelled_name_is_read_and_every_other_file_is_unreadable() {
        for (text, want) in [
            ("name: ci\non: push\n", "ci"),
            ("# header\nname: ci\r\non: push\r\n", "ci"),
            (
                "on: push\nname:   Build & Test (linux)  \n",
                "Build & Test (linux)",
            ),
            ("name: ci # the tests\n", "ci"),
            ("name: \"CI: build\"\n", "CI: build"),
            ("name: 'Run Tests'   # quoted\n", "Run Tests"),
            ("name: \"123\"\n", "123"),
            ("name: release/v2.build_all\n", "release/v2.build_all"),
            ("name: ci\njobs:\n  a:\n    name: leg\n", "ci"),
        ] {
            assert_eq!(name(text).as_deref(), Ok(want), "{text:?}");
        }
        for text in [
            "on: push\njobs:\n  a:\n    name: leg\n",
            "",
            "\"name\": ci\n",
            "name : ci\n",
            "name:ci\n",
            "{name: ci, on: push}\n",
            " name: ci\n",
            "\u{feff}name: ci\n",
        ] {
            assert_eq!(name(text), Err(Unreadable::None), "{text:?}");
        }
        assert_eq!(name("name: a\nname: b\n"), Err(Unreadable::Several));
        for value in [
            "",
            "Build*",
            "Build C++",
            "what?",
            "!ci",
            "[ci]",
            "'it''s'",
            "\"ci\" extra",
            "\"ci",
            "'ci\"",
            "\"\"",
            "\" ci\"",
            "|",
            ">-",
            "&anchor ci",
            "*alias",
            "!!str ci",
            "true",
            "No",
            "123",
            "2 fast",
            "ci: build",
            "a \"b\"",
        ] {
            assert_eq!(
                name(&format!("name: {value}\n")),
                Err(Unreadable::Spelling(value.to_owned())),
                "{value:?}"
            );
        }
        for (value, shown) in [
            ("ci\ttests", "ci\\u{0009}tests"),
            ("caf\u{e9}", "caf\\u{00E9}"),
            ("a\\b", "a\\u{005C}b"),
            ("\"a\\\"b\"", "\"a\\u{005C}\"b\""),
            ("ci \u{1b}[2J", "ci \\u{001B}[2J"),
        ] {
            assert_eq!(
                name(&format!("name: {value}\n")),
                Err(Unreadable::Spelling(shown.to_owned())),
                "{value:?}"
            );
        }
    }
}
