# Changelog

Keep a Changelog style, semver pre-1.0 as described in `docs/RELEASING.md`. Every pull request
bumps the version and adds its entries under Unreleased, tagged with that version. A release pull
request moves them under a dated heading and bumps nothing.

## [Unreleased]

### Fixed

- (0.0.15) `JT-SUP-001` and `JT-TEST-002` count the copies of a multi-line attribute per item
  it decorates, so a copy moved to another item is new (issue 17). The scanner follows `${ }`
  depth in a JS template and `/* */` depth in a Rust block comment, so a closer inside either
  no longer leaves the rest of the file in the wrong state (issue 18). A column-0 `#[cfg(test)]
  mod` inside a test tail nests instead of resetting the tail's depth, as `scripts/loc-cap.sh`
  reads it (issue 19). `JT-CI-001` reads a `#` line continuing a `"` or `'` scalar as text,
  not a comment (issue 20).
- (0.0.14) A ref under `refs/jtac/roots/` that is not `<xx>/<commit>` is skipped with a note
  instead of refusing `check`, `status`, and the hook in every clone that fetched it (issue
  30; DESIGN.md 9.6). Every git child jtac spawns runs without `GIT_CONFIG`, which steers
  `git config` alone, so a config read can no longer differ from what `git push` sees (issue
  32). The mirror refusal names `--file <origin> --unset-all` per file the key is set in
  (issue 33). A remote, branch, ref, key or path that enters a refusal's command is rendered
  by one `sh` word renderer: plain as is, quoted otherwise, a placeholder when no code span
  can show it; the gate printed them bare since M10.

### Changed

- (0.0.18) The README says what the gate does today, command by command, how to adopt it,
  and the roadmap by version; the preview-only status is gone.
- (0.0.16) The Windows test job runs on a push to `main`, a release tag and a manual
  dispatch, not on a pull request, so a pull request bills one Ubuntu test leg; the required
  check is `test`.

### Added

- (0.0.19) This repository is gated by jtac: `.jtac/policy.toml` runs the fmt, clippy and test
  bars with CI's flags and protects `scripts/**` beside the defaults; `.jtac/signers` trusts the
  maintainer's key. The adoption commit trips `JT-PROT-001` on `.jtac/`, so its root carries
  the rule waiver the README's quick start names.
- (0.0.18, M12) `jtac waive`: a signed `lever` node scoped to one commit (`declared_off`)
  or to one rule under one path for `--expires <days>d`, at most 30, kept in the store
  until a verdict applies it. `check` offers every stored waiver to the verdict and embeds
  the applied ones in the root as `levers`, leaves and all; `verify` judges each
  `levers[].sig` and lists the waivers with their trust; `audit` totals them; `status`
  prints the waivers a check on HEAD would apply; the push hook names the waivers behind a
  passing ref; `replay` recomputes with them. Under an agent session the command runs,
  records the marker in the lever node's new optional `agent` field, warns on stderr with
  the file that withdraws it, and every reader flags it (DESIGN.md 9.2, 11.6, 11.9). The
  adoption commit of this repository, which trips `JT-PROT-001` on `.jtac/`, is the first
  rule waiver's use.
- (0.0.17, M11) Reading evidence. `jtac verify` judges every root that bears on a range:
  shape, tree, signature against `.jtac/signers` at HEAD or `--signers`, each `covers` entry
  on the chain, each parent id against the clone; exit 1 on an invalid root or, with
  `--require-coverage`, an absent commit. `jtac audit` prints the coverage table, tree
  fallback included. `jtac replay` recomputes a root's Merkle root, counts and verdict from
  the local nodes and the stored resolved policy. `jtac show` prints the root refs a push
  would send, byte for byte. A range is a `rev-list` expression. `check` keeps the resolved
  policy in the store by its hash (DESIGN.md 11.9, decision 35). Closes issue 25.
