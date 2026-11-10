# jtac

Provenance and verification for AI-written code.

[![ci](https://github.com/dmriding/jtac/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/dmriding/jtac/actions/workflows/ci.yml)
[![jtac](https://github.com/dmriding/jtac/actions/workflows/jtac.yml/badge.svg?branch=main)](https://github.com/dmriding/jtac/actions/workflows/jtac.yml)

jtac is a pre-push gate. `jtac check` runs a repository's own checks and rules
on the diff, and on a pass signs a root: a record of what ran on that tree,
kept in git beside the commits. The pre-push hook refuses a branch tip that no
root covers, by commit or by tree. A push around the hook still lands, and
anyone with a clone can verify the roots offline and see which commits have
none. In CI, the same binary re-runs the gate on pull requests, and a pull
request merged into the default branch by a commit of the forge's gets a root
signed by a CI key that pull request runs cannot reach.

It is for solo developers and teams who write code with AI agents faster than
they can review it: the gate keeps that pace and leaves a record anyone can
check.

No account, no server, no model call. One binary, git, and `ssh-keygen`.

**Status: v0.2.9 is the latest release: the local gate (v0.1), the CI gate and
the release gate (v0.2).** Tiering by blast radius is next.
[`DESIGN.md`](DESIGN.md) is the source of truth; `main` is the development
line; a release is a signed `v*` tag ([`docs/RELEASING.md`](docs/RELEASING.md)).

jtac gates this repository. Every commit on `main` since v0.2.7 is covered
(`jtac audit v0.2.7..main`), and v0.2.9 carries a release attestation the CI
key signed (`jtac release v0.2.9 --verify`). Older commits read `absent` where
they predate the gate or skipped it; nothing backfills them.

## Install

Linux x86_64 and macOS arm64, from the latest release, verified against the
release attestation the CI key signs, with `ssh-keygen`:

```sh
curl -fsSL https://raw.githubusercontent.com/dmriding/jtac/main/install.sh | sh
```

It puts `jtac` in `~/.local/bin` (`| sh -s -- --to DIR` for another place,
`--version vX.Y.Z` for a pinned one, v0.2.9 or later) and never edits PATH. It is 131
lines of `sh`, worth reading first. Windows takes the zip from the release page: the test suite runs
there, the hooks are unverified. From source, the latest release, with Rust 1.98
or later:

```sh
cargo install --locked --git https://github.com/dmriding/jtac --tag v0.2.9 jtac
```

Build from a release tag, not `main`: `jtac init --ci github` pins the action to
the binary's own version, which must exist as a release.

Everything needs git 2.31 or later and OpenSSH 8.9 or later for `ssh-keygen`.

## Quick start

Start from a clean tree whose checks pass. If the checks write files (cargo
writes `Cargo.lock` in a crate that has none), run them once and commit what
they wrote: `jtac check` refuses when the tree changes under it.

```sh
cd your-repo
jtac init                 # key, store, .jtac/policy.toml, pre-push hook, refspec;
                          # prints an "allowed signer:" line for your key
echo '<that line>' > .jtac/signers
git add .jtac && git commit -m "Adopt jtac"
jtac check                # CLEARED HOT: the adoption commit is judged under the
                          # policy it brings, and a signed root is written
git push                  # the hook sends the root ref ahead of the code
```

`jtac init` reads the tree and writes a policy for what it finds: Rust, Python,
Go, JavaScript/TypeScript, or only the diff rules when it finds nothing. The
policy is a TOML file you own; `jtac init --dry-run` prints it without writing.

From then on every push needs a root. `jtac status` says whether the next one
passes, and `jtac why` repeats the last verdict. On GitHub, the first push of a
new branch can be rejected once with `missing necessary objects` after the hook
sends its roots; the same push again succeeds
([#113](https://github.com/dmriding/jtac/issues/113)).

The terms used below: a **root** is the signed record of one gate run on one
commit's tree, kept under `refs/jtac/roots/` and pushed with the code.
`jtac audit` reads each commit as **covered**, **declared off** or **absent**
from the roots that bear on it; [`DESIGN.md`](DESIGN.md) section 9.5 defines
the three. `CLEARED HOT` is the gate's pass and `ABORT` its block; the
vocabulary follows the tool's name, which `DESIGN.md` explains.

## What a block looks like

A commit that adds a lint suppression and a token-shaped string, with the
tree, policy and tier lines trimmed:

```
ABORT: 2 reasons block
  commit  sha1:e449e906eb6f5079707dd60ea8374128369f8a34
  base    0cdeae1cdcec320108372029cb2adb315de3ac5f (nearest covered ancestor)
  root    none written
  1. JT-SUP-001 blocker src/main.rs:1: new suppression `#[allow(dead_code)]`
     why  A suppression turns a tool's finding off at one line. Each one is a decision that a person should see; past the budget it blocks.
     fix  fix what the tool reported instead of suppressing it, or raise integrity.max_new_suppressions in the policy with a reviewer's approval
  2. JT-SEC-001 blocker src/main.rs:2: credential-shaped string (GitHub token)
     why  A string shaped like a credential is an immediate leak once pushed, whatever it was for. History is not rewritten, so it is caught before the commit leaves the machine.
     fix  remove the string, rotate the credential it came from, and read it from the environment or a secret store
  checks
     integrity pass 18ms  diff-integrity rules
     rust-fmt pass 30ms  cargo fmt --all -- --check
     rust-lint pass 70ms  cargo clippy --workspace --all-targets -- -D warnings
     rust-test pass 80ms  cargo test --workspace
```

Every block and refusal has three parts: what happened, why the rule exists, and
what to do next. A refusal says whether it is a fence (no path to yes from inside
a session) or a negotiation (run this). Exit 0 is pass, 1 is block, 2 is a
refusal: the gate could not answer.

## What it does

- Runs the repository's own format, lint and test checks on HEAD's tree, plus
  deterministic diff-integrity rules on the change since the last covered
  commit: removed or weakened tests (`JT-TEST`), new lint suppressions
  (`JT-SUP`), weakened CI (`JT-CI`), protected paths touched (`JT-PROT`),
  credential-shaped strings (`JT-SEC`), oversized files and functions
  (`JT-SIZE`), comment discipline (`JT-CMT`), lines added (`JT-ADD`).
- Decides with a pure function. The verdict reads no clock, no network and no
  model, so `jtac replay` recomputes the same answer from the recorded nodes.
- On a pass, signs one root per push tip with an SSH key (`ssh-keygen -Y`, no
  new key format) and stores it under `refs/jtac/roots/`, pushed and fetched
  beside the code. The `pre-push` hook refuses a branch tip that no root
  covers, by commit or by tree, and a push that rewinds a protected branch.
- Says, per commit, whether the gate was on (`covered`), deliberately off
  (`declared_off`, a signed waiver), or not there (`absent`). Bypass is
  recorded, not prevented: `git push --no-verify` lands with no root, and
  `jtac verify` shows it.
- Judges the change, not the author. A waiver is signed, carries a reason, and
  covers one commit, or one rule under one path for at most 30 days. A waiver
  pulled under an agent session is flagged by every reader.

| Command | Does |
| --- | --- |
| `jtac init` | Signing key, local store, default policy from what the tree holds, the `pre-push` hook, the evidence refspec; `--ci github` adds the CI workflow, the first `.jtac/signers` and a CI key (`docs/CI.md`). `--dry-run` writes nothing |
| `jtac check` | The gate on HEAD; a pass writes a signed root, and `--push <remote>` sends it. `--preview` writes nothing and is the one mode that runs on a dirty tree |
| `jtac status` | Whether a push would pass; unpushed commits by coverage; waivers in effect |
| `jtac why` | The last verdict's reasons, each with its rule, why, and the fix |
| `jtac waive` | The signed off-switch: one commit, one rule under one path for up to 30 days, or with `--ci` the CI gate on a commit merged without it, with a reason |
| `jtac verify [range]` | Every root bearing on the range: tree, signature, waivers, chain; exit 1 on an invalid one, or on an absent commit with `--require-coverage` |
| `jtac replay <commit>` | Recomputes a root's Merkle root, counts and verdict from the local nodes, on the machine that ran the check |
| `jtac audit [range]` | The coverage table: covered, declared off, absent, by which root, signed by whom; a `ci` tag where a CI waiver stands in for CI's root |
| `jtac show [range]` | The root refs a push would send, byte for byte |
| `jtac release <tag>` | The release gate: every commit since the previous attested release (or `--from`) covered or declared off by a root the tag's tree trusts, else refused naming each; `--require-ci` holds the first-parent line to CI's roots; `--push <remote>` signs the attestation in `jtac-release-v1` and sends it; `--verify` holds a tag's attestation to this clone's roots |
| `jtac bench` | The public proof: ten controls attacked through their real path in a scratch repository, each row quoting the gate |
| `jtac init --agent claude-code` | Registers the advisory `PreToolUse` hook: Claude Code is told about a fence or a missing root before it pushes, and is never blocked |

Every command but `hook` takes `--json` and prints the structure the text is
rendered from.

## Checking someone else's evidence

Only roots and signatures travel. In any clone:

```sh
git fetch origin '+refs/jtac/*:refs/jtac/*'   # jtac init adds this refspec
jtac verify main                               # exit 1 on a bad root
jtac audit main                                # the coverage table
jtac release v0.2.9 --verify                   # the release attestation against the roots
```

```
AUDIT main: 2 commits, 1 covered, 0 declared off, 1 absent (50% covered); 0 roots invalid, 0 signed by unknown keys
  signers HEAD:.jtac/signers
  0cdeae1cdcec320108372029cb2adb315de3ac5f covered      own root  dev@example.com trusted  policy 1  gate 0.1.2
  ad364106b22a2bf283bbc48cefcf1cbd2b9cd955 absent       no root
```

Trusted keys are the allowed-signers lines in `.jtac/signers` at HEAD, so a
repository declares who may sign for it and the declaration is reviewed like
any other change.

**What a v0.1 root proves.** That a holder of the signing key ran this gate
binary with this policy on this tree and recorded these results. It does not
prove the results are true; the CI gate narrows that gap by re-running the
gate on the forge's runner under a CI key that pull request runs cannot reach,
though the machine that ran `jtac init --ci github` keeps a copy until
[#130](https://github.com/dmriding/jtac/issues/130).

## CI gate

The same binary in a GitHub Action, under a CI key kept in an environment
secret that only the default branch and `v*` tags reach:
`jtac ci` judges every pull request again and compares the pushed root with its
own run, and a merge commit gets a root the CI key signed. One command writes
the workflow, the trust file and the CI key, then prints the forge settings to
apply: an environment holding the key, a CODEOWNERS line, and two rulesets:

```sh
jtac init --ci github
```

[`docs/CI.md`](docs/CI.md) has the steps it prints, what the check says, fork
pull requests, bypass merges, and what reads as the developer's.

## Numbers

Measured on this repository at v0.1, release build, Linux, 2026-10-19:

| Number | Value |
| --- | --- |
| Controls `jtac bench` attacks, and holds | 10 of 10 |
| Time `jtac check --preview` adds beyond the repository's own checks | 0.17 s of 85 s (two runs: 0.17, 0.18) |
| `jtac bench` end to end, ten repositories | 1.1 s |

Catch rate and false block rate are unmeasured: a number from one developer's
repositories would mean nothing. The method, for anyone who runs it: over a
period, count the pushes `jtac check` blocked and how many of those blocks a
human overruled with `jtac waive`; `jtac audit` shows every waiver. Catch rate
is blocks over changes a human judged bad, which needs a label per change;
false block rate is overruled blocks over blocks. Send both with the
repository's language and the agent that wrote the code.

## Roadmap

- **v0.1, the local gate** (released): the checks, the diff rules, signed
  roots, the `pre-push` hook, the shell command classifier, the advisory
  tool-call hook for Claude Code, `jtac bench`, the install script.
- **v0.2, CI gate** (CI gate and release gate released): the same binary in a
  GitHub Action verifies the roots, re-runs the gate on the same tree, fails
  the check when a local root lied, and signs a root for the merge commit
  under its own key; the release gate attests a tag only when every commit in
  its range is covered or declared off. Tiering by blast radius is next, then
  `jtac policy`, `jtac scope` and `jtac who` (`DESIGN.md` section 7).
- **v0.3, review round and packet.** Model reviewers against recorded
  evidence; a human sees only what could not be proven.
- **v0.4, collector.** Optional, self-hosted, server to server. Developer
  machines never report to it.

## Privacy

A developer's machine reports to nothing. Evidence leaves only with a push,
only roots and signatures are shared, and `jtac show` prints exactly what a
push will carry. See `DESIGN.md` section 19.

## Support the work

jtac is free to use. If it saved you a bad merge, the maintainer takes
coffee: https://buymeacoffee.com/netviper. Organisations that want to offer
it as part of a product need a commercial license, which is the
competing-use case the license reserves.

## Contributing

Read [`CONTRIBUTING.md`](CONTRIBUTING.md), then [`DESIGN.md`](DESIGN.md).
Design decisions are recorded in `DESIGN.md` section 21, licensing and
contribution terms in [`docs/adr/`](docs/adr/), and proposals to change the
design go through [`docs/rfc/`](docs/rfc/). Security reports go through
[`SECURITY.md`](SECURITY.md).

## License

jtac is source-available under the Functional Source License, version 1.1,
with an Apache 2.0 future grant ([`LICENSE.md`](LICENSE.md), SPDX `FSL-1.1-ALv2`). You may
use, copy, modify, and redistribute it for any purpose except offering it, or
a derivative, as a commercial product or service that competes with it. Each
release becomes Apache 2.0 two years after publication. This is not an
OSI-approved open source license. For a commercial license, contact the
maintainer.
