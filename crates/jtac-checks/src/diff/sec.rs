//! `JT-SEC-001`: a string shaped like a credential on an added line, in any file. Shapes are
//! known prefixes, never entropy, so a digest or a lockfile hash cannot block; the message
//! never carries the value. A quoted value with whitespace, a reference, a placeholder word, or
//! a password word and no digit is text, so a label or a template does not block.
//! `credential_shapes` rows the table, bypasses first.

use jtac_core::Severity;
use regex::Regex;
use std::sync::LazyLock;

use super::report::Finding;
use super::rule::Rule;
use super::view::View;

const TABLE: &[(&str, &str)] = &[
    ("AWS access key id", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
    (
        "GitHub token",
        r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})\b",
    ),
    ("Slack token", r"\bxox[abprs]-[0-9A-Za-z-]{10,}\b"),
    (
        "Slack webhook",
        r"hooks\.slack\.com/services/T[A-Za-z0-9]+/B[A-Za-z0-9]+/[A-Za-z0-9]+",
    ),
    ("Stripe key", r"\b[sr]k_live_[0-9A-Za-z]{20,}\b"),
    ("secret key", r"\bsk-[A-Za-z0-9_-]{32,}"),
    ("Google API key", r"\bAIza[0-9A-Za-z_-]{35}\b"),
    (
        "SendGrid key",
        r"\bSG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}\b",
    ),
    ("npm token", r"\bnpm_[A-Za-z0-9]{36}\b"),
    (
        "private key block",
        r"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY(?: BLOCK)?-----",
    ),
    ("credential in URL", r"://[^/\s:@]+:([^/\s@]{4,})@"),
    (
        "secret assignment",
        r#"(?i)\b[a-z0-9_.-]*(?:password|passwd|secret|api[_-]?key|access[_-]?token|auth[_-]?token|private[_-]?key)["']?\s*[:=]+\s*(?:["']([^"'\s]{8,})["']|([a-z0-9_\-+/=]{8,})(?:[\s;,"']|$))"#,
    ),
];

/// A matched value starting with one of these is a reference, not a token.
const REFERENCE: &[&str] = &["$", "{", "<", "%("];

/// A matched value with one of these inside is a template or a placeholder, not a token.
const NOT_A_VALUE: &[&str] = &[
    "${",
    "$(",
    "{{",
    "example",
    "placeholder",
    "changeme",
    "redacted",
    "xxx",
    "your_",
    "your-",
    "dummy",
    "sample",
    "fake",
    "test",
];

static PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    TABLE
        .iter()
        .map(|(name, re)| (*name, Regex::new(re).expect("literal")))
        .collect()
});

/// The shape `line` carries, if any. An unquoted assigned value needs a digit, or it is a name.
pub(crate) fn shape(line: &str) -> Option<&'static str> {
    PATTERNS.iter().find_map(|(name, re)| {
        re.captures_iter(line)
            .any(|caps| {
                let unquoted = caps.get(2);
                let value = unquoted.or(caps.get(1)).or(caps.get(0));
                let value = value.map_or(String::new(), |m| m.as_str().to_ascii_lowercase());
                let digit = value.chars().any(|c| c.is_ascii_digit());
                !REFERENCE.iter().any(|p| value.starts_with(p))
                    && !NOT_A_VALUE.iter().any(|p| value.contains(p))
                    && (unquoted.is_none() || digit)
                    && (digit || !value.contains("passw"))
            })
            .then_some(*name)
    })
}

