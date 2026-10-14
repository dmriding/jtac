# jtac: design

Provenance and verification for AI-written code.

The name is short for Joint Terminal Attack Controller: the controller who clears a release only on positive identification and under the rules in force. Here the agent does the flying, and nothing leaves the machine until it is cleared. The word "terminal" also happens to be where the tool lives.

Status: design, pre-implementation. Date: 2026-10-10. License: FSL-1.1-ALv2.

This document is the source of truth for building jtac. Sections 1 to 6 say what the system is and why. Sections 7 to 14 specify v0.1 in enough detail to build. Sections 15 to 20 design the later releases, so v0.1 does not paint them into a corner. Section 21 lists the decisions made here that the maintainer should confirm, and section 22 lists open questions.

---

## 1. Summary

jtac is for solo developers and teams who work at AI speed and are bottlenecked by slop and pull request review. It is the quality and safety gate that lets them keep that speed without bloat or broken code reaching production: it proves quality and review with evidence, instead of trusting a model or a rubber stamp.

It works at three points on one evidence chain:

1. **Local gate.** Checks a change before it leaves the developer's machine and records signed evidence.
2. **CI gate.** Verifies that evidence independently and decides: merge, human required, or reject.
3. **Review packet.** Hands a human reviewer only what could not be proven.

One sentence for the README: your agent cannot push slop, and you can prove it.

v0.1 is the local gate alone. It is complete and useful by itself for one developer, needs no account, no server, and no model.

## 2. Problem

AI agents produce changes faster than people can read them, and review processes built for human-speed work have not adapted.

- **Slop reaches main.** Code that passed a self-check is merged without anything independent looking at it.
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
| A developer in a hurry | Skip the hooks with `--no-verify`, or uninstall jtac | Cannot be prevented locally. The commit is recorded as absent, and CI treats it as unverified |
| A developer forging evidence | Sign a root for checks that did not pass | A local root is a claim. From v0.2, CI re-runs checks and compares. Mismatch is a finding |
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
| `jtac init` | Installs hooks, sets up the signing key and local store, writes a default policy if none exists |
| `jtac check` | Runs the gate on HEAD. It judges code and writes a root on pass |
| `jtac status` | Says whether a push would pass and what is standing: waivers, coverage of unpushed commits |
| `jtac verify [range]` | Checks signatures and chain on roots, yours or anyone's. It judges evidence |
| `jtac replay <commit>` | Recomputes the verdict from recorded nodes and compares it with the root |
| `jtac audit [range]` | Coverage table for a range: covered, declared off, absent |
| `jtac why` | Explains the last block and the fix |
| `jtac show [range]` | Prints exactly what the next push will carry, byte for byte |
| `jtac waive` | The recorded, signed off-switch, with a reason |
| `jtac bench` | Model-free self-test: attacks every control through its real path |

Every command accepts `--json` and returns the same structure the human output is rendered from.

### 7.2 Shorthand

Each shorthand is an alias for a plainly named command. Documentation and error messages use the plain names.

| Shorthand | Plain command | Shows | Arrives |
| --- | --- | --- | --- |
| `jtac roe` | `jtac policy` | The rules in force and where they came from | v0.1 |
| `jtac ao` | `jtac scope` | The paths and branches an agent is cleared to work in | v0.1 |
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

v0.1 node kinds and bodies:

| Kind | Body fields |
| --- | --- |
| `check` | `id`, `argv`, `tool_version`, `exit`, `outcome` (`pass`, `fail`, `skipped`, `error`), `stdout` and `stderr` (each `{ "sha", "bytes", "truncated" }`), `duration_ms`, `cached_from` (id of the original node; absent on a fresh run) |
| `finding` | `check` (node id), `rule`, `severity` (`blocker`, `major`, `minor`), `path`, `line`, `message` |
| `lever` | `action` (`init`, `waive`, `policy_change`, `trust_change`), `signer` (`{ "principal", "fingerprint" }`), `reason`, `scope` (`{ "commit": <id> }` or `{ "rule": <rule>, "path": <prefix> }`; absent when the action has none), `expires` (absent when none) |

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
- `levers` holds every waiver the verdict applied, each as its lever node in full plus the SSHSIG over the node's canonical bytes (namespace `jtac-lever-v1`). A waiver may have been written on an earlier tree. They are leaves of `evidence` like any other node and the only nodes that leave the machine. Other lever kinds stay local.
- `counts` is derived: `checks` is the number of check nodes, `pass` those with outcome `pass`, `fail` the rest, `findings` by severity, `waivers` the length of `levers`. `jtac replay` recomputes it, and a mismatch invalidates the root.
- `policy.version` is the policy schema version (section 10.2).
- `signer.principal` is the identity looked up in the allowed-signers file (section 11.8). `signer.fingerprint` is for people and audit rows.

