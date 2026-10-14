//! The `SHA256:` fingerprint read off ssh-keygen's own output (DESIGN.md section 11.8).

/// The last token of ssh-keygen's `Good ... signature ... with <type> key <fp>` line.
pub(crate) fn printed(stdout: &[u8]) -> Option<String> {
    let fields = fields(stdout);
    sha256(fields.last())
}

/// The second token of an `ssh-keygen -l` line: `<bits> <fp> <comment> (<type>)`.
pub(crate) fn listed(stdout: &[u8]) -> Option<String> {
    let fields = fields(stdout);
    sha256(fields.get(1))
}

fn fields(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn sha256(field: Option<&String>) -> Option<String> {
    field.filter(|t| t.starts_with("SHA256:")).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "SHA256:p8O8b2P+OjA2b4lu9ZzmfMUF1o2YsHXYiurmjNDAag8";

    #[test]
    fn the_fingerprint_is_read_from_where_each_command_prints_it() {
        let good = format!("Good \"jtac-root-v1\" signature with ED25519 key {FP}\r\n");
        assert_eq!(printed(good.as_bytes()).as_deref(), Some(FP));
        let list = format!("256 {FP} a b c (ED25519-CERT)\nmore\n");
        assert_eq!(listed(list.as_bytes()).as_deref(), Some(FP));
        for text in ["", "Good signature\n", "Good signature with key MD5:aa\n"] {
            assert_eq!(printed(text.as_bytes()), None, "{text:?}");
            assert_eq!(listed(text.as_bytes()), None, "{text:?}");
        }
        assert_eq!(listed(good.as_bytes()), None);
    }
}
