# jtac: design

Provenance and verification for AI-written code.

The name is short for Joint Terminal Attack Controller: the controller who clears a release only on positive identification and under the rules in force. Here the agent does the flying, and nothing leaves the machine until it is cleared. The word "terminal" also happens to be where the tool lives.

Status: v0.1 released; v0.2's CI gate and release gate released (v0.2.9); tiering and the later v0.2 patches next. Date: 2026-11-09. License: FSL-1.1-ALv2.

This document is the source of truth for building jtac. Sections 1 to 6 say what the system is and why. Sections 7 to 14 specify v0.1 in enough detail to build. Sections 15 to 20 design the later releases, so v0.1 does not paint them into a corner. Section 21 lists the decisions made here that the maintainer should confirm, and section 22 lists open questions.

---

## 1. Summary

jtac is for solo developers and teams who work at AI speed and are held back by pull request review and by code nothing independent has checked. It is the quality and safety gate that lets them keep that speed: it runs their own checks before code leaves the machine and records what ran as signed evidence, instead of trusting a model or a rubber stamp.

It works at three points on one evidence chain:

1. **Local gate.** Checks a change before it leaves the developer's machine and records signed evidence.
2. **CI gate.** Verifies that evidence independently and decides: merge, human required, or reject.
3. **Review packet.** Hands a human reviewer only what could not be proven.

One sentence for the README: jtac is a pre-push gate whose passes are signed records, kept in git beside the commits, that anyone with a clone can verify.

v0.1 is the local gate alone. It is complete and useful by itself for one developer, needs no account, no server, and no model.

## 2. Problem

AI agents produce changes faster than people can read them, and review processes built for human-speed work have not adapted.

- **Unchecked code reaches main.** Code that passed a self-check is merged without anything independent looking at it.
- **Human review collapses.** At high volume, reviewers click approve or point another agent at the diff.
- **Audit evidence gets faked.** Under certification pressure, teams are tempted to let an agent approve under a colleague's account so the record shows a human.

The gap is not a better reviewer bot. It is a way to make the honest path as fast as the dishonest one.

## 3. Goals and non-goals

**Goals**

- Block low-quality changes before they leave the developer's machine.
- Record evidence that travels with the work and verifies offline.
- Make every verdict replayable from recorded evidence.
- Record honestly who or what did each thing: agent, model, human, or mixed.
- Prove, per commit, whether the system was on, deliberately off, or not there.
- Keep developers in control: nothing leaves a machine except with a push.
- Work with any agent, harness, or workflow.

**Non-goals**

- Detecting whether code is AI-written. Provenance is recorded, not guessed.
- Writing new static analyzers. Existing tools are wrapped.
- Being an LLM reviewer bot. Models advise. Deterministic policy decides.
- Guaranteeing a certification outcome. jtac produces evidence. An auditor judges it.
- Monitoring developers. There is no activity, timing, or presence telemetry.
- Learning continuously inside the gate. Learned behavior ships as versioned policy.

## 4. Principles

These are constraints on every design and code decision. When two conflict, the lower number wins.

1. **The verdict is a pure function.** `gate(tree, policy, evidence) -> verdict` returns the same answer every time it is replayed. It reads no clock, no network, and no model.
2. **Model output is evidence, never computation.** If a model is involved, its output is recorded once, content-addressed, and treated as input.
3. **Fail closed where a check could silently not happen.** A check that did not run must never look like one that passed.
4. **Self-attestation proves the gate ran, not that the result is honest.** Anything signed on the author's machine is a claim. Independence comes only from a different identity on a different machine.
5. **Honest attribution everywhere.** No action is ever recorded under an identity that did not perform it.
6. **The refusal is the product.** Every block says what was refused, why the rule exists, and the exact command to run next.
7. **The agent sees everything and touches nothing.** An agent can read policy, rules, and the reason for every failure. It cannot write the gate's binary, keys, policy, or evidence.
8. **The proper way is the easy way.** An agent that understands the failure and has a clear route to fix it does not go looking for a way around.
9. **Evidence travels with the work.** No machine reports anywhere. Evidence crosses the boundary only when commits do.
10. **Shrink the review problem instead of scaling review.** Remove volume and noise before push so little reaches a human.
11. **No security or correctness feature is ever paywalled or withheld.**

## 5. Control stance

As models get more capable, the control requirement goes up, because a capable model satisfies a check instead of breaking it: it edits the test, adds the suppression, mocks the unit under test, or loosens the CI config. jtac takes that position on purpose.

Consequences:

- Checks on the diff look for the ways a check gets satisfied dishonestly (section 10.3).
- Changing the gate's own policy is the highest-risk change there is.
- Levers exist for humans. Every lever pull is a signed, recorded event. No agent tool surface reaches one.
- The rules are open. Four things are not disclosed to an agent: signing keys, tripwire locations, seeded defects, and which results CI will spot-check.

## 6. Threat model

| Actor | Can do | jtac's answer |
| --- | --- | --- |
| A careless or overeager agent | Produce plausible wrong code, weaken tests, add suppressions, edit CI config | Local gate blocks with an explained reason and a fix |
| An agent steered by injected content | Try to bypass the gate, self-approve, or flip a lever | No agent-facing tool reaches a lever. Bypass leaves the commit provably uncovered |
| A developer in a hurry | Skip the hooks with `--no-verify`, or uninstall jtac | Cannot be prevented locally. The commit is recorded as absent; from v0.2, CI judges the tree itself and records the absence |
| A developer forging evidence | Sign a root for checks that did not pass | A local root is a claim. From v0.2, CI re-runs the gate and compares state, counts and policy hash. A disagreement fails the check |
| Someone with push access | Delete or overwrite evidence refs | Evidence is signed and bound to a commit, so it cannot be altered. Deletion yields absent. From v0.4 the collector keeps the durable copy |
| A compromised policy approver | Ship a weakened policy | Releases need multiple signatures, cannot roll back, and are visible as a diff |

Out of scope: a compromised operating system or toolchain on the developer's machine, and malicious code inside a wrapped analyzer. v0.1 is not a sandbox. It wraps the tools a repository already trusts.

**What v0.1 does and does not prove.** A valid v0.1 root proves that a holder of the signing key ran this gate binary with this policy on this tree and recorded these results. It does not prove the results are true. That limit is stated in `jtac verify` output and in the README, and it is what v0.2 exists to close.

---

## 7. Concepts and vocabulary

| Term | Meaning |
| --- | --- |
| Node | One content-addressed evidence record: a check result, a finding, a lever pull |
| Root | The signed record for one push tip. It names the commit, tree, policy, gate version, and the Merkle root of its nodes |
| Coverage state | Per commit: `covered`, `declared_off`, or `absent` |
| Policy | The rules in force: which checks run, protected paths, tiers, fences |
| Lever | A human-only control: waive, change policy, change trust |
| Fence | A refusal that is never negotiable from inside a session |
| Negotiation | A refusal that names the exact command that satisfies it |
| Tier | Blast radius class of a change, decided by policy from paths and content |

### 7.1 Commands

| Command | Does |
| --- | --- |
| `jtac init` | Installs hooks, sets up the signing key and local store, writes a default policy if none exists; `--ci github` adds the CI workflow, the first `.jtac/signers` and a CI key (section 15) |
| `jtac check` | Runs the gate on HEAD. It judges code and writes a root on pass; `--from <head> --base <old tip>` is the merge job's form (section 15) |
| `jtac status` | Says whether a push would pass and what is standing: waivers, coverage of unpushed commits |
| `jtac verify [range]` | Checks signatures and chain on roots, yours or anyone's. It judges evidence |
| `jtac replay <commit>` | Recomputes the verdict from recorded nodes and compares it with the root |
| `jtac audit [range]` | Coverage table for a range: covered, declared off, absent |
| `jtac why` | Explains the last block and the fix |
| `jtac show [range]` | Prints exactly what the next push will carry, byte for byte |
| `jtac waive` | The recorded, signed off-switch, with a reason |
| `jtac bench` | Model-free self-test: attacks every control through its real path |
| `jtac backtest <range>` | Runs the diff-integrity rules over merged history and reports what they would have said; writes nothing (section 11.10) |

Every command accepts `--json` and returns the same structure the human output is rendered from.

### 7.2 Shorthand

Each shorthand is an alias for a plainly named command. Documentation and error messages use the plain names.

| Shorthand | Plain command | Shows | Arrives |
| --- | --- | --- | --- |
| `jtac roe` | `jtac policy` | The rules in force and where they came from | v0.2 |
| `jtac ao` | `jtac scope` | The paths and branches an agent is cleared to work in | v0.2 |
| `jtac pid` | `jtac who` | Which agent, model, and human produced which lines | v0.2 |
| `jtac dope` | `jtac ledger` | How each workflow, model, and practice has performed | v0.4 |
| `jtac bda` | `jtac report` | What a change did after it landed | v0.4 |

Pass and block banners are `CLEARED HOT` and `ABORT`, each followed by a plain-language reason.

---

## 8. Architecture

```
            policy (signed, versioned)
                      |
   +------------------+-------------------+
   |                  |                   |
local gate  ---->  CI gate  ---->  review packet
 (v0.1)            (v0.2)            (v0.3)
   |                  |                   |
   +------------------+-------------------+
                      |
        evidence: nodes -> root -> git refs
                      |
            collector (v0.4, optional)
```

- One binary, `jtac`, in several modes: local CLI and hooks, CI verifier, and later the collector.
- One evidence format shared by every mode.
- Git is the transport. Evidence roots are git objects under a dedicated ref namespace, pushed beside the commits they describe.
- Analyzers are wrapped, pinned, and run as subprocesses with argv arrays, never through a shell.

### 8.1 Workspace layout

Rust 2024 edition, one cargo workspace. Start with these crates and split further only when a crate holds more than one concern.

| Crate | Concern | Purity |
| --- | --- | --- |
| `jtac-core` | Types, canonical encoding, hashing, Merkle tree, the verdict function | Pure. No I/O |
| `jtac-policy` | Parse, validate, hash, and evaluate policy | Pure |
| `jtac-gate` | Shell command classifier and the decision function for tool calls and git operations | Pure |
| `jtac-checks` | Check runner: subprocess execution, caching, diff-integrity checks | Impure |
| `jtac-store` | Local content-addressed store and the git ref layer | Impure |
| `jtac-sign` | SSHSIG signing and verification | Impure |
| `jtac` | The `jtac` binary (the crates.io package), hook entry points, rendering | Impure |

The binary re-exports the library crates. Tests exercise the same code the binary runs.

**Layering rule.** Everything that decides is pure and takes its facts as arguments. Everything that touches a process, a file, or git gathers facts and decides nothing. This is what lets every rule be tested against struct literals with no repository on disk.

---

## 9. Evidence model

### 9.1 Encoding and identity

- Nodes and roots are JSON, canonicalized with the JSON Canonicalization Scheme (RFC 8785).
- A node's id is `sha256:<hex>` over its canonical bytes.
- Hashes of git objects are recorded as git reports them, with the algorithm named, so SHA-1 and SHA-256 repositories both work.
- Digests are lowercase hex. A node id is `sha256:` followed by 64 hex digits.
- Numbers are integers up to 2^53-1. A float or a larger integer anywhere in a node, root, or policy is refused: RFC 8785 formats every number as a double, and nothing here needs one.
- `ts` fields are whole-second UTC in one spelling, `2026-10-10T18:30:00Z`. Any other spelling is refused.
- A field with no value is omitted, never `null`.
- A reader accepts a document only when its canonical bytes equal the input bytes. Duplicate keys, dropped keys, whitespace, and alternate spellings are refused, so a stored document hashes to its id by construction.

### 9.2 Node

```json
{
  "v": 1,
  "kind": "check",
  "tree": "sha1:<git tree id>",
  "policy": "sha256:<policy hash>",
  "ts": "2026-10-10T18:30:00Z",
  "body": { }
}
```

A node's `ts` is informational, with one exception: a waiver's `ts` is when it was written, and its 30-day cap runs from there (section 11.6). The verdict function reads the root's `ts` (section 9.4) for everything else.

Node version 2 (section 15, decision 48) adds one optional field, `gate`, to a waiver's commit scope. It is written on the levers that carry it and nowhere else, so every other node keeps its version 1 id. A version 1 reader refuses a version 2 node, and a root carrying one reads invalid to it: absent, the honest answer from a reader that cannot read the reason.

v0.1 node kinds and bodies:

| Kind | Body fields |
| --- | --- |
| `check` | `id`, `argv`, `tool_version`, `exit`, `outcome` (`pass`, `fail`, `skipped`, `error`), `stdout` and `stderr` (each `{ "sha", "bytes", "truncated" }`), `duration_ms`, `cached_from` (id of the original node; absent on a fresh run) |
| `finding` | `check` (node id), `rule`, `severity` (`blocker`, `major`, `minor`), `path`, `line`, `message` |
| `lever` | `action` (`init`, `waive`, `policy_change`, `trust_change`), `signer` (`{ "principal", "fingerprint" }`), `reason`, `scope` (`{ "commit": <id> }`, from version 2 `{ "commit": <id>, "gate": "ci" }`, or `{ "rule": <rule>, "path": <prefix> }`; absent when the action has none), `expires` (absent when none), `agent` (the environment marker of the agent session the lever was pulled in, section 11.6; absent when none) |

Later kinds, reserved now so the schema does not need a break: `attribution`, `claim`, `review`, `disposition`, `egress`.

Rules:

- `skipped` and `error` are not `pass`. A required check that is skipped or errors fails the verdict.
- `stdout.sha` and `stderr.sha` reference blobs in the local store. `bytes` is what was stored and `truncated` says the capture cap cut it, so a truncated capture is distinguishable from a short one.
- A `finding` always points at the `check` node that produced it. Diff-integrity findings point at the reserved `integrity` check, which `jtac check` writes with an empty `argv`, jtac's own version as `tool_version`, `pass` when the rules ran, and `error` when the base or a blob could not be read (section 10.3).

Known answer (test `node_id_known_answer`). This check node, canonical:

```
{"body":{"argv":["cargo","test","--workspace"],"duration_ms":1234,"exit":0,"id":"cargo-test","outcome":"pass","stderr":{"bytes":0,"sha":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","truncated":false},"stdout":{"bytes":0,"sha":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","truncated":false},"tool_version":"1.98.0"},"kind":"check","policy":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1}
```

has id `sha256:7dbd2c1d6519975840d8a0b6df2df44e9c32b9aff38102eabc5554058cb5696e`.

Known answer (test `node_v2_id_known_answer`). A CI waiver on the same tree and policy, canonical:

```
{"body":{"action":"waive","reason":"example reason","scope":{"commit":"sha1:0000000000000000000000000000000000000001","gate":"ci"},"signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"}},"kind":"lever","policy":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":2}
```

has id `sha256:26e125af62c32f0afebd37d2424e3b8e857fc3951aad464f207443de115f8715`.

### 9.3 Merkle tree

The nodes for one root form a Merkle tree in the RFC 6962 shape over the raw 32-byte digests of their ids. This gives inclusion proofs, so a developer can later reveal one node and prove it was part of a root without revealing the rest.

- Leaves are the digests sorted ascending as bytes. A duplicate digest is a refused root.
- `MTH([]) = SHA-256("")`.
- `MTH([d]) = SHA-256(0x00 || d)`.
- `MTH(D[0:n]) = SHA-256(0x01 || MTH(D[0:k]) || MTH(D[k:n]))` for `n > 1`, where `k` is the largest power of two smaller than `n`.
- An inclusion proof is the RFC 6962 audit path: leaf index, tree size, and the sibling hashes from the leaf upward.

Known answers (test `merkle_known_answers`). Leaves are the SHA-256 digests of the single ASCII bytes `a` through `e`, which sort to `d, c, b, e, a`:

