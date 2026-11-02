# The CI gate

BLUF: one action, one environment secret, one commit. Every pull request and every push to the
default branch, save the one that creates it, is judged again under a key the developer does not hold, and every pull
request merged by a commit of the forge's gets a root that key signed; the root job needs `contents: write` and
`pull-requests: read`, which the written workflow grants. The design is `DESIGN.md` section 15; this page is the
operator's.

## Adopt

1. In a clone with the GitHub remote, run `jtac init --ci github`. It writes
   `.github/workflows/jtac.yml`, writes `.jtac/signers` when the repository has none (your
   line for every namespace, the CI key's confined to `jtac-ci-root-v1,jtac-release-v1`),
   makes a CI key under the user config directory (`jtac/ci/<owner>/<repo>_ed25519`), and
   prints the steps below with your values filled in. The private key is never printed. It
   refuses before writing anything when the remote is not GitHub or the clone does not know
   the default branch (`git remote set-head origin --auto`).
2. Add the CODEOWNERS line it prints, `/.jtac/ @<owner>`, so the forge asks a second human on
   every trust or policy change.
3. Create the environment `jtac-ci` with the three `gh api` calls it prints: the environment,
   then the default branch and `v*` tags as the only deployment targets. That rule is what
   keeps the key off pull request runs: a pull request from the same repository runs its
   own workflow file, so a repository secret would hand the key to any branch.
4. `gh secret set JTAC_CI_KEY --env jtac-ci < <key path>`.
5. Require the `jtac` check in the default branch's ruleset, and a tag ruleset that lets only
   administrators create `v*` tags: the environment admits any `v*` tag by name, so without
   that rule a writer who tags an unreviewed commit holds the key.
6. Commit the workflow, the trust file and the policy together. In a fresh repository that
   commit is the adoption commit: every protected-path finding on it is minor, locally and in
   CI (decision 45). In a repository that already runs the local gate, the commit is a
   `JT-PROT-001` major locally and takes one rule waiver, since a signers file alone must not
   lift the fence the policy already is; in CI it is adoption, since the branch held no
   signers. The next change under `.jtac/` or `.github/` is major everywhere.

## What runs

| Event | Command | Result |
| --- | --- | --- |
| `pull_request` | `jtac ci --base origin/<base>` on the pull request head, full history, roots fetched from the repository the head lives in | Exit 0 pass, 1 fail (three parts in the log and the step summary), 2 could not answer |
| `push` to the default branch, the forge names the tip as a merged pull request's | `jtac check --from <pull request head> --base <old tip> --push origin` under the CI key, the roots of the pull request's own commits fetched from its repository where the base holds none, runs queued per branch | CI's one verdict from the old tip with the head chain's waivers in scope; a root on the tip signed in `jtac-ci-root-v1`, pushed to `refs/jtac/*`; a merge commit, a squash or a rebase alike; a root already on the tip is kept only as CI's own for this head (a re-run), any other is refused with the ref to delete |
| `push` to the default branch, no pull request or a fast-forward | `jtac ci --base <old tip>` on the tip | A forged root or failing code pushed straight to the branch fails the job; nothing signs over a direct push |
| `push` that rewrote the branch, or whose old tip the clone cannot fetch | refused, exit 2 | A gate that cannot anchor at the old tip judges nothing, and says so |
| `push` that created the branch | nothing judged, exit 0 | There is no old tip to diff from; the next push is judged |
| any other event | refused, exit 2 | `pull_request_target` would hand the pull request head the secrets |
| either job | `actions/upload-artifact` of `.git/jtac` as `jtac-store-<job>` | `jtac replay` on a downloaded store |

The action pins the release it installs to its own version; `scripts/action-version.sh`
holds that literal to the workspace version and the release workflow refuses a tag where
they differ. A `binary` input runs a built `jtac` instead, which is how this repository
gates itself from its own tree before a release carries the change. The `root` job queues
up to a hundred runs per branch (`queue: max`); a run dropped beyond that is re-run from
the Actions tab with its own event, which restores the root.

## What the check says about the pushed root

| The head's root | `jtac ci` |
| --- | --- |
| Agrees with CI's own run | pass |
| Missing | pass, the absence named; `require-local-root: true` on the action makes it fail |
| From another gate version, or needs history this clone lacks | unverifiable: as missing |
| Lied about its policy, disagrees with CI's run, or is invalid | fail |

CI's own verdict fails the check whatever the root said. Trust is always the base branch's
`.jtac/signers`; a change under `.jtac/**` is judged under the base branch's policy, so no
pull request clears the policy it brings.

## Fork pull requests

Roots come from the fork, since that is where the developer pushed them; trust comes from the
base branch, so the fork's `.jtac/signers` counts for nothing. The pull request run has no
secret. The merge job fetches the fork's roots again, so the head's root and the waivers it
carried are what the merge root records.

## Bypass merges

When the check cannot run and an administrator merges around it, nothing backfills. The
person who merged records why:

```sh
git pull
jtac waive <merge> --ci --reason "runners were down; reviewed by hand"
jtac check --push origin
```

The commit then reads CI `declared_off` in `jtac audit`: not covered, explained. The CI key
cannot sign that waiver, so a job never explains its own absence.

## What reads as the developer's

A fast-forward merge makes no commit of its own and a direct push bypassed the gate: neither
gets a CI root. Both are judged by `jtac ci` on the push; a direct push that fails the gate
fails the job. Require pull requests in the ruleset.

## A leaked key

Remove its line from `.jtac/signers` and rotate the secret. Never retire it with
`valid-before`: that is judged at the root's own timestamp, which the signer writes, so a
backdated root would still read as trusted. The roots the key signed then read as an unknown
signer's, which is the honest state.

## What the fence cannot do

A composite action cannot see the environment its job runs in, so the deployment rule and
the tag ruleset are the forge's to keep. The merge job checks the signing key against the old
tip's `.jtac/signers` and says when no line names it for the CI namespace: every reader then
sees an unknown signer's root, which is what a key outside the trust file deserves.
