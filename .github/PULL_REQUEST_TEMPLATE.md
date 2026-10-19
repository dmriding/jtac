<!-- Ten-second read. Bullets, no narrative unless the change needs one. -->

- What:
- Why:
- Risky parts, review hardest:
- Verified: <!-- the exact commands you ran and their result -->
- Left out:

Checklist:

- [ ] `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace` pass locally
- [ ] Line coverage stays at or above 80% (`cargo llvm-cov --workspace --summary-only`)
- [ ] Workspace version bumped and `CHANGELOG.md` entry added
- [ ] No real transcripts, logs, paths, hostnames, credentials, or organisation names anywhere in the diff
- [ ] Every commit is signed and carries `Signed-off-by` (a merge GitHub wrote itself is exempt)
- [ ] If it changes a shipped format, the verdict function, policy, signing, storage, or a hook: `DESIGN.md` updated in this PR and the change called out above
- [ ] I am the author of this change and can answer for every line, whatever tools helped write it (see `CONTRIBUTING.md`)
