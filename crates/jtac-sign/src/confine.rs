//! The allowed-signers lines a CI root may be trusted under (DESIGN.md section 15, Identity):
//! those whose `namespaces` option names the namespace in so many words. ssh-keygen reads a
//! line with no option as every namespace, so jtac reads the option itself before ssh-keygen
//! sees the file; a developer's line, which names none, is not in what it sees. A line is
//! read as ssh-keygen reads it, so no spelling means one thing here and another there.

/// `trust` reduced to the lines whose `namespaces` option names `ns`, bytes unchanged.
pub fn confine(trust: &[u8], ns: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in trust.split(|b| *b == b'\n') {
        if names(&String::from_utf8_lossy(line), ns) {
            out.extend_from_slice(line);
            out.push(b'\n');
        }
    }
    out
}

/// ssh-keygen's field separators; no other whitespace counts.
const SPACE: [char; 4] = [' ', '\t', '\r', '\n'];

/// One line is `principals [options] key-type key [comment]`. The principals field ends at
/// the first separator or at a double quote, which runs to its match and ends the field; the
/// options token ends at a space or tab outside quotes, `\"` the one escape; a `namespaces`
/// value is double-quoted and comma separated, given once. A line ssh-keygen would reject
/// is not kept; the rows ssh-keygen and this must agree on are in
/// `only_a_line_whose_namespaces_option_names_the_namespace_survives`, which runs them
/// through ssh-keygen too.
fn names(line: &str, ns: &str) -> bool {
    let line = line.trim_start_matches(SPACE);
    if line.is_empty() || line.starts_with('#') {
        return false;
    }
    let Some(options) = past_principals(line).and_then(options_token) else {
        return false;
    };
    if is_key_type(options) {
        return false;
    }
    let mut found = None;
    for opt in split_outside_quotes(options, ',') {
        let Some((name, value)) = opt.split_once('=') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("namespaces") {
            continue;
        }
        if found.is_some() {
            return false;
        }
        found = Some(value);
    }
    let Some((list, _)) = found.and_then(|v| v.strip_prefix('"')?.split_once('"')) else {
        return false;
    };
    list.split(',').any(|named| named == ns)
}

/// What follows the principals field; `None` for a line with nothing after it or an unmatched
/// quote, which ssh-keygen rejects.
fn past_principals(line: &str) -> Option<&str> {
    let end = line.find(|c| SPACE.contains(&c) || c == '"')?;
    let rest = match line[end..].strip_prefix('"') {
        Some(quoted) => &quoted[quoted.find('"')? + 1..],
        None => &line[end..],
    };
    Some(rest.trim_start_matches(SPACE))
}

/// The options token, `None` when a quote is left open.
fn options_token(rest: &str) -> Option<&str> {
    let mut quoted = false;
    let mut skip = false;
    for (i, c) in rest.char_indices() {
        match c {
            _ if skip => skip = false,
            '\\' if rest[i + 1..].starts_with('"') => skip = true,
            '"' => quoted = !quoted,
            ' ' | '\t' if !quoted => return Some(&rest[..i]),
            _ => {}
        }
    }
    (!quoted).then_some(rest)
}

fn split_outside_quotes(s: &str, sep: char) -> impl Iterator<Item = &str> {
    let mut quoted = false;
    s.split(move |c: char| {
        if c == '"' {
            quoted = !quoted;
        }
        c == sep && !quoted
    })
}

fn is_key_type(token: &str) -> bool {
    ["ssh-", "ecdsa-sha2-", "sk-"]
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample comment";

    #[test]
    fn confine_keeps_the_matching_lines_byte_for_byte_and_drops_the_rest() {
        let trust = format!(
            "# comment\ndev@example.com {KEY}\n\nci@example.com namespaces=\"jtac-ci-root-v1\" {KEY}\r\n"
        );
        let got = confine(trust.as_bytes(), "jtac-ci-root-v1");
        assert_eq!(
            String::from_utf8(got).unwrap(),
            format!("ci@example.com namespaces=\"jtac-ci-root-v1\" {KEY}\r\n")
        );
        assert!(confine(b"dev@example.com ssh-ed25519 AAAA\n", "jtac-ci-root-v1").is_empty());
    }

    #[test]
    fn a_quoted_value_keeps_its_spaces_and_commas_in_one_option_token() {
        assert_eq!(options_token("a=\"b c\",d=e rest"), Some("a=\"b c\",d=e"));
        assert_eq!(options_token("a=\"b\\\" c\" rest"), Some("a=\"b\\\" c\""));
        assert_eq!(options_token("plain"), Some("plain"));
        assert_eq!(options_token("a=\"open"), None);
        assert_eq!(past_principals("a,\"b c\"  rest"), Some("rest"));
        assert_eq!(past_principals("a\u{0b}b rest"), Some("rest"));
        assert_eq!(past_principals("alone"), None);
        let parts: Vec<&str> = split_outside_quotes("a=\"x,y\",b=z", ',').collect();
        assert_eq!(parts, ["a=\"x,y\"", "b=z"]);
    }
}
