# jtac

Provenance and verification for AI-written code.

Your agent cannot push slop, and you can prove it. jtac is a local gate that
checks a change before it leaves the developer's machine and records signed
evidence that travels with the commits. Anyone with the repository can verify
that evidence offline. A CI gate that re-runs the checks under its own
identity and a review packet for humans follow in later releases.

**Status: v0.1 in progress, no release yet.** The local gate runs end to end:
`init`, `check`, the `pre-push` hook, signed roots as git refs, `verify`,
`replay`, `audit`, `show`, `waive`. Left before v0.1: the shell command
classifier, the agent tool-call hook, `bench`, and the install script
(`DESIGN.md` section 14). `DESIGN.md` is the source of truth. `main` is the
development line; releases are signed `v*` tags (`docs/RELEASING.md`).

## What it does today

- Runs the repository's own format, lint and test checks on HEAD's tree, plus
  deterministic diff-integrity rules on the change since the last covered
  commit: removed or weakened tests (`JT-TEST`), new lint suppressions
  (`JT-SUP`), weakened CI (`JT-CI`), protected paths touched (`JT-PROT`),
  credential-shaped strings (`JT-SEC`), oversized files and functions
  (`JT-SIZE`), comment discipline (`JT-CMT`), lines added (`JT-ADD`).
- Decides with a pure function. The verdict reads no clock, no network and no
  model, so `jtac replay` recomputes the same answer from the recorded nodes.
- On a pass, signs one root per push tip with an SSH key (`ssh-keygen -Y`,
  no new key format) and stores it under `refs/jtac/roots/`, fetched beside
  the code. A `pre-push` hook refuses a tip that has no root.
- Says, per commit, whether the gate was on (`covered`), deliberately off
  (`declared_off`, a signed waiver), or not there (`absent`). Bypass is
  recorded, not prevented.
- Refuses with three parts, every time: what happened, why the rule exists,
  and the exact command to run next. A refusal says whether it is a fence or
  a negotiation.

| Command | Does |
| --- | --- |
| `jtac init` | Signing key, local store, default policy from what the tree holds (Rust, Python, Go, JavaScript/TypeScript), the `pre-push` hook, the evidence refspec |
| `jtac check` | The gate on HEAD; a pass writes a signed root. `--preview` writes nothing |
| `jtac status` | Whether a push would pass; unpushed commits by coverage; waivers in effect |
| `jtac why` | The last verdict's reasons, each with its rule, why, and the fix |
| `jtac waive` | The signed off-switch: one commit, or one rule under one path for up to 30 days |
| `jtac verify [range]` | Every root bearing on the range: tree, signature, waivers, chain; exit 1 on an invalid one |
| `jtac replay <commit>` | Recomputes a root's Merkle root, counts and verdict from the local nodes |
| `jtac audit [range]` | The coverage table: covered, declared off, absent, by which root, signed by whom |
| `jtac show [range]` | The root refs a push would send, byte for byte |

Every command takes `--json` and prints the structure the text is rendered
from. Exit 0 is pass, 1 is block, 2 is a refusal: the gate could not answer.

**What a v0.1 root proves.** That a holder of the signing key ran this gate
binary with this policy on this tree and recorded these results. It does not
prove the results are true; the CI gate in v0.2 exists to close that gap.

## Quick start

Needs git, OpenSSH 8.9 or later (`ssh-keygen`), and a Rust toolchain
(1.98 or later) until the install script lands.

```sh
cargo install --locked --path crates/jtac
cd your-repo
jtac init                     # key, store, .jtac/policy.toml, hook, refspec;
                              # prints the allowed-signers line for your key
echo '<that line>' > .jtac/signers
git add .jtac && git commit -m "Adopt jtac"
jtac check                    # ABORT: adding .jtac/ is itself a protected-path finding
jtac waive --rule JT-PROT-001 --path .jtac/ --expires 14d --reason "adopting jtac"
jtac check                    # CLEARED HOT, root written, waiver recorded in it
git push                      # the hook pushes the root ref ahead of the code
```

From then on every push needs a root, `jtac status` says whether the next one
passes, and `jtac why` explains a block. Linux is primary; the test suite also
runs on Windows.

## Roadmap

- **v0.1, the local gate** (this line): shell command classifier for the
  hook, the agent tool-call hook (Claude Code `PreToolUse` first; advisory,
  never blocking), `jtac bench`, install script, measured catch and
  false-block rates.
- **v0.2, CI gate.** The same binary as a GitHub Action: fetches the roots,
  verifies, re-runs the required checks under its own key, reports a status
  check, issues a signed root for the merge commit. Tiering by blast radius.
- **v0.3, review round and packet.** Model reviewers against recorded
  evidence; a human sees only what could not be proven; a release gate.
- **v0.4, collector.** Optional, self-hosted, server to server. Developer
  machines never report to it.

No model call anywhere in v0.1. The gate judges the change, not the author.

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

Read `CONTRIBUTING.md`, then `DESIGN.md`. Decisions are recorded in
`docs/adr/`, and changes to the design go through `docs/rfc/`.

## License

jtac is source-available under the Functional Source License, version 1.1,
with an Apache 2.0 future grant (`LICENSE.md`, SPDX `FSL-1.1-ALv2`). You may
use, copy, modify, and redistribute it for any purpose except offering it, or
a derivative, as a commercial product or service that competes with it. Each
release becomes Apache 2.0 two years after publication. This is not an
OSI-approved open source license. For a commercial license, contact the
maintainer.
