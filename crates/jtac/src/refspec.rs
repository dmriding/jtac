//! The remote `jtac init` gives the evidence refspec (DESIGN.md sections 9.6 and 11.7): fetch
//! only, because a push refspec would change what a bare `git push` does.

use std::path::Path;

use jtac_checks::git::{git, quoted};

use crate::init::ask;
use crate::refuse::Refusal;
use crate::repo::{RepoError, probe};

/// What `git fetch` adds so every root on the remote comes down with the code.
pub const REFSPEC: &str = "+refs/jtac/*:refs/jtac/*";

/// `flag` when it names a remote, else `origin`, else the only remote; `None` when there is
/// none. Several without `origin` is the caller's choice, never a guess.
pub fn choose(remotes: &[String], flag: Option<&str>) -> Result<Option<String>, Refusal> {
    let has = |name: &str| remotes.iter().any(|r| r == name);
    match (flag, remotes) {
        (Some(name), _) if has(name) => Ok(Some(name.to_owned())),
        (Some(name), _) => Err(Refusal::init_no_remote(name, remotes)),
        (None, _) if has("origin") => Ok(Some("origin".to_owned())),
        (None, []) => Ok(None),
        (None, [one]) => Ok(Some(one.clone())),
        (None, several) => Err(Refusal::init_which_remote(several)),
    }
}

/// The remotes `git remote` lists.
pub fn remotes(repo: &Path) -> Result<Vec<String>, Refusal> {
    let out = git(repo, &["remote"]).map_err(|e| Refusal::repository(&e.into()))?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Whether `remote` already fetches [`REFSPEC`].
pub fn present(repo: &Path, remote: &str) -> Result<bool, Refusal> {
    let key = format!("remote.{remote}.fetch");
    let found =
        probe(repo, &["config", "--get-all", &key], 1).map_err(|e| Refusal::repository(&e))?;
    Ok(found.is_some_and(|all| all.lines().any(|l| l == REFSPEC)))
}

/// Add [`REFSPEC`] to `remote`'s fetch list in the repository's own config.
pub fn add(repo: &Path, remote: &str) -> Result<(), Refusal> {
    let key = format!("remote.{remote}.fetch");
    git(repo, &["config", "--add", &key, REFSPEC])
        .map(|_| ())
        .map_err(|e| Refusal::repository(&RepoError::from(e)))
}

impl Refusal {
    /// `--remote` names a remote this repository does not have.
    pub fn init_no_remote(name: &str, remotes: &[String]) -> Self {
        let other = match remotes.first() {
            Some(r) => format!(
                ", or name one it has: `jtac init --remote {}`",
                quoted(r, "<remote>")
            ),
            None => String::new(),
        };
        let q = quoted(name, "<remote>");
        ask(
            format!("`--remote {q}`: this repository has no remote named `{q}`."),
            "init adds the evidence fetch refspec to one remote; a name git does not know \
             would leave the roots on the server unfetched while init reported it done.",
            format!("Run `git remote add {q} <url>`, then `jtac init --remote {q}`{other}."),
        )
    }

    /// Several remotes and none is `origin`.
    pub fn init_which_remote(remotes: &[String]) -> Self {
        let first = remotes.first().map_or("origin", String::as_str);
        ask(
            format!(
                "this repository has several remotes and none is `origin`: {}.",
                remotes.join(", ")
            ),
            "init adds the evidence fetch refspec to one remote, the one roots are pushed to; \
             guessing would fetch evidence from a remote nobody pushes it to.",
            format!(
                "Run `jtac init --remote {}`, naming the remote you push to.",
                quoted(first, "<remote>")
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Row<'a> = (
        &'a [&'a str],
        Option<&'a str>,
        Result<Option<&'a str>, &'a str>,
    );

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_flag_then_origin_then_the_only_remote_is_chosen_and_several_refuse() {
        let rows: [Row; 8] = [
            (&["up", "origin"], Some("up"), Ok(Some("up"))),
            (&["up"], Some("nope"), Err("git remote add nope")),
            (&[], Some("origin"), Err("git remote add origin")),
            (&["up", "origin"], None, Ok(Some("origin"))),
            (&["up"], None, Ok(Some("up"))),
            (&[], None, Ok(None)),
            (&["up", "down"], None, Err("jtac init --remote up")),
            (&["origin"], None, Ok(Some("origin"))),
        ];
        for (remotes, flag, want) in rows {
            let got = choose(&names(remotes), flag);
            match (got, want) {
                (Ok(got), Ok(want)) => assert_eq!(got.as_deref(), want, "{remotes:?} {flag:?}"),
                (Err(r), Err(how)) => assert!(r.how.contains(how), "{remotes:?}: {}", r.how),
                (got, want) => panic!("{remotes:?} {flag:?}: {got:?} wanted {want:?}"),
            }
        }
    }
}
