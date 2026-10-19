# Contributing to jtac

Before anything is merged:

1. **Accept the Contributor Assignment Agreement.** Read `CLA.md`. Contributions
   are assigned to the Owner. You keep a license to your own work and you are
   credited as a contributor in the history, contributor list, and release
   notes. Post the acceptance comment from section 10 on your first pull
   request and add a `Signed-off-by` trailer with your real name and email to
   every commit. Merge commits are exempt.
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

## Contributions written with AI tools

Welcome. jtac exists because AI-written code can be good, and this project
uses these tools itself. The condition is accountability: the person who
submits a pull request is its author, whatever produced the text. That means
you have read every line, you understand why it is there, you have run the
quality bars yourself, and you can answer questions about it in review.

A pull request whose author cannot do those things is closed without review.
Signs that lead there: a description that does not match the diff, tests that
were weakened or removed to get green, changes outside the stated scope,
a file past the cap in `AGENTS.md` with no defended exemption,
commentary that narrates the code instead of explaining it, or answers in
review that are plainly generated rather than considered. None of this is a
judgment on the tool. It is the same standard a human-written change meets,
and the same one jtac enforces on its users.
