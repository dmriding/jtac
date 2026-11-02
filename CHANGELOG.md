# Changelog

Keep a Changelog style, semver pre-1.0 as described in `docs/RELEASING.md`. Every pull request
bumps the version and adds its entries under Unreleased, tagged with that version. A release pull
request moves them under a dated heading and bumps nothing.

## [Unreleased]

### Changed

- (0.2.6) CI: the Linux `test` job is gone; `coverage` already runs every test once,
  instrumented, and a second uninstrumented build proved nothing more. Debuginfo is off for
  CI builds (`CARGO_PROFILE_DEV_DEBUG=0` in the workflow, local builds unchanged). The
  ruleset's required checks lose `test`; `coverage` and the rest stand.

### Added

- (0.2.7) The GitHub action (`action.yml`): `jtac ci` on a pull request, a CI-signed merge
  root on a push to the default branch, the evidence store as an artifact. `jtac init --ci
  github` writes the workflow, the first `.jtac/signers` and a CI key, and prints the
  operator's steps (`docs/CI.md`). Locally a fresh repository's adoption commit, the policy and the
  first `.jtac/signers` together, clears with every protected-path finding minor
  (decision 45); a repository gated locally first adds CI under one waiver. This
  repository runs the action on itself; `scripts/action-version.sh` holds the action's
  version to the workspace's. (M17, decision 49)
- (0.2.5) The CI waiver (DESIGN.md section 15, decision 48, M16c): `jtac waive <commit> --ci
  --reason` records that a commit was merged without CI's root, on purpose. It is a commit
  waiver whose scope names the gate, node version 2 with a version 1 reader, carried unapplied
  by the root `jtac check` writes on that commit, and read by `audit` as the CI gate's
  `declared_off` beside the local state. `jtac check --push <remote>` now works without
  `--from` and sends HEAD's root. `--ci` is refused with `--rule`, `--path` or `--expires`,
  and when this clone already holds a root for the commit.

### Changed

- (0.2.4) shuck-parser and shuck-ast 0.2.4 to 0.2.7, which bound their own nesting, chain
  length and stack. The command reader refuses brackets past twelve deep instead of sixteen,
  under the depth a debug build of the parser reads for bare `$(`, so that shape answers alike
  in debug and release; an operator chain past 1000 links or keywords past 100 deep are
  unreadable with the parser's reason. Supersedes dependabot pull request #98.
- (0.2.4) What the parser drops under a clean status is unreadable: shuck's nested reparse of a
  `${…}` operand, a subscript, a pattern or an arithmetic body stops at a cumulative depth of its
  own and hands back a substitution with an empty body, an operand or pattern as plain text
  holding an unescaped backtick, a `$` that opens an expansion, or a `<(` where bash runs one
  (unquoted, or in a `${…}` pattern, replacement, case operand or subscript even inside `"…"`),
  or an arithmetic command or expansion with no expression, so
  `x=${a:-$(git push; <eight nested ifs>)}`, `echo "${PWD/x/<(git push)}"`, `(( $(git push) 1 ))`
  and `$((true) && git push)` read as nothing. A backslash before a dollar or a backslash in a
  backtick body is unreadable like one before a backtick: bash cooks all three before it parses
  the body, and the parser's spans no longer match the source. Found by the review rounds; 0.2.4
  of shuck did the same a little deeper, and never built a process substitution inside an operand.
- (0.2.4) The command reader holds four invariants in place of a rule per finding (`DESIGN.md`
  section 11): line continuations joined once before the parser runs, a part that can run a
  command laid where the source has it, a heredoc read only in its plain shape, and stdin kept
  only as one heredoc or here-string on descriptor 0. Twenty review rounds on this bump each
  found a hole in one of those families; their rows stay as the tests of the invariant that
  closes each. The price, until shuck lays a cooked literal at its source width: a `"…"` with an
  escaped quote, dollar or backslash before a `$(…)` in it (`curl -d "{\"text\":\"$(cat f)\"}"`)
  is unreadable, as is a continuation inside a quoted heredoc body, and a shell whose stdin is a
  dup, a closed descriptor or a file (`bash <&0`, `bash <&-`, `bash < run.sh`) fails closed;
  `${BRANCH//\//-}`, `"${N:-\$HOME}"` and `${x//\\/\\\\}` read, `gi\<newline>t push` reads as
  the push it is, and `bash -c '# c\<newline>git push'` hands the inner shell the text as
  written, where the comment rule refuses it.

