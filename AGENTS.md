# Working agreement for the jtac repository

jtac is provenance and verification for AI-written code: a local gate, signed evidence that travels with commits, and later a CI gate and review packet. It is written in Rust (2024 edition) and licensed FSL-1.1-ALv2: source-available, free for any use except a competing product or service, converting to Apache-2.0 two years after each release.

**`DESIGN.md` is the source of truth.** Read it before any work. If the code and the design disagree, stop and say so. Do not silently pick one. A design change is an edit to `DESIGN.md` in the same branch as the code that needs it, called out in the PR.

## How work gets done

- **One milestone per branch.** The build plan is `DESIGN.md` section 14. Branch off fresh `main`, never off another feature branch.
- **Plan first.** Settle scope and open design questions before writing code. Ask at decision points, not mid-implementation.
- **Stop at the PR.** Open it with a full title and body: what changed, why, the risky parts, what to review hardest. The maintainer reviews and merges. Never merge, and never present a merge as done.
- **Explicit `git add` paths only.** Never `-A` or `.`.
- **No AI attribution** in commits, PR bodies, docs, or comments: no co-author trailers, no session links, no "generated with" footers. Saying what a tool found and why it mattered is fine. A byline is not.
- **Bump the workspace version in every PR** and add a CHANGELOG entry. Every PR is a patch until `v0.1.0` is cut at the last v0.1 milestone; after that, `docs/RELEASING.md` decides.
- Sections 21 and 22 of `DESIGN.md` list decisions awaiting confirmation and open questions. Do not build past one that your milestone depends on without asking.

## Quality bars, every commit

Run them. Do not assume them.

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo llvm-cov --workspace --summary-only     # line coverage at or above 80%
```

- Warnings are errors. No `#[allow(...)]` without a comment saying why the lint is wrong in that place.
- 80% coverage is a floor. A change that drops below it is incomplete.
- Adversarial self-review before pushing anything that touches the classifier, the verdict function, signing, storage, or hooks: attack it, then verify or refute each finding against the code.

## Architecture rules

- **Deciding code is pure.** The verdict function, the policy evaluator, the command classifier, and the decision function take their facts as arguments. They read no file, run no process, and consult no clock.
- **Impure code decides nothing.** It gathers facts and hands them over.
- **One implementation of every rule,** whichever command or hook reaches it.
- **Fail closed** wherever a check could silently not happen. A check that did not run must never look like one that passed. `skipped` and `error` are not `pass`.
- **Hooks own stdout.** In a hook, stdout is a protocol wire. Diagnostics go to stderr, always.
- **Subprocesses take argv arrays.** No shell, no string interpolation into a command.
- **Shipped formats are never edited.** Evidence and policy schemas are versioned. A change is a new version with a reader for the old one.
- **No model call anywhere in v0.1.**
- **New code is born in the right module.** One concern per file, split at birth, never in a later refactor. A file holds at most 800 production lines (everything outside its `#[cfg(test)] mod` tail) and a function at most 150; `scripts/loc-cap.sh` and clippy enforce both in CI, and `scripts/loc-cap-test.sh` is the script's bypass table. The only way past the file cap is `// jtac-loc-exempt: <reason>` on line 1 and a defense in the PR body. Grep for an existing helper before writing one.
- **Shortest readable form first.** Write the version a person reads top to bottom, then stop. Lines added is a cost the PR body accounts for, not a sign of work.
- **Comments sit at a block's head or tail,** sized for grep: enough to find the thing and the WHY. Nothing inside a block, nothing that narrates the next line. Explanation belongs in the PR body. Fewer plain `//` lines than code lines in production code; doc comments the `missing_docs` lint demands are not counted. CI counts.

## Tests

- Pure code is tested against struct literals. No test spawns a server.
- A screen, tokenizer, or classifier is tested by its table: every spelling ever found, bypasses first. A fix adds the rule's own edge case to the table.
- Integration tests build throwaway git repositories in a temp directory. No test touches the network, the real home directory, or real git config.
- Tests are named for the claim they assert.
- Grep-first: verify a function or type exists by reading its definition before calling it.

## Writing for the reader, who is often a model

Every error and refusal that reaches a person or a model has three parts, in order: what happened, why the rule exists, and the exact command to run next with values filled in. Where the wrong instinct is predictable, name it and say not to. A refusal says whether it is a fence or a negotiation. One test per user-facing surface iterates every error variant and asserts all three parts. Library errors carry the fields that surface needs and nothing more. See `DESIGN.md` section 12.

## Hard rules

- **This repository has its own history.** Never import history from any other repository.
- **Nothing proprietary, patented, or contract-protected enters this tree,** as code or as a dependency. If you are unsure whether something qualifies, leave it out and ask.
- **No field data.** Fixtures, samples, and docs are written new, with generic names: `example-app`, `example.com`, `/workspace/...`. Never paste real transcripts, logs, audit trails, or telemetry.
- **No private identifiers** in code, tests, comments, commit messages, or PR text: no employer or client names, no real filesystem paths, no machine names.
- **`docs/ignored/` and files matching `*.private.md` are local working notes.** Both are gitignored. Never commit them, and never quote or reference their contents in anything that is committed.
- **Never rewrite published history,** and never weaken `.gitignore`.
- **Ask before system-level actions:** `sudo`, installs, process kills, anything touching PATH or shell rc files. Routine cargo, git, and file operations need no ask.
- **Never touch the user's PATH from a script.** Invoke binaries by absolute path.
- **Do not launch interactive terminal programs from a tool call.**

## Platform

Linux is primary. Assume macOS and Windows will follow: no hard-coded path separators, and never blame one platform without evidence from more than one.
