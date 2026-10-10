# 0002. Contributions are assigned to the owner, with credit guaranteed

Date: 2026-10-10
Status: accepted

## Context

A source-available project with a commercial path must be able to relicense,
sell commercial licenses, and enforce against infringers without asking every
past contributor for permission. A license-grant agreement in the Apache
style leaves contributors as co-owners of record, which is the ambiguity this
decision exists to remove. Contributors, for their part, want their work
credited.

## Decision

`CLA.md` is a copyright assignment. The contributor assigns the contribution
to the owner and receives a perpetual non-exclusive license back to their own
contribution. Where a jurisdiction will not permit assignment, it converts to
an exclusive irrevocable license, with moral rights waived as far as the law
allows. In return the owner guarantees credit: commit authorship is kept, and
contributors are named in the contributor list and release notes unless they
ask not to be. Acceptance is a fixed comment on the first pull request and a
`Signed-off-by` trailer on every commit, enforced by the `cla` workflow and
the main ruleset.

## Alternatives

- Apache-style grant CLA: sublicensable, so most relicensing works, but
  contributors remain co-owners and enforcement standing is split.
- DCO only: records provenance, assigns nothing.
- Joint assignment (Oracle style): both parties own it, which reintroduces
  the co-owner question.

## Consequences

Fewer contributions from people who refuse assignment agreements on principle;
those largely overlap with people who refuse source-available projects.
Contributors can reuse their own work anywhere. The owner carries an
obligation to credit, in writing, that a contributor can hold them to.