- (0.0.13, M10) The push gate goes live. `jtac_gate::decide` is the one push decision: delete,
  tag, and `refs/jtac/` pass; a non-fast-forward update or a deletion of a protected branch
  is the fence, judged from the tip's policy joined with the branch's old tip's; an old oid
  the clone lacks refuses naming `git fetch`; then the tip needs a root whose signature
  holds, or a root for the same tree. `jtac hook pre-push` gathers git's stdin
  lines into facts, renders the decision on stderr, and pushes the root refs ahead of the
  code with `JTAC_HOOK=1` as the recursion guard; a root push that fails warns. `jtac init`
  finds the remote, resolves or generates the signing key (ed25519 under the user config
  directory, named by the global `jtac.signingkey`), creates the store, writes the detected
  policy, installs the hook (refusing a file it did not write), adds the fetch refspec, and
  records an `init` lever node; `--dry-run`; idempotent. `jtac status` asks the same
  decision, tree fallback included. A root ref is created, never replaced (issue 27).
  DESIGN.md 11.1, 11.3, 11.7, 11.9, 14 edited; decision 34; the shell classifier is M10b.
- (0.0.12, M9) `jtac check` writes: on a clean tree it resolves the signing identity, diffs
  from the nearest covered ancestor, runs the checks through the file cache, and on a pass
  assembles the root (`covers` from the diff base, `parents` by commit), signs it, and
  points its ref at it; a block keeps the nodes and writes no root; a covered HEAD is
  checked again and keeps its root. `state.json` holds the last verdict; `jtac status` says
  whether a push would pass and lists the unpushed commits by their roots; `jtac why`
  renders the last verdict with a note when HEAD moved. `jtac_core::counts`,
  `jtac_checks::run_cached`, `Store::put_state`/`state`. DESIGN.md 9.4, 9.6, 11.1, 11.9
  edited; decision 33.
- (0.0.11, M8) `jtac-sign`: SSHSIG through `ssh-keygen`. `resolve` decides the key, principal
  and binary from git config (pure); `identity` asks `ssh-keygen -l` for the fingerprint;
  `sign` signs bytes in a `Namespace` over stdin and refuses a signature from any other key;
  `verify` answers trusted, unknown, or invalid against `.jtac/signers` judged at the
  document's `ts`, with ssh-keygen run in UTC. Never prompts for a passphrase. DESIGN.md 11.7
  and 11.8 edited; decision 32; open question 2 closed.
- (0.0.10, M7) `jtac-store`: the local store under the git common dir (`objects/`, `roots/`
  with a per-root `nodes.json` manifest, `cache/` as the file-backed `Cache`) and the ref
  layer: `put_root` writes a parentless, git-unsigned, fixed-author commit holding `root.json`
  and `root.sig` at `refs/jtac/roots/<xx>/<commit>`; `root` reads one back from a ref and
  refuses non-canonical bytes, a failed `validate`, or a root for another commit; `roots`
  lists them; `push` sends them unforced and reports each ref. Every local read re-hashes
  the file. `jtac_checks::git` gains stdin and env; `Cache::put` returns a result.
  `jtac-testkit` gains `bare` and `clone_of`. `scripts/ref-scale.sh` measures open question
  1. DESIGN.md 9.6 edited; decision 31.
- (0.0.9, M6) `jtac check --preview`: the first verdict. The binary reads HEAD and its policy,
  diffs from `--base`, the upstream, or the default branch (the last commit when that is HEAD), runs the diff-integrity rules into
  an in-memory `integrity` check node, runs every policy check, calls `gate`, and prints
  `CLEARED HOT` or `ABORT` with each reason in three parts; `--json` prints the structure the
  text is rendered from. Exit 0, 1, or 2; nothing is written; a shallow boundary refuses. `--help` and `--version` through
  clap. `jtac-testkit` holds the throwaway-repository helper for every crate's integration
  tests. `Rule::from_id`, `diff::git::git`, and `Display` for `Outcome` and `Severity` support
  the rendering. DESIGN.md 10.3 and 11.1 edited; decision 30.
