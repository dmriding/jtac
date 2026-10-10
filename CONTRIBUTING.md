# Contributing to jtac

Before anything is merged:

1. **Accept the Contributor Assignment Agreement.** Read `CLA.md`. Contributions
   are assigned to the Owner. You keep a license to your own work and you are
   credited as a contributor in the history, contributor list, and release
   notes. Post the acceptance comment from section 10 on your first pull
   request and add a `Signed-off-by` trailer with your real name and email to
   every commit.
2. **Read `DESIGN.md`.** It is the source of truth. If the code and the design
   disagree, say so in the pull request instead of picking one.
3. **Pass the quality bars.** Every commit runs clean through:

   ```
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace
   cargo llvm-cov --workspace --summary-only     # line coverage at or above 80%
   ```

4. **Keep the tree clean.** No real transcripts, logs, paths, hostnames,
   credentials, or organisation names in code, tests, fixtures, or commit
   messages. Fixtures use generic names: `example-app`, `example.com`,
   `/workspace/...`.
5. **One milestone or fix per branch**, off fresh `main`. Bump the workspace
   version and add a `CHANGELOG.md` entry in the same pull request.

Pull requests say what changed, why, the risky parts, and what to review
hardest. The maintainer reviews and merges.
