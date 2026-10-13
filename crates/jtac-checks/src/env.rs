//! The environment a check sees: an allowlist of names, nothing else (DESIGN.md section 10.2).
//! A value is never inspected; a token with no recognizable shape is still a token. Names that
//! change what a tool does (`GOFLAGS`, `PYTHONPATH`) stay out so a check cannot become a
//! cacheable no-op; config a tool reads under `HOME` or `CARGO_HOME` is DESIGN.md section 6's
//! compromised toolchain, out of scope.

const NAMES: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "TERM",
    "TZ",
    "TMPDIR",
    "CI",
    "NO_COLOR",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "GOPATH",
    "GOCACHE",
    "GOMODCACHE",
    "VIRTUAL_ENV",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PATHEXT",
    "HOMEDRIVE",
    "HOMEPATH",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "WINDIR",
    "COMSPEC",
];

/// Whether a variable named `name` reaches a check. Windows names are case-insensitive, so the
/// rule is too; `allowed_names_are_a_closed_list` holds the rows.
pub fn allowed(name: &str) -> bool {
    NAMES.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// `vars` with every variable `allowed` refuses dropped.
pub fn scrub<I>(vars: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    vars.into_iter().filter(|(k, _)| allowed(k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_names_are_a_closed_list() {
        let rows = [
            ("GITHUB_TOKEN", false),
            ("AWS_SECRET_ACCESS_KEY", false),
            ("DATABASE_URL", false),
            ("CARGO_REGISTRY_TOKEN", false),
            ("CARGO_MANIFEST_DIR", false),
            ("GOFLAGS", false),
            ("PYTHONPATH", false),
            ("RUSTFLAGS", false),
            ("SSH_AUTH_SOCK", false),
            ("LC", false),
            ("LC_", false),
            ("LC_SECRET", false),
            ("XLC_ALL", false),
            ("PATHX", false),
            ("", false),
            ("PATH", true),
            ("Path", true),
            ("HOME", true),
            ("TMPDIR", true),
            ("LC_ALL", true),
            ("lc_ctype", true),
            ("SystemRoot", true),
            ("PATHEXT", true),
            ("ProgramFiles(x86)", true),
            ("CARGO_HOME", true),
        ];
        for (name, want) in rows {
            assert_eq!(allowed(name), want, "{name:?}");
        }
    }

    #[test]
    fn scrub_keeps_only_allowed_pairs() {
        let vars = [("PATH", "/bin"), ("GITHUB_TOKEN", "x"), ("LC_ALL", "C")]
            .map(|(k, v)| (k.to_string(), v.to_string()));
        assert_eq!(
            scrub(vars),
            vec![
                ("PATH".to_string(), "/bin".to_string()),
                ("LC_ALL".to_string(), "C".to_string())
            ]
        );
    }
}
