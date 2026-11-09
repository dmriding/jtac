# Releasing

`main` is the development line. Every merge is signed and green on the pull request gates,
the Windows tests among them. `main` is never promised to anyone. A release is a signed tag
`vX.Y.Z` on a commit of `main`.
Nothing is "released" because it merged.

## Rules enforced by the repository

- Only the admin role can create a `v*` tag. The tag ruleset also requires a
  signature and blocks moving or deleting a tag, but the admin role bypasses
  all of it and immutable releases are off: tags are signed and kept by
  practice, not by the repository (#122).
- `main` takes changes only through a pull request, with a code-owner review,
  signed commits, and green checks. The admin role can merge a pull request
  around the review, the checks and the signature rule, and with one code
  owner every merge so far has gone around the review; `docs/CI.md` says how
  a merge around the checks is recorded.

## Cutting a release

1. On `main`, the version in the workspace manifest is already the one to
   release, bumped by the pull requests since the last tag.
2. In `CHANGELOG.md`, move the `[Unreleased]` entries under a new
   `## [X.Y.Z] - YYYY-MM-DD` heading, on a branch named `rel/vX.Y.Z`, in a
   pull request titled `release: vX.Y.Z`. Not `release/vX.Y.Z`: the default
   policy fences `release/*` as a long-lived release line, which this
   repository does not keep, and the pre-push hook refuses the force-push
   that a rebase onto `main` after review needs. That pull request is the
   one exception to the bump rule: it changes no version, because the tag
   must equal the manifest. Merge it.
3. Tag the merge commit, never the pull request's own commit: the release gate holds the
   first-parent line to CI's roots, and a pull request commit carries the developer's alone.
   A tag is never moved, deleted or reused, by practice rather than by the repository (#122),
   so the check comes before the tag,
   on the commit, and only its last line earns the next two commands:

   ```
   scripts/release-check.sh vX.Y.Z
   git tag -s vX.Y.Z -m "jtac vX.Y.Z"
   git push origin vX.Y.Z
   ```

4. The release workflow runs the full quality bars on the tagged commit,
   builds the binaries, attests the range with `jtac release` under the CI
   key (every commit since the previous attestation covered or declared off,
   the first-parent line under CI; `docs/CI.md`), and publishes a GitHub release
   with the changelog section as its notes and the attestation beside the
   archives. A failed workflow means no release; fix on `main` and tag the
   next patch. Tags are never reused. When the spent tag's version is still the manifest's,
   the next release pull request bumps it: the one case where one does. A commit the gate names as absent takes
   `jtac waive <commit> --ci --reason` on `main` before the next tag.

## Version policy

Semver, pre-1.0: a breaking change to the CLI, a policy key, a hook, or an
evidence format is a minor bump; anything else is a patch. Every pull request
bumps the version, in `[workspace.package]` and in every `[workspace.dependencies]`
path entry (the published `jtac` crate needs a version on each, and `cargo deny`
refuses a bare path), and adds its `[Unreleased]` entry. Shipped evidence and
policy schemas are never edited: a change is a new schema version with a
reader for the old one, whatever the crate version does.
