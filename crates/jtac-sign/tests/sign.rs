//! Signing and verifying with throwaway ed25519 keys. No agent, no home directory, no
//! prompt: every failure here is milliseconds.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use jtac_core::{Signer, Ts};
use jtac_sign::{
    GitConfig, Identity, KeyRef, Namespace, Resolved, SignError, TRUST_CAP, Tool, Trust, Verified,
    identity, resolve, sign, verify,
};

/// A trust-file row: principal, key name, options.
type Line<'a> = (&'a str, &'a str, &'a str);

const MSG: &[u8] = br#"{"a":1}"#;
const AT: &str = "2026-10-10T18:30:00Z";

struct Keys {
    dir: tempfile::TempDir,
}

impl Keys {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (name, passphrase) in [("key", ""), ("other", ""), ("locked", "secret")] {
            let out = Command::new("ssh-keygen")
                .args([
                    "-q",
                    "-t",
                    "ed25519",
                    "-C",
                    "dev@example.com",
                    "-N",
                    passphrase,
                    "-f",
                ])
                .arg(dir.path().join(name))
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let keygen = |args: &[&str]| {
            let out = Command::new("ssh-keygen").args(args).output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        let ca = dir.path().join("ca").to_string_lossy().into_owned();
        let key = dir.path().join("key.pub").to_string_lossy().into_owned();
        keygen(&["-q", "-t", "ed25519", "-C", "ca", "-N", "", "-f", &ca]);
        keygen(&["-q", "-s", &ca, "-I", "dev", "-n", "dev@example.com", &key]);
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn pub_line(&self, name: &str) -> String {
        let text = std::fs::read_to_string(self.path(&format!("{name}.pub"))).unwrap();
        let mut fields = text.split_whitespace();
        format!("{} {}", fields.next().unwrap(), fields.next().unwrap())
    }

    fn id(&self, key: KeyRef) -> Identity {
        let r = resolve(&GitConfig {
            jtac_signingkey: Some(match &key {
                KeyRef::File(p) => p.to_string_lossy().into_owned(),
                KeyRef::Literal(l) => format!("key::{l}"),
            }),
            user_email: Some("dev@example.com".into()),
            ..GitConfig::default()
        })
        .unwrap();
        assert_eq!(r.key, key);
        identity(r).unwrap()
    }

    /// An allowed-signers file, one line per row.
    fn trust(&self, rows: &[Line]) -> PathBuf {
        let text: String = rows
            .iter()
            .map(|(principal, name, options)| {
                let opts = if options.is_empty() {
                    String::new()
                } else {
                    format!("{options} ")
                };
                format!("{principal} {opts}{}\n", self.pub_line(name))
            })
            .collect();
        let path = self.path("signers");
        std::fs::write(&path, text).unwrap();
        path
    }
}

fn at() -> Ts {
    AT.parse().unwrap()
}

fn check(id: &Identity, trust: Option<&Path>, ns: Namespace, msg: &[u8], sig: &str) -> Verified {
    let trust = trust.map_or(Trust::None, Trust::File);
    verify(&id.tool, trust, &id.signer, ns, at(), msg, sig).unwrap()
}

fn invalid(v: &Verified) -> &str {
    match v {
        Verified::Invalid { what } => what,
        other => panic!("{other:?}"),
    }
}

fn unknown(v: &Verified) -> &str {
    match v {
        Verified::Unknown { why } => why,
        other => panic!("{other:?}"),
    }
}

fn kind(v: &Verified) -> &'static str {
    match v {
        Verified::Trusted => "trusted",
        Verified::Unknown { .. } => "unknown",
        Verified::Invalid { .. } => "invalid",
    }
}

#[test]
fn a_signed_root_verifies_as_trusted_when_the_trust_file_names_the_key() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("key")));
    assert_eq!(id.signer.principal, "dev@example.com");
    assert!(
        id.signer.fingerprint.starts_with("SHA256:"),
        "{}",
        id.signer.fingerprint
    );
    let sig = sign(&id, Namespace::Root, MSG).unwrap();
    assert!(sig.starts_with("-----BEGIN SSH SIGNATURE-----\n"), "{sig}");
    assert!(sig.ends_with("-----END SSH SIGNATURE-----\n"), "{sig}");
    let trust = k.trust(&[
        ("other@example.com", "other", ""),
        ("dev@example.com", "key", ""),
    ]);
    assert_eq!(
        check(&id, Some(&trust), Namespace::Root, MSG, &sig),
        Verified::Trusted
    );
}