- `covers` lists the commits between the nearest covered ancestors and the tip, oldest first, ending at the tip. Checks run on the tip's tree, and the root vouches for the path that led to it. Past a bound (default 256) the root records `{ "base", "tip" }` instead of the list.
- `parents` links to the roots of the nearest covered ancestors. An empty list on a non-initial commit is a visible break in coverage.
- `state` is `covered` or `declared_off`. There is no root for `absent`. Absence is the missing root.
- A `declared_off` root carries a waiver whose scope is `commit` and names this root's commit. A rule-scoped waiver leaves the root `covered`.

Known answer (test `root_id_known_answer`). A `declared_off` root over the empty tree with one commit waiver and no checks, canonical:

```
{"commit":"sha1:0000000000000000000000000000000000000001","counts":{"checks":0,"fail":0,"findings":{"blocker":0,"major":0,"minor":0},"pass":0,"waivers":1},"covers":["sha1:0000000000000000000000000000000000000001"],"evidence":"sha256:60d73f36e3a666adcfe47ee42cd116a0c910364dcb1f096477d9d1e56e9b7d40","gate":{"binary":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","version":"0.1.0"},"levers":[{"node":{"body":{"action":"waive","reason":"example reason","scope":{"commit":"sha1:0000000000000000000000000000000000000001"},"signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"}},"kind":"lever","policy":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1},"sig":"example-signature"}],"parents":[],"policy":{"hash":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","source":"repo","version":1},"signer":{"fingerprint":"SHA256:AAAA","principal":"dev@example.com"},"state":"declared_off","tree":"sha1:4b825dc642cb6eb9a060e54bf8d69288fbee4904","ts":"2026-10-10T18:30:00Z","v":1}
```

has id `sha256:01e449f228a88b336568a36424831f2c866c94d24b0916312442da7d89b67091`. Its lever node has id `sha256:936b6c55423cccc86fb7a22c9f90de3a359073f814ffd9be80ac9f316b9a7922`, and `evidence` is the one-leaf Merkle root over it.
- The root is signed as a detached SSHSIG signature, namespace `jtac-root-v1`, over its canonical bytes.

### 9.5 Coverage states

| State | What exists | What it proves |
| --- | --- | --- |
| Covered | A valid signed root that names or covers the commit | The gate ran with this version and policy, and passed |
| Declared off | A valid signed root with a waiver | The gate was turned off on purpose, by this key, for this reason |
| Absent | No root, or one that fails verification | The gate was not there |

**Tree fallback.** Checks judge a tree, so a commit with no root of its own counts as covered if its tree equals the tree of a covered root. This handles squash and rebase merges that produce a new commit with identical content. `jtac audit` reports these as `covered (by tree)`.

### 9.6 Storage

**Local.** `<git common dir>/jtac/` (`.git/jtac/` in a plain clone; linked worktrees share the main one) holds the content-addressed store: `objects/` for nodes and captured output, each file named by the SHA-256 of its bytes; `roots/<root id>/` for `root.json`, `root.sig`, and `nodes.json`, the node ids that are the root's Merkle leaves, which replay needs and the root does not carry; `cache/` for the check cache; and `state.json` for the last verdict. It lives inside `.git` so it is never committed and is per clone. Every read re-hashes what it finds: a file that is not what its name says is an error, never a document.

**Shared.** Each root is a parentless git commit whose tree holds `root.json` and `root.sig`, written with plumbing and unsigned at the git level (`root.sig` is the signature), authored `jtac <signer principal>` at the root's `ts`, so one root and signature give one commit id on every machine. It is referenced at:

```
refs/jtac/roots/<first two hex of commit>/<commit>
```

