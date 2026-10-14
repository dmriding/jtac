//! `ssh-keygen -Y sign` over bytes on stdin, the armored signature from stdout.

use std::ffi::OsStr;

use crate::fingerprint::printed;
use crate::identity::Identity;
use crate::key::KeyRef;
use crate::{Namespace, SignError};

/// Sign `msg` in `ns` as `id`. The empty `-P` is load-bearing (DESIGN.md section 11.8;
/// test `a_locked_key_never_takes_its_passphrase_from_the_message`). ssh-keygen then
/// re-reads the signature and must name the identity's key, or nothing is returned.
pub fn sign(id: &Identity, ns: Namespace, msg: &[u8]) -> Result<String, SignError> {
    let literal = match &id.key {
        KeyRef::File(_) => None,
        KeyRef::Literal(line) => Some(literal_file(line)?),
    };
    let key: &OsStr = match (&id.key, &literal) {
        (KeyRef::File(path), _) => path.as_os_str(),
        (_, Some(file)) => file.as_os_str(),
        (KeyRef::Literal(_), None) => unreachable!("a literal key was written to a file"),
    };
    let args: [&OsStr; 9] = [
        "-Y".as_ref(),
        "sign".as_ref(),
        "-P".as_ref(),
        "".as_ref(),
        "-f".as_ref(),
        key,
        "-n".as_ref(),
        ns.as_str().as_ref(),
        "-".as_ref(),
    ];
    let out = id.tool.ok(&args, msg)?;
    let sig = String::from_utf8(out).map_err(|_| SignError::KeyMismatch {
        expected: id.signer.fingerprint.clone(),
        found: "a signature that is not UTF-8".into(),
    })?;
    let sig_file = crate::verify::temp("jtac-sig-", sig.as_bytes())?;
    let check = crate::verify::novalidate(&id.tool, ns, &sig_file, msg)?;
    let found = printed(&check.stdout).unwrap_or_else(|| "no key".to_owned());
    if !check.status.success() || found != id.signer.fingerprint {
        return Err(SignError::KeyMismatch {
            expected: id.signer.fingerprint.clone(),
            found,
        });
    }
    Ok(sig)
}

/// ssh-keygen reads a public key from a file; the agent holds the private half.
pub(crate) fn literal_file(line: &str) -> Result<tempfile::TempPath, SignError> {
    use std::io::Write as _;
    let io = |source| SignError::Io {
        path: std::env::temp_dir(),
        source,
    };
    let mut file = tempfile::Builder::new()
        .prefix("jtac-key-")
        .suffix(".pub")
        .tempfile()
        .map_err(io)?;
    writeln!(file, "{line}").map_err(io)?;
    Ok(file.into_temp_path())
}