#[test]
fn a_rewritten_body_and_a_wrong_namespace_both_fail_with_or_without_a_trust_file() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("key")));
    let trust = k.trust(&[("dev@example.com", "key", "")]);
    let sig = sign(&id, Namespace::Root, MSG).unwrap();
    let other = br#"{"a":2}"#;
    for trust in [Some(trust.as_path()), None] {
        let body = check(&id, trust, Namespace::Root, other, &sig);
        assert!(invalid(&body).contains("incorrect signature"), "{body:?}");
        let ns = check(&id, trust, Namespace::Lever, MSG, &sig);
        assert!(invalid(&ns).contains("namespace"), "{ns:?}");
    }
    let lever = sign(&id, Namespace::Lever, MSG).unwrap();
    assert_eq!(
        check(&id, Some(&trust), Namespace::Lever, MSG, &lever),
        Verified::Trusted
    );
    assert_eq!(
        unknown(&check(&id, None, Namespace::Lever, MSG, &lever)),
        "no trust file"
    );
}

#[test]
fn a_valid_signature_from_a_key_the_trust_file_does_not_name_is_unknown() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("key")));
    let sig = sign(&id, Namespace::Root, MSG).unwrap();
    let rows: [(&str, &[Line]); 3] = [
        ("empty", &[]),
        ("another key", &[("dev@example.com", "other", "")]),
        ("another principal", &[("other@example.com", "key", "")]),
    ];
    for (why, lines) in rows {
        let trust = k.trust(lines);
        let got = check(&id, Some(&trust), Namespace::Root, MSG, &sig);
        assert!(!unknown(&got).is_empty(), "{why}: {got:?}");
    }
}

/// Times in the trust file carry no zone; the tool runs in UTC so a verifier in any zone
/// agrees.
#[test]
fn a_key_retired_in_the_trust_file_is_judged_at_the_root_ts_in_utc() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("key")));
    let sig = sign(&id, Namespace::Root, MSG).unwrap();
    let rows = [
        ("valid-before=\"20270101\"", "trusted"),
        ("valid-before=\"20260101\"", "unknown"),
        ("valid-before=\"20261010180000\"", "unknown"),
        ("valid-before=\"20261010190000\"", "trusted"),
        ("valid-after=\"20261010183000\"", "trusted"),
        ("valid-after=\"20261010183001\"", "unknown"),
        ("valid-after=\"20261010183001Z\"", "unknown"),
        ("namespaces=\"jtac-root-v1\"", "trusted"),
        ("namespaces=\"jtac-lever-v1\"", "unknown"),
    ];
    for (options, want) in rows {
        let trust = k.trust(&[("dev@example.com", "key", options)]);
        let got = check(&id, Some(&trust), Namespace::Root, MSG, &sig);
        assert_eq!(kind(&got), want, "{options}: {got:?}");
    }
}

#[test]
fn a_signer_whose_fingerprint_is_not_the_signing_key_is_invalid() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("key")));
    let other = k.id(KeyRef::File(k.path("other")));
    let trust = k.trust(&[
        ("dev@example.com", "key", ""),
        ("dev@example.com", "other", ""),
    ]);
    let sig = sign(&id, Namespace::Root, MSG).unwrap();
    let claimed = Signer {
        principal: "dev@example.com".into(),
        fingerprint: other.signer.fingerprint.clone(),
    };
    let got = verify(
        &id.tool,
        Trust::File(&trust),
        &claimed,
        Namespace::Root,
        at(),
        MSG,
        &sig,
    )
    .unwrap();
    assert_eq!(
        invalid(&got),
        format!("signed by another key, {}", id.signer.fingerprint)
    );
}

