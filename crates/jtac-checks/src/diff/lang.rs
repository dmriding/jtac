//! Language by path, and each language's own convention for which files hold tests
//! (DESIGN.md section 10.3).

/// A language the rules know. TypeScript shares JavaScript's rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    /// `.rs`.
    Rust,
    /// `.py`, `.pyi`.
    Python,
    /// `.go`.
    Go,
    /// `.js`, `.jsx`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.mts`, `.cts`.
    Js,
}

impl Lang {
    /// By extension; `None` for a path the rules have no table for.
    pub fn of(path: &str) -> Option<Self> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let (_, ext) = name.rsplit_once('.')?;
        match ext {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => Some(Self::Js),
            _ => None,
        }
    }

    /// Whether `path` holds tests by the language's convention. A Rust `tests/` directory
    /// counts unless it sits under `src/`; a line-1 `#![cfg(test)]` is `lines::counts`' call.
    pub fn is_test_path(self, path: &str) -> bool {
        let parts: Vec<&str> = path.split('/').collect();
        let (name, dirs) = parts.split_last().unwrap_or((&"", &[]));
        let dir = |d: &str| dirs.contains(&d);
        match self {
            Self::Rust => dirs
                .iter()
                .position(|d| *d == "tests")
                .is_some_and(|i| !dirs[..i].contains(&"src")),
            Self::Python => {
                name.starts_with("test_")
                    || name.ends_with("_test.py")
                    || *name == "conftest.py"
                    || dir("tests")
                    || dir("test")
            }
            Self::Go => name.ends_with("_test.go"),
            Self::Js => {
                let stem = name.rsplit_once('.').map_or(*name, |(stem, _)| stem);
                stem.ends_with(".test")
                    || stem.ends_with(".spec")
                    || dir("__tests__")
                    || dir("tests")
                    || dir("test")
            }
        }
    }

    /// The line-comment opener, for the exemption marker.
    pub fn line_comment(self) -> &'static str {
        match self {
            Self::Python => "#",
            Self::Rust | Self::Go | Self::Js => "//",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_is_by_extension_only() {
        let rows = [
            ("src/lib.rs", Some(Lang::Rust)),
            ("a/b.py", Some(Lang::Python)),
            ("stubs/x.pyi", Some(Lang::Python)),
            ("main.go", Some(Lang::Go)),
            ("app.ts", Some(Lang::Js)),
            ("app.tsx", Some(Lang::Js)),
            ("lib.mjs", Some(Lang::Js)),
            ("lib.cts", Some(Lang::Js)),
            ("Makefile", None),
            ("rs", None),
            ("dir.rs/README", None),
            ("x.RS", None),
            ("notes.md", None),
        ];
        for (path, want) in rows {
            assert_eq!(Lang::of(path), want, "{path}");
        }
    }

    #[test]
    fn test_paths_follow_each_languages_convention() {
        let rows = [
            (Lang::Rust, "crates/x/tests/run.rs", true),
            (Lang::Rust, "tests/it.rs", true),
            (Lang::Rust, "tests/sub/it.rs", true),
            (Lang::Rust, "src/tests/mod.rs", false),
            (Lang::Rust, "src/tests.rs", false),
            (Lang::Rust, "src/lib.rs", false),
            (Lang::Rust, "crates/tests/src/lib.rs", true),
            (Lang::Python, "tests/test_a.py", true),
            (Lang::Python, "pkg/test_a.py", true),
            (Lang::Python, "pkg/a_test.py", true),
            (Lang::Python, "conftest.py", true),
            (Lang::Python, "test/helpers.py", true),
            (Lang::Python, "pkg/testing.py", false),
            (Lang::Python, "pkg/a.py", false),
            (Lang::Go, "pkg/a_test.go", true),
            (Lang::Go, "pkg/test_a.go", false),
            (Lang::Go, "tests/a.go", false),
            (Lang::Js, "src/a.test.ts", true),
            (Lang::Js, "src/a.spec.js", true),
            (Lang::Js, "src/__tests__/a.ts", true),
            (Lang::Js, "test/a.js", true),
            (Lang::Js, "src/atest.ts", false),
            (Lang::Js, "src/a.ts", false),
        ];
        for (lang, path, want) in rows {
            assert_eq!(lang.is_test_path(path), want, "{path}");
        }
    }
}