| n | root |
| --- | --- |
| 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| 1 | `de22f76c222682c331f7dda7349654b6a9f4f710077025e9b29130023712780f` |
| 2 | `19abe2ce03b25e3d5616841e998bf9a6d82385a4aa54420cb4543fde06731b47` |
| 3 | `5a5a959d18f4993fbdb0337315966dd7d3555b530ab5d095e47490150806e868` |
| 4 | `f777efb8147617bc84463a6e7954eccad236ca12e1e87c22e98c2dcbbc39ac50` |
| 5 | `006b72c2b127f9e2011db36182468cb6c3a92836fc3e39eb4fdbc3ea604f62b8` |

Audit path for leaf index 1 of 3: `de22f76c…780f`, `a0d9f0a50b35b9f7d7edc57fb64f4771ddef0fefeaca4e6f949a1514db5b136d`. Audit path for leaf index 4 of 5: `f777efb8…ac50` alone.

### 9.4 Root

One root per push tip.

```json
{
  "v": 1,
  "commit": "sha1:<tip commit>",
  "tree": "sha1:<tip tree>",
  "ts": "2026-10-10T18:30:00Z",
  "covers": ["sha1:<commit>", "..."],
  "parents": [{ "commit": "sha1:<ancestor>", "root": "sha256:<its root id>" }],
  "policy": { "hash": "sha256:...", "version": 1, "source": "repo" },
  "gate": { "version": "0.1.0", "binary": "sha256:..." },
  "state": "covered",
  "evidence": "sha256:<merkle root>",
  "levers": [{ "node": { "...": "a lever node" }, "sig": "<SSHSIG>" }],
  "counts": { "checks": 6, "pass": 6, "fail": 0, "findings": { "blocker": 0, "major": 0, "minor": 2 }, "waivers": 1 },
  "signer": { "principal": "dev@example.com", "fingerprint": "SHA256:<ssh key fingerprint>" }
}
```

- `ts` is the instant of the verdict and the `now` the verdict function was given (section 11.2). Replay hands it back, so a waiver's expiry is judged the same on every machine.
- `levers` holds every waiver the verdict applied and every CI waiver (section 15) naming this root's commit, each as its lever node in full plus the SSHSIG over the node's canonical bytes (namespace `jtac-lever-v1`). A waiver may have been written on an earlier tree. They are leaves of `evidence` like any other node and the only nodes that leave the machine. Other lever kinds stay local.
- `counts` is derived: `checks` is the number of check nodes, `pass` those with outcome `pass`, `fail` the rest, `findings` by severity, `waivers` the length of `levers`. `jtac replay` recomputes it, and a mismatch invalidates the root.
- `policy.version` is the policy schema version (section 10.2).
- `signer.principal` is the identity looked up in the allowed-signers file (section 11.8). `signer.fingerprint` is for people and audit rows.

- `covers` lists the commits between the nearest covered ancestors and the tip, oldest first, ending at the tip. Checks run on the tip's tree, and the root vouches for the path that led to it. Past a bound (default 256) the root records `{ "base", "tip" }` instead of the list: the oldest and the newest commit; the set between them is not recoverable from the root, so a reader counts the commits on a path from `base` to `tip` (`rev-list --ancestry-path`), both ends included, and a side line merged in between them reads absent. The walk stops at a commit with a root of its own, never at a tree match (decision 33); a root for a commit that is not a proper ancestor of HEAD, or that the clone lacks, is ignored by the walk. The first root in a repository covers what its rules judged, the diff base to the tip; the history before it stays absent.
- `parents` links to the roots of the nearest covered ancestors. An empty list is the first root in a repository or a visible break in coverage.
- `state` is `covered` or `declared_off`. There is no root for `absent`. Absence is the missing root.
- A `declared_off` root carries a waiver whose scope is `commit`, names this root's commit and names no gate. A rule-scoped waiver leaves the root `covered`; so does a CI waiver, which speaks for another gate.

Known answer (test `root_id_known_answer`). A `declared_off` root over the empty tree with one commit waiver and no checks, canonical:

```
{"commit":"sha1:0000000000000000000000000000000000000001","counts":{"checks":0,"fail":0,"findings":{"blocker":0,"major":0,"minor":0},"pass":0,"waivers":1},"covers":["sha1:0000000000000000000000000000000000000001"],"evidence":"sha256:60d73f36e3a666adcfe47ee42cd116a0c910364dcb1f096477d9d1e56e9b7d40","gate":{"binary":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","version":"0.1.0"},"levers":[{"node":{"body":{"action":"waive","reason":"example reason","scope":{"commit":"sha1:0000000000000000000000000000000000000001"},"signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"}},"kind":"lever","policy":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1},"sig":"example-signature"}],"parents":[],"policy":{"hash":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","source":"repo","version":1},"signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"},"state":"declared_off","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1}
```

has id `sha256:01e449f228a88b336568a36424831f2c866c94d24b0916312442da7d89b67091`. Its lever node has id `sha256:936b6c55423cccc86fb7a22c9f90de3a359073f814ffd9be80ac9f316b9a7922`, and `evidence` is the one-leaf Merkle root over it.
- The root is signed as a detached SSHSIG signature, namespace `jtac-root-v1`, or `jtac-ci-root-v1` for a root with `from` (section 11.8), over its canonical bytes. Root version 2 (section 15) adds `base` and `from`, both absent from the bytes when unset, so a version 1 root's id is unchanged.

Known answer (test `root_v2_id_known_answer`). A `covered` version 2 root over the empty tree, no checks and no waivers, whose rules diffed from `sha1:0000000000000000000000000000000000000000`, canonical:

```
{"base":"sha1:0000000000000000000000000000000000000000","commit":"sha1:0000000000000000000000000000000000000001","counts":{"checks":0,"fail":0,"findings":{"blocker":0,"major":0,"minor":0},"pass":0,"waivers":0},"covers":["sha1:0000000000000000000000000000000000000001"],"evidence":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","gate":{"binary":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","version":"0.1.0"},"levers":[],"parents":[],"policy":{"hash":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","source":"repo","version":1},"signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"},"state":"covered","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":2}
```

has id `sha256:d82da976599cef7f551d5ce1e0d8b25c498f70d0ce505f1dacaf50aecce48f49`.

### 9.5 Coverage states

| State | What exists | What it proves |
| --- | --- | --- |
| Covered | A valid signed root that names or covers the commit | The gate ran with this version and policy, and passed |
| Declared off | A valid signed root with a waiver | The gate was turned off on purpose, by this key, for this reason |
| Absent | No root, or one that fails verification | The gate was not there |

**Two gates, one table.** The CI gate (section 15) has the same three states for a commit on the default branch: covered by a root in the CI namespace, declared off by a CI waiver carried in a root on the commit, absent otherwise. A CI root on the commit outranks a CI waiver, which then reads as a note.

**Tree fallback.** Checks judge a tree, so a commit with no root of its own counts as covered if its tree equals the tree of a covered root. This handles squash and rebase merges that produce a new commit with identical content. `jtac audit` reports these as `covered (by tree)`.

### 9.6 Storage

**Local.** `<git common dir>/jtac/` (`.git/jtac/` in a plain clone; linked worktrees share the main one) holds the content-addressed store: `objects/` for nodes, captured output, and the resolved policy each root was judged under, each file named by the SHA-256 of its bytes, so `replay` reads the policy by the hash the root names; `roots/<root id>/` for `root.json`, `root.sig`, and `nodes.json`, the node ids that are the root's Merkle leaves, which replay needs and the root does not carry; `cache/` for the check cache; and `state.json`, the last verdict as `jtac check` printed it with the root id, versioned like every output, which `status` and `why` render. It lives inside `.git` so it is never committed and is per clone. Every read re-hashes what it finds: a file that is not what its name says is an error, never a document.

**Shared.** Each root is a parentless git commit whose tree holds `root.json` and `root.sig`, written with plumbing and unsigned at the git level (`root.sig` is the signature), authored `jtac <signer principal>` at the root's `ts`, so one root and signature give one commit id on every machine. It is referenced at:

```
refs/jtac/roots/<first two hex of commit>/<commit>
```

- One ref per covered tip means no contention between developers pushing different tips at once. Two machines that each check the same tip both write a root for it, and the second push is rejected as a non-fast-forward: the remote already holds a root for that commit, which is reported and is not a failure of the gate.
- A ref under the namespace that is not `<xx>/<commit>` was not written by jtac. Readers skip it and say so in a note; it never stops the tip from being judged, since the fetch refspec would otherwise spread one stray ref to every clone as a refusal.
- `jtac init` adds the fetch refspec `+refs/jtac/*:refs/jtac/*` to the remote. It does not add a push refspec, because that would change what a bare `git push` does.
- The `pre-push` hook pushes the root refs for the commits being pushed, in a separate `git push` with an environment guard against recursion.
- If the root push fails, the developer's push still proceeds and jtac warns loudly. The commits will read as absent until the roots are pushed. A strict mode that blocks instead is a policy option.

Only roots and signatures are pushed. Applied waivers travel inside the root with their signatures. Every other node, captured output, and attempt history stays local.

---

## 10. Policy

### 10.1 Source and trust

v0.1 reads policy from `.jtac/policy.toml` in the repository. If there is none, `jtac init` writes a default built from what it detects. From v0.2 a policy can also come from a signed release of an org policy repository (section 18.3).

The policy hash is the SHA-256 of the canonical JSON (section 9.1) of the resolved policy: the file as written with every default filled in. A gate release that changes a default changes the hash, because the rules in force changed. The hash goes into every node and root.

No policy file is a refusal that names `jtac init`. Built-in defaults are what `init` writes, never what `check` assumes.

`.jtac/` is always a protected path. A change that touches it escalates, and from v0.2 it cannot be attested by the same run that made it.

### 10.2 Shape

```toml
version = 1

[checks.fmt]
run = ["cargo", "fmt", "--all", "--", "--check"]

[checks.lint]
run = ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]

[checks.test]
run = ["cargo", "test", "--workspace"]
timeout_secs = 1200

[integrity]
block_at = "major"
max_new_suppressions = 0
allow_removed_tests = false
allow_newly_skipped_tests = false
max_file_lines = 800
max_function_lines = 150

[protected]
paths = [
  ".jtac/**", ".github/**", ".gitlab-ci.yml", ".circleci/**", ".buildkite/**", "Jenkinsfile",
  "azure-pipelines.yml", "bitbucket-pipelines.yml", ".travis.yml", ".drone.yml", ".woodpecker.yml",
  "clippy.toml", "deny.toml", "rust-toolchain.toml",
]

[tiers.high]
paths = ["**/auth/**", "**/migrations/**", "**/billing/**"]

[fences]
protected_branches = ["main", "release/*"]
force_push_protected = "deny"
no_verify = "deny"

[scope]
branches = ["*"]
paths = ["**"]
```

- `run` is an argv array. There is no shell, no interpolation, and no environment passthrough beyond an allowlist of names built into the runner: `PATH`, `HOME`, `USER`, `LANG`, `LC_*`, `TERM`, `TZ`, `TMPDIR`, `CI`, `NO_COLOR`, the Cargo, rustup, Go, and Python location variables, and the Windows system set. No policy key widens it in v0.1.
- A check id is `[a-z0-9_-]`, at most 64 bytes, because it names node bodies and cache keys. `required` defaults to true and `timeout_secs` to 600. A required check runs every time; there is no file-presence switch, so `init` writes only the checks that apply.
- `block_at` is the lowest severity that fails the verdict (section 11.2): `blocker`, `major`, or `minor`. The default `major` leaves minor as the advisory tier.
- Fence keys take `deny` or `allow`; `deny` is the default and the only value `init` writes.
- Globs are `globset` syntax, `**` crossing separators, matched against repository-relative paths with `/`. A pattern is printable ASCII only, so a diff shows every byte of it: a path with other characters is matched with `*`, and `?` matches one byte: one ASCII character, or part of any other. A pattern that can never match such a path is refused rather than silently guarding nothing: empty, a leading `/`, `./`, or `!`, a trailing `/`, leading or trailing whitespace, a `.` or `..` or empty component, a backslash. Alternation `{a,b}` and character classes `[..]` are refused too: a class can smuggle a leading `/`, and one pattern per branch reads better. The pattern language is literals, `?`, `*`, and `**`. A branch pattern is also refused when `git check-ref-format --branch` would refuse every name it could match: `HEAD`, `-`, a component starting with `.` or ending in `.lock`, `..`, a trailing `.`, a space, `~`, `^`, or `:`.
- `[scope]` is the area an agent is cleared to work in: `jtac scope` prints it and the tool-call advisory (section 11.3) names it. It never narrows what the gate checks; the gate judges the whole tree, and `.jtac/**` is protected whatever the file says.
- A policy file that fails to parse denies. It never falls back to defaults, because a gate that silently stops guarding is worse than none.
- Unknown keys are an error, so a typo cannot disable a rule.

Built-in defaults exist for Rust, Python, JavaScript and TypeScript, and Go: format check, lint, test, each using the project's own configured commands where a manifest declares them. Detection reads the tree and is the CLI's; the mapping from detected facts to a policy is pure and lives in `jtac-policy`. Go gets no format check because `gofmt` exits 0 whether or not files differ, and the defaults never wrap a shell. A JavaScript format check is a `format:check`, `fmt:check`, or `prettier:check` script; `format` and `fmt` rewrite files and are not checks.

### 10.3 Diff-integrity checks

These run on the diff between the tip and its nearest covered ancestor (or the merge base with the default branch when there is none). They are deterministic and need no model.

| Rule | Detects | Default |
| --- | --- | --- |
| `JT-PROT-001` | A change to a protected path | Finding, severity major, escalates the tier |
| `JT-TEST-001` | A test function or file removed | Blocker |
| `JT-TEST-002` | A test newly marked skip or ignore, or focused so the others are skipped | Blocker |
| `JT-TEST-003` | Assertions removed from a test that still exists | Major |
| `JT-SUP-001` | A new lint or type suppression (`#[allow]`, `// eslint-disable`, `# type: ignore`, `//nolint`, and peers) | Blocker above the configured budget |
| `JT-CI-001` | A CI step removed or made non-blocking | Blocker |
| `JT-SEC-001` | A string shaped like a credential added | Blocker |
| `JT-SIZE-001` | A changed file past the production-line cap for its language, with no exemption marker | Major |
| `JT-SIZE-002` | A changed function past the line cap for its language | Major |
| `JT-CMT-001` | A changed file with at least as many comment lines as code lines, or a comment inside a block rather than at its head or tail | Minor |
| `JT-ADD-001` | Lines added by the diff past `max_added_lines` | Minor |

Each rule has an id, a one-paragraph reason, and a fix. Detection is per language with a table of patterns, and every pattern has a test that names the spelling it catches.

The rules are pure over a change set: for each path, its content before and after, and the old name of a rename. The gatherer reads paths and blobs with `git diff --raw -M` and `git cat-file`; the line diff is jtac's own (Myers over lines), so no `diff.algorithm` or whitespace setting can move a finding and a CI replay sees the same lines. A blob with a NUL in its first 8000 bytes is binary, as git decides, and no rule reads it. Which base to diff against is the caller's: the merge base with `--base <rev>` when the pusher names one (decision 38), else the nearest covered ancestor once a store exists, else the merge base with the branch's upstream, else with the default branch (origin's HEAD, then `main`, then `master`), by full ref name so a tag cannot shadow a branch; a version 2 root records the base it judged from (section 15), so a `--base` below the covered chain is a declared fact and the commits between read as that root covers them, those outside its walk absent. When that merge base is HEAD, or nothing names one, the rules judge the last commit against its first parent; a root commit has no change to judge. A diff git cannot read errors the `integrity` check; it is never an empty diff. Findings point at the reserved `integrity` check (section 9.2): `facts` lists it first and required, so a run that skipped the rules fails the verdict, and a policy file may not name a check `integrity`. Markers match with strings blanked, so a marker quoted in a test table or a template is not one, and a Rust attribute split across lines is read whole, one that read the same before being no new marker.