- One ref per covered tip means no contention between developers pushing different tips at once. Two machines that each check the same tip both write a root for it, and the second push is rejected as a non-fast-forward: the remote already holds a root for that commit, which is reported and is not a failure of the gate.
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

The rules are pure over a change set: for each path, its content before and after, and the old name of a rename. The gatherer reads paths and blobs with `git diff --raw -M` and `git cat-file`; the line diff is jtac's own (Myers over lines), so no `diff.algorithm` or whitespace setting can move a finding and a CI replay sees the same lines. A blob with a NUL in its first 8000 bytes is binary, as git decides, and no rule reads it. Which base to diff against is the caller's: the nearest covered ancestor once a store exists; before that, `--base <rev>` names it, else the merge base with the branch's upstream, else with the default branch (origin's HEAD, then `main`, then `master`), by full ref name so a tag cannot shadow a branch. When that merge base is HEAD, or nothing names one, the rules judge the last commit against its first parent; a root commit has no change to judge. A diff git cannot read errors the `integrity` check; it is never an empty diff. Findings point at the reserved `integrity` check (section 9.2): `facts` lists it first and required, so a run that skipped the rules fails the verdict, and a policy file may not name a check `integrity`. Markers match with strings blanked, so a marker quoted in a test table or a template is not one, and a Rust attribute split across lines is read whole, one that read the same before being no new marker.

- `JT-PROT-001` matches either name of a rename, and the tiers a changed path matches are reported beside the findings.
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

1. **Preflight.** The working tree must be clean and HEAD must be the commit under test. If git cannot answer, that is a refusal, never a pass. `jtac check --preview` runs the checks, prints the verdict with its reasons, and writes nothing; it is the only mode that runs on a dirty tree. Preview judges HEAD: the policy is HEAD's `.jtac/policy.toml`, the rules diff the base to HEAD, and every node names HEAD's tree; on a dirty tree the output says the checks saw the work tree and the rules did not. Exit codes: 0 `CLEARED HOT`, 1 `ABORT`, 2 the gate could not answer (no repository, no policy, git failed); a refusal goes to stderr in three parts and a verdict to stdout, and the `--json` structure carries `v` like every shipped shape.
2. **Load policy** and compute its hash.
3. **Run diff-integrity checks** first. They are fast and their failures are the most informative.
4. **Run configured checks**, consulting the cache. Cache key: tree id, policy hash, check id, tool version. Only a `pass` is cached; a failure reruns, because a cached failure would block until a cache nobody sees is cleared. A hit is a new node whose `cached_from` names the original.
5. **Compute the verdict** with the pure function.
6. **On pass:** build the Merkle tree, write and sign the root, print `CLEARED HOT`.
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
- Otherwise passes.

`Verdict` carries pass or fail, the state, the ordered list of reasons (checks in policy order, unknown check ids in node order, then findings in node order), and the waivers it applied: every commit waiver in scope, and each rule waiver that covered a finding at or above `block_at`. Those become the root's `levers`. It is the only place a pass or fail is decided, and `replay` calls the same function.

### 11.3 Hooks

**git `pre-push`** is the gate. For each ref being pushed:

- Deleting a ref, or pushing a tag, passes without checks.
- A force push to a protected branch is a fence.
- For the tip commit, there must be a valid root (state `covered` or `declared_off`), or a root for an identical tree. If not, the push is refused with the exact command: `jtac check`.
- The hook never runs the checks itself by default, so a push is never slow or surprising. `jtac check` is the explicit step. A config option runs it inline.

**git `commit-msg`** is reserved for policy rules on commit text. It ships empty in v0.1.