#[test]
fn garbage_is_invalid_not_an_error() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("key")));
    let got = check(&id, None, Namespace::Root, MSG, "not a signature");
    assert!(invalid(&got).contains("signature"), "{got:?}");
}

#[test]
fn a_named_trust_file_that_is_missing_a_link_a_directory_or_huge_is_an_error() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("key")));
    let sig = sign(&id, Namespace::Root, MSG).unwrap();
    let trust = k.trust(&[("dev@example.com", "key", "")]);
    let big = k.path("big");
    std::fs::write(&big, vec![b'\n'; usize::try_from(TRUST_CAP).unwrap() + 1]).unwrap();
    let missing = k.path("no-such-signers");
    let err = verify(
        &id.tool,
        Trust::File(&missing),
        &id.signer,
        Namespace::Root,
        at(),
        MSG,
        &sig,
    )
    .unwrap_err();
    assert!(
        matches!(&err, SignError::Io { path, source } if *path == missing && source.kind() == std::io::ErrorKind::NotFound),
        "{err}"
    );
    let dir = k.dir.path().to_path_buf();
    let err = verify(
        &id.tool,
        Trust::File(&dir),
        &id.signer,
        Namespace::Root,
        at(),
        MSG,
        &sig,
    )
    .unwrap_err();
    let refused_as_directory = match &err {
        SignError::Trust { what, .. } => *what == "not a plain file",
        SignError::Io { source, .. } => source.kind() == std::io::ErrorKind::PermissionDenied,
        _ => false,
    };
    assert!(refused_as_directory, "{err}");
    #[cfg(unix)]
    let unix_rows = {
        let link = k.path("link");
        std::os::unix::fs::symlink(&trust, &link).unwrap();
        let fifo = k.path("fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        vec![(link, "not a plain file"), (fifo, "not a plain file")]
    };
    #[cfg(not(unix))]
    let unix_rows = Vec::new();
    let rows = [(big, "larger than the trust file cap")]
        .into_iter()
        .chain(unix_rows);
    let started = Instant::now();
    for (path, want) in rows {
        let err = verify(
            &id.tool,
            Trust::File(&path),
            &id.signer,
            Namespace::Root,
            at(),
            MSG,
            &sig,
        )
        .unwrap_err();
        assert!(err.to_string().contains(want), "{}: {err}", path.display());
        assert!(
            matches!(err, SignError::Io { .. } | SignError::Trust { .. }),
            "{err}"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        check(&id, Some(&trust), Namespace::Root, MSG, &sig),
        Verified::Trusted
    );
}

/// With no terminal and no `-P`, ssh-keygen reads the passphrase from stdin: the first line
/// of the message. The guard is a message that starts with the right one.
#[test]
fn a_locked_key_never_takes_its_passphrase_from_the_message() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("locked")));
    let started = Instant::now();
    for msg in [MSG, b"secret\n{\"a\":1}"] {
        let err = sign(&id, Namespace::Root, msg).unwrap_err();
        assert!(
            matches!(&err, SignError::Failed { stderr, .. } if stderr.contains("passphrase")),
            "{err}"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_public_key_file_signs_with_the_private_key_beside_it() {
    let k = Keys::new();
    let id = k.id(KeyRef::File(k.path("key.pub")));
    let by_private = k.id(KeyRef::File(k.path("key")));
    assert_eq!(id.signer, by_private.signer);
    let sig = sign(&id, Namespace::Root, MSG).unwrap();
    let trust = k.trust(&[("dev@example.com", "key", "")]);
    assert_eq!(
        check(&id, Some(&trust), Namespace::Root, MSG, &sig),
        Verified::Trusted
    );
}

#[test]
fn a_literal_key_in_either_git_spelling_not_in_the_agent_fails_at_once() {
    let k = Keys::new();
    let id = k.id(KeyRef::Literal(k.pub_line("key")));
    let by_file = k.id(KeyRef::File(k.path("key")));
    assert_eq!(id.signer, by_file.signer);
    let bare = resolve(&GitConfig {
        jtac_signingkey: Some(k.pub_line("key")),
        user_email: Some("dev@example.com".into()),
        ..GitConfig::default()
    })
    .unwrap();
    assert_eq!(bare.key, id.key);
    let started = Instant::now();
    let err = sign(&id, Namespace::Root, MSG).unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(matches!(err, SignError::Failed { .. }), "{err}");
}

#[test]
fn a_signature_from_a_key_other_than_the_identity_claims_is_refused() {
    let k = Keys::new();
    let mut id = k.id(KeyRef::File(k.path("key")));
    let other = k.id(KeyRef::File(k.path("other")));
    id.signer.fingerprint = other.signer.fingerprint.clone();
    let err = sign(&id, Namespace::Root, MSG).unwrap_err();
    assert!(
        matches!(&err, SignError::KeyMismatch { expected, found } if *expected == other.signer.fingerprint && *found != *expected),
        "{err}"
    );
    std::fs::copy(k.path("other.pub"), k.path("key.pub")).unwrap();
    let disagreeing = k.id(KeyRef::File(k.path("key")));
    let err = sign(&disagreeing, Namespace::Root, MSG).unwrap_err();
    assert!(
        matches!(&err, SignError::Failed { stderr, .. } if stderr.contains("match")),
        "{err}"
    );
}

#[test]
fn the_fingerprint_is_the_same_for_a_private_public_or_certificate_key() {
    let k = Keys::new();
    let by_private = k.id(KeyRef::File(k.path("key")));
    std::fs::remove_file(k.path("key.pub")).unwrap();
    let no_pub = k.id(KeyRef::File(k.path("key")));
    let cert = k.id(KeyRef::File(k.path("key-cert.pub")));
    let locked = k.id(KeyRef::File(k.path("locked")));
    assert_eq!(no_pub.signer, by_private.signer);
    assert_eq!(cert.signer, by_private.signer);
    assert_ne!(locked.signer, by_private.signer);
    let resolved = |key: KeyRef| Resolved {
        key,
        principal: "dev@example.com".into(),
        program: PathBuf::from("ssh-keygen"),
    };
    let err = identity(resolved(KeyRef::File(k.path("absent")))).unwrap_err();
    assert!(
        matches!(&err, SignError::Key { path, .. } if *path == k.path("absent")),
        "{err}"
    );
    let err = identity(resolved(KeyRef::Literal("ssh-ed25519 !!!".into()))).unwrap_err();
    assert!(
        matches!(&err, SignError::Key { path, .. } if path.ends_with("jtac.signingkey")),
        "{err}"
    );
}

#[test]
fn a_certificate_signs_and_is_trusted_through_its_authority_only() {
    let k = Keys::new();
    let cert = k.id(KeyRef::File(k.path("key-cert.pub")));
    let sig = sign(&cert, Namespace::Root, MSG).unwrap();
    let plain = k.trust(&[("dev@example.com", "key", "")]);
    assert_eq!(
        kind(&check(&cert, Some(&plain), Namespace::Root, MSG, &sig)),
        "unknown"
    );
    let authority = k.trust(&[("dev@example.com", "ca", "cert-authority")]);
    assert_eq!(
        check(&cert, Some(&authority), Namespace::Root, MSG, &sig),
        Verified::Trusted
    );
    let key = k.id(KeyRef::File(k.path("key")));
    let plain_sig = sign(&key, Namespace::Root, MSG).unwrap();
    assert_eq!(
        kind(&check(
            &key,
            Some(&authority),
            Namespace::Root,
            MSG,
            &plain_sig
        )),
        "unknown"
    );
}

#[test]
fn a_missing_ssh_keygen_is_a_spawn_error_for_both_operations() {
    let k = Keys::new();
    let mut id = k.id(KeyRef::File(k.path("key")));
    let sig = sign(&id, Namespace::Root, MSG).unwrap();
    id.tool = Tool::new("jtac-no-such-tool-7f3a");
    assert!(matches!(
        sign(&id, Namespace::Root, MSG).unwrap_err(),
        SignError::Spawn { .. }
    ));
    let err = verify(
        &id.tool,
        Trust::None,
        &id.signer,
        Namespace::Root,
        at(),
        MSG,
        &sig,
    )
    .unwrap_err();
    assert!(matches!(err, SignError::Spawn { .. }), "{err}");
}