- `JT-PROT-001` matches either name of a rename, and the tiers a changed path matches are reported beside the findings. When the change adds `.jtac/policy.toml` and no parent's history ever held one, every protected-path finding is minor: nothing was in force to protect (decisions 38 and 45). The fact comes from every parent's history, not from the diff's base, which `--base` lets the pusher choose.
- `JT-TEST-001` is a test present before and absent after by name, with every copy of a name counted: a function under a `#[test]`-style attribute anywhere in Rust, `Test*` in `_test.go`, `test*` in a Python test file, an `it` or `test` title in a JavaScript test file. A deleted file, a path renamed away from its language, and a test renamed are all removals; a name that gains a copy in another file of the same change is where a test moved: that removal is dropped and `JT-TEST-003` compares the two. Copies of one name pair strongest first, so a copy dropped is the weakest. A test file deleted with no test the finder names is one finding. `allow_removed_tests` makes it minor, never silent.
- `JT-TEST-002` is an added skip, ignore, or focus marker (`.only`, `fit`, `fdescribe` skip every other test), including `#[cfg(any())]`, `pytestmark`, a bare `@skip`, `raise SkipTest`, `//go:build ignore`, and `it['only']`: anywhere in Rust, in test files elsewhere. `allow_newly_skipped_tests` makes it minor.
- `JT-TEST-003` is a lower assertion count in a test present on both sides by name, or moved between files. Major in every language; a test refactored to assert through a helper trips it and takes a waiver.
- `JT-SUP-001` makes every added marker a finding: all blocker when the count passes `max_new_suppressions`, minor otherwise, so a waiver by rule and path covers what a reviewer saw. Only added markers count; a moved one is new.
- `JT-CI-001` reads CI definitions by a path table (GitHub, GitLab, CircleCI, Buildkite, Jenkins, Azure, Bitbucket, Travis, Drone, Woodpecker): fewer step lines after than before, counting a step opener (`- run`, `- uses`, `- name`, `- id` and peers, GitLab `script`, Jenkins `sh` and `stage`) and each command beyond the first under it, as list items or `|` block lines, so a step's key order changes no count, or an added line that lets a step fail (`continue-on-error`, `allow_failure`, `|| true`, `set +e`, `if: false`, `when: manual`, and peers). A path renamed away from the table counts as every step removed. No YAML is parsed, so a step replaced by another of the same shape is not seen; every path in the table is protected by default, so `JT-PROT-001` names the file either way.
- `JT-SEC-001` is a table of known prefixes and an assignment to a password-like name (any prefix, quoted or not; an unquoted value needs a digit), never entropy, so a digest or a lockfile hash cannot block. The message names the shape and never the value; a quoted value starting like a reference, or holding a template, whitespace, or a placeholder word, is text.
- `JT-SIZE-001` counts the whole file; `JT-SIZE-002` judges a non-test function that an added line falls in. Spans come from brace counting over code with strings and comments blanked, or from indentation in Python, with no parser: a Rust attribute split across lines and a `where` clause before the brace are followed, a JavaScript regex literal is a string, a line-1 shebang is a comment, and a lone `\r` ends a line as it does for JavaScript and Python. Not seen: a test call split across lines, a Go subtest by name, a nested Rust block comment.
- `JT-CMT-001` takes the ratio over the whole file, doc comments on neither side, and the position of each added plain comment: at a block's head, at its tail, or directly above the line that opens one is in place; anything else is inside.
- `JT-ADD-001` is minor, and the count is reported whether or not a budget is set.

The size and comment rules exist because model-written code arrives correct at several times the necessary length, and length is what review cannot keep up with. Defaults: `max_file_lines = 800`, `max_function_lines = 150`, and no `max_added_lines`; the added-line count is reported by `jtac status` either way. The comment ratio is fixed: plain comment lines fewer than code lines, doc comments excluded. Production lines exclude test code by the language's own convention (the `#[cfg(test)] mod` tail in Rust, `*_test.go`, `*.test.ts`, `test_*.py`). The exemption marker is `jtac-loc-exempt: <reason>` in the language's line-comment syntax on line 1, or line 2 after a shebang; a blank reason is no marker. A marker the diff adds is a suppression under `JT-SUP-001`, so an agent cannot waive its own finding: a new marker needs the budget or a waiver, and an existing one is honored.

The reason a blocker can still be legitimate (a test really was obsolete) is why `jtac waive` exists and why a waiver is recorded and loud, not silent.

---

## 11. The local gate

### 11.1 `jtac check`

1. **Preflight.** The working tree must be HEAD's tree and HEAD must be the commit under test. Clean means `git status` shows nothing, untracked files included; no index entry is marked skip-worktree or assume-unchanged, which hides a changed file from status; and no untracked file is hidden only by a local exclude (`info/exclude`, `core.excludesFile`), since the tree under test carries `.gitignore` and not those. A tree that is not clean refuses and names `--preview`. The same test runs again after the checks and before anything is written: a tree or HEAD that changed under the checks refuses and writes nothing. Replace refs and the commit graph are ignored on every git call, a partial clone never fetches on demand, and a grafts file refuses, so the history the evidence names is the one git stores. A shallow clone gets no root: a shallow mark, real or written by hand, cuts the walk where git cannot tell the two apart; `--preview` still answers there. The signing identity is resolved before any check runs, so a missing key costs nothing. If git cannot answer, that is a refusal, never a pass. A HEAD that already has a root is checked again: a pass keeps the root and writes none, so a changed toolchain stays visible and evidence is never rewritten. `state.json` is written after the root; a write that fails is a warning on stderr, never a changed verdict, and `status` says when it cannot read the file. `jtac check --preview` runs the checks, prints the verdict with its reasons, and writes nothing; it is the only mode that runs on a dirty tree. Preview judges HEAD: the policy is HEAD's `.jtac/policy.toml`, the rules diff the base to HEAD, and every node names HEAD's tree; on a dirty tree the output says the checks saw the work tree and the rules did not. Exit codes: 0 `CLEARED HOT`, 1 `ABORT`, 2 the gate could not answer (no repository, no policy, git failed); a refusal goes to stderr in three parts and a verdict to stdout, and the `--json` structure carries `v` like every shipped shape.
2. **Load policy** and compute its hash.
3. **Run diff-integrity checks** first. They are fast and their failures are the most informative.
4. **Run configured checks**, consulting the cache. Cache key: tree id, policy hash, check id, tool version. Only a `pass` is cached; a failure reruns, because a cached failure would block until a cache nobody sees is cleared. A hit is a new node whose `cached_from` names the original. Only `jtac check` reads or writes the cache; `--preview` runs every check, since a pass computed on a dirty tree would be cached under HEAD's tree id.
5. **Compute the verdict** with the pure function.
6. **On pass:** build the Merkle tree, write and sign the root, print `CLEARED HOT`. The root's ref is created, never replaced: a run that loses the race to another on the same tip reports the root that won, since decision 33 left nothing that legitimately rewrites one.
7. **On fail:** write nodes and the verdict to the local store, write no root, print `ABORT` with each reason and the fix.

Subprocess rules: argv arrays only, working directory at the repository root, stdin closed, a timeout that bounds the whole run: at the deadline the process group (a job object on Windows) is killed, whether the check still runs or has exited leaving a descendant that holds its output, and stdout and stderr captured with a 1 MiB cap per stream that keeps the head and marks the cut. The environment is the allowlist of names in section 10.2 plus `JTAC_CHECK=1`, which the runner sets so a check that runs `jtac check` refuses instead of recursing until the timeout, and nothing else; a value is never inspected, because a token with no recognizable shape is still a token. Exit 0 is `pass` and a positive code is `fail`; a timeout, a spawn failure, and a negative code (a signal, a crash) are `error`, never `skipped`. `tool_version` is the first line of a version probe chosen by the tool's file name from a table built into the runner (`cargo --version`, `go version`, and peers), a rustup `+toolchain` selector carried along, and a tool that runs on Node probing `node --version` (the `node` beside it when the tool is given by path, as the npm launcher prefers) because its own version is in the lockfile and the runtime's is not; the probe spends from the check's own timeout; a tool the table does not know has an empty `tool_version` and is never cached. Project-pinned tools are already in the tree id through their lockfile, so the probe covers the global toolchain.

### 11.2 The verdict function

```rust
pub fn gate(facts: &Facts, nodes: &[Node], now: Ts) -> Verdict
```

`Facts` is what the caller gathered: the commit under test, its tree, the policy hash, the `integrity` check first and then the check ids with their `required` flags in policy order, and `integrity.block_at`. `jtac-policy` builds it from a resolved policy; `jtac-core` cannot take `Policy` itself, since `jtac-policy` depends on `jtac-core`. `now` is the only clock the function knows. `jtac check` passes the instant it records as the root's `ts`; `jtac replay` passes the root's `ts` back.

A check or finding node counts only when its `tree` and `policy` are the facts'. A waiver may be older than the tree. A waiver is in scope when its `ts` is not after `now`, its `expires` is not before `now` and not more than 30 days past its `ts`, and, for a rule waiver, `expires` is present. The verdict judges this itself, so a hand-written lever node is refused the same way on every machine. A rule waiver covers a finding with its rule whose path equals the prefix or sits under it by whole component; a prefix or path with an empty, `.`, or `..` component or a backslash is covered by nothing. A waiver node repeated in the input counts once, and one whose tree uses another hash algorithm than the commit is out of scope, as `Root::validate` would refuse it.

- Fails if any required check has no node for this tree and policy.
- Fails if any required check's outcome is not `pass`.
- Fails if one check id has two nodes, or a node names a check the policy does not.
- Fails if any finding at or above `integrity.block_at` is present and not covered by a rule waiver in scope.
- A commit waiver in scope for this commit passes with state `declared_off`; the reasons it overrode stay in the verdict.
- A CI waiver (section 15) naming this commit changes nothing here: the verdict carries it into `levers` and never applies it, whichever gate is running.
- Otherwise passes.

`Verdict` carries pass or fail, the state, the ordered list of reasons (checks in policy order, unknown check ids in node order, then findings in node order), and the waivers it applied: every commit waiver in scope, each rule waiver that covered a finding at or above `block_at`, and every CI waiver naming the commit, carried unapplied. Those become the root's `levers`. It is the only place a pass or fail is decided, and `replay` calls the same function.

### 11.3 Hooks

**git `pre-push`** is the gate. For each ref being pushed:

- Pushing a tag, deleting one, and pushing evidence under `refs/jtac/` pass without checks. Deleting a protected branch is the fence, judged from the branch's old tip's policy: deleting and recreating would otherwise rewind it.
- A non-fast-forward update of a protected branch is the force fence: the hook sees the remote's old oid and the new one, never `--force`. An old oid this clone does not hold refuses on every branch update, naming `git fetch`: a rewind cannot be ruled out, and the branch's own policy is half the fence. A grafts file refuses the hook and `status` as it refuses `check`. Protected branches and the fence's `deny` come from the tip's own `.jtac/policy.toml`, the tree its root was judged under, joined with the remote's old tip's policy when this clone holds it: either one's `deny` holds, so a tip cannot carry its own exemption. A tip without a policy is refused naming `jtac init`.
- For the tip commit, there must be a valid root (state `covered` or `declared_off`), or a root for an identical tree: one that parses, names the tip's commit and tree, and whose signature holds. Who signed it is `verify`'s question. The one exception is a root with `from` (section 15), whose signer the hook judges against the `.jtac/signers` of the remote's default branch tip, `refs/remotes/<remote>/HEAD` for the remote the push names, which clone and `git remote set-head <remote> --auto` set, never the branch's upstream, or of the old tip on an update, a push that names a URL having the old tip alone, since its waiver scope is wider than its commit; a clone holding neither refuses naming `git fetch <remote>` and then `git remote set-head <remote> --auto`, which sets the ref only for a branch already fetched, or, when the push named a URL, a push through a named remote, since a URL names no tracking ref; a `from` root a URL push refuses is retried through a named remote, never cleared, and one refused with the default branch tip held is a developer's, cleared by the stale-root route the hook prints, `git update-ref -d` on its ref and then `jtac check`, since `check` keeps a standing root. If not, the push is refused with the exact command: `jtac check`. `jtac status` asks the same decision function, so the two never disagree.
- The hook never runs the checks itself by default, so a push is never slow or surprising. `jtac check` is the explicit step. A later release adds a config option that runs it inline.
- Before the code push it pushes the root refs of the commits being pushed, in its own `git push` with `JTAC_HOOK=1` set, which the hook reads as "run nothing". A root push that fails warns and lets the code push proceed (decision 8).

**git `commit-msg`** is reserved for policy rules on commit text. It ships empty in v0.1.

