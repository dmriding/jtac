# The CI gate

BLUF: one command writes the workflow, the trust file and a CI key, and prints the forge settings
to apply: an environment holding the key, a CODEOWNERS line, and two rulesets. Every pull request
and every push to the
default branch, save the one that creates it, is judged again on the forge's runner, and every pull request merged by
a commit of the forge's gets a root signed by a CI key that pull request runs cannot reach; the root job needs `contents: write` and
`pull-requests: read`, which the written workflow grants, and `actions: read` where it reads CI results. The design is `DESIGN.md` section 15; this page is the
operator's.

## Adopt

1. In a clone with the GitHub remote, run `jtac init --ci github`. It writes
   `.github/workflows/jtac.yml`, writes `.jtac/signers` when the repository has none (your
   line for every namespace, the CI key's confined to `jtac-ci-root-v1,jtac-release-v1`),
   makes a CI key under the user config directory (`jtac/ci/<owner>/<repo>_ed25519`), and
   prints the steps below with your values filled in. With `--read`, the policy it writes
   reads the named checks from CI (below). The private key is never printed. It
   refuses before writing anything when the remote is not GitHub or the clone does not know
   the default branch (`git remote set-head origin --auto`).
2. Add the CODEOWNERS line it prints, `/.jtac/ @<owner>`, so the forge asks a second human on
   every trust or policy change.
3. Create the environment `jtac-ci` with the three `gh api` calls it prints: the environment,
   then the default branch and `v*` tags as the only deployment targets. That rule is what
   keeps the key off pull request runs: a pull request from the same repository runs its
   own workflow file, so a repository secret would hand the key to any branch.
4. `gh secret set JTAC_CI_KEY --env jtac-ci < <key path>`. The private key stays at that
   path afterwards, and whoever holds it can sign as CI; removing it safely is #130.
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
| `push` to the default branch, the forge names the tip as a merged pull request's | `jtac check --from <pull request head> --base <old tip> --push origin` under the CI key, the roots of the pull request's own commits fetched from its repository where the base holds none, runs queued per branch | CI's one verdict from the old tip with the head chain's waivers in scope; a root on the tip signed in `jtac-ci-root-v1`, pushed to `refs/jtac/*`; a merge commit, a squash or a rebase alike; a root on the tip, found at the job's fetch or on the remote when its push is refused, stands only as CI's own for this head (a re-run or a concurrent run); any other is refused with the ref to delete. A verdict that fails exits 1 and writes a failure record at `refs/jtac/failed/` and no root, pushed the same way: the merge reads `failed` |
| `push` to the default branch: no pull request, a fast-forward, or a tip with the pull request's own commits on its first-parent line | `jtac ci --base <old tip>` on the tip | A forged root or failing code pushed straight to the branch fails the job; nothing signs over a direct push |
| `push` that rewrote the branch, or whose old tip the clone cannot fetch | refused, exit 2 | A gate that cannot anchor at the old tip judges nothing, and says so |
| `push` that created the branch | nothing judged, exit 0 | There is no old tip to diff from; the next push is judged |
| `workflow_run`: a workflow the policy reads completed | the push rows' commands, `check --from` with `--results <file> --branch <name>`, for the completed commit and every merge above the newest the CI gate reads, oldest first | A doorbell: rings only for a push run of the default branch whose commit is on its first-parent line. A result still running is a wait, exit 0, and later merges wait behind it; a result that is missing fails the job. Needs `actions: read` |
| any other event | refused, exit 2 | `pull_request_target` would hand the pull request head the secrets |
| either job | `actions/upload-artifact` of `.git/jtac` as `jtac-store-<job>` | `jtac replay` on a downloaded store |

The action pins the release it installs to its own version; `scripts/action-version.sh`
holds that literal to the workspace version and the release workflow refuses a tag where
they differ. A `binary` input runs a built `jtac` instead, which is how this repository
gates itself from its own tree before a release carries the change. The `root` job queues
up to a hundred runs per branch (`queue: max`); a run dropped beyond that is re-run from
the Actions tab with its own event, which restores the root.

## Checks read from CI

Policy version 2 lets a check name the workflow and job that already run it,
`ci = { workflow = ".github/workflows/ci.yml", job = "test" }`, in place of `run`
(`DESIGN.md` section 15.2). The team's CI runs the tests once and no jtac gate runs them
again:

- `jtac check` and `jtac ci` run the `run` checks and the rules and name each `ci` check
  as unread. The developer's root lists them and reads `covered, ci unread` in `jtac audit`.
- The root job starts when a workflow the policy reads completes for a push to the default
  branch (`workflow_run`, the table above), writes a results file from the forge's runs and
  reads each `ci` check from it. With every result in, the merge root holds them and the
  merge and the pull request's commits read covered. A result still running is a wait, and
  later merges wait behind it; a missing one fails the job; both write nothing. A check CI
  concluded failed, skipped or cancelled fails the verdict, and the failure record names the
  job and the run.
- The pull request job reads no result. What holds a merge on those jobs is the forge's own
  required checks.

### Adopt with it

```sh
jtac init --ci github --read rust-test=.github/workflows/ci.yml:test
```

`--read <id>=<workflow path>[:<job>]`, once for each check: the check's id, the definition
file's path from the repository root, and the job by the name the forge reports, a matrix
leg in full (`test (ubuntu-latest)`). With no job the run's own conclusion is the result.
init proposes none. An id detection also found, `rust-test` above, is replaced; any other is
added. The policy it writes is version 2 and printed in full, and the workflow's root job
starts on `workflow_run` for each named file, listed by the `name:` the file gives.

init refuses, before it writes anything:

- a named file that is not in the work tree;
- a workflow with no `name:` line at the left margin, more than one, or a name outside
  letters, digits, spaces and `-_./()&,:`: the trigger lists names as patterns, and one
  init got wrong would never fire, silently. Rename it, or write the workflow by hand;
- a policy already in the tree that lacks a named check: init never rewrites one, and
  prints the lines to add.

Three things the forge keeps, which init prints with your values:

- **Require the jobs the policy reads** in the default branch's ruleset, beside `jtac`.
- **No tag named like the default branch.** GitHub's run listing cannot tell such a tag's
  run from the branch's, so the root job writes no results file while one stands.
- **Every listed workflow runs on every push to the default branch**: `on: push`, with no
  `paths` filter. The root job starts only when such a run completes, so a workflow that
  runs on pull requests alone never starts it and no merge gets a root, with no failed job
  to say so. A pull request's run tests a trial merge, a tree no commit holds, and is
  never read. A push a filter skipped is found by the next run that completes, as a
  missing result that fails the root job.

Renaming a workflow moves the trigger in `jtac.yml` in the same change; renaming a job moves
the policy's `job`. Both files are protected, so each change is a finding a waiver clears.

### Move an adopted repository

1. Every clone's gate reads version 2 (0.2.21 or newer), and `jtac.yml` pins an action
   release whose notes say it reads CI results.
2. In `.jtac/policy.toml` set `version = 2` and give each check CI already runs its `ci`
   line in place of `run` and `timeout_secs`.
3. `git rm .github/workflows/jtac.yml`, then `jtac init --ci github`: it keeps the keys and
   the trust file and writes the workflow with the trigger that policy needs.
4. Commit both in one pull request. It changes `.jtac/` and `.github/`, so it takes a
   `JT-PROT-001` waiver for each path and is itself judged under the old policy; the merge
   after it is the first to read a result.

Never move the policy alone: started by a push, the root job passes no results file, exits
2 and writes no merge root, so no commit reads covered, `verify --require-coverage` fails
and `jtac release` refuses. init says so when it keeps a workflow with no `workflow_run`
trigger under such a policy. This repository's `.github/workflows/jtac.yml` and
`.jtac/policy.toml` are a hand-written example, built from the tree and not a release.

The results file's format is `DESIGN.md` section 15.2; a team on another forge writes it
from its own runs and passes it to
`jtac check --from <head> --base <old tip> --results <file> --branch <name>`.

## The release job

`release.yml` runs `jtac release <tag> --sums SHA256SUMS --require-ci --push origin` under
the CI key between the build and the publish (DESIGN.md section 16.3): every commit since
the previous attestation covered or declared off by a key the tag's `.jtac/signers` names, every
first-parent commit under CI's root, else no release and the commits named. The attestation,
`release.json` and `release.sig` at `refs/jtac/releases/<tag>`, is published beside the
archives; `jtac release <tag> --verify` in any clone that fetched `refs/jtac/*` holds it to
the clone's roots. The CI key's line names `jtac-release-v1` and a developer's does not, so
only the job attests. A rerun finds the attestation standing.

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

## A merge CI failed

A merge job whose verdict fails writes a failure record, signed by the CI key, at
`refs/jtac/failed/`. `jtac audit` reads the merge, and each commit of a rebase, as CI
`failed` with what failed; `jtac release` refuses it and says it failed. A re-run that
passes writes the merge root, which outranks the record, and the record stays as a note.
Shipping over a failure takes the waiver above, once for each commit of a rebase.

A record on the tip that is not CI's own for this head, another key's or another head's,
stops a job whose verdict failed, with the ref to inspect and delete. It stops nothing
else: a passing job does not read it, and a reader notes it and counts nothing by it.

## What reads as the developer's

A fast-forward merge makes no commit of its own and a direct push bypassed the gate: neither
gets a CI root. Nor does a tip whose first-parent line holds the pull request's own
commits, where the forge's would be, as a merge pushed with the pull request as its first
parent leaves it; `jtac check --from` refuses that shape. All three are judged by `jtac ci` on the push; one that fails the gate
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
