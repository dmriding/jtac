//! Where `tool_version` comes from: a probe argv per known tool (DESIGN.md section 11.1).
//! No probe, or a probe that fails, gives an empty version, and an empty version is never
//! cached (`cache::store`).

use std::time::Duration;

/// How long a version probe may run.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest `tool_version` kept, in bytes.
pub const VERSION_CAP: usize = 256;

const PROBES: &[(&str, &[&str])] = &[
    ("cargo", &[SELF, "--version"]),
    ("rustc", &[SELF, "--version"]),
    ("go", &[SELF, "version"]),
    ("node", &[SELF, "--version"]),
    ("deno", &[SELF, "--version"]),
    ("bun", &[SELF, "--version"]),
    ("python", &[SELF, "--version"]),
    ("python3", &[SELF, "--version"]),
    ("ruff", &[SELF, "--version"]),
    ("npm", &["node", "--version"]),
    ("npx", &["node", "--version"]),
    ("pnpm", &["node", "--version"]),
    ("yarn", &["node", "--version"]),
    ("eslint", &["node", "--version"]),
    ("prettier", &["node", "--version"]),
    ("tsc", &["node", "--version"]),
];

const SELF: &str = "$0";

/// The argv that prints the version that decides what `argv` does, keyed by `argv[0]`'s file
/// name with a Windows launcher extension stripped, case-insensitively. A tool that runs on
/// Node reports Node: its own version is in the lockfile, the runtime's is not. Given by path,
/// it probes the `node` beside it, which is the one the npm launcher prefers. A rustup
/// `+toolchain` selector in `argv[1]` rides along. `None` for a tool the table does not know.
pub fn probe_argv(argv: &[String]) -> Option<Vec<String>> {
    let argv0 = argv.first()?;
    let dir_len = argv0.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let base = &argv0[dir_len..];
    let base = [".exe", ".cmd", ".bat"]
        .iter()
        .find_map(|ext| strip_suffix_ignore_case(base, ext))
        .unwrap_or(base);
    let (_, probe) = PROBES
        .iter()
        .find(|(tool, _)| tool.eq_ignore_ascii_case(base))?;
    let selector = argv
        .get(1)
        .filter(|a| probe[0] == SELF && a.starts_with('+'));
    let runtime = if probe[0] == SELF {
        argv0.clone()
    } else {
        format!("{}{}", &argv0[..dir_len], probe[0])
    };
    Some(
        std::iter::once(runtime)
            .chain(selector.cloned())
            .chain(probe[1..].iter().map(|a| (*a).to_string()))
            .collect(),
    )
}

fn strip_suffix_ignore_case<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = s.len().checked_sub(suffix.len())?;
    (s.is_char_boundary(cut) && s[cut..].eq_ignore_ascii_case(suffix)).then(|| &s[..cut])
}

/// The first line of a probe's stdout, trimmed and capped at `VERSION_CAP` on a char boundary.
pub fn first_line(stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().next().unwrap_or("").trim();
    let mut end = line.len().min(VERSION_CAP);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_argv_keys_on_file_name_and_keeps_a_toolchain_selector() {
        let v = |xs: &[&str]| xs.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let rows = [
            (v(&["cargo", "test"]), Some(v(&["cargo", "--version"]))),
            (
                v(&["/usr/bin/cargo"]),
                Some(v(&["/usr/bin/cargo", "--version"])),
            ),
            (
                v(&["C:\\tools\\cargo.exe"]),
                Some(v(&["C:\\tools\\cargo.exe", "--version"])),
            ),
            (v(&["Cargo.EXE"]), Some(v(&["Cargo.EXE", "--version"]))),
            (
                v(&["npm.cmd", "run", "lint"]),
                Some(v(&["node", "--version"])),
            ),
            (v(&["yarn.bat"]), Some(v(&["node", "--version"]))),
            (v(&["npm", "+x"]), Some(v(&["node", "--version"]))),
            (
                v(&["C:\\tools\\node-a\\npm.cmd", "test"]),
                Some(v(&["C:\\tools\\node-a\\node", "--version"])),
            ),
            (
                v(&["/opt/node/bin/npx"]),
                Some(v(&["/opt/node/bin/node", "--version"])),
            ),
            (v(&["./eslint"]), Some(v(&["./node", "--version"]))),
            (v(&["pytest"]), None),
            (v(&["go", "vet"]), Some(v(&["go", "version"]))),
            (
                v(&["cargo", "+nightly", "clippy"]),
                Some(v(&["cargo", "+nightly", "--version"])),
            ),
            (v(&["cargo", "+"]), Some(v(&["cargo", "+", "--version"]))),
            (v(&["sh", "-c", "x"]), None),
            (v(&["cargo-nextest"]), None),
            (v(&[".exe"]), None),
            (v(&[""]), None),
            (v(&[]), None),
        ];
        for (argv, want) in rows {
            assert_eq!(probe_argv(&argv), want, "{argv:?}");
        }
    }

    #[test]
    fn first_line_is_trimmed_capped_and_lossy() {
        let long = "x".repeat(VERSION_CAP + 1);
        let rows: [(&[u8], String); 6] = [
            (
                b"cargo 1.98.0 (abc 2026-01-01)\nmore",
                "cargo 1.98.0 (abc 2026-01-01)".into(),
            ),
            (b"  v22.1.0 \r\n", "v22.1.0".into()),
            (b"", String::new()),
            (b"\n\nlate", String::new()),
            (long.as_bytes(), "x".repeat(VERSION_CAP)),
            (b"\xff1.0", "\u{fffd}1.0".into()),
        ];
        for (bytes, want) in rows {
            assert_eq!(first_line(bytes), want);
        }
        let wide = "é".repeat(VERSION_CAP);
        assert_eq!(first_line(wide.as_bytes()).len(), VERSION_CAP);
    }
}