### Added

- (0.2.3) `jtac check --from <head> --base <old tip> [--push <remote>]` (`DESIGN.md`
  section 15, M16b): the merge job. HEAD is the commit the forge made; the rules diff from
  the old tip under its trust, the head's chain offers its trusted waivers in the pull
  request's scope, the head's root is compared again with its checks run on its own tree, and
  the root, signed in the CI namespace with `covers` from the old tip and the old tip's root
  as its parent, records in `from` the head, its root and what the comparison made of it.
  `--push` sends the merge root and, when the head agreed, the head's root outside the hook.
  `jtac audit` reads a merge root's head: `verified`, `absent`, `unverified`, `disagreed`,
  `lied` or `invalid`, with the head root's signer when its ref holds the root `from` names
  and `root not held` when not. A root push's "not sent" note prints once per push.

- (0.2.2) The CI namespace `jtac-ci-root-v1` (`DESIGN.md` sections 11.8 and 15, M16a): a
  root with `from` is signed and verified in it, and trusted only under a `.jtac/signers`
  line whose `namespaces` option names it, which jtac reads itself before ssh-keygen sees
  the file, so a developer's line, which names none, trusts no CI root and a CI key signs
  no ordinary root and no waiver. The pre-push hook and `status` accept a root with `from`
  at a tip only under a signer the remote's default branch tip's signers, or the branch's
  old tip's, name for that namespace; a clone holding neither refuses naming `git fetch`
  and `git remote set-head`, or a named remote for a push that named a URL.

- (0.2.1) `jtac ci --base <rev> [--signers <file>] [--require-local-root]` (`DESIGN.md`
  section 15, M15b): one verdict on HEAD from its merge base with the base branch, under the
  base's policy when the diff touches `.jtac/**` or HEAD carries no readable policy, with
  trust from the base rev's tree, the
  chain's waivers offered when base trusts the lever's signer and the root carrying it and,
  for a commit waiver on an earlier commit, every commit after it is covered by a trusted root,
  the withheld ones named with why and what they would have covered, the pull request's
  commits as the commit-waiver scope, and every protected-path finding minor when the base
  holds no signers file (decision 45). HEAD's root is verified as
  a document and compared with this gate's run of its own diff: invalid, lied, or disagreed
  fails with both sides' counts; missing or unverifiable is reported and passes unless
  `--require-local-root`. Signs nothing, writes no root and no `state.json`; evidence nodes go
  to the store.

- (0.2.0) Root version 2 (`DESIGN.md` section 15, M15a): `check` writes `base`, the commit
  the rules diffed from, absent on a parentless tip, and the reader refuses a root whose base
  is not an ancestor of its tip, whose `covers` reaches at or below it, or whose version 1
  form carries a version 2 field. The `from` field and its five `compared` outcomes are read
  and validated; nothing writes one before M16. The verdict takes the commits a commit waiver
  may name as a fact, and `compare` in `jtac-core` is the pure comparison `jtac ci` (M15b)
  will call. A 0.1.x reader sees a version 2 root as unreadable, which is why this is the
  minor bump `docs/RELEASING.md` names for an evidence format change.

### Changed

- (0.2.2) `replay` judges a root's signature in the namespace its shape picks, as `verify`
  and `audit` do (issue 88, item 1); the M15 reader's rule that every root with `from` is an
  unknown signer's is gone with the namespace that replaces it.

- (0.2.1) `DESIGN.md` section 15 and decision 40: `--signers` is an override; trust defaults
  to the base rev's `.jtac/signers`, and no file there is the adoption fact. Section 15 also
  states that a lever is offered only from a root base trusts and that a commit waiver carries
  forward only across commits a trusted root covers (red team, M15b round 1).
