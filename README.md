# jtac

Provenance and verification for AI-written code.

Your agent cannot push slop, and you can prove it. jtac is a local gate that
checks a change before it leaves the developer's machine and records signed
evidence that travels with the commits. A CI gate that verifies that evidence
independently and a review packet for humans follow in later releases.

**Status: pre-implementation.** `DESIGN.md` is the source of truth and the
build plan is its section 14. Nothing here runs yet.

## What it will do

- Run the repository's own format, lint, and test checks on the commit under
  test, plus deterministic diff-integrity rules: removed tests, new
  suppressions, weakened CI, credential-shaped strings.
- Decide with a pure function. The verdict reads no clock, no network, and no
  model, so it replays to the same answer from the recorded evidence.
- Sign a root per push tip with the developer's SSH key and store it as a git
  ref beside the commits. Anyone with the repository can verify it offline.
- Say, per commit, whether the gate was on, deliberately off, or not there.
  Bypass is recorded, not prevented.
- Refuse with three parts, every time: what happened, why the rule exists,
  and the exact command to run next.

## Privacy

A developer's machine reports to nothing. Evidence leaves only with a push,
only roots and signatures are shared, and `jtac show` prints exactly what a
push will carry. See `DESIGN.md` section 19.

## Contributing

Read `CONTRIBUTING.md`, then `DESIGN.md`. Decisions are recorded in
`docs/adr/`, and changes to the design go through `docs/rfc/`.

## License

jtac is source-available under the Functional Source License, version 1.1,
with an Apache 2.0 future grant (`LICENSE.md`, SPDX `FSL-1.1-ALv2`). You may
use, copy, modify, and redistribute it for any purpose except offering it, or
a derivative, as a commercial product or service that competes with it. Each
release becomes Apache 2.0 two years after publication. This is not an
OSI-approved open source license. For a commercial license, contact the
maintainer.
