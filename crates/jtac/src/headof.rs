//! What a merge root says about its head (DESIGN.md section 15): `audit` reads the label
//! from `from` alone, where the merge job recorded what it found, so a root published later
//! changes nothing; the head root's signer is named only when its ref holds the very root
//! `from` names, verified as a document and never as coverage of a commit the clone may lack.

use jtac_core::{Compared, FromHead, GitOid, Signer};
use jtac_sign::{Namespace, Tool, Verified};
use jtac_store::Store;
use serde::Serialize;

use crate::refuse::Refusal;
use crate::survey::short;
use crate::trust::Signers;

/// A merge root's head as `audit` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeadOut {
    /// The pull request head commit.
    pub commit: GitOid,
    /// `absent`, `unverified`, `disagreed`, `lied`, `invalid` or `verified`.
    pub label: &'static str,
    /// The head root's ref holds the root `from` names.
    pub held: bool,
    /// Who signed that root, when held.
    pub signer: Option<Signer>,
    /// `trusted`, `unknown` or `invalid` for that root's signature, when held.
    pub trust: Option<&'static str>,
}

/// The label `from` gives, with no ref read.
pub fn label(from: &FromHead) -> &'static str {
    match from.compared {
        None => "absent",
        Some(Compared::Unverifiable) => "unverified",
        Some(Compared::Disagreed) => "disagreed",
        Some(Compared::Lied) => "lied",
        Some(Compared::Invalid) => "invalid",
        Some(Compared::Agreed) => "verified",
    }
}

/// The head as shown; a ref that is missing, unreadable or holds another root is not held.
pub fn head_of(
    store: &Store,
    tool: &Tool,
    signers: &Signers,
    command: &str,
    from: &FromHead,
) -> Result<HeadOut, Refusal> {
    let mut out = HeadOut {
        commit: from.head,
        label: label(from),
        held: false,
        signer: None,
        trust: None,
    };
    let Some(id) = from.root else {
        return Ok(out);
    };
    let Some(stored) = store.root(&from.head).ok().flatten() else {
        return Ok(out);
    };
    if stored.root.id().map_err(Refusal::internal)? != id {
        return Ok(out);
    }
    let bytes = stored.root.canonical_bytes().map_err(Refusal::internal)?;
    let verified = jtac_sign::verify(
        tool,
        signers.as_sign(),
        &stored.root.signer,
        Namespace::of_root(&stored.root),
        stored.root.ts,
        &bytes,
        &String::from_utf8_lossy(&stored.sig),
    )
    .map_err(|e| signers.refusal(command, &e))?;
    out.held = true;
    out.signer = Some(stored.root.signer.clone());
    out.trust = Some(match verified {
        Verified::Trusted => "trusted",
        Verified::Unknown { .. } => "unknown",
        Verified::Invalid { .. } => "invalid",
    });
    Ok(out)
}

impl HeadOut {
    /// One line: `head <commit> verified, root held by <principal> (trusted)`.
    pub fn line(&self) -> String {
        let root = match (&self.signer, self.trust, self.held) {
            (Some(s), Some(t), _) => format!(", root held by {} ({t})", s.principal),
            (_, _, false) if self.label != "absent" => ", root not held".to_owned(),
            _ => String::new(),
        };
        format!("head {} {}{root}", short(&self.commit), self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_comes_from_compared_alone_and_the_line_names_what_is_held() {
        let head: GitOid = "sha1:0123456789abcdef0123456789abcdef01234567"
            .parse()
            .unwrap();
        let root = "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .parse()
            .unwrap();
        let from = |compared: Option<Compared>| FromHead {
            head,
            root: compared.map(|_| root),
            compared,
        };
        let rows = [
            (None, "absent"),
            (Some(Compared::Unverifiable), "unverified"),
            (Some(Compared::Disagreed), "disagreed"),
            (Some(Compared::Lied), "lied"),
            (Some(Compared::Invalid), "invalid"),
            (Some(Compared::Agreed), "verified"),
        ];
        for (compared, want) in rows {
            assert_eq!(label(&from(compared)), want);
        }
        let mut out = HeadOut {
            commit: head,
            label: "disagreed",
            held: false,
            signer: None,
            trust: None,
        };
        assert_eq!(out.line(), "head 0123456789ab disagreed, root not held");
        out.label = "absent";
        assert_eq!(out.line(), "head 0123456789ab absent");
        out.label = "verified";
        out.held = true;
        out.signer = Some(Signer {
            principal: "dev@example.com".into(),
            fingerprint: "SHA256:x".into(),
        });
        out.trust = Some("trusted");
        assert_eq!(
            out.line(),
            "head 0123456789ab verified, root held by dev@example.com (trusted)"
        );
    }
}
