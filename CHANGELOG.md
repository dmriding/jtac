# Changelog

Keep a Changelog style, semver pre-1.0 as described in `docs/RELEASING.md`. Every pull request
bumps the version and adds its entries under Unreleased, tagged with that version. A release pull
request moves them under a dated heading and bumps nothing.

## [Unreleased]

### Added

- (0.0.1, M0) Cargo workspace with the seven crates from DESIGN.md section 8.1 as documented
  stubs. The `jtac` binary prints its version and refuses every other argument with a non-zero
  exit, so an unbuilt gate can never read as a pass.
- (0.0.1, M0) Toolchain pinned to Rust 1.98.0. Workspace lints forbid unsafe code and warn on
  missing docs. Clippy function-length threshold 150.
- (0.0.1, M0) CI: fmt, clippy, test on Linux and Windows, docs, line coverage at or above 80%,
  cargo-deny. Release workflow on `v*` tags checks the tag is on main and matches the manifest,
  builds static Linux, macOS, and Windows binaries, and publishes a GitHub release from the
  changelog section.
