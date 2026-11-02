# The CI gate

BLUF: one action, one environment secret, one commit. Every pull request is judged again under
a key the developer does not hold, and every merge commit gets a root that key signed. The
design is `DESIGN.md` section 15; this page is the operator's.

## Adopt

1. In a clone with the GitHub remote, run `jtac init --ci github`. It writes
   `.github/workflows/jtac.yml`, writes `.jtac/signers` when the repository has none (your
   line for every namespace, the CI key's confined to `jtac-ci-root-v1,jtac-release-v1`),
   makes a CI key under the user config directory (`jtac/ci/<owner>-<repo>_ed25519`), and
   prints the steps below with your values filled in. The private key is never printed.
2. Add the CODEOWNERS line it prints, `/.jtac/ @<owner>`, so the forge asks a second human on
   every trust or policy change.
3. Create the environment `jtac-ci` with deployment branches and tags limited to the default
   branch and `v*`. That rule is what keeps the key off pull request runs: a pull request
   from the same repository runs its own workflow file, so a repository secret would hand
   the key to any branch.
4. `gh secret set JTAC_CI_KEY --env jtac-ci < <key path>`.
5. Require the `jtac` check in the default branch's ruleset, and merge commits only.
6. Commit the workflow, the trust file and the policy together. On the commit that adds the
   first `.jtac/signers` every protected-path finding is minor, locally and in CI (decision
   45); the next change under `.jtac/` or `.github/` is major and clears only through a
   waiver a trusted signer put in the chain.

## What runs

| Event | Command | Result |
| --- | --- | --- |
| `pull_request` | `jtac ci --base origin/<base>` on the pull request head, full history, roots fetched from the repository the head lives in | Exit 0 pass, 1 fail (three parts in the log and the step summary), 2 could not answer |
| `push` to the default branch, a merge commit | `jtac check --from HEAD^2 --base <old tip> --push origin` under the CI key, one job at a time per branch | A root on the merge commit signed in `jtac-ci-root-v1`, pushed to `refs/jtac/*` |
| `push`, not a merge commit | nothing | The commit reads as the developer's root says, or absent; the log says why |
| either | `actions/upload-artifact` of `.git/jtac` as `jtac-store-<job>` | `jtac replay` on a downloaded store |

The action pins the release it installs to its own version; `scripts/action-version.sh`
holds that literal to the workspace version and the release workflow refuses a tag where
they differ. An `input: binary` runs a built `jtac` instead, which is how this repository
gates itself from its own tree before a release carries the change.

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
base branch, so the fork's `.jtac/signers` counts for nothing. The run has no secret.

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

A fast-forward merge makes no commit of its own, a squash or rebase merge names no head the
push event carries, and a direct push bypassed the gate: none gets a CI root. Require merge
commits in the ruleset. Reading the pull request head for squash and rebase merges through
the forge is a follow-up.

## A leaked key

Remove its line from `.jtac/signers` and rotate the secret. Never retire it with
`valid-before`: that is judged at the root's own timestamp, which the signer writes, so a
backdated root would still read as trusted. The roots the key signed then read as an unknown
signer's, which is the honest state.

## What the fence cannot do

A composite action cannot see the environment its job runs in, so the deployment rule is the
forge's to keep. The merge job checks the signing key against the old tip's `.jtac/signers`
and says when no line names it for the CI namespace: every reader then sees an unknown
signer's root, which is what a key outside the trust file deserves.
