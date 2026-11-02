//! What `jtac init --ci github` writes and prints (DESIGN.md section 15, Adoption), as text:
//! the workflow that calls the action, the CI key's allowed-signers line, and the GitHub
//! steps an operator does by hand. Pure; `cicfg` reads and writes.

use std::path::{Path, PathBuf};

use jtac_checks::git::quoted;

/// Where the workflow lands.
pub const WORKFLOW_PATH: &str = ".github/workflows/jtac.yml";
/// The environment whose deployment rule keeps the CI key off pull request runs.
pub const ENVIRONMENT: &str = "jtac-ci";
/// The environment secret that holds the CI private key.
pub const SECRET: &str = "JTAC_CI_KEY";
/// `user.email` of the roots CI signs: the principal of its line in `.jtac/signers`.
pub const CI_PRINCIPAL: &str = "jtac-ci@users.noreply.github.com";
/// The namespaces the CI key may sign in and no other (DESIGN.md section 15, Identity).
pub const CI_NAMESPACES: &str = "jtac-ci-root-v1,jtac-release-v1";
/// The job whose name the ruleset requires.
pub const CHECK_NAME: &str = "jtac";
const CHECKOUT: &str = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1";
const DOCS: &str = "https://github.com/dmriding/jtac/blob/main/docs/CI.md";

/// `owner` and `repo` of a github.com remote URL in the spellings git accepts; a name that
/// is a path step (`.`, `..`) or holds a separator is none, since both name a key file.
pub fn github_slug(url: &str) -> Option<(String, String)> {
    let path = [
        "https://github.com/",
        "http://github.com/",
        "ssh://git@github.com/",
        "git://github.com/",
        "git@github.com:",
    ]
    .iter()
    .find_map(|p| url.trim().strip_prefix(p))?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    let ok = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    (ok(owner) && ok(repo)).then(|| (owner.to_owned(), repo.to_owned()))
}

/// The CI key's path under `<user config>/jtac/ci/`: a directory per owner, a file per
/// repository, so two repositories never share one.
pub fn key_path(owner: &str, repo: &str) -> PathBuf {
    Path::new(owner).join(format!("{repo}_ed25519"))
}

/// `<key>.pub` as ssh-keygen names it, whatever dots the file name holds.
pub fn pub_path(key: &Path) -> PathBuf {
    let mut name = key.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}

/// The workflow: the pull request gate and the merge root, each the action pinned to this
/// release; `root` carries the environment, the queue that keeps every merge's run, and the
/// write permission the root push needs.
pub fn workflow(version: &str, default_branch: &str) -> String {
    format!(
        "# The jtac CI gate: `jtac ci` on every pull request and every push to `{default_branch}`,\n\
         # a CI-signed root on every merged pull request. Written by `jtac init --ci github`;\n\
         # {DOCS}\n\
         name: jtac\n\
         \n\
         on:\n\
         \x20 pull_request:\n\
         \x20 push:\n\
         \x20   branches: [{default_branch}]\n\
         \n\
         permissions:\n\
         \x20 contents: read\n\
         \n\
         jobs:\n\
         \x20 {CHECK_NAME}:\n\
         \x20   if: github.event_name == 'pull_request'\n\
         \x20   runs-on: ubuntu-latest\n\
         \x20   steps:\n\
         \x20     - uses: {CHECKOUT}\n\
         \x20       with:\n\
         \x20         fetch-depth: 0\n\
         \x20     - uses: dmriding/jtac@v{version}\n\
         \n\
         \x20 root:\n\
         \x20   if: github.event_name == 'push'\n\
         \x20   runs-on: ubuntu-latest\n\
         \x20   environment: {ENVIRONMENT}\n\
         \x20   concurrency:\n\
         \x20     group: jtac-root-${{{{ github.ref }}}}\n\
         \x20     cancel-in-progress: false\n\
         \x20     queue: max\n\
         \x20   permissions:\n\
         \x20     contents: write\n\
         \x20   steps:\n\
         \x20     - uses: {CHECKOUT}\n\
         \x20       with:\n\
         \x20         fetch-depth: 0\n\
         \x20     - uses: dmriding/jtac@v{version}\n\
         \x20       with:\n\
         \x20         key: ${{{{ secrets.{SECRET} }}}}\n"
    )
}

/// The CI key's line for `.jtac/signers`, confined to its namespaces so it signs no ordinary
/// root and no waiver; `public` is the `.pub` file, whose comment is dropped.
pub fn ci_signers_line(public: &str) -> Option<String> {
    let mut words = public.split_whitespace();
    let (kind, key) = (words.next()?, words.next()?);
    Some(format!(
        "{CI_PRINCIPAL} namespaces=\"{CI_NAMESPACES}\" {kind} {key}\n"
    ))
}

/// Whether a trust file's lines name `line`'s key.
pub fn names_key(trust: &str, line: &str) -> bool {
    let Some(blob) = line.split_whitespace().last() else {
        return false;
    };
    trust
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .any(|l| l.split_whitespace().any(|w| w == blob))
}

/// The first trust file: the operator's line for every namespace, then the CI key's.
pub fn signers_file(dev_line: &str, ci_line: &str) -> String {
    format!("{}\n{ci_line}", dev_line.trim_end())
}

