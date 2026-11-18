//! A GitHub workflow's name, read from its file for `on.workflow_run.workflows`, whose
//! entries are glob patterns over names. Pure and not a YAML parser, so it accepts one
//! exact form and calls every other file unreadable: a name init is not sure of is never
//! written, since a wrong one is a trigger that never fires
//! (`only_a_plainly_spelled_name_is_read_and_every_other_file_is_unreadable`). The form:
//! every line break is a line feed, alone or after a carriage return, so these lines are
//! any YAML reader's; the first line that is not spaces or a comment is `name:` with its
//! value whole on it, or the document marker `---` with that `name:` line next; the next such line starts at the margin; every other margin line
//! is a plain `key:` that is not `name`, so no line names the workflow again or starts a
//! document. Whitespace is the space character alone.

/// Why a workflow file gave no name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// A bare carriage return, U+0085, U+2028 or U+2029: a line break to some YAML reader.
    Breaks,
    /// The first line that is not blank or a comment is not a `name:` line. With no name
    /// GitHub shows the path, which no page says the trigger matches.
    NotFirst,
    /// The line after the `name:` line is indented, so the value may run on.
    Continued,
    /// Another margin line is not a plain `key:`, or its key is `name`.
    Again,
    /// The value as written, outside the spelling, escaped to printable ASCII.
    Spelling(String),
}

/// The punctuation a name may hold beside letters, digits and spaces: none is special in
/// GitHub's filter patterns or inside a double-quoted YAML scalar.
pub const PUNCTUATION: &str = "-_./()&,:";

/// YAML reads these bare words as something other than a string.
const NOT_STRINGS: &[&str] = &["true", "false", "null", "yes", "no", "on", "off", "y", "n"];

fn plain(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(' ')
        && !name.ends_with(' ')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || PUNCTUATION.contains(c))
}

/// The value of a `name:` line: quoted whole, or bare from a letter to the comment with no
/// colon in it.
fn value(rest: &str) -> Option<&str> {
    let rest = rest.trim_matches(' ');
    if let Some(quote) = rest.chars().next().filter(|c| "'\"".contains(*c)) {
        let (name, tail) = rest[1..].split_once(quote)?;
        let comment = tail.trim_start_matches(' ');
        let ends = comment.is_empty() || (comment.starts_with('#') && comment.len() < tail.len());
        return (ends && plain(name)).then_some(name);
    }
    let name = rest.split(" #").next()?.trim_end_matches(' ');
    let bare = name.starts_with(|c: char| c.is_ascii_alphabetic())
        && !name.contains(':')
        && !NOT_STRINGS.iter().any(|w| w.eq_ignore_ascii_case(name));
    (bare && plain(name)).then_some(name)
}

/// Blank or a comment, led by spaces alone.
fn skipped(line: &str) -> bool {
    let rest = line.trim_start_matches(' ');
    rest.is_empty() || rest.starts_with('#')
}

/// The document marker alone on its line, or with a comment after a space.
fn marker(line: &str) -> bool {
    line.strip_prefix("---").is_some_and(|rest| {
        let comment = rest.trim_start_matches(' ');
        comment.is_empty() || (comment.starts_with('#') && comment.len() < rest.len())
    })
}

/// A margin line that is a top-level key and not `name`: one bare or quoted word, a
/// colon, then a space or the line's end. A tag, an anchor, `?`, a merge key, an escape, a
/// document marker and a flow collection all fall outside it.
fn other_key(line: &str) -> bool {
    let split = match line.chars().next() {
        Some(quote @ ('"' | '\'')) => line[1..].split_once(quote),
        _ => line.find(':').map(|at| line.split_at(at)),
    };
    split.is_some_and(|(key, rest)| {
        let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        !key.is_empty()
            && key.chars().all(word)
            && key != "name"
            && (rest == ":" || rest.starts_with(": "))
    })
}