- (0.0.8, M5) `jtac-checks::diff`: the eleven diff-integrity rules of DESIGN.md section 10.3 as
  pure functions over a change set (`report(integrity, matchers, changes)`), with per-language
  pattern tables for Rust, Python, Go, and JavaScript/TypeScript, and `diff::git` to gather the
  changes from `git diff --raw -M`. `jtac-policy` reserves the check id `integrity`, which
  `facts` lists first and required, and protects every CI definition path by default (the
  design example's hash moves). DESIGN.md 9.2, 10.2, 10.3, 11.2 edited; open question 4 closed;
  decision 29.
- (0.0.7, M4) `jtac-checks` runner: `run(spec, ctx)` spawns a policy check from its argv in its
  own process group (job object on Windows), with stdin closed, an allowlisted environment, a
  1 MiB head capture per stream marked when cut, and a group kill at the deadline, whether the
  check still runs or left a descendant holding its output. Exit 0 is
  `pass`, a positive code `fail`, a timeout, spawn failure, or negative code `error`.
  `tool_version` comes from a per-tool probe table (Node-hosted tools probe Node); `cache::store` keeps only a fresh pass with a
  known version and `cache::lookup` re-issues it with `cached_from`. `specs` maps a policy to
  runner inputs. DESIGN.md 10.2 and 11.1 edited; decision 28.
- (0.0.6, M3) `jtac-core` verdict function: `gate(facts, nodes, now)` decides pass, fail, or
  `declared_off` from struct-literal facts and returns ordered reasons plus the waivers it
  applied. Waiver scope (expiry, 30-day cap, written-after-`now`, commit match, path prefix by
  component) lives once in `waiver.rs` and `Root::validate` shares it. `jtac_policy::facts`
  builds the inputs from a resolved policy. DESIGN.md 11.2 rewritten; decision 27.

### Fixed

- (0.0.5, issue #6) `jtac-policy` globs: a branch pattern git could never name (`HEAD`,
  `main.lock`, `.hidden`, `v1..2`, a space, `~`, `^`, `:`) is refused like an unmatchable path;
  `?` is documented as one byte; the refusal table covers control characters, DEL, and lone
  brackets.

### Added

- (0.0.4, M2) `jtac-policy`: parse `.jtac/policy.toml`, refuse unknown keys, floats, and
  integers past 2^53, validate check ids, argv, timeouts, and globs (literals, `?`, `*`, `**` only; a pattern that
  can match no repository path is refused), hash the resolved policy
  (defaults filled in, `.jtac/**` always protected), and build per-language default policies
  from detected facts. `design_example_parses_validates_and_hashes` pins DESIGN.md 10.2 to the
  code. DESIGN.md decisions 25 and 26: the hash covers the resolved policy; `when` is gone and
  a missing policy file refuses; `integrity.block_at` is the blocking severity 11.2 names.
- (0.0.3) Size and comment discipline, decision 24. `scripts/loc-cap.sh` enforces the
  `AGENTS.md` file cap and comment ratio in CI; `scripts/loc-cap-test.sh` is its bypass
  table. `DESIGN.md` section 10.3 adds `JT-SIZE`, `JT-CMT`, and `JT-ADD` for any project
  the gate guards; a marker added in the same diff counts under `JT-SUP-001`.
- (0.0.2, M1) `jtac-core`: `sha256:` digests, git object ids, and whole-second `ts` values
  with one spelling each; RFC 8785 canonical bytes that refuse floats and integers past 2^53;
  a reader that accepts only canonical bytes and refuses unknown fields and versions; evidence
  node and root types; the RFC 6962 Merkle tree with inclusion proofs. Known answers from
  DESIGN.md sections 9.2, 9.3, and 9.4 are asserted by `node_id_known_answer`,
  `merkle_known_answers`, and `root_id_known_answer`.
- (0.0.2, M1) DESIGN.md decisions 14 to 23: root `ts` and a `now` argument to the verdict,
  applied waivers embedded in the root with their signatures, signer principal plus
  fingerprint, integers only, pinned Merkle leaves, replay recomputes counts, preview before
  store and sign, canonical-only readers, one `ts` spelling, error contract at the user
  surface. `AGENTS.md` is the public working agreement.
- (0.0.1, M0) Cargo workspace with the seven crates from DESIGN.md section 8.1 as documented
  stubs. The `jtac` binary prints its version and refuses every other argument with a non-zero
  exit, so an unbuilt gate can never read as a pass.
- (0.0.1, M0) Toolchain pinned to Rust 1.98.0. Workspace lints forbid unsafe code and warn on
  missing docs. Clippy function-length threshold 150.
- (0.0.1, M0) CI: fmt, clippy, test on Linux and Windows, docs, line coverage at or above 80%,
  cargo-deny. Release workflow on `v*` tags checks the tag is on main and matches the manifest,
  builds static Linux, macOS, and Windows binaries, and publishes a GitHub release from the
  changelog section.