pub(crate) fn check(views: &[View]) -> Vec<Finding> {
    views
        .iter()
        .flat_map(|v| {
            v.edit.added.iter().filter_map(move |(n, text)| {
                shape(text).map(|name| {
                    Finding::new(
                        Rule::Sec001,
                        Severity::Blocker,
                        v.path(),
                        *n,
                        format!("credential-shaped string ({name})"),
                    )
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_shapes() {
        let rows: &[(&str, Option<&str>)] = &[
            ("key = \"AKIAIOSFODNN7EXAMPLE\"", None),
            ("key = AKIAZZZZZZZZZZZZZZZZ", Some("AWS access key id")),
            ("ASIAZZZZZZZZZZZZZZZZ", Some("AWS access key id")),
            ("AKIAZZZZZZZZZZZZZZZ", None),
            ("akiazzzzzzzzzzzzzzzz", None),
            (
                &format!("token: ghp_{}", "a".repeat(36)),
                Some("GitHub token"),
            ),
            (&format!("ghs_{}", "a".repeat(36)), Some("GitHub token")),
            (&format!("ghp_{}", "a".repeat(35)), None),
            (
                &format!("github_pat_{}", "a".repeat(22)),
                Some("GitHub token"),
            ),
            (
                &format!("ghp_{} ghp_{}", "test".repeat(9), "a".repeat(36)),
                Some("GitHub token"),
            ),
            ("xoxb-1234567890-abc", Some("Slack token")),
            ("xoxz-1234567890-abc", None),
            (
                "https://hooks.slack.com/services/T0AB12CD3/B0EF45GH6/abcdefghijklmnopqrstuvwx",
                Some("Slack webhook"),
            ),
            ("https://hooks.slack.com/services/T000/B000/XXXX", None),
            (&format!("sk_live_{}", "a".repeat(24)), Some("Stripe key")),
            (&format!("sk_test_{}", "a".repeat(24)), None),
            (&format!("sk-ant-{}", "a".repeat(40)), Some("secret key")),
            (&format!("sk-proj-{}", "a".repeat(40)), Some("secret key")),
            (&format!("sk-{}", "a".repeat(20)), None),
            (&format!("AIza{}", "a".repeat(35)), Some("Google API key")),
            (
                &format!("SG.{}.{}", "a".repeat(22), "b".repeat(43)),
                Some("SendGrid key"),
            ),
            (&format!("npm_{}", "a".repeat(36)), Some("npm token")),
            ("-----BEGIN RSA PRIVATE KEY-----", Some("private key block")),
            (
                "-----BEGIN OPENSSH PRIVATE KEY-----",
                Some("private key block"),
            ),
            (
                "-----BEGIN PGP PRIVATE KEY BLOCK-----",
                Some("private key block"),
            ),
            ("-----BEGIN PUBLIC KEY-----", None),
            ("-----BEGIN CERTIFICATE-----", None),
            (
                "postgres://app:hunter2hunter@db.example.com/x",
                Some("credential in URL"),
            ),
            (
                "postgres://admin:S3cr%21tpw@db.example.com/x",
                Some("credential in URL"),
            ),
            ("postgres://app:${DB_PASS}@db/x", None),
            ("postgres://app:pw@db/x", None),
            ("https://db.example.com/x", None),
            (
                "password = \"correcthorsebattery\"",
                Some("secret assignment"),
            ),
            ("password = \"S3cure$Pass9!\"", Some("secret assignment")),
            ("PASSWORD: 'correcthorsebattery'", Some("secret assignment")),
            ("password = \"correct horse battery\"", None),
            ("\"password\": \"Password\",", None),
            ("\"password\": \"Passwort:\",", None),
            ("password = \"password123\"", Some("secret assignment")),
            ("\"forgot_password\": \"Forgot your password?\",", None),
            ("password: \"is too short (minimum is 8 characters)\"", None),
            ("password = \"$DB_PASSWORD\"", None),
            ("password = \"pre-${DB_PASSWORD}\"", None),
            ("api_key=\"0123456789abcdef\"", Some("secret assignment")),
            ("API-KEY = \"0123456789abcdef\"", Some("secret assignment")),
            (
                "auth_token: \"0123456789abcdef\"",
                Some("secret assignment"),
            ),
            (
                "private_key = \"0123456789abcdef\"",
                Some("secret assignment"),
            ),
            (
                "DB_PASSWORD = \"hunter2hunter2\"",
                Some("secret assignment"),
            ),
            (
                "client_secret = \"q8Zr4mN2vX7pL0sK\"",
                Some("secret assignment"),
            ),
            (
                "\"password\": \"hunter2hunter2\",",
                Some("secret assignment"),
            ),
            ("password := \"hunter2hunter2\"", Some("secret assignment")),
            ("DB_PASSWORD=hunter2hunter2", Some("secret assignment")),
            (
                "config.password = 'hunter2hunter2'",
                Some("secret assignment"),
            ),
            ("password = some_variable_name", None),
            ("password = get_password()", None),
            ("password_hash = \"0123456789abcdef\"", None),
            ("is_secret = true", None),
            ("secret: str = \"\"", None),
            ("password = \"short\"", None),
            ("password = \"${{ secrets.DB_PASSWORD }}\"", None),
            ("password = \"{{ db_password }}\"", None),
            ("password = \"%(db_password)s\"", None),
            ("password = \"<your password here>\"", None),
            ("password = \"changeme-please\"", None),
            ("password = \"example-password\"", None),
            ("password = \"xxxxxxxxxxxx\"", None),
            ("password = \"test-password-1\"", None),
            ("password = os.environ[\"DB_PASSWORD\"]", None),
            ("password: str = field(default=\"\")", None),
            ("secret_name = \"db-password-secret\"", None),
            (
                "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                None,
            ),
            (
                "integrity: sha512-aGVsbG8gd29ybGQgdGhpcyBpcyBhIGxvY2tmaWxlIGhhc2g=",
                None,
            ),
            ("", None),
        ];
        for (line, want) in rows {
            assert_eq!(shape(line), *want, "{line:?}");
        }
    }
}