**Agent tool-call hook.** For agents that expose a pre-tool-call hook (Claude Code's `PreToolUse` first, others by adapter), `jtac hook pre-tool` reads the proposed shell command, classifies it, and returns an advisory note on the tool call: what it is about to hit and the command that clears it. It tells the agent about a fence before the agent walks into it. It advises and never blocks. The git hook is the enforcement point, because it covers every agent and every person identically.

Hook rules:

- A hook's stdout is a protocol wire. Nothing prints to stdout except the protocol. Diagnostics go to stderr.
- A hook that errors internally fails closed for push and open for the advisory.
- `jtac init` refuses to overwrite a hook file it did not write, and reports it.
- Hook commands are written with absolute paths and single quotes.

### 11.4 Command classification

The classifier turns a shell script into the list of actions it performs: push, commit, merge, force, hook bypass, config change. It is pure.

It must read the script as the shell does: `&&`, `;`, pipes, subshells, command substitution, leading assignments, wrappers (`env`, `sudo`, `nice`), `git -C`, a leading `cd`, and the body of `bash -lc '...'`.

What it cannot read, it reports as unreadable: `bash -c "$VAR"`, `eval`, a command inside another interpreter, substitution nested past a depth limit. "I cannot see what this runs" is never treated as "this runs nothing".

Bypass spellings it must recognize as hook bypass: `--no-verify`, `-n` on commit, `-c core.hooksPath=`, `--config-env=core.hooksPath=`, `GIT_CONFIG_KEY_n=core.hooksPath`, and `git config core.hooksPath`.

Every spelling lives in one table test. A new bypass found later is a new row, with the fix.

### 11.5 Bypass is recorded, not prevented

jtac cannot stop someone from running `git push --no-verify` or uninstalling it. It does not try. The commit simply has no root, and every later reader (`jtac audit`, CI, an auditor) sees it as absent. This is the honest design: the local gate provides speed and early feedback, and the proof of what was and was not checked comes from the evidence chain.

### 11.6 `jtac waive`

```
jtac waive HEAD --reason "obsolete test removed with the feature it covered"
jtac waive --rule JT-SUP-001 --path src/ffi/ --reason "..." --expires 14d
```

- Writes a `lever` node, signed, with the reason verbatim.
- A waiver is scoped: one commit, or one rule on one path with an expiry at most 30 days after the waiver was written. A waiver written after the root that cites it is refused. The verdict function and root validation apply these bounds (section 11.2); `waive` only refuses earlier.
- A waived root has state `declared_off` when the waiver covers the whole commit, and stays `covered` with `waivers > 0` when it covers one rule.
- Every waiver in effect is printed by `jtac status` and on every `check` and push it affects.
- Fences are never waivable from the CLI in v0.1.
- The `waive` command refuses to run when it detects an agent session (environment markers set by known agents). This is a speed bump and is documented as one: one account runs both the person and the agent, so it cannot be a lock. The safety comes from the waiver being signed, recorded, and loud.

### 11.7 `jtac init`

1. Confirms it is in a git repository and finds the remote.
2. Finds a signing key: `user.signingkey` when `gpg.format` is `ssh`, else asks to use an existing public key or generate a dedicated one under the user config directory.
3. Creates `.git/jtac/`.
4. Writes `.jtac/policy.toml` from detection if none exists, and prints it.
5. Installs the git hooks and, where an agent is detected, its tool-call hook. Prints exactly what it wrote and where.
6. Adds the fetch refspec for `refs/jtac/*`.
7. Writes a `lever` node with action `init`.

`--dry-run` prints everything and writes nothing. Running `init` twice is a no-op. It never touches PATH, shell rc files, or any config file it cannot parse.

Target: a working gate on any repository in under five minutes, with no account and no network beyond the user's own git remote.

### 11.8 Signing

- Signatures use SSHSIG via `ssh-keygen -Y sign` and `-Y verify`, the same mechanism git uses for SSH-signed commits. No new key format and no new trust store.
- Namespaces: `jtac-root-v1` for roots, `jtac-lever-v1` for lever nodes. A signature made in one namespace does not verify in another.
- A waiver's signature travels in the root's `levers[].sig`, so a remote verifier checks the waiver's signer as well as the root's.
- Verification reads trusted keys from `.jtac/signers` (an allowed-signers file, a protected path). With no trust file, `jtac verify` reports the signature as valid and the signer as unknown, and says so in plain words.
- A missing or unreadable trust file is an operational error, never a silent false.

### 11.9 Reading evidence

- **`jtac verify [range]`** checks, for each commit: a root exists or a tree match exists, the signature verifies, the signer is trusted or unknown, the root's commit and tree match the repository, and the parent links resolve. Exit code is non-zero if any root is invalid. Absent commits are reported and do not fail verification unless `--require-coverage` is set.
- **`jtac replay <commit>`** loads the root and its nodes from the local store, recomputes the Merkle root, the counts, and the verdict with the root's `ts` as `now`, and compares. Any mismatch invalidates the root. On another person's machine the nodes are not present, so replay reports that the root verifies and the detail is not available locally.
- **`jtac audit [range]`** prints one row per commit with its coverage state, signer, policy version, and gate version, then the totals and the percentage in each state.
- **`jtac show [range]`** prints the roots and signatures that a push of this range would send, exactly as they would be sent.
- **`jtac why`** prints the last verdict's reasons, each with its rule, why the rule exists, and the next command.

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

**`jtac bench`** is the public proof. It needs no model and no network. It builds a scratch repository and attacks each control through the real code path: push without a root, push with a forged signature, a root for a different tree, a weakened test, a new suppression, each hook-bypass spelling, a policy file that does not parse, a check that cannot spawn. Each case must be refused, and the output quotes the gate's own message. It exits non-zero if any control fails, and it runs as a test so a regression fails the build.

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
| M10 | `jtac-gate` classifier, `pre-push` hook, `jtac init` | A push without a root is refused with the exact command. Every bypass spelling is in the table |
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

The CI gate closes the gap stated in section 6: it is a different identity on a different machine.

- **Form.** The same `jtac` binary run in CI, shipped first as a GitHub Action, with the forge behind an adapter so GitLab and others follow.
- **Verification.** For a pull request it fetches the roots, verifies signatures and chain, and re-runs the required checks. Where its results differ from the local root, that is a finding against the root, not just a failed build.
- **Reuse.** With policy permitting, checks whose cache key matches a trusted prior CI run are reused. Local results are a hint for ordering and never a substitute.
- **Identity.** The CI gate signs with its own key, held by the CI system and never present on a developer machine. It reports a required status check. It never posts an approval.
- **Merge roots.** At merge it issues a root for the merge commit, signed by the CI identity, linking to the roots of the commits merged.
- **Tiering.** Policy classifies each change by blast radius. Low tier merges on the CI verdict. High tier requires a named human with ownership of the area.
- **Policy changes.** A change to `.jtac/` is always high tier and cannot be cleared by the run that contains it.
- **Attribution.** Agent adapters record line-level attribution in the open Agent Trace format where the agent provides it. `jtac who` reads it.
- **Org policy.** Policy can come from a signed release of a policy repository (section 18.3).

## 16. v0.3: review round, review packet, release gate

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

- `jtac release <tag>` attests a release only if every commit in it has a complete evidence chain, and names the commits that do not.
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

## 22. Open questions

1. **Ref scale.** Decided in M7: one ref per tip stands. Measured with `scripts/ref-scale.sh` on a local filesystem: at 10k roots every operation is under a second (push of one new root ref 0.3s, fetch with nothing new 0.3s); at 100k roots `for-each-ref` is 0.5s, a push of one new root ref 3.2s, a fetch with nothing new 2.6s, and a plain `git fetch origin main` 1.2s, because both ends still walk the namespace. `git status` and `rev-parse` do not change. GitHub accepts pushes to and deletions of `refs/jtac/*`; other forges are unconfirmed. The fallback, if a repository approaches 100k roots, is periodic compaction into a single notes-style ref, as a new ref layout version.
2. **Key handling.** Is a dedicated jtac key better than reusing the commit-signing key? Reuse means no enrollment. A dedicated key means a compromise of one does not affect the other.
3. **Which agents get a tool-call adapter in v0.1** beyond the first.
4. **Test-integrity precision.** Decided in M5: `JT-TEST-003` ships as major in every language, scoped to a test present on both sides by name. Its false block rate is one of the launch numbers (section 14).
5. **Windows.** Hooks and `ssh-keygen` behave differently there. Decide whether v0.1 supports it or says so plainly.
6. **Where the launch numbers come from.** Catch rate and false block rate need outside testers, not one developer's machines.
7. **CI identity in v0.2.** A long-lived key in CI secrets, or keyless signing bound to the workflow identity.
8. **Collector storage in v0.4.** A relational database, an object store with a signed index, or a transparency log.
9. **Whether the certification an adopting org targets accepts a low tier with no human approver.** Confirm with an auditor before tiering is presented as satisfying a control.

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