**Agent tool-call hook.** For agents that expose a pre-tool-call hook (Claude Code's `PreToolUse` first, others by adapter, one file per agent), `jtac hook pre-tool <agent>` reads the proposed shell command, classifies it (section 11.4), judges each push where it would run with the decision the `pre-push` hook asks, and returns an advisory note on the tool call: a push with no root names `jtac check`; a rewind or deletion of a protected branch is named as a fence with no clearing command; a hook bypass is told what lands and that `verify` shows it, and named no command, since the bypass may be the operator's own fence; a push whose directory the script does not name, or a command the classifier cannot read and whose text says push, names `jtac status`. A pass, a dry run, a tag, and a bypass on a command that is not a push and does not outlive it (`git commit --no-verify`: jtac's gate is the pre-push hook) are silent. It tells the agent about a fence before the agent walks into it. It advises and never blocks: the note travels as the agent's advisory field (`additionalContext` for Claude Code), never as a decision, and every internal failure prints nothing and exits 0 with the reason on stderr. The work runs under a one-second cap on a thread the hook abandons, since the agent waits on every call and the classifier's time is not bounded on every script; `JTAC_PRETOOL_CAP_MS` in the hook's environment sets another cap, for a machine where git starts slowly, under the timeout the agent's own settings give the hook. The git hook is the enforcement point, because it covers every agent and every person identically.

Hook rules:

- A hook's stdout is a protocol wire. Nothing prints to stdout except the protocol. Diagnostics go to stderr.
- A hook that errors internally fails closed for push and open for the advisory.
- `jtac init` refuses to overwrite a hook file it did not write, and prints the line to add to it; `jtac init --keep-hook` then keeps that file unread, since no reading of a script proves the line is reached, and the owner answers for it. A hook git cannot execute is made executable, judged as git judges it. `.jtac` as a symlink is refused: nothing is written through a link.
- A mirror remote is refused whole, before the empty-stdin pass: git deletes what the remote has and the clone lacks without a line on the hook's stdin, so a protected branch could vanish unjudged. The hook reads `remote.<name>.mirror` as git's bool. A bare `git push --mirror` sets no config and is one of section 11.5's bypasses: the hook cannot see it, and the deleted branch reads as deleted. A deletion through a symbolic ref on the server is judged by the name pushed; what it points at is the server's to know.
- A hook file is `#!/bin/sh`, a marker line that names its version, and `exec '<absolute jtac>' hook pre-push "$@"`: absolute path, single quotes. `jtac hook pre-push` is the entry point and reads what git writes to its stdin.

### 11.4 Command classification

The classifier turns a shell script into the list of actions it performs: a push, with its remote, refspecs, force form, deletions and the directory it runs in; a hook bypass; a change of directory; and "unreadable". It is pure. Commit and merge are not actions in v0.1: no jtac hook runs on either, so nothing would consume them.

The grammar comes from a maintained bash parser (`shuck-parser`); `jtac-gate` adapts its syntax tree behind one module, so the crate can be replaced without touching a rule. The classifier reads the tree as the shell would run it: `&&`, `||`, `;`, pipes, subshells, groups, loop and conditional bodies, command and process substitution wherever the grammar holds one, leading assignments and exports, wrappers read with their own options, abbreviated as getopt allows (`env`, `sudo`, `nice`, `time`, `nohup`, `timeout`, `command`, `exec`, `flock`, a `busybox` applet), `git -C`, `git-push`, a leading `cd`, `env -C` and `sudo -D`, and the body of `bash -lc '...'`, `eval`, `trap`, and a heredoc or here-string handed to a shell. Git's own rules are git's: a long option may be abbreviated to any unambiguous prefix, a short cluster may hold a digit, an option's value may be `--`. A pipe's sides and a backgrounded command are subshells, so their `cd` moves nothing; the right side of `&&` or `||` may not run; a trap body runs at exit, after every later `cd`, so its pushes name no directory. Bash expands an arithmetic body, a subscript, `${x@P}`, `PS4` under trace and the words of `let`, `unset`, `declare -i` and their kin a second time, so a quoted `$(…)` in one of those is unreadable, never nothing. A `cd` to a directory the text does not name (a variable, `cd -`, `pushd +1`, a bare name under `CDPATH`, an exported `GIT_DIR`, a body that may not run) leaves every later action in a directory nobody can name, and the action says so.

What it cannot read, it reports as unreadable, with the reason: a parse that is not clean (an unterminated quote, a construct the parser recovered from, a backslash before a backtick, dollar or backslash in a backtick body, which bash cooks before it parses the body), a program word, a git option slot, or a git variable the shell builds (`$GIT push`, `git $OPTS push`, `git -c "$X"`, `GIT_CONFIG_KEY_0=$k`, `export GIT_DIR` with no value, a brace expansion), `bash -c "$VAR"`, a shell reading its script from a pipe, `eval` on a word the shell builds, a program that takes a program as a string (`python -c`, `ssh`, `xargs`, `find -exec`, `docker`, `git submodule foreach`, `git rebase -x`) when a word of it quotes git or a shell, `env -S`, an alias in any config spelling, config pulled from a file the text does not show (`include.path`, `GIT_CONFIG_GLOBAL`), shells nested past three deep, brackets past twelve deep or four thousand in all, keywords nested past the parser's hundred or an operator chain past its thousand links, an arithmetic or `${…}` body over 4 KiB (the parser is quadratic there), a nameref declaration, or a script over 64 KiB. Those bracket counts read the text without regard to quoting, on purpose: a scan that knew quotes would be a second lexer, drifting from the parser's, and a quoted bracket counted blind can only make a script unreadable, never hide one. The parser bounds its own depth, chain length and stack, and the bracket cap sits under the depth a debug build of it reads for bare `$(`, so that shape gets one answer in either build and heavier frames or keywords nested inside brackets can only be refused earlier in a debug build, never read; what the parser drops under a clean status (the nested reparse of a `${…}` operand, a subscript, a pattern or an arithmetic body past a cumulative depth of its own: a substitution with an empty body under non-blank text, an operand or pattern kept as plain text that holds an unescaped backtick, a `$` before what can open an expansion, or a `<(` or `>(` where bash runs one, an arithmetic command or expansion with no expression under non-blank text), a word bash reads again as a subscript (`let`, `unset`, `[[ -v`) whose `${…}` holds a `<(` or `>(`, and whatever breaks one of four invariants, each a difference between what the parser read and what bash runs, held in place of a rule per finding. Line continuations are joined once, before the parser sees the text, as bash joins them everywhere but inside `'…'`, a comment (the next line runs anyway) and a quoted heredoc body (kept), so a continuation inside a comment (a `#` outside every word the parser read, found in the text because the parser lists only some comments), a quoted heredoc body or a heredoc delimiter is unreadable, a `'…'` holding one keeps its text as written (except inside an unquoted heredoc body or a backtick body, which bash joins before it reads any quote), and a split `$`, `{`, keyword, `[[`, `((` or `<<-` never reaches the parser. A part that can run a command (a substitution, an arithmetic body, a `${…}` with an operand, pattern or subscript, a `"…"` holding one) is laid where the source has it, framed by its own opener and closer and tiling its span with the siblings before it, else unreadable: the parser lays a literal it cooked (`\"` in `"…"`, `\$`, `\\` or `\/` in a `${…}` operand) at its cooked width and every span after it falls left of the source, and what the reader then slices is not what runs; where no part runs the layout does not matter, since bash runs nothing there, so `${BRANCH//\//-}` and `"${N:-\$HOME}"` read while `"{\"k\":\"$(date)\"}"` does not; a name or plain `${…}` laid over an opener bash reads (`${M:-\${$(x)}`, where the cooked reparse saw an opener bash does not) hides it and is unreadable. A heredoc is read only in its plain shape: a bare delimiter or one wholly quoted (`'E'`, `"E"`, `\E`), the delimiter line exactly the delimiter (tabs stripped under `<<-`), no digit the parser read as a word before `<<`, `<<-` or `<<<`, inside a substitution no body line that starts with the delimiter and holds a `)`, under `<<-` no tab after a newline inside an expansion, and a body the parser took as quoted under an unquoted delimiter judged whole; an unquoted body handed to a shell is cooked as bash hands it on, the backslash before `$`, a backtick, a backslash or a newline dropped. Stdin is kept only as one heredoc or here-string on descriptor 0 with nothing else touching that descriptor (a redirect with no number written is stdin for the input kinds only; `{fd}<` names a variable); a file, a dup, a closed descriptor, a second redirect there or a process substitution is stdin the reader cannot see, so the command is walked as a pipe's right side and a shell reading it fails closed, a substitution in any redirect target and a process substitution anywhere are walked the same way, and stdin handed to a compound command or a function reaches every command inside. The reader runs on its own thread with a stack sized for the deepest shape a 64 KiB script can hold, since the parser's budget does not meter every recursion, and a parser panic or a thread the system refuses is unreadable, never an abort of the hook. "I cannot see what this runs" is never treated as "this runs nothing".

Bypass spellings it must recognize as hook bypass: `--no-verify` on any git command, `-n` on commit (on push the same letter is `--dry-run`), `-c core.hooksPath=`, `--config-env core.hooksPath=` in both spellings, `GIT_CONFIG_KEY_n=core.hooksPath`, `GIT_CONFIG_PARAMETERS`, exported or inline, and a write through `git config core.hooksPath`, each without regard to case, because git's keys have none.

Every spelling lives in one table test. A new bypass found later is a new row, with the fix.

### 11.5 Bypass is recorded, not prevented

jtac cannot stop someone from running `git push --no-verify` or uninstalling it. It does not try. The commit simply has no root, and every later reader (`jtac audit`, CI, an auditor) sees it as absent. This is the honest design: the local gate provides speed and early feedback, and the proof of what was and was not checked comes from the evidence chain. A bare `git push --mirror` with no `remote.<name>.mirror` set is in the same class: its deletions reach the hook as no line at all, so the hook cannot judge them, and the branch reads as deleted.

### 11.6 `jtac waive`

```
jtac waive HEAD --reason "obsolete test removed with the feature it covered"
jtac waive --rule JT-SUP-001 --path src/ffi/ --reason "..." --expires 14d
jtac waive <merge> --ci --reason "runners out; merged on the local gates"
```

- Writes a `lever` node, signed, with the reason verbatim.
- A waiver is scoped: one commit, or one rule on one path with an expiry at most 30 days after the waiver was written. A waiver written after the root that cites it is refused. The verdict function and root validation apply these bounds (section 11.2); `waive` only refuses earlier.
- A waived root has state `declared_off` when the waiver covers the whole commit, and stays `covered` with `waivers > 0` when it covers one rule.
- Every waiver in effect is printed by `jtac status` and on every `check` and push it affects.
- Fences are never waivable from the CLI in v0.1.
- `--expires` is `<days>d`, 1 to 30, and is required on a rule waiver; a commit waiver takes none. A `--path` with an empty, `.`, or `..` component or a backslash is refused, since the verdict would cover nothing with it.
- Under an agent session (an environment marker a known agent exports to every child: `CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`, `CODEX_SANDBOX`, `CODEX_THREAD_ID`, `CURSOR_AGENT`, `GEMINI_CLI`, `NYX_AGENT_ACTIVE`, `CHANNELS_AGENT`, `AI_AGENT`) the `waive` command still runs: it records the marker in the lever node's `agent` field, prints a three-part note on stderr naming the file that withdraws the waiver before a verdict applies it, and every reader of the root flags the waiver (`status`, `check`, the push hook, `verify` with a note and `agent_waivers`, `audit`). It is not a lock and is documented as not one: one account runs both the person and the agent, so a refusal would be theatre and a flag the agent cannot remove is the record. The safety comes from the waiver being signed, recorded, and loud.
- A waiver sits in the local store until a verdict applies or carries it; those travel in the root as `levers`, the rest never leave the machine.
- `--ci` writes a CI waiver (section 15): scope `commit` with `gate` `ci`, no expiry, refused with `--rule` or `--expires`, and refused when this clone already holds a root for the commit, since only the root written on that commit after the waiver can carry it.

### 11.7 `jtac init`

1. Confirms it is in a git repository and finds the remote: `origin`, else the only one. Several without `origin` refuse naming `--remote`; none is noted, and the refspec lands on the next run.
2. Finds a signing key: `jtac.signingkey`, else `user.signingkey` when `gpg.format` is `ssh`, else generates a dedicated ed25519 key with no passphrase at `jtac/id_ed25519` under the user config directory (`XDG_CONFIG_HOME` when set, else `~/.config` on Linux, Application Support on macOS, `APPDATA` on Windows) and sets `jtac.signingkey` in the global git config: one identity per person, as git's `user.signingkey` is (section 11.8). `user.email` is the principal; without it `init` refuses naming the command that sets it.
3. Creates `.git/jtac/`.
4. Writes `.jtac/policy.toml` from detection if none exists, and prints it.
5. Installs the `pre-push` hook in the directory git names, `core.hooksPath` honoured. The agent's tool-call hook is opt-in: `jtac init --agent claude-code` adds one entry to `.claude/settings.local.json`, the per-clone file, since the entry names this binary by absolute path, lists that file in `.git/info/exclude` so the work tree stays clean for `check`, and refuses a link or a file it cannot read as that shape; without the flag, a clone that shows the agent (a `.claude` directory, or the session's own marker) is told the command in a note. Prints exactly what it wrote and where.
6. Adds the fetch refspec for `refs/jtac/*`.
7. Writes a `lever` node with action `init`.

It prints the allowed-signers line for the key and writes `.jtac/signers` only under `--ci github`, which also writes the forge's workflow and makes the CI key under the user config directory, one per repository (section 15, Adoption); otherwise a repository that wants `verify` to know the signer commits the line (M11). `--dry-run` prints everything and writes nothing. Running `init` twice is a no-op. It never touches PATH, shell rc files, or any config file it cannot parse.

Target: a working gate on any repository in under five minutes, with no account and no network beyond the user's own git remote.

### 11.8 Signing

- Signatures use SSHSIG via `ssh-keygen -Y sign` and `-Y verify`, the same mechanism git uses for SSH-signed commits. No new key format and no new trust store.
- Namespaces: `jtac-root-v1` for roots, `jtac-ci-root-v1` for a root with `from` (section 15), `jtac-lever-v1` for lever nodes. A signature made in one namespace does not verify in another, and a root's shape picks its namespace, so a reader tries one.
- A waiver's signature travels in the root's `levers[].sig`, so a remote verifier checks the waiver's signer as well as the root's.
- The key is the one git config names, in order: `jtac.signingkey`, then `user.signingkey` when `gpg.format` is `ssh`, else a refusal that names `jtac init`. Both are spelled as git spells an SSH `user.signingkey`: a key file, or a public key line for a key in the agent, after `key::` or bare from `ssh-`. The principal is `user.email`, the identity git itself verifies SSH commits with. `ssh-keygen` is `gpg.ssh.program` when set, else the name on PATH. No new knob.
- The fingerprint a root or lever names is what `ssh-keygen -l` prints for the key before signing: it reads a public key, the public half inside a private key file, or the plain key behind a certificate, and never asks for a passphrase. ssh-keygen re-reads every signature it makes and every one it verifies, and the key it prints on a good signature must be the one the document names. Nothing else computes a fingerprint, since ssh-keygen re-encodes a key before hashing it.
- jtac never prompts for a passphrase: a key that needs one must be in the agent and named by its `.pub` file or `key::` line. The sign call passes an empty `-P`; without it ssh-keygen blocks on a prompt when a terminal exists and, when none does, reads the passphrase off the first line of the message. ssh-keygen also runs with no askpass program, so a PIN or a passphrase cannot open a window. What can still wait is the device or the agent: a hardware key's touch, or a key added with `ssh-add -c` whose agent asks before each use.
- A signature is the armored text `ssh-keygen -Y sign` writes to stdout, with CRLF folded to LF so Windows writes the same bytes; `root.sig` and `levers[].sig` hold it unchanged.
- Verification reads trusted keys from `.jtac/signers` (an allowed-signers file, a protected path). It is the only source of trust; git's `gpg.ssh.allowedSignersFile` is not consulted. With no trust file in the tree, `jtac verify` reports the signature as valid and the signer as unknown, and says so in plain words. A trust file that is named but cannot be read is an operational error, never unknown.
- A key's `valid-after` and `valid-before` in the trust file are judged at the document's `ts`, not at verification time, the way a waiver's expiry is (decision 14): retiring a key keeps the roots it signed. ssh-keygen reads a time with no zone in the local one, so jtac runs it with `TZ=UTC0`: the trust file and the `verify-time` jtac passes mean the same instant on every machine. `verify-time` predates OpenSSH 8.9 (February 2022).
- A signature that does not hold is a verification result, never an error. Only a tool that could not run, or a named trust file that is not a plain readable file of bounded size, is one: the tree can hold a symlink to a device.

### 11.9 Reading evidence

- **A range** is what `git rev-list` reads, after `--end-of-options` so an option never rides in: `A..B`, `A...B`, a rev for its whole history. `verify` and `audit` default to `HEAD`; `show` defaults to the upstream to `HEAD`, what the next push carries; `replay` takes one commit.
- **Trust** is `.jtac/signers` in HEAD's tree, read through git and never from the work tree, so an edit or a link there changes no verdict; `--signers <file>` reads a file of the verifier's own instead. A key's validity is judged at each root's `ts` (section 11.8). The tree you stand on says whom you trust: checked out on a commit that adds its author's key, a verifier trusts that author, so a CI checkout of a pull request reads trust from the base branch or passes `--signers` (v0.2).
- **`jtac verify [range]`** judges every root that bears on the range: its commit in it, a `covers` entry in it, or its tree matching a commit in it. A root is invalid when it cannot be read, names a tree other than its commit's, its signature does not hold, any `levers[].sig` does not hold over its node in namespace `jtac-lever-v1` judged at the lever's `ts`, a `covers` entry is not reachable from its tip past its parents, or a parent names a root id other than the one this clone holds for that commit. A waiver's signer the trust file does not name is reported as unknown, like a root's; `verify` and `audit` list every applied waiver with its signer and trust. A parent root the clone lacks is a note, since a deleted ref is absence; a root for a commit the clone lacks (a tree match the hook sent for an unpushed branch) counts for a tree match only, with a note; an unknown signer is reported, never invalid. Exit 1 when any root is invalid, or with `--require-coverage` when any commit is absent; 2 when the gate could not answer.
- **`jtac replay <commit>`** loads the root, verifies its signature, and recomputes from the local nodes the Merkle root, the counts, and the verdict with the root's `ts` as `now` and the facts rebuilt from the resolved policy the root names, which `check` keeps in the store by its hash. Any mismatch invalidates the root: exit 1. Where the nodes are not local, or the policy is not stored and the file at the commit's tree resolves to another hash under this build, replay says so and exits 2; the root's signature is still the answer it gives.
- **`jtac audit [range]`** prints one row per commit with its coverage state, which root gave it (its own, a `covers` entry, or a tree match, `covered (by tree)` inheriting that root's state), signer and trust, policy version, and gate version, then the totals and the percentage covered. A report: exit 0.
- **`jtac show [range]`** prints each root ref a push of the range would send with its root commit, `root.json` and `root.sig`, byte for byte, and names the commits that have none. Exit 0.
- **`jtac status`** says whether a push would pass from HEAD's own root, lists the commits ahead of the upstream with and without roots, and shows the last verdict. A tree match counts, read by the decision function the hook uses. Exit 0 when a push would pass, 1 when it would be refused.
- **`jtac why`** prints the last verdict's reasons as `check` printed them, each with its rule, why the rule exists, and the next command, with a note when HEAD has moved since. Before any check it refuses.

### 11.10 `jtac backtest`

`jtac backtest <range>` measures the diff-integrity rules (section 10.3) on changes that already merged, so a repository sees what the gate would have said before it gates anything, and the rates of section 14 have a method anyone can rerun. It is a report: it runs no check, signs nothing, and writes no root, node, cache entry or ref, so it needs no `jtac init`, no key and no store, and runs in any clone.

- **The unit** is one commit on the range's first-parent line, judged against its first parent: what that commit landed on the branch. A merge commit and a squash are each one pull request; a rebase merge is one unit per commit, which the forge alone could group and the binary never asks (decision 40). A parentless commit has no change to judge (section 10.3); it is skipped and counted. The range is section 11.9's, walked with `--first-parent`.
- **The policy is one, for the whole range:** the file `--policy <file>` names, else `.jtac/policy.toml` in the tree of the range's newest commit; neither is a refusal that names `jtac init --dry-run`, since `backtest` assumes no defaults either (decision 26). Only `[integrity]`, `[protected]` and `[tiers]` are read. The question is what today's rules would have said, so the policy a commit carried is not consulted, no commit is an adoption commit, and no waiver is applied: a finding a waiver cleared at the time is still a finding here. The summary names the policy by its hash (section 10.1) and says which of the two it was, since that choice sets every count; a range with more than one tip (`A B`, `A...B`) has for newest commit the first one git walks, so pin such a run with `--policy`.
- **A row** per unit: the commit, its parent count, the changed paths and added lines as counts, the tiers, every finding as the rules report it, and `blocks`, whether any finding is at or above `block_at`. A finding of `JT-SIZE-001`, `JT-SIZE-002` or `JT-CMT-001` carries `preexisting`: the same cap already broken on the before side, by the same counter, for the file, the function of that name (of several, the one that alone holds most of the lines the diff kept, so one long `new` never vouches for another and a tie pairs nothing; a rewrite that keeps no line pairs by name only when one function has it on each side), or the file's ratio. It marks what the change did not introduce; it changes no severity.
- **The summary** is what prints, and what `--json` returns: units judged and skipped, the share that would block and the share with no finding, findings by rule and severity with the preexisting count beside each, units blocked by each rule alone, and the same by language. Beside the policy's hash it holds counts and rule ids only: no path, no message, no commit id, no author, so it can leave a private repository. `--rows <file>` writes the rows as JSON lines for adjudication, with paths and messages, and says so; the file is created, never replaced.
- **What it does not say.** A finding is not a defect and a silent unit is not a clean one: precision needs each sampled finding judged by someone who knows the change, and recall needs seeded changes, since history holds only what merged. A merged diff is not a first push either, so a rate here bounds what CI would have said, not what the local gate would. The output states this.
- Exit 0 when it ran, 2 when it could not (no repository, a range git cannot read, no policy). It is never 1: it judges nothing. A refusal names the policy that was read, the `--policy` file or the commit, and `jtac backtest` with the range; it never sends the reader to the work tree's policy or to `jtac check`.

---

## 12. Error and message contract

The reader is often a model. A model that gets a terse error does not stop. It improvises, and the improvisation routes around the rule.

Every error and every refusal has three parts, in order:

1. **What** happened or was refused.
2. **Why** the rule exists.
3. **How** to proceed: the exact command, with values filled in, ready to copy.

Rules:

- Where the wrong instinct is predictable (a bigger `--force`, `--no-verify`), the message names it and says not to.
- A refusal says which kind it is: a fence (there is no path to yes from here) or a negotiation (run this).
- Truncated output names the command that shows the rest.
- Error text is product surface. One test per user-facing surface iterates every error variant and asserts the three parts are present.
- The contract binds where an error reaches a person or a model: the CLI and the hooks. Library errors carry the fields that surface needs and nothing more.

---

## 13. Testing and quality bars

Every commit passes:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo llvm-cov --workspace --summary-only     # line coverage at or above 80%
```

- Pure crates are tested against struct literals. No test spawns a server.
- The classifier and every screen is tested by a table that holds every spelling ever found, with the bypasses first.
- A fix to a rule adds the rule's own edge case to the table.
- Integration tests build throwaway git repositories in a temp directory. No test touches the network or the developer's real config.
- No `#[allow]` without a comment explaining why the lint is wrong in that place.
- One concern per file. `JT-SIZE` and `JT-CMT` from section 10.3 apply to this repository through `scripts/loc-cap.sh`; `AGENTS.md` has the mechanics.
- Tests are named for the claim they assert.
- Schema and evidence format are versioned. A shipped version is never edited. A change is a new version with a reader for the old one.

**`jtac bench`** is the public proof. It needs no model and no network. It builds a scratch repository under an isolated home with a throwaway key and attacks each control through the real code path: `jtac init`, `jtac check`, and `git push` through the installed `pre-push` hook. One row per control. A clean change passes and lands with its root, so a gate that refuses everything fails too. A push without a root, a root whose signature was replaced, a root for a different tree, a removed test, a new suppression, a policy file that does not parse, a check that cannot spawn, and a rewind of a protected branch are each refused. Each hook-bypass spelling in the classifier's table classifies as one, and the advisory names what lands wherever one is due: on a push, and on a bypass that outlives its command; a bypass on a commit, merge or am alone is silent by 11.3, and the row counts it as such. A bypass is recorded, not prevented (section 11.5). Every row quotes the gate's own line. Exit 0 when every control held, 1 when one did not, 2 when bench could not run (no git, no `ssh-keygen`). It runs as a test, so a regression fails the build.

**Dogfood.** The jtac repository is gated by jtac from the first commit that can run it.

---

## 14. v0.1 build plan

Each milestone is one branch and ends with the quality bars green.

| # | Milestone | Done when |
| --- | --- | --- |
| M0 | Workspace scaffold, CI for the quality bars, license, this document | `cargo test` runs on an empty workspace in CI |
| M1 | `jtac-core`: node and root types, canonical JSON, hashing, Merkle tree with inclusion proofs | Round-trip and known-answer tests pass. Proofs verify |
| M2 | `jtac-policy`: parse, validate, hash, defaults per language | A malformed or unknown-key policy is refused with a three-part error |
| M3 | `jtac-core` verdict function | Every verdict rule has a struct-literal test, including skipped and error outcomes |
| M4 | `jtac-checks`: subprocess runner, capture caps, env scrub, timeouts, cache | A check that cannot spawn is `error`. Cache hits are marked |
| M5 | Diff-integrity checks | Each rule in section 10.3 has per-language pattern tests |
| M6 | `jtac check --preview` | A repository gets a verdict with reasons and nothing is written or signed. The first demo |
| M7 | `jtac-store`: local store and git ref layer | A root written on one clone is fetched and read on another |
| M8 | `jtac-sign`: sign, verify, namespaces, trust file | A rewritten body and a wrong namespace both fail verification |
| M9 | `jtac check`, `status`, `why` | A passing repository prints `CLEARED HOT` and writes a signed root. A failing one explains itself |
| M10 | `jtac-gate` push decision, `pre-push` hook, `jtac init` | A push without a root is refused with the exact command. The hook and `status` agree, tree fallback included |
| M10b | `jtac-gate` shell classifier | Every bypass spelling is in the table |
| M11 | `verify`, `replay`, `audit`, `show` | Coverage states including tree fallback are reported correctly on a scripted history |
| M12 | `waive` | Waivers are signed, scoped, expiring, and shown everywhere they apply |
| M13 | Agent tool-call hook for the first agent | The advisory names the fence and the clearing command |
| M14 | `jtac bench`, README with measured numbers, install script | Bench passes. A stranger can install and gate a repository in five minutes |

Not in v0.1: any model call, CI verification, review rounds, tiers beyond recording, the collector, scoring, shared lessons, practices. Their node kinds and fields are reserved.

**Numbers to publish at launch**, measured on real agent output across more than one person's repositories:

- Catch rate: share of bad changes blocked before push.
- False block rate: share of blocks a human judged wrong. This decides adoption.
- First-pass rate and attempts per change.
- Time added per push.

---

## 15. v0.2: CI gate

The CI gate closes the gap stated in section 6: it is a different identity on a different machine. The first `v0.2` release is the independence slice alone, M15 to M17; the rest of this section follows as later patches, one milestone per branch, each opened with a design round.

- **Form.** The same `jtac` binary, knowing no forge. `jtac ci` takes `--base <rev>`, the base branch, diffs from its merge base with the head, trusts the `.jtac/signers` in that rev's tree or the file `--signers <file>` names, resolves no key and signs nothing, and answers with exit codes, 0 pass, 1 fail, 2 could not answer. A forge adapter is a workflow file that supplies those from its event and turns the exit code into a status check. GitHub's ships first, a composite action in this repository that installs a pinned release through `install.sh`; GitLab and others are a second file. No forge API call enters the binary. The verifier's own stance is a flag, `--require-local-root`, set from the workflow and never a policy key: a key added to policy version 1 is an edit to a shipped format that every 0.1.x hook refuses, and a filled default changes every resolved policy hash (section 10.1). CI knobs in policy wait for policy version 2, with tiering.
- **Root version 2.** A root gains two fields: `base`, the commit the rules diffed from, absent on a root for a parentless tip, whose rules judged no change (section 10.3); and `from`, present on a merge root alone: the pull request head commit it stands for, the id of the head's root when the merge job found one, and `compared`, what the merge job made of that root: `agreed`, `disagreed`, `lied`, `invalid` or `unverifiable`, so what the head had at merge time, and whether it held, is inside the signed evidence and a root published afterwards changes nothing. `covers` is the commits reachable from the tip, from no covered commit, and not from `base`, the walk of section 9.4 bounded by `base`, so it begins right after `base` and holds no commit below the diff the rules judged; `parents` names the roots the walk stopped at, as in version 1, `base`'s among them when it has one, and an empty list is a break in coverage as before; for a root with `from` the walk stops at `base` alone, so `covers` is every commit reachable from the tip and not from `base`, the diff its verdict judged, and `parents` is `base`'s root alone, never a root of the head's chain, which the base repository may not hold and which a root published or withheld later must not change. A `base` must be a proper ancestor of the tip, a root with none must name a parentless tip, and every `covers` entry must be reachable from the tip and not from `base`, and a `from` root's bounded form begins at `base` itself; any of these failing is an invalid root. A reader never checks that `covers` is the whole set, since what other roots cover changes with the reader's refs, and a root published or deleted later must change nothing. Everything else is version 1's. A version 1 root is read as evidence as before and is never compared, since it is from another gate version. Section 9.4's bounded form of `covers` records the oldest commit listed, which is not the diff base; `base` is why the field exists, and it also makes a `--base` the pusher chose (decision 38) a declared fact, and a commit outside the path it declares read absent instead of covered by it. Above the bound a root with `from` records `{ "base", "tip" }` with its own `base`, and means `base..tip`, every commit reachable from the tip and not from `base`, which the graph gives whole at any size, so a side line merged inside the pull request reads covered and not absent as section 9.4's ancestry path would read it; a root without `from` keeps section 9.4's bounded form, since its walk stopped where the writer's roots were.
- **Waiver scope in CI.** The verdict function takes, as a fact, the commits a commit waiver may name: for the local gate the commit under test alone, as section 11.2 has it; for a pull request every commit after the merge base up to the head, since the person who signed a waiver on an earlier push approved that push, and a commit waiver in scope declares the whole verdict off, as it declares a push off locally, with the reasons it overrode kept, and does so only when every commit after the one it names up to the head is covered by a root base trusts, since its signer approved that push and not what was pushed around the hook after it, else it is named and applied nowhere; `jtac waive HEAD` says so in its note, and a rule waiver, which applies by rule and path wherever it is, is the precise form. The scope is a fact of a root's shape, not of trust, so `replay` needs no trust file: a root without `from` has the one-commit scope, and a root with `from` has the pull request's commits, after the merge base of `base` and `from` up to `from`, and the tip. That set is graph data a clone after a squash or rebase may not hold, so `Root::validate` accepts a commit waiver in a `from` root naming any commit, and a `declared_off` root with `from` carries one such waiver rather than one naming its own commit; that the commit was in the pull request is what the merge job checked before applying the waiver and what the CI signature attests, and `replay` on the job's own clone checks it again. Whether such a root's signer may make that claim is the reader's trust question (section 11.9): a `from` root verifies in the CI namespace under a line that names it, or its signer is unknown; and `jtac check --from` always signs in the CI namespace. The `pre-push` hook is the fence against a `from` root on a developer key: it accepts one only when the signer's line in the remote's default branch tip's `.jtac/signers`, or the old tip's on an update, names the CI namespace, either sufficing, so a tip cannot carry its own CI key, a branch cut at or fast-forwarded to a forge-merged commit pushes its standing CI root, and a clone holding neither tip refuses naming `git fetch <remote>` and then `git remote set-head <remote> --auto`, or a push through a named remote when the push named a URL; it is one push deep on purpose, since a signers line that names a new key is a `.jtac/**` change CI judges under base's policy, and CI is the fence; and the pull request check judges the head in its own scope with its own chain, so a borrowed waiver clears nothing there.
- **What is judged.** The pull request head in a full clone with `refs/jtac/*` fetched from the head repository, which for a fork is the fork, with trust the base branch's `.jtac/signers`, never the head's (section 11.9), and `--base` the merge base with the base branch. CI's verdict is one run: the configured checks on the head's tree, the rules on the diff from the merge base, which no root of the pusher's can move, so every commit of the pull request is judged whatever an intermediate root declares, with the waiver nodes the levers of every root on the head's first-parent line above the merge base, the head's own or its nearest covered ancestor's and each root's `parents` from there, whose signature holds and whose signer base trusts, carried by a root whose signer base trusts, since the root is what put it in the chain and a lever copied from another pull request's root is not its signer's approval of this change, so a rootless merge from the base branch at the head drops nothing; a lever base does not trust, or one carried by a root it does not trust, is applied in CI's verdict nowhere and named. The clock is CI's. Policy is the head's, except that a change under `.jtac/**` anywhere in that diff is judged under the base branch's policy, so no run clears the policy it brings; the head's policy takes effect after merge. A head with no policy, or one this gate cannot read, and no `.jtac/**` change in the diff is judged under the base branch's policy too, since that is what stays in force after the merge, and the output says so; both sides holding none is a refusal, since the gate could not answer. Under base's policy that change is a major `JT-PROT-001` finding (section 10.3), so a policy change passes CI only through a waiver a trusted signer put in the chain: the signed waiver is the human approval, in evidence. `jtac init --ci github` prints the CODEOWNERS line for `.jtac/` so the forge asks a second human. Adoption in CI is the base branch holding no `.jtac/signers`: with no trust nothing could have been waived, so the pull request that brings the first signers file, with the workflow and the policy where those are new too, is judged under base's policy where one exists and the head's otherwise, and every protected-path finding is minor (decision 45); the first signers file is the trust root and only the forge's review vets it, which is why `jtac init --ci github` prints the CODEOWNERS line first. A later `.jtac/**` change is major as before. The local gate keeps decision 38's test as decision 45 extends it, since its base is the pusher's; a branch from before adoption that adds a policy file again is judged in CI under base's, as every `.jtac/**` change is.
- **Verification.** `jtac ci` verifies the head's root as a document: it parses, names the tip and its tree, its signature holds, its `base` and `covers` are what version 2 requires, and its signer is judged against base's trust and reported trusted or unknown, never invalid for being unknown (section 11.9). The comparison needs the root's own diff, so the rules run a second time from the root's `base` to the head, with the root's own levers as the root applied them, whose signatures hold whether or not base trusts the signer, since the comparison asks whether the root told the truth and CI's own verdict is where trust decides, with the root's `ts` as `now`, as `replay` does (decision 14), and the check nodes of CI's run, which judged the same tree under the same policy; the sides are compared, state and counts, only when the root's `gate.version` is this gate's and its `policy.hash` is the hash this gate computes for the policy the root should have judged under: the head's, or for a root with `from`, `base`'s when the diff from `base` touches `.jtac/**`, as the merge job judges; and a `from` root whose scope needs history this clone lacks is unverifiable, treated as a missing root. A root from this gate version naming another hash lied about the policy it was judged under and fails the check; a root from another gate version is unverifiable, since a rule table can change counts under one hash, and the check treats it as it treats a missing one, so a forged root gains nothing that skipping jtac does not, and the merge root keeps the difference visible (below). The action pins the release it installs, so a developer on another patch version gets unverifiable roots until the two match, and the output names both versions. Where CI's verdict and the root were judged under different policies, a `.jtac/**` change or a `from` root, the checks of the root's policy run too, since a check node counts only under its policy (section 11.2); the cost is a policy change's. Nodes never leave the machine that wrote them, so there is no node-by-node comparison; a root `covered` with no major against a comparison run that finds one is the finding, named with both sides. Roots behind the head are not compared: CI's verdict already judged what they cover. A `from` root at the head is compared like any other, in the scope its shape gives it. A root that is invalid, that lied, or that disagrees fails the check whatever CI's own verdict. A missing or unverifiable head root is reported in the output, and passes; `--require-local-root` makes it fail. The default passes because CI's own run is the gate, the merge root records what the head had (below), and a fork's contributor may have nothing to offer.
- **Identity.** An SSH key held in a CI environment secret whose deployment rules allow the default branch and protected `v*` tags alone, written to a file by the jobs that run there and named through `jtac.signingkey`, with `user.email` as the CI principal; its public key is a line in `.jtac/signers` like any developer's, reviewed like any other change, with `namespaces="jtac-ci-root-v1,jtac-release-v1"`: a merge root is signed in `jtac-ci-root-v1` and `SHA256SUMS` in `jtac-release-v1`, so the CI key can sign no ordinary root and no waiver. ssh-keygen reads a line with no `namespaces` option as every namespace, so jtac reads the option itself: a root with `from` is a CI root only under a line that names `jtac-ci-root-v1` explicitly, and a developer's line, which names none, makes it an unknown signer's root, read as any such root is. The environment rule is the fence, not a narrowing: a same-repository pull request run uses the pull request's own workflow file with repository secrets, and a tag pushed on an unreviewed commit runs that commit's release workflow, so a repository secret would hand the key to any branch. `jtac init --ci github` prints the environment to create and its rules as a step; the action cannot see the environment its job runs in, so the rule is the forge's to keep, and the merge job notes a signing key the old tip's `.jtac/signers` does not name for the CI namespace, whose root every reader sees as an unknown signer's. The default-branch job runs the merged tree's own checks beside the key file, so the key is as safe as what the pull request gate and the forge's review let onto that branch. A key that leaks is removed from `.jtac/signers`, never retired with `valid-before`: that is judged at the root's `ts`, which the signer writes, so a backdated root would still read as trusted; the roots the key signed then read as signed by an unknown key, which is the honest state. A CI root proves that the forge's runner, holding the CI key, judged this tree with this gate and policy. Keyless signing bound to the workflow identity, when it comes, is a new signature namespace with its own reader, never an edit to this one. That closes the deferral in decision 39. The gate reports a required status check and never posts an approval.
- **Merge roots.** The default-branch job runs on every push to the branch, one job at a time per branch through the workflow's concurrency group, since two pushes close together would otherwise let the second merge root land before the first's and name no parent, with `--base` the push's old tip, which the forge reports, so trust is the old tip's `.jtac/signers` and a signers change a pull request brings takes effect after its merge, as its policy does; an old tip with no such file, the adoption merge, trusts nobody, which is what the repository had, and the adoption root applies no waiver. A root already standing on the tip is kept only when it is CI's own for this head, signed in the CI namespace by a key the old tip's signers name, or by the job's own key whether or not they name it, as the adoption merge's cannot and a rotated signers file does not, with `from` naming the head, which is what a re-run finds; any other is refused, since a root pushed onto the forge's commit around the job would otherwise stand as the commit's evidence; the root the remote holds when it refuses the merge root's push gets the same test, a concurrent run's kept and this run's left unsent, since a root pushed onto the tip between the job's fetch and its push would otherwise stand with the job green, and a remote that refuses the ref while holding none is refusing the push itself, which the job says instead of passing on a root only its clone holds; a direct push's tip is judged by `jtac ci`, which compares its own root; requiring pull requests in the forge is the fence. A direct push gets no CI root: its evidence is the developer's root or none, and none reads absent as anywhere, so a bypass straight to the branch is never signed over. A forge that merges by fast-forward makes no commit of its own: the tip is the head, which keeps its developer root (decision 33) and gets no CI root, so such a branch has developer evidence and CI's verdict and never a merge root; `check --from` refuses a head that is HEAD and names `jtac ci`, and a forge adapter that cannot require a merge commit documents that its merges read as the developer's. When the forge reports the push as a pull request's merge, with the head commit as `--from`, `jtac check --from <head>` under the CI identity first compares the head's root as the pull request job did, since that job's result is not durable and a root may arrive after it ran, with the head's checks run again, hits when the tip's tree is the head's and otherwise run in a detached worktree of the head outside the clone, in a fresh private directory of the job's own under the system temp dir, created and never adopted, since a tool that searches upward would read the merge tree's configuration as the head's and a guessable path in a shared temp dir can be planted by another user, which the job removes when the comparison is done, and under no policy but the head's own or the branch's, since the job holds the key: a head root with `from` whose declared base names another policy is unverifiable there, and the pull request job, which holds no key, is where it is compared in full; a merge CI's own verdict blocks compares nothing, since no root would record it; and records the outcome in `from`, one of the five the field carries; the outcome never stops the merge root, since the head repository's owner controls that ref and must not control the branch's evidence, and a disagreement recorded under the CI key is worse for a forger than absence. Only CI's own verdict fails the job, and then no root is written, so a merge forced past the check reads absent. It is then one verdict, as every root is: the configured checks on the tip's tree, the rules on the diff from the old tip to the tip, which is what the push added to the branch and holds a rebase's every commit, under the old tip's policy when that diff touches `.jtac/**` and the tip's otherwise, with the waiver nodes the trusted levers of the head's chain down to its merge base with the old tip, in the scope above, and adoption judged as CI judges it, the old tip holding no signers file. It writes the root for a commit that has none, which a commit the forge's merge made never has, with `base` the old tip, `parents` the old tip's root, `from` the head, its root and `compared`, and `levers` every waiver applied, signed in the CI namespace; it pushes beside it the head's own root ref, where the base repository does not hold it, when `compared` is `agreed` and never otherwise, since a root CI found disagreeing, lying or invalid must not enter a ref every clone fetches, and one it could not compare vouched for nothing CI checked; the roots behind the head on the chain stay in the head repository, since the merge root's `covers` is the whole pull request and CI's verdict judged that diff, so a fork's earlier pushes read covered by the CI root in the base repository, never absent; a ref withheld is named in the output, as is each ref `--push <remote>` sent: the job pushes outside the hook with the hook's guard set, the merge root's ref and, when `compared` is `agreed`, the head's, and a root push that fails otherwise warns and never fails the job (decision 8); `from` records the head's root and its outcome either way, and `audit` reads such a head as `head root not held` beside its outcome. A same-repository head's roots are already there, pushed by the developer's hook (section 11.3), and stand as the developer's evidence as a direct push's does, each commit under its own root and `audit` showing whose; CI withholds and forwards nothing there, and the merge root's `from` records what it made of the head beside it. So a fork's evidence enters the base repository by CI's hand only when it held, and nothing a developer did not push themselves enters `refs/jtac/*` unverified. A direct push with no root that a branch merged in before its next root is on that root's path, as section 9.4 has always had it, and `audit` says path entry with that root's signer, never own root. Nothing from the pull request job is reused: that run used the pull request's own workflow, so its store is not this job's cache. A commit pushed there with a root of its own keeps it (decision 33): CI's run is reported and writes nothing, and `audit` shows whose root it is. A default branch whose merges the forge makes has a CI root on every merge CI passed, and a direct push is visible as the developer's. `audit` reads a merge root's head from its `from`: `head absent` when it names no root; `head unverified` when it names one the job could not compare, a gate version apart or history the job lacked; `head disagreed`, `head lied` or `head invalid` by `compared`; else `head verified`, with the head root's signer and trust, read from the root at its ref when that root's id is the one `from` names, as a document whether or not the clone holds the head commit, never as coverage of a commit the clone lacks (section 11.9), and `head root not held` when the ref is missing or holds another root, so a root published later changes nothing; the merge job is where that root was last compared, and the merge root records what it found. A commit behind a rooted tip is a path entry, which neither looks like. `--require-local-root` is the earlier refusal. The run's store is uploaded as a workflow artifact named by the job, so `replay` is possible for the artifact's retention; the durable copy is the collector's (section 17). A fork's run has no secret, writes no root, and still judges.
- **Bypass merges.** When the required check cannot run and an administrator merges around it, the merge has no CI root and no later job covers it: the next merge root diffs from it as `base`. Nothing backfills. The person who merged records why with `jtac waive <merge> --ci --reason`: a commit waiver whose scope names the gate, signed in `jtac-lever-v1` by a developer key the merge's `.jtac/signers` trusts; the CI key is confined to its namespaces and cannot sign one, so a job never explains its own absence. The root `jtac check` then writes on that commit carries it, `jtac check --push <remote>` sends the root the way the hook would, and the commit reads CI `declared_off`: not covered, explained. `audit` prints the state with the reason and `verify` judges the signer as it judges any waiver's; whether `release` attests such a commit is decided with `release` (M18).
- **Adoption.** On the adoption commit every protected-path finding is minor, not only `.jtac/**`: nothing was in force to protect (decision 45, extending decision 38). `jtac init --ci github` therefore writes the workflow, the signers file and the policy where there is none in one adoption commit that clears without a waiver; the signers file names two keys, the operator's own, which `jtac init` already prints, for every namespace, and the CI key's confined line, since the CI key signs no waiver and a signers file naming it alone leaves every later `.jtac/**` change with no one who can clear it, so `init` refuses to write one, locally under decision 38's test widened by the first signers file and in CI under this section's; a later `.github/**` change is major as before.
- **Shallow clones.** The action fetches full history (issue 28); narrowing the shallow refusal to the commits the walk reaches is its own change.
- **After the first `v0.2` release.** The release gate (section 16.3) as the next patch. Then tiering: policy classifies each change by blast radius, low tier merges on the CI verdict, high tier requires a named human with ownership of the area, and a `.jtac/` change is always high tier. Then reuse of checks whose cache key matches a trusted prior CI run, local results a hint for ordering and never a substitute; attribution in the open Agent Trace format where the agent provides it, read by `jtac who`; org policy from a signed release of a policy repository (section 18.3); `jtac policy` and `jtac scope`.

### 15.1 v0.2 build plan

| # | Milestone | Done when |
| --- | --- | --- |
| M15 | Root version 2 with its version 1 reader; the commit-waiver scope as a fact of the verdict; `jtac ci`: verify the head's root with trust from base, one verdict from the merge base with the chain's trusted levers, the comparison run on the root's own base, compared under one gate version and the hash of the policy the root should have judged under; `--require-local-root`; the comparison a pure function in `jtac-core` | On a scripted repository a false head root fails with the three parts and both sides' counts, and so does one from this gate version naming another policy hash; a test removed in a commit before the diff the head root declares fails; a finding waived by a trusted commit waiver on an earlier push stays waived; a root whose only cover for a finding in CI's diff is an untrusted waiver fails CI's verdict naming the waiver; an unknown signer is reported and compared; a root from another gate version, like a missing one, is reported and passes, and fails under `--require-local-root`; a `.jtac/**` change passes only with a trusted waiver in the chain, except when the base branch holds no signers file |
| M16 | The CI namespace and the signers line that confines the key to it, read by jtac and by the hook; the default-branch job on every push: `jtac ci` on the tip, then on a merge `check --from` under the CI identity comparing the head and judging the merge, the merge root and the verified head root pushed outside the hook, the store as an artifact; every `head` label of section 15 in `audit` | `jtac audit main` in a second clone reports a forge-made merge commit, squash and rebase included with every rebased commit covered, by the CI principal with its head verified, a fork's head the same, a developer-pushed commit by the developer's key, a merge whose head was pushed with `--no-verify` as `head absent` whatever root is published later, one whose head root is from another gate version as `head unverified`, one whose head root names a tree not its commit's as `head invalid`, and one whose head root was published disagreeing after the check as `head disagreed` with its CI root written and, for a fork's head, its ref withheld, and one whose head root named another policy hash as `head lied`, with `head root not held` when the head was a fork's; a direct push with no root and a tree of its own reads absent and is in no CI root's `covers`, while one whose tree a root already judged reads covered by tree (section 9.5); a head whose trusted commit waiver cleared its push merges under the same waiver; a root with `from` on a developer key is refused by the hook and, pushed around it, clears nothing in CI; a merge whose pull request held more commits than the `covers` bound with a side line merged inside it reads covered throughout |
| M16c | The CI waiver (decision 48): node version 2 with its version 1 reader, `jtac waive --ci`, the verdict carrying it unapplied, `check --push` without `--from`, `audit` and `verify` reading it | A bypass merge waived with a reason reads `ci declared_off` with that reason in `audit` on another clone; a 0.2.4 reader reads the root as invalid, never as covered; the waiver changes no verdict on either gate |
| M17 | The GitHub action with its environment rule, `jtac init --ci github`, decision 45 in the rules, the docs, and this repository on it | A stranger adds the action and one environment secret in one adoption commit that clears, merged as a merge commit; within ten minutes a pull request that removes a test fails the required check with the three parts, and one pushed with `--no-verify` passes with its absence named, and one that edits `.jtac/policy.toml` fails until the stranger's waiver is in its chain. Tag the first `v0.2` release |
| M18 | `jtac release <tag>` (section 16.3), designed first | A tag whose range holds an absent commit is refused naming it; an attested tag verifies in any clone. Tag the next patch |

## 16. v0.3: review round, review packet, export

### 16.1 Review round

- Reviewer agents are launched by `jtac review`, from a terminal or CI, never from inside an agent session. A worker that could start a round on itself could clear its own work.
- A round refuses a tree that is not the commit.
- Each reviewer receives a brief and must end its report with a verdict line: `FINDINGS: <n>`, alone on the last line, with `n` findings listed above in a fixed grammar (`F<i>: <severity> <path>:<line> <summary>`).
- The parser reads the last non-empty line only. A quoted line is never a verdict. A count that disagrees with the listed findings is not counted. A missing verdict is not a clean verdict.
- The round records which models actually ran, from each run's own trail, and policy can refuse to count a report from a model below a stated floor.
- Reviewers have no approve power. Their reports become `review` and `finding` nodes. The sign-off is computed by the verdict function.
- Rounds are bounded by policy: a cap on rounds, later rounds read only the diff since the last reviewed commit, and a dirty verdict at the cap goes to a human.

### 16.2 Review packet

For a high-tier change, the verifier builds a packet for the human reviewer and their agent:

- **Proven.** Checks that passed, with hashes. Not to be re-read.
- **Claims against diff.** The author agent's declared intent (a `claim` node: "touches billing, no auth changes, no new dependencies") checked against what the diff does: paths, public API surface, dependency delta, test delta. A mismatch is the strongest signal in the system and needs no model.
- **Residual.** Ranked hunks nothing could prove, each with the reason it needs judgment.

The packet is written by the verifier, never by the author agent. The author's description appears labeled as an untested claim. The reviewer records one signed `disposition` per residual item, so the record shows what was looked at.

### 16.3 Release gate and export

`jtac release` is M18 (decision 44), the patch after the first `v0.2` release; `jtac export` stays here.

- **`jtac release <tag>`** judges the range from the previous release to the tag and attests it only when every commit in it is covered or declared off by a root whose signer the tag's own `.jtac/signers` names, and names each commit that is not: absent, signed by an unknown key, or with `--require-ci` a first-parent commit the CI gate never judged, so a tag cut on a pull request's own commit rather than the merge the forge made is refused, which is how the first `v0.2` tag was cut. The range starts at the previous attestation, the nearest ancestor whose document at `refs/jtac/releases/*` verifies trusted under the tag's own trust, so releases chain through documents a reader holds them to, and no commit falls between two; a document there that does not read or does not verify is noted and is not a start, since anyone with push access could have put it there; `--from <rev>`, an ancestor, names the first one's start and is recorded as a commit alone, so a `from.tag` in a document always names an attestation and `--verify` holds it to one; a tag with neither, or a start that is the tag's own commit, is refused. Declared off by either gate counts and is listed. Recovery from an absent commit is the CI waiver (decision 48) and the next patch: a tag is never moved.
- **The attestation** is `release.json` version 1: the tag name, the tag object (so a re-tag is another release), the commit and tree, `from`, the counts, the waived commits, every root the range stands on, whether the first-parent line was held to CI, the artifacts a `--sums SHA256SUMS` names, the gate and the signer; signed as a whole in `jtac-release-v1` over its canonical bytes and stored as a parentless commit at `refs/jtac/releases/<tag>`, inside the refspec `init` adds and outside the root listing. Only a signers line whose `namespaces` option names `jtac-release-v1` trusts it, the reader a CI root has (section 15, Identity), so the CI key attests by construction and a developer's cannot. The counts are a reader's convenience: `jtac release --verify <tag>` re-derives the verdict from the clone's roots and holds the document to it, as `replay` holds a root to its nodes.
- **In the release job.** `jtac release <tag> --sums SHA256SUMS --require-ci --push <remote>` runs under the CI key between the build and the publish: it signs, stores and pushes the ref, or refuses the release naming the commits; the two files are published beside the archives. A rerun finds the attestation standing and changes nothing; a rerun whose build differs from the attested artifacts is refused, and the next patch gets its own. This is where decision 41's `SHA256SUMS` signature lives: one document, not two.
- `jtac export` produces the evidence for a range in a form an auditor can read and independently verify, organized by control.

## 17. v0.4: collector (optional, self-hosted)

An org that wants a durable central record runs a collector. Individuals and small teams never need one.

### 17.1 No phone-home

Developer machines report to nothing. The collector learns of a push or merge from the forge (a webhook or a poll), fetches the roots from the repository, verifies them, and appends them to its log. That is server to server. For work that was never pushed, no central data exists.

### 17.2 Two planes

| | Evidence plane | Telemetry plane |
| --- | --- | --- |
| Purpose | Proof for gates and audits | Visibility for operators |
| Carries | Verified roots, CI verdicts, dispositions, lever pulls | Metrics derived from verified evidence |
| Delivery | Lossless and idempotent | Best effort |
| Storage | The collector's append-only log | Any OpenTelemetry backend |
| Used for decisions | Yes | Never |

Telemetry is derived from verified evidence and exported over OTLP, so dashboards cannot disagree with the ledger. It carries hashes, counts, and timings, never source, prompts, or model output.

### 17.3 Tamper evidence

No store is immutable to its own administrator. The goal is that tampering is always detectable.

- Roots are signed at the source, so nothing can be forged at the collector.
- The collector's log is hash-chained, with a signed checkpoint at a fixed interval.
- Checkpoint hashes are published somewhere the collector's operators cannot rewrite.
- Where the org has write-once storage, segments land there.
- `jtac verify --log` replays the log against the checkpoints, so nobody has to trust the collector's word.

Server components (collector, CI gate) report their own health. Developer machines do not.

### 17.4 Scoring

The system scores workflows, not people. A workflow is the combination a change was produced with: model, effort, harness, adopted practices.

| Scale | Unit scored | Computed where | Who sees it |
| --- | --- | --- | --- |
| One developer | Their own workflows | On their machine, from the local store | The developer |
| Repository | Workflows and practices used there, plus outcome measures | The collector | The org |
| Company | The same, pooled across repositories | The collector | The org |

- Each merged change is a trial: first pass or not, review rounds, defects at or above major blamed to it later, tokens and wall time where known.
- Per workflow, the probability of a clean outcome is a decayed Beta estimate: a success fades faster than a failure.
- A repository with little data borrows from the company-wide estimate until its own evidence builds.
- Changes are compared within the same kind, size, and tier.
- Every number carries an interval and a sample count. Below a minimum sample the output says "not enough evidence".
- An incident such as a false green pins its workflow until a human clears it.
- Scores recommend. They never gate, and they are not an input to any verdict.
- The collector computes no per-developer score and has no view that ranks people.
- Unknown cost is shown as unknown, never as zero.

Outcome measures per repository and release: escaped defects per merged change, reverts and hotfixes, review findings by severity, rounds and time to merge, coverage share by state, cost per merged change.

To show cause and not just correlation, enable repository by repository on a schedule fixed in advance, declare the measures before the rollout, compare each repository with itself and with those not yet enabled, and name the confounders in the report.

## 18. Shared knowledge (opt-in)

### 18.1 The rule

No text written by an agent ever reaches another agent's context automatically. That path is how one poisoned agent would steer every other.

### 18.2 Lessons

The default form of a shared lesson is a deterministic rule in the signed policy. Other agents meet it as a gate result, with its explanation, only when it is relevant.

Pipeline: candidates come from structured finding and outcome rows, never transcripts. A candidate is a proposal. A named human promotes it with a second approver. It ships in a policy release. Its effect is measured and it is retired if the mistake rate does not fall.

When a lesson cannot be a check, an advisory note is allowed under limits: written and signed by a human, fixed fields with tight length caps, linted for commands, URLs, outside paths, encoded text, and instruction-style phrasing, able only to add caution, scoped to named paths, and handed to the agent labeled as reference material.

### 18.3 Policy repository

Policy, lessons, and practices live in one dedicated repository per org. Every gate pulls from it.

- A change is a pull request, reviewed, merged, and released as a signed tag.
- A gate loads only a verified release and never tracks a branch head.
- **Signatures decide, not origin.** A release counts only if enough named approvers signed it.
- **The trust root lives outside the repository.** Approver keys are set at `jtac init` and rotate only by a change signed with the current keys.
- **No rollback.** A gate refuses a release older than the one it has loaded.
- A gate keeps the last verified release and works offline.

### 18.4 Developer opt-in ceremony

An org signature makes a note eligible. It does not put the note in anyone's agent.

1. The developer sees a catalogue first: count, size, and one row per note with scope, author, and approvers.
2. They pick notes individually. The consent prompt comes after the pick and names the exact content. The default is no.
3. Adopted notes go in a separate local store, apart from the developer's own instructions.
4. Text is neutralized at storage so it cannot pose as the developer's authority.
5. The agent's context carries one line per note under a standing frame: reference material, facts and not instructions, possibly stale. Bodies are fetched on demand.
6. A new release never changes adopted notes silently. The developer sees a diff and accepts or declines.
7. The developer can list, remove, or clear adopted notes at any time.

Two boundaries: checks are org policy and context is the developer's. Adopted org notes never gain the developer's own authority.

### 18.5 Practices

A developer can package a way of working and share it, like a skill. Nobody is required to share or adopt.

| Part | Risk | Handling |
| --- | --- | --- |
| Gate configuration | Low: deterministic, can only tighten | Reviewed, signed, adopted by choice |
| Agent instructions | Medium: text an agent will act on | Shown in full at adoption, pinned by hash, updates arrive as a diff |
| Executable parts | High: runs on the adopter's machine | Out of scope until the rest is proven |

A practice can carry figures computed from its author's local store, attached only by the author's choice and labeled as one person's result. Adopters can opt in to contribute their own. The listing says how many independent adopters stand behind a number.

## 19. Privacy commitments

These are product promises and belong in the README.

- A developer's machine never reports anywhere. Evidence leaves only with a push.
- A block caught locally is never recorded outside the machine.
- `jtac show` prints exactly what a push will carry.
- Only roots and signatures are shared. Attempt history and detail stay local and can be disclosed one node at a time, by the developer's choice.
- The collector computes nothing per developer.
- Nothing enters a developer's agent context without that developer's consent.
- Roots are signed with a developer's key, so the roots a person pushed are attributable to that key, as their commits already are. The product does not build per-person views from that. An org with repository access could, as it can today with git history.

## 20. Standards mapping

| Standard | Covers | How jtac uses it |
| --- | --- | --- |
| ISO/IEC 5055 | Automated source code quality measures, expressed as weaknesses | Rule ids map to weakness ids where one applies |
| ISO/IEC 20246 | A generic process for work product reviews | The high-tier review flow follows its roles and records |
| ISO/IEC 27001 Annex A | Change management, secure development, segregation of duties | Exports are organized per control. Exact control mapping to be confirmed with an auditor |
| SLSA Source Track | Attestations about how a source revision was produced | The merge root and release attestation follow its shape |
| Agent Trace | An open format for line-level AI and human attribution | The attribution format read by `jtac who` |

The honest route to certification is to write the control to match this process and prove it is followed, not to simulate a human approval.

---

## 21. Decisions made in this document

These were chosen to make the design buildable. Each should be confirmed or changed before the code that depends on it.

1. **The `pre-push` gate applies to every push, by a person or an agent.** The gate judges the change, not the author, and coverage needs a root either way.
2. **`pre-push` does not run checks by default.** It requires a root and tells you to run `jtac check`.
3. **One root per push tip, with `covers` for the commits behind it.** Checks run on the tip's tree only.
4. **Tree fallback counts as covered.**
5. **Roots are stored as one git ref per tip under `refs/jtac/roots/`.** See open question 1.
6. **Canonical JSON (RFC 8785) and SHA-256.**
7. **Signing is SSHSIG through `ssh-keygen`,** using the developer's existing signing key where one is configured.
8. **A failed root push warns and lets the code push proceed.**
9. **The agent tool-call hook advises and never blocks.**
10. **v0.1 has no model call anywhere.**
11. **No default rule forbids or requires AI attribution in commit messages.** Provenance belongs in evidence. A repository can add a commit-text rule in its own policy.
12. **Waivers are capped at 30 days and fences are not waivable in v0.1.**
13. **License is FSL-1.1-ALv2 (Functional Source License): free for any use except a competing product or service, each release converting to Apache-2.0 after two years. It is source-available, not OSI open source, and the README says so. A contributor license agreement is in place before the first outside contribution.**
14. **The root carries `ts`, and the verdict function takes `now`.** Replay passes the root's `ts` back, so expiry is reproducible.
15. **Applied waivers are embedded in the root, each with its own signature.** They are the only nodes that leave the machine; other levers stay local.
16. **The signer is a principal plus a fingerprint.** The principal is what the allowed-signers file keys on.
17. **Integers only.** No float in any node, root, or policy.
18. **Merkle leaves are raw digests, RFC 6962 shape, with the known answers in section 9.3.**
19. **`counts` is derived and replay recomputes it.** A mismatch invalidates the root.
20. **Diff-integrity and an unsigned `jtac check --preview` land before store and sign.** A demo exists after six milestones.
21. **A reader accepts only canonical bytes.** Anything that would re-serialize differently is refused.
22. **`ts` is whole-second UTC in one spelling.**
23. **The error contract binds at the user surface.** Library errors are structured; the CLI and hooks write the three parts.
24. **Size and comment discipline is a gate rule, not only a house rule.** Section 10.3 `JT-SIZE`, `JT-CMT`, and `JT-ADD`; this repository runs the same caps in CI.
25. **The policy hash covers the resolved policy,** defaults filled in, so it names the rules actually applied.
26. **No `when` switch on checks, and no policy file refuses.** A required check always runs; `init` writes only the checks that apply; `check` never assumes defaults.
27. **The verdict takes gathered facts, not the policy, and enforces waiver scope itself.** `jtac-policy` depends on `jtac-core`, so `gate` cannot take `Policy`; and replay must not trust whoever wrote a waiver. A commit waiver passes with `declared_off` and keeps the reasons; a rule waiver covers findings only, never a failed check.
28. **Checks see an allowlisted environment, only a pass is cached, and the tool version comes from a probe table.** A value scrub cannot recognize every token; a cached failure blocks until a cache nobody sees is cleared; the policy has no version key, and a launcher shim's hash hides a toolchain upgrade.
29. **Diff-integrity findings point at a reserved `integrity` check, the rules diff lines themselves, and spans come from braces and indentation, not a parser.** A finding must name a check node and the verdict must fail when the rules did not run; a git setting must not move a finding between a machine and CI; a parser per language is a C build on three platforms for a v0.1 heuristic.
30. **Preview judges HEAD; the base is `--base`, else the upstream, else the default branch, and the last commit when that is HEAD; exit codes are 0 pass, 1 block, 2 could not answer.** A verdict about uncommitted content is evidence about nothing; the upstream is the nearest stand-in for a covered ancestor before a store exists, and a base equal to HEAD would let a test removal pass with an empty diff; hooks and CI read the code, not the banner.
31. **The store is the git common dir's `jtac/`, a root's leaves live in a local `nodes.json`, the root commit is plumbing-written, git-unsigned, and fixed-author at the root's `ts`, and a same-tip push collision is reported, not forced.** Linked worktrees must see one store; the root carries only the Merkle root and replay needs the leaves; `commit-tree` honours `commit.gpgsign` and a gpg prompt would hang every root write; one root and signature then give one commit id everywhere; and a second root for a tip already covered on the remote is evidence the remote has, not a reason to overwrite it. Open question 1 measured and closed.
32. **The signing key is `jtac.signingkey`, else git's SSH `user.signingkey`, else a dedicated key `init` generates; the principal is `user.email`; jtac never prompts for a passphrase; the trust file is `.jtac/signers` alone, judged at the document's `ts` with ssh-keygen run in UTC; the key ssh-keygen prints on a good signature is the one the document must name.** A developer already signing with SSH enrolls nothing, and everyone else gets a key whose compromise touches evidence only; one identity rule shared with git means no new knob; a hook that blocks on a passphrase prompt is a gate that hangs; two trust sources would leave a reader unsure which one said yes; a root's validity must not change when a key is later retired, nor with the verifier's time zone; and only ssh-keygen's own reading of a key gives the fingerprint `ssh-keygen -l` prints. Open question 2 closed.
33. **A root covers from the diff base, the ancestor walk is by commit, a covered HEAD is checked again and keeps its root, and `--preview` never touches the cache.** The first root vouches only for what its rules judged, so adopting jtac does not mark history as covered; a tree match is a reader's rule, since the writer would need every root's tree; evidence is never rewritten and a toolchain change must stay visible; a pass computed on a dirty tree under HEAD's tree id would poison the cache.
34. **The `pre-push` force fence is a non-fast-forward update of a protected branch, judged from the pushed tip's policy joined with the branch's old tip's; a valid root parses, names the tip, and its signature holds; a root with `from` also needs a signer the default branch's or the old tip's signers name for the CI namespace (section 15); the dedicated key lives under the user config directory and the global `jtac.signingkey` names it; a root ref is created, never replaced; the shell classifier is its own milestone.** A hook sees old and new oids, never flags; the tip's tree is the one the root judged, and an orphan commit with no base to diff from could otherwise carry its own `allow`; who signed is for `verify`; one identity per person is git's own rule; decision 33 removed every legitimate rewrite, so the loser of a race reports the winner; and nothing consumes the classifier before the advisory hook.
35. **A range is a `rev-list` expression; trust is `.jtac/signers` at HEAD read through git, or `--signers`; a root's `covers` and parent ids are judged against the clone, a missing parent root is a note; `check` keeps the resolved policy by hash for `replay`.** An option smuggled into a range would change what was walked; a trust file read from a root's own tree would let a pusher trust themselves, and one read from the work tree would let an unstaged edit or a link change a verdict; a forged `covers` would otherwise mark any commit covered, while a parent root deleted later is absence the row already shows; and a gate release that changes a default would otherwise make every older root unreplayable on the machine that wrote it.
36. **The classifier emits push, hook bypass, directory change and unreadable; its grammar is a parser crate behind one adapter module; a parse that is not clean is unreadable.** Nothing in v0.1 consumes a commit or a merge; a shell tokenizer is the one sharp edge in this tree that a maintained crate closes better than hand-rolled code, and the adapter is the seam that lets it go; a parser's recovery means something ran that this reader did not see.
37. **The agent tool-call hook advises on its agent's own advisory field and never decides; a bypass is told what lands and named no command; the hook caps itself at one second, `JTAC_PRETOOL_CAP_MS` sets another, and fails open; its install is opt-in per clone.** An advisory that could block would be a second gate with a weaker reach than the git hook; a bypass can be the operator's fence, and a command that undoes it would be a guess; the agent waits on every call, so an unbounded classifier is the agent's latency; and the settings file is the agent's own, named with an absolute path, so it is a per-clone write the owner asks for, not a merge into a tracked file.
38. **The commit that adds `.jtac/policy.toml` where no parent's history ever held one is judged under it, and a `.jtac/**` change on that commit is a minor finding, not a major one.** Nothing was in force to protect; the first root still records the finding, and a later change under `.jtac/` is major as before. The fact is gathered from the history behind every parent: the diff's base is the pusher's to choose with `--base`, and a merge whose first parent never held a policy would otherwise weaken the one its second parent holds. Adopting by merging a side branch that already holds the policy is therefore major, and takes a waiver. Without this the adoption commit aborts on the policy it introduces, and the first five minutes need a waiver.
39. **The install script verifies the release attestation (section 16.3) against the `jtac-release-v1` lines of `.jtac/signers` it carries, held equal by a test, installs only the archive that document names, at its digest, refuses a release without one, installs under `~/.local/bin` or `--to`, and never edits PATH. The README publishes bench's control count and the gate's measured overhead on this repository, and states catch rate and false block rate as unmeasured, with the method. `jtac policy` and `jtac scope` move to v0.2.** A provenance tool whose installer downloads an unverified binary contradicts itself, and a checksum published beside the archive changes with it; an older signed release put in place of `latest` still installs, which only immutable releases close (#122); a rate from one developer's repositories is the number section 23 warns against; v0.1 ships what exists.
40. **The CI gate is the same binary knowing no forge: `jtac ci --base`, trust from the base rev's tree unless `--signers` names a file, signing nothing, answering with exit codes; the forge adapter is a workflow file, GitHub's a composite action in this repository; the verifier's stance is the flag `--require-local-root`, never a policy version 1 key.** A forge client in the binary would be a second trust root and a second thing to verify offline; one workflow file per forge is the whole adapter; a new policy key is a shipped-format edit that every 0.1.x hook refuses and a hash change for every root they wrote.
41. **CI signs with an SSH key held in an environment secret whose rules allow the default branch and protected `v*` tags alone, under a CI principal in `.jtac/signers` whose line confines it to the namespaces `jtac-ci-root-v1` and `jtac-release-v1`, which jtac reads itself; a leaked key's line is removed, never retired; keyless signing, when it comes, is a new namespace with its own reader.** No new signing code, the trust file already says who may sign, and verification stays offline; a repository secret reaches a same-repository pull request's own workflow and any tag's release job, so the environment rule is the fence; a CI key confined to its namespaces can sign no waiver, and ssh-keygen reads a line with no option as every namespace, so only an explicit option can say which key may sign a merge root; `valid-before` is judged at a `ts` the signer writes; a CI root proves the runner holding the key judged the tree, and that limit is written in section 15. Open question 7 closed.
42. **CI's verdict is one run from the merge base with the base branch, trust from base, with the trusted levers of the head's chain as its waivers and the pull request's commits as the commit-waiver scope, a commit waiver in scope declaring the verdict off, a `.jtac/**` change under base's policy and a base holding no signers file as adoption; the head's root alone is compared, on its own declared base with its own levers as it applied them at its own `ts`, state and counts, only under this gate's version and the hash this gate computes for the policy the root should have judged under, with that policy's checks run too when it is not CI's; an invalid, lying or disagreeing head root fails the check; an unknown signer is reported, never invalid; a missing head root, or one from another gate version, is recorded and passes unless `--require-local-root`.** A base no pusher's evidence can move judges every commit, so roots behind the head need no comparison; a waiver signed on an earlier push approved that push's diff, so the pull request is its scope; the comparison must diff what the root diffed and judge waivers when the root did; nodes never travel, so counts under one gate and one policy are the comparison that exists; the root's own hash and version cannot be what switches the comparison off, and a rule table can change counts under one hash; the waivers base trusts are the only human approval CI can see; a default that fails every fork pull request, or every contributor whose key is not yet trusted, stops adoption, and absence recorded is the honest state (section 11.5).
43. **CI runs `jtac ci` on every push to the default branch, with the push's old tip as `--base`, and writes a root only for the forge's merge of a pull request, in the CI namespace, as one verdict: the old tip to the tip, under the old tip's policy when `.jtac/**` is in that diff, with the head chain's trusted levers in the pull request's scope, the old tip holding no signers file as the adoption test, the head's root compared again and `from` carrying the head, its root and what the comparison made of it, recorded and never a veto, nothing reused from the pull request job; it pushes the head's root ref only when that outcome is agreed, and no root behind it, since the merge root covers the whole pull request; a `from` root whose signer's line does not name the CI namespace is an unknown signer's, and the hook refuses one that neither the default branch tip's signers nor the old tip's name, the one trust question the hook asks; `audit` reads the head from `from`; the CI run's store is a workflow artifact.** One root per tip is the ref layout and a standing root is kept; `jtac ci` on every push is what catches a forged root pushed straight to the branch, and a CI root for a direct push would sign over a bypass; the old tip is the one base that covers a merge commit, a squash and every commit of a rebase, and the diff from it is what landed; a root is one verdict (section 9.4); the pull request job ran the pull request's own workflow, so nothing it cached is this identity's; the pull request job writes nothing durable and a root can arrive after it ran, so the merge job compares again and the merge root is where the outcome, or the absence, is recorded; an unknown signer is never invalid (section 11.9), and the scope is the root's shape, so trust decides only who is believed; decision 38's every-parent test would make every forge merge of an adoption pull request major; a fork's root lives in the fork until the merge carries it, and only one CI found honest enters a ref every clone fetches by CI's hand, a same-repository head's standing as the developer's, and a fork owner who controls that ref must not control whether the branch gets a root; the durable copy is the collector's.
44. **The first `v0.2` release is M15 to M17; the release gate is M18 as the next patch; tiering, reuse, attribution and org policy follow as later patches.** The independence slice is what makes absence cost something, and a release gate is thin once `verify --require-coverage` exists.
45. **On the adoption commit every protected-path finding is minor, not only `.jtac/**`; locally the adoption test is decision 38's, widened to every protected path when that same commit adds the first `.jtac/signers`, and in CI and under `check --from` it is the branch, or the old tip, holding no signers file.** Decision 38's reason, nothing was in force to protect, holds for `.github/**` as much as for `.jtac/**`, and with no signers file no waiver could ever have cleared anything, so the first signers file is where CI adoption begins in a fresh repository, while one gated locally first adds CI under one waiver, since its local gate already fences an agent and a signers file an agent adds must not lift it; the stranger's first commit must carry the workflow, the signers line and any policy together, or it blocks on the gate it installs; the local base is the pusher's, so history is the test there, while CI's base is the branch.
46. **Root version 2 adds `base`, absent on a parentless tip, and `from` with the head root's id, bounds section 9.4's `covers` walk by `base`, which a reader checks entry by entry and never for completeness, and keeps version 1's `parents`, every root the walk stopped at, which for a root with `from` is `base`'s alone; the verdict takes the commits a commit waiver may name as a fact, one for the local gate, and `Root::validate` accepts any commit in a `from` root's waivers; version 1 stays readable and is never compared.** A shipped format is never edited (section 13), the bounded `covers` does not record the diff base, a parentless commit is judged against itself (section 10.3), a merge root must name the head it stands for and what it had, a root must vouch for nothing below the diff its rules judged, or a rootless commit under a merge would read as covered by it, a merge root's `parents` must name no root of the head's chain, or a root published or withheld later would change what it means, and a pure function cannot know which commits a pull request holds unless told, nor can a clone without the head's history, so the CI signature carries that check; a version 1 root is always from another gate version.
47. **`check --from` runs the head's checks in a detached worktree outside the clone when the tip's tree is not the head's, and under no policy but the head's own or the branch's; pushes the merge root and an agreeing head root through `--push <remote>` outside the hook with its guard set; refuses a head that is HEAD, a head reachable from the old tip, and an old tip off HEAD's first-parent line; and the default-branch job runs one at a time per branch.** The pull request job's result is not durable, so the comparison needs the head's checks, and they need the head's tree, which the merge commit's checkout is not, and a worktree inside the clone would let cargo or node read the merge tree's configuration as the head's, while a guessable path in a shared temp dir would let another user fill it first; the job holds the CI key, and a root's declared base is its signer's to choose, so a policy from neither the head nor the branch must not run there; a push through the hook would be judged against the pushed tip's own signers; a fast-forwarding forge leaves the head as the tip, whose developer root stands under decision 33; a merge, a squash and a rebase all leave the old tip on the tip's first-parent line, so a base off it is the head's own side, a rewind or another line, and a head behind the old tip added nothing; two jobs writing one branch's roots race for `parents`. Issue 85 closed.
48. **A bypass merge's missing CI root is recorded as a CI waiver: a commit waiver whose scope names the gate, node version 2 with its version 1 reader, written on that lever alone, signed by a developer key in `jtac-lever-v1`, carried unapplied by the root `check` writes on the commit and read as the CI gate's `declared_off`; no new lever action, no new coverage state, no expiry; whether it attests is `release`'s decision.** Section 9.5 already names the state a person produces by turning a gate off on purpose with a reason, so an explanation-only lever would be a fourth state every reader learns twice; `Action` and `Scope` refuse unknown words, so a new word is a format edit unless the version moves, and moving it on one node keeps every other id; the CI key cannot sign a lever, so a job never explains its own absence; a verdict that applied it would let a developer's signature stand in for the other machine's. Issue 100.
49. **The GitHub adapter is one composite action at the repository root that the event steers: `jtac ci` on a pull request, on a push to the default branch `jtac check --from <head> --base <old tip> --push` when the forge's merged pull request lookup names the tip and its head is not the tip, else `jtac ci` from the old tip, nothing for the push that creates the branch, and a refusal for a force push or an old tip the clone cannot fetch; the head repository supplies roots for the pull request's own commits alone; it installs the release its own text pins, held equal to the workspace version by a script the release workflow runs, and uploads the store as an artifact; `jtac init --ci github` writes the workflow, the first `.jtac/signers` and a CI key under the user config directory, one per repository, never printed; the first v0.2 tag is the version the manifest carries after M17.** a push event names no pull request head, so the adapter asks the forge which merged pull request the tip closes, a merge, a squash or a rebase alike, and the binary never does; a key per repository scopes a leak to one; a literal version is the one thing a pinned action can trust about itself, since a ref pinned by hash is not a release name; a private key printed to a terminal is a leak by design; the manifest passed 0.2.0 with the patches M15 and M16 shipped under.
50. **`jtac release <tag>` attests the range from the previous attestation, or `--from` for the first, only when every commit is covered or declared off by a root the tag's own tree trusts, `--require-ci` holding the first-parent line to the CI gate; the attestation is `release.json` version 1 signed in `jtac-release-v1` at `refs/jtac/releases/<tag>`, trusted only under a signers line that names the namespace, carrying the artifacts' digests, written once and re-derived by `--verify`.** An unknown signer's root is absence for a release, since a release vouches for what a trusted key judged; the first `v0.2` tag sat on the pull request commit with the developer's root alone, which `--require-ci` names; a line that names no namespace would let any developer attest, and `--signers` never signs; a start taken from any tag, or from any document under the namespace, would let one be planted that nothing judged, so a start verifies as the attestation it claims to be; a parentless commit under `refs/jtac/` travels on the refspec every clone has; and counts in a signed document are a convenience the reader re-derives, as `replay` does a root's.
51. **`jtac backtest <range>` reports the diff-integrity rules over merged history, one unit per first-parent commit against its first parent, under one policy named by `--policy` or the newest commit's tree, with no waiver, no adoption and no check run, writing nothing; its summary holds the policy's hash, counts and rule ids only; a size or comment finding says whether the before side already broke the cap.** Catch rate and false block rate were unmeasured for want of a method (decision 39, open question 6), and a team asked to require a gate needs its numbers on their own history first; the first-parent diff is what landed whatever the merge style, and grouping a rebase's commits needs the forge; one policy answers what today's rules would have said; a summary with no path or message can be shared from a repository whose code cannot; a blocking finding on a condition the change did not create is the false block most likely in a large tree, so the report has to tell the two apart before any rule changes.

## 22. Open questions

1. **Ref scale.** Decided in M7: one ref per tip stands. Measured with `scripts/ref-scale.sh` on a local filesystem: at 10k roots every operation is under a second (push of one new root ref 0.3s, fetch with nothing new 0.3s); at 100k roots `for-each-ref` is 0.5s, a push of one new root ref 3.2s, a fetch with nothing new 2.6s, and a plain `git fetch origin main` 1.2s, because both ends still walk the namespace. `git status` and `rev-parse` do not change. GitHub accepts pushes to and deletions of `refs/jtac/*`; other forges are unconfirmed. The fallback, if a repository approaches 100k roots, is periodic compaction into a single notes-style ref, as a new ref layout version.
2. **Key handling.** Decided in M8 (decision 32): both, in order. A developer who signs commits with SSH reuses that key; anyone else gets a dedicated key from `jtac init`; `jtac.signingkey` names a dedicated key for anyone who wants one regardless.
3. **Which agents get a tool-call adapter in v0.1** beyond the first. M13 ships Claude Code's; a second is a sibling of `wire/claude.rs` and a name on `jtac hook pre-tool` and `jtac init --agent`.
4. **Test-integrity precision.** Decided in M5: `JT-TEST-003` ships as major in every language, scoped to a test present on both sides by name. Its false block rate is one of the launch numbers (section 14).
5. **Windows.** Decided in M14: the test suite runs on Windows in CI, the hooks are unverified there, the install script is `sh`, and a Windows user takes the zip from the release page. The README says so.
6. **Where the launch numbers come from.** Catch rate and false block rate need outside testers, not one developer's machines.
7. **CI identity in v0.2.** Decided (decision 41): an SSH key in a CI secret under a CI principal in `.jtac/signers`; keyless signing as a later namespace.
8. **Collector storage in v0.4.** A relational database, an object store with a signed index, or a transparency log.
9. **Whether the certification an adopting org targets accepts a low tier with no human approver.** Confirm with an auditor before tiering is presented as satisfying a control.
10. **Who may sign a CI waiver.** Any key the merge's `.jtac/signers` trusts in M16c; whether tiering requires a named owner of the area is decided with tiering.

## 23. Landscape (as of October 2026)

| Project | What it is | Where it stops |
| --- | --- | --- |
| LLM review platforms | Hosted reviewers with policy features | The model is trusted. SaaS first |
| Static analysis with AI modes | Deterministic gates, stricter for projects flagged as containing AI code | Project-level and self-declared. No per-commit evidence |
| Agent attribution specs | Open formats for which lines an agent wrote | A data format, not a gate |
| Supply chain attestation (SLSA, gittuf) | Attestations about how source and builds were produced | Integrity of the pipeline, not quality of the change |
| Public review benchmarks | Open benchmarks for AI review agents on pull requests. The best published result at launch was about 40% grounded F1 | They measure review, not a pre-push gate. Useful as an external reference for reviewer agents from v0.3 |
| On-chain agent receipts | Signed receipts for agent actions on a blockchain | Requires a chain. jtac's evidence rides in git and verifies offline |

No public benchmark measures a pre-push gate. Publishing catch rate and false block rate with a reproducible method is an opening.
