//! `ssh-keygen -Y verify` against the trust file, judged at the document's `ts`.

use std::ffi::OsStr;
use std::io::Read as _;
use std::path::Path;
use std::process::Output;

use jtac_core::{Signer, Ts};

use crate::fingerprint::printed;
use crate::tool::first_line;
use crate::{Namespace, SignError, Tool};

/// A trust file larger than this is refused unread.
pub const TRUST_CAP: u64 = 1 << 20;

/// What a signature proved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verified {
    /// The signature holds and the trust file names the signer's key under its principal.
    Trusted,
    /// The signature holds, but no trust file named the key under that principal at `ts`.
    Unknown {
        /// No trust file, or what ssh-keygen said when it refused.
        why: String,
    },
    /// The signature does not hold for these bytes, this namespace, or this signer.
    Invalid {
        /// What failed.
        what: String,
    },
}

/// Verify `sig` over `msg` in `ns` as `signer`, with the keys in `trust` (`.jtac/signers`,
/// `None` when the tree has none) judged at `at`. The key ssh-keygen names must be the
/// signer's. A named trust file that is not a plain readable file is an error; ssh-keygen
/// is given a private copy of the bytes that passed that check.
pub fn verify(
    tool: &Tool,
    trust: Option<&Path>,
    signer: &Signer,
    ns: Namespace,
    at: Ts,
    msg: &[u8],
    sig: &str,
) -> Result<Verified, SignError> {
    let trust = match trust {
        Some(path) => Some(temp("jtac-trust-", &read_trust(path)?)?),
        None => None,
    };
    let sig_file = temp("jtac-sig-", sig.as_bytes())?;
    let out = novalidate(tool, ns, sig_file.path(), msg)?;
    if !out.status.success() {
        return Ok(invalid(&first_line(&out.stderr)));
    }
    match printed(&out.stdout) {
        Some(found) if found == signer.fingerprint => {}
        Some(found) => return Ok(invalid(&format!("signed by another key, {found}"))),
        None => return Ok(invalid("ssh-keygen named no key")),
    }
    let Some(trust) = trust else {
        return Ok(Verified::Unknown {
            why: "no trust file".into(),
        });
    };
    let time = format!("verify-time={}", verify_time(at));
    let args: [&OsStr; 12] = [
        "-Y".as_ref(),
        "verify".as_ref(),
        "-f".as_ref(),
        trust.path().as_os_str(),
        "-I".as_ref(),
        signer.principal.as_ref(),
        "-n".as_ref(),
        ns.as_str().as_ref(),
        "-s".as_ref(),
        sig_file.path().as_os_str(),
        "-O".as_ref(),
        time.as_ref(),
    ];
    let out = tool.run(&args, msg)?;
    Ok(if out.status.success() {
        Verified::Trusted
    } else {
        Verified::Unknown {
            why: Some(first_line(&out.stderr))
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "not in the trust file".to_owned()),
        }
    })
}

/// `ts` as ssh-keygen spells an instant, `YYYYMMDDHHMMSS`, read as UTC because the tool runs
/// with `TZ=UTC0`.
pub fn verify_time(at: Ts) -> String {
    at.to_string().replace(['-', ':', 'T', 'Z'], "")
}

/// `check-novalidate`: the signature holds over `msg` in `ns`; stdout names the key.
pub(crate) fn novalidate(
    tool: &Tool,
    ns: Namespace,
    sig: &Path,
    msg: &[u8],
) -> Result<Output, SignError> {
    let args: [&OsStr; 6] = [
        "-Y".as_ref(),
        "check-novalidate".as_ref(),
        "-n".as_ref(),
        ns.as_str().as_ref(),
        "-s".as_ref(),
        sig.as_os_str(),
    ];
    tool.run(&args, msg)
}

pub(crate) fn temp(prefix: &str, bytes: &[u8]) -> Result<tempfile::NamedTempFile, SignError> {
    use std::io::Write as _;
    let io = |source| SignError::Io {
        path: std::env::temp_dir(),
        source,
    };
    let mut file = tempfile::Builder::new()
        .prefix(prefix)
        .tempfile()
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    Ok(file)
}

/// A symlink, a device or a FIFO in the tree must not be followed or read without end, and
/// the file judged is the one opened: the type and size come from the open handle.
fn read_trust(path: &Path) -> Result<Vec<u8>, SignError> {
    let io = |source| SignError::Io {
        path: path.to_path_buf(),
        source,
    };
    let refuse = |what| SignError::Trust {
        path: path.to_path_buf(),
        what,
    };
    if std::fs::symlink_metadata(path).map_err(io)?.is_symlink() {
        return Err(refuse("not a plain file"));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(
        &mut options,
        libc::O_NOFOLLOW | libc::O_NONBLOCK,
    );
    let mut file = options.open(path).map_err(io)?;
    let meta = file.metadata().map_err(io)?;
    if !meta.is_file() {
        return Err(refuse("not a plain file"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(TRUST_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > TRUST_CAP {
        return Err(refuse("larger than the trust file cap"));
    }
    Ok(bytes)
}

fn invalid(what: &str) -> Verified {
    Verified::Invalid {
        what: what.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_time_is_the_ts_in_ssh_keygen_spelling() {
        let at: Ts = "2026-10-10T18:30:00Z".parse().unwrap();
        assert_eq!(verify_time(at), "20261010183000");
    }
}