/// What the operator does on GitHub, in order; `key` is the private key's path.
pub fn next_steps(owner: &str, repo: &str, key: &str, default_branch: &str) -> [String; 4] {
    let env = format!("repos/{owner}/{repo}/environments/{ENVIRONMENT}");
    [
        format!(
            "CODEOWNERS: add `/.jtac/ @{owner}` (a team, for an organization) so the forge asks \
             a second human on every trust or policy change"
        ),
        format!(
            "environment: `gh api -X PUT {env} -F \
             'deployment_branch_policy[protected_branches]=false' -F \
             'deployment_branch_policy[custom_branch_policies]=true'`, then `gh api -X POST \
             {env}/deployment-branch-policies -f name={default_branch} -f type=branch` and \
             the same with `-f name='v*' -f type=tag`; that rule is what keeps the key off \
             pull request runs"
        ),
        format!(
            "secret: `gh secret set {SECRET} --env {ENVIRONMENT} < {}` uploads the CI private \
             key; it leaves this machine no other way, and is never committed",
            quoted(key, "<key path>")
        ),
        format!(
            "rulesets: require the `{CHECK_NAME}` check on `{default_branch}`, and let only \
             administrators create `v*` tags: the environment admits any `v*` tag, so without \
             that rule a writer who tags an unreviewed commit holds the key ({DOCS})"
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_github_url_spelling_yields_the_slug_and_nothing_else_does() {
        let want = Some(("example-org".to_owned(), "example-app".to_owned()));
        for url in [
            "https://github.com/example-org/example-app.git",
            "https://github.com/example-org/example-app",
            "https://github.com/example-org/example-app/",
            "git@github.com:example-org/example-app.git",
            "ssh://git@github.com/example-org/example-app.git",
            "git://github.com/example-org/example-app",
            " https://github.com/example-org/example-app.git\n",
        ] {
            assert_eq!(github_slug(url), want, "{url}");
        }
        for url in [
            "https://example.com/example-org/example-app.git",
            "https://gitlab.com/example-org/example-app.git",
            "https://github.com.example.com/example-org/example-app.git",
            "https://github.com/example-org",
            "https://github.com//example-app",
            "https://github.com/example-org/example app",
            "https://github.com/example-org/example-app/extra",
            "https://github.com/../example-app",
            "https://github.com/example-org/..",
            "",
        ] {
            assert_eq!(github_slug(url), None, "{url}");
        }
    }

    #[test]
    fn a_key_per_repository_and_its_pub_file_keep_every_dot_in_the_name() {
        assert_eq!(
            key_path("example-org", "example-app"),
            Path::new("example-org/example-app_ed25519")
        );
        assert_ne!(
            key_path("example-team", "app"),
            key_path("example", "team-app")
        );
        let key = Path::new("/workspace/jtac/ci/example/my.app_ed25519");
        assert_eq!(
            pub_path(key),
            Path::new("/workspace/jtac/ci/example/my.app_ed25519.pub")
        );
    }

    #[test]
    fn the_workflow_pins_the_release_and_the_root_job_carries_the_fence() {
        let text = workflow("0.2.7", "trunk");
        assert_eq!(text.matches("uses: dmriding/jtac@v0.2.7").count(), 2);
        assert_eq!(text.matches(CHECKOUT).count(), 2);
        assert!(text.contains("    branches: [trunk]\n"));
        assert!(text.contains("  jtac:\n    if: github.event_name == 'pull_request'"));
        assert!(text.contains("  root:\n    if: github.event_name == 'push'"));
        assert!(text.contains("    environment: jtac-ci\n"));
        assert!(text.contains("      group: jtac-root-${{ github.ref }}\n"));
        assert!(text.contains("      cancel-in-progress: false\n      queue: max\n"));
        assert!(text.contains("      contents: write\n"));
        assert!(text.ends_with("          key: ${{ secrets.JTAC_CI_KEY }}\n"));
    }

    #[test]
    fn the_ci_line_is_confined_and_the_first_trust_file_names_both_keys() {
        let line = ci_signers_line("ssh-ed25519 AAAAC3 jtac ci@example.com\n").unwrap();
        assert_eq!(
            line,
            "jtac-ci@users.noreply.github.com namespaces=\"jtac-ci-root-v1,jtac-release-v1\" \
             ssh-ed25519 AAAAC3\n"
        );
        assert_eq!(ci_signers_line("ssh-ed25519"), None);
        let file = signers_file("dev@example.com ssh-ed25519 BBBB\n", &line);
        assert_eq!(file, format!("dev@example.com ssh-ed25519 BBBB\n{line}"));
        assert!(names_key(&file, &line));
        assert!(!names_key("dev@example.com ssh-ed25519 BBBB\n", &line));
        assert!(!names_key("# AAAAC3 in a comment\n", &line));
    }

    #[test]
    fn the_next_steps_name_the_owner_the_environment_the_quoted_key_and_the_tag_rule() {
        let steps = next_steps(
            "example-org",
            "example-app",
            "/work space/ci_ed25519",
            "main",
        );
        assert!(steps[0].contains("`/.jtac/ @example-org`"));
        assert!(
            steps[1].contains("gh api -X PUT repos/example-org/example-app/environments/jtac-ci")
                && steps[1].contains("-f name=main -f type=branch")
                && steps[1].contains("-f name='v*' -f type=tag")
        );
        assert!(
            steps[2].contains("gh secret set JTAC_CI_KEY --env jtac-ci < '/work space/ci_ed25519'")
        );
        assert!(steps[3].contains("`jtac` check") && steps[3].contains("`v*` tags"));
    }
}
