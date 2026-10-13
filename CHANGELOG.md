# Changelog

Keep a Changelog style, semver pre-1.0 as described in `docs/RELEASING.md`. Every pull request
bumps the version and adds its entries under Unreleased, tagged with that version. A release pull
request moves them under a dated heading and bumps nothing.

## [Unreleased]

### Added

- (0.0.4, M2) `jtac-policy`: parse `.jtac/policy.toml`, refuse unknown keys, floats, and
  integers past 2^53, validate check ids, argv, timeouts, and globs, hash the resolved policy
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