- (0.2.0) `DESIGN.md` section 10.3 states that `--base` overrides the nearest covered
  ancestor and that the root records it (#84).
- (0.1.7) `DESIGN.md` section 15 is the v0.2 plan: milestones M15 to M18 with done-whens,
  decisions 40 to 46, open question 7 closed; the README roadmap follows it.
- (0.1.6) `docs/RELEASING.md` names the release branch `rel/vX.Y.Z`, outside the default
  `release/*` fence (#81).
- (0.1.5) The README is written for a reader who has never seen jtac: the install, the quick
  start, a real block with its three parts, and how a clone checks someone else's evidence,
  every command and line of output taken from the v0.1.2 release binary.
- (0.1.4) The Windows test job runs on every pull request and is a required check, so a
  Windows-only break stops at the pull request instead of landing on `main`. It skipped pull
  requests while the repository was private, where Windows minutes bill double.

### Fixed

- (0.2.4) Command-reader holes older than the shuck bump, found by its review rounds and closed
  by the invariants above: a quoted word bash reads again as a subscript
  (`let 'm[${a:-<(git push)}]=1'`, `unset`, `[[ -v`) ran the `<(` in its `${…}` operand unseen;
  a backslash shuck drops from a `${…}` operand or pattern shifted every span inside it, so
  `${PWD/\/$(git push)/y}` read as nothing; a line continuation split a `$`, `{`, keyword, `[[`,
  `((`, `<<-` or delimiter line and the parser read the pieces; an unquoted heredoc handed to a
  shell was passed on uncooked, so `bash <<EOF` with `\$(git push)` in the body ran unseen; a
  heredoc was read other than bash reads it (a trailing blank on the delimiter line, a `)` after
  it inside a substitution, a second heredoc on a line the parser took as quoted, a backslash or
  brace in the delimiter, a digit before `<<` lexed as a word); and stdin was resolved other than
  bash does (a numberless output redirect taken as stdin, `0>&0` clearing it, a substitution in
  a later redirect target running with the heredoc on stdin, `> >(bash)` and `bash < <(…)` read
  as nothing, stdin of a compound command or a function not reaching the commands inside).
- (0.1.3) The `signoff` CI job failed every pull request after GitHub's "Update branch" button:
  the merge commit it writes carries no `Signed-off-by` trailer. A merge whose tree is exactly
  the clean merge of its two parents is now exempt, proven with `git merge-tree` on the runner;
  any other merge can carry content of its own and is checked like any commit. The job walks
  the commits with git instead of the API, which listed at most 250 and passed the rest unseen.

## [0.1.2] - 2026-10-19

### Fixed

- (0.1.2) `jtac hook pre-tool` compared a push's URL with each configured remote's as text,
  and on Windows git opens `C:\srv\app`, `C:/srv/app` and `c:/srv/app` as one directory: a
  push by the forward-slash spelling, which bash passes through where a backslash is an
  escape, or through an `insteadOf` base ending in the other separator, found no old tip and
  reported no fence. The three pre-tool tests that push by URL failed on every Windows run of
  `main`; they also wrote the backslash path straight into the bash script. On Windows a path
  by drive letter or UNC share now compares forward-slashed in lower case; the tests spell the
  path as bash does, and one row keeps the backslashes in quotes.

## [0.1.1] - 2026-10-19

### Fixed

- (0.1.1) `jtac hook pre-tool` gives up at its one-second cap with no note on a machine where
  git starts slowly: one push judgment is thirty-odd git processes, which the Windows runner
  cannot finish in time, so the pre-tool tests failed on every push to `main` since 0.0.21 and
  the v0.1.0 tag built no release (issue 64). `JTAC_PRETOOL_CAP_MS` in the hook's environment
  now sets the cap, under the two seconds the agent's settings allow; the tests run under a
  minute and fail with the reason on a hit instead of reading silence as a verdict. Two tests
  cover the cap: its parse, and the hook giving up.

## [0.1.0] - 2026-10-19

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
- (0.0.5, issue #6) `jtac-policy` globs: a branch pattern git could never name (`HEAD`,
  `main.lock`, `.hidden`, `v1..2`, a space, `~`, `^`, `:`) is refused like an unmatchable path;
  `?` is documented as one byte; the refusal table covers control characters, DEL, and lone
  brackets.

### Changed

- (0.1.0, M14) The commit that adds `.jtac/policy.toml` where no parent's history held one
  had nothing in force under `.jtac/`, so `JT-PROT-001` there is minor and says so; adopting jtac no longer needs a
  waiver (DESIGN.md decision 38). `jtac policy` and `jtac scope` move to v0.2 (7.2). The
  release workflow publishes `SHA256SUMS` over the archives. Open question 5 is closed:
  Windows runs the test suite, the hooks are unverified there, the install script is `sh`.
- (0.0.18) The README says what the gate does today, command by command, how to adopt it,
  and the roadmap by version; the preview-only status is gone.
- (0.0.16) The Windows test job runs on a push to `main`, a release tag and a manual
  dispatch, not on a pull request, so a pull request bills one Ubuntu test leg; the required
  check is `test`.

### Added

- (0.1.0, M14) `jtac bench`: the public proof. In a lab under the temp directory with its
  own home, global git config and generated key, one throwaway repository with a bare origin
  per control, attacked through the binary and `git push` through the installed hook: a
  clean change lands with its root; no root, a replaced `root.sig`, a `root.json` naming
  another tree, a deleted `#[test]`, a new `#[allow]`, a policy that does not parse, a check
  naming a missing tool and a `--force` rewind of a protected branch are refused; every
  bypass spelling in the classifier's table classifies as one, and the advisory names what
  lands where one is due. Each row quotes the
  gate's line; `--json` carries the rows; exit 0, 1 when a row did not hold, 2 when git or
  `ssh-keygen` cannot start (DESIGN.md section 13). `install.sh` fetches the release
  archive named by `SHA256SUMS`, verifies it, installs under `~/.local/bin` or `--to`, and
  never edits PATH (decision 39). The README has the install line, the quick start without a
  waiver, and the numbers: what is measured, and the method for what is not.
- (0.0.21, M13) `jtac hook pre-tool claude-code`: Claude Code's `PreToolUse` hook reads the
  proposed Bash command through the classifier, judges each push where it would run with the
  decision the `pre-push` hook asks, and returns a note as `additionalContext`: no root names
  `jtac check`, a rewind or deletion of a protected branch is named as a fence, a bypass is
  told what lands and what `jtac verify` shows, an unreadable or unplaced push names
  `jtac status`. It never blocks: every failure and a one-second cap print nothing and exit
  0 (DESIGN.md 11.3, decision 37). `jtac init --agent claude-code` registers it in
  `.claude/settings.local.json` and lists that file in `.git/info/exclude`; without the flag
  a clone that shows the agent gets the command in a note (DESIGN.md 11.7 step 5). The
  classifier's push gained `tags` (`--tags`) and a `:dst` deletion keeps its slot among the
  sources.
- (0.0.20, M10b) `jtac-gate` shell command classifier: a script's pushes (remote, refspecs,
  force form, deletions, the directory each runs in), hook bypasses in every spelling of
  `--no-verify` and `core.hooksPath`, directory changes, and what it cannot read, with the
  reason. The grammar comes from `shuck-parser` behind one adapter module; a parse that is
  not clean is unreadable (DESIGN.md 11.4, decision 36). Every spelling is a row of one table
  test. Nothing calls it yet; M13 wires `jtac hook pre-tool`.
- (0.0.19) This repository is gated by jtac: `.jtac/policy.toml` runs the fmt, clippy, test,
  doc and loc-cap bars with CI's flags and protects `scripts/**`, `.cargo/**`, the rustfmt and
  clippy config in every directory and the legacy `rust-toolchain` file beside the defaults;
  `.jtac/signers` trusts the maintainer's key. `.cargo/config.toml` denies rustdoc warnings as
  CI does, since the check runner passes no `RUSTDOCFLAGS`. The adoption commit trips
  `JT-PROT-001` on `.jtac/` and `.cargo/`, so its root carries a one-day rule waiver on each.
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
