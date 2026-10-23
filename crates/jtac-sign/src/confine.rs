//! The allowed-signers lines a CI root may be trusted under (DESIGN.md section 15, Identity):
//! those whose `namespaces` option names the namespace in so many words. ssh-keygen reads a
//! line with no option as every namespace, so jtac reads the option itself before ssh-keygen
//! sees the file; a developer's line, which names none, is not in what it sees.

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

/// One line is `principals [options] key-type key [comment]`; options are one token in which a
/// double-quoted value may hold spaces and commas, and a key type starts the key.
fn names(line: &str, ns: &str) -> bool {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return false;
    }
    let Some((_, rest)) = line.split_once(char::is_whitespace) else {
        return false;
    };
    let options = token(rest.trim_start());
    if is_key_type(options) {
        return false;
    }
    split_outside_quotes(options, ',')
        .filter_map(|opt| opt.split_once('='))
        .filter(|(name, _)| name.eq_ignore_ascii_case("namespaces"))
        .any(|(_, value)| {
            value
                .trim_matches('"')
                .split(',')
                .any(|named| named.trim() == ns)
        })
}

/// The first whitespace-delimited token of `s`, whitespace inside double quotes kept; a
/// backslash escapes the next character inside quotes.
fn token(s: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => return &s[..i],
            _ => {}
        }
    }
    s
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
    fn only_a_line_whose_namespaces_option_names_the_namespace_survives() {
        let rows: [(&str, bool); 12] = [
            (
                "ci@example.com namespaces=\"jtac-ci-root-v1,jtac-release-v1\" ",
                true,
            ),
            (
                "ci@example.com namespaces=\"jtac-release-v1,jtac-ci-root-v1\" ",
                true,
            ),
            ("ci@example.com namespaces=\"jtac-ci-root-v1\" ", true),
            ("ci@example.com namespaces=jtac-ci-root-v1 ", true),
            ("ci@example.com NAMESPACES=\"jtac-ci-root-v1\" ", true),
            (
                "ci@example.com valid-after=\"20260101\",namespaces=\"jtac-ci-root-v1\",valid-before=\"20270101\" ",
                true,
            ),
            (
                "ci@example.com cert-authority,namespaces=\"jtac-ci-root-v1\" ",
                true,
            ),
            ("dev@example.com ", false),
            (
                "dev@example.com namespaces=\"jtac-root-v1,jtac-lever-v1\" ",
                false,
            ),
            ("dev@example.com namespaces=\"jtac-ci-root-v1x\" ", false),
            ("dev@example.com valid-before=\"20270101\" ", false),
            ("# namespaces=\"jtac-ci-root-v1\" ", false),
        ];
        for (prefix, want) in rows {
            let line = format!("{prefix}{KEY}");
            assert_eq!(names(&line, "jtac-ci-root-v1"), want, "{line}");
        }
        assert!(!names("", "jtac-ci-root-v1"));
        assert!(!names("lonely", "jtac-ci-root-v1"));
    }

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
        assert_eq!(token("a=\"b c\",d=e rest"), "a=\"b c\",d=e");
        assert_eq!(token("a=\"b\\\" c\" rest"), "a=\"b\\\" c\"");
        assert_eq!(token("plain"), "plain");
        let parts: Vec<&str> = split_outside_quotes("a=\"x,y\",b=z", ',').collect();
        assert_eq!(parts, ["a=\"x,y\"", "b=z"]);
    }
}