/// The workflow's name from its file's text.
pub fn name(text: &str) -> Result<String, Unreadable> {
    let breaks = ['\r', '\u{85}', '\u{2028}', '\u{2029}'];
    if text.replace("\r\n", "\n").contains(breaks) {
        return Err(Unreadable::Breaks);
    }
    let mut content = text.lines().enumerate().filter(|(_, l)| !skipped(l));
    let lead = content.next().ok_or(Unreadable::NotFirst)?;
    let (opened, (at, first)) = match marker(lead.1) {
        true => (Some(lead.0), content.next().ok_or(Unreadable::NotFirst)?),
        false => (None, lead),
    };
    let rest = first
        .strip_prefix("name:")
        .filter(|rest| rest.is_empty() || rest.starts_with(' '))
        .ok_or(Unreadable::NotFirst)?;
    let shown = || jtac_policy::shown(rest.trim_matches(' '));
    let name = value(rest).ok_or_else(|| Unreadable::Spelling(shown()))?;
    if content
        .next()
        .is_some_and(|(_, l)| l.starts_with([' ', '\t']))
    {
        return Err(Unreadable::Continued);
    }
    let margin = |l: &str| !skipped(l) && !l.starts_with(' ');
    let named = |i: usize| i == at || Some(i) == opened;
    let mut others = text.lines().enumerate().filter(|(i, _)| !named(*i));
    if others.any(|(_, l)| margin(l) && !other_key(l)) {
        return Err(Unreadable::Again);
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values outside the spelling that echo as written.
    const SPELLING: &[&str] = &[
        "",
        "Build*",
        "Build C++",
        "what?",
        "!ci",
        "[ci]",
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
        "False",
        "null",
        "YES",
        "No",
        "on",
        "Off",
        "y",
        "N",
        "123",
        "2 fast",
        "ci: build",
        "ci:",
        "a:b",
        "a \"b\"",
        "ci#x",
    ];

    /// Values outside the spelling, and their echo in printable ASCII.
    const ESCAPED: &[(&str, &str)] = &[
        ("ci\ttests", "ci\\u{0009}tests"),
        ("ci\t", "ci\\u{0009}"),
        ("caf\u{e9}", "caf\\u{00E9}"),
        ("ci\u{a0}", "ci\\u{00A0}"),
        ("\u{a0}ci", "\\u{00A0}ci"),
        ("ci\u{a0} # c", "ci\\u{00A0} # c"),
        ("\u{a0}\"ci\"", "\\u{00A0}\"ci\""),
        ("\"ci\"\u{a0}", "\"ci\"\\u{00A0}"),
        ("ci\u{2003}", "ci\\u{2003}"),
        ("a\\b", "a\\u{005C}b"),
        ("\"a\\\"b\"", "\"a\\u{005C}\"b\""),
        ("ci \u{1b}[2J", "ci \\u{001B}[2J"),
    ];

    #[test]
    fn only_a_plainly_spelled_name_is_read_and_every_other_file_is_unreadable() {
        for (text, want) in [
            ("name: ci\non: push\n", "ci"),
            (
                "# header\n\n  # indented comment\nname: ci\r\non: push\r\n",
                "ci",
            ),
            (
                "name:   Build & Test (linux)  \n\non: push\n",
                "Build & Test (linux)",
            ),
            ("name: ci # the tests\n", "ci"),
            ("name: \"CI: build\"\n", "CI: build"),
            ("name: 'Run Tests'   # quoted\n", "Run Tests"),
            ("name: \"123\"\n", "123"),
            ("name: \"on\"\n", "on"),
            ("name: release/v2.build_all", "release/v2.build_all"),
            ("name: ci\n  # an indented comment\non: push\n", "ci"),
            (
                "name: ci\njobs:\n  a:\n    name: leg\nnames: x\n\"on\": push\n'run-name': x\nx_1:\n",
                "ci",
            ),
            (
                "name: ci\n# caf\u{e9} \u{a0} in a comment\non: push\n",
                "ci",
            ),
            ("---\nname: ci\non: push\n", "ci"),
            ("# header\n\n---   \r\n\n# c\nname: \"ci\"\r\n", "ci"),
            ("--- # yamllint asks for it\nname: ci\n", "ci"),
        ] {
            assert_eq!(name(text).as_deref(), Ok(want), "{text:?}");
        }
        for text in [
            "on: push\nname: ci\n",
            "on: push\njobs:\n  a:\n    name: leg\n",
            "",
            "# only a comment\n",
            "\"name\": ci\n",
            "name : ci\n",
            "name:ci\n",
            "{name: ci, on: push}\n",
            " name: ci\n",
            "\u{feff}name: ci\n",
            "---\n",
            "---\n---\nname: ci\n",
            "---#c\nname: ci\n",
            "----\nname: ci\n",
            "--- name: ci\n",
            "---\t# c\nname: ci\n",
            "--- |\nname: ci\n",
            " ---\nname: ci\n",
            "%YAML 1.2\n---\nname: ci\n",
            "...\n---\nname: ci\n",
            "---\n name: ci\n",
            "---\non: push\nname: ci\n",
            "run-name: \"Deploy\nname: ci\"\n",
            "env: {\nname: ci\n}\n",
            "\u{a0}\nname: ci\n",
            "\t# a tab is no comment's lead\nname: ci\n",
            "\u{2003}# nor is an em space\nname: ci\n",
        ] {
            assert_eq!(name(text), Err(Unreadable::NotFirst), "{text:?}");
        }
        for text in [
            "name: Build and test\n  the workspace\non: push\n",
            "name: ci\n\n  tests\n",
            "name: ci\n\ttests\n",
            "name: ci\n# a comment at the margin\n  indented: after it\n",
            "name: \"ci\"\n  on: push\n",
        ] {
            assert_eq!(name(text), Err(Unreadable::Continued), "{text:?}");
        }
        for text in [
            "#\rrun-name: \"x\nname: ci\n\"\non: push\n",
            "# c\renv: {\nname: ci\n}\n",
            "#\u{2028}run-name: \"x\nname: ci\n\"\n",
            "#\u{85}run-name: \"x\nname: ci\n\"\n",
            "name: ci\non: push\n# late\u{2029}name: other\n",
            "name: ci\ron: push\n",
            "name: ci\n\r\r\non: push\n",
        ] {
            assert_eq!(name(text), Err(Unreadable::Breaks), "{text:?}");
        }
        for rival in [
            "name: again",
            "name:",
            "name : again",
            "name\t: again",
            "\"name\": again",
            "'name' : again",
            "? name",
            "\"na\\x6de\": again",
            "\"\\u006eame\": again",
            "<<: {name: again}",
            "! name: again",
            "!    name: again",
            "!!str name: again",
            "&a name: again",
            "\tname: again",
            "}",
            "- x",
            "on:push",
            "\"on\" : push",
            "---",
            "--- # second document",
            "...",
        ] {
            let text = format!("name: ci\non: push\n{rival}\n");
            assert_eq!(name(&text), Err(Unreadable::Again), "{text:?}");
        }
        for value in SPELLING {
            assert_eq!(
                name(&format!("name: {value}\n")),
                Err(Unreadable::Spelling((*value).to_owned())),
                "{value:?}"
            );
        }
        for (value, shown) in ESCAPED {
            assert_eq!(
                name(&format!("name: {value}\n")),
                Err(Unreadable::Spelling((*shown).to_owned())),
                "{value:?}"
            );
        }
    }
}
