#!/bin/sh
# Replays release.yml's version job before `git tag` (docs/RELEASING.md step 3): a v* tag that
# fails it cannot be deleted or moved, so the check runs on the commit before the push, never after.
# Usage: release-check.sh vX.Y.Z   with the commit to tag checked out and a clean tree; main is origin/main.
set -eu
cd "$(git rev-parse --show-toplevel)"
tag=${1:?usage: release-check.sh vX.Y.Z}
version=${tag#v}
case "$tag" in v[0-9]*) ;; *)
  echo "'$tag' is not a release tag"
  echo "Fix: name it vX.Y.Z, the workspace version with a v in front."
  exit 1
esac
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "the working tree is dirty; the tag covers HEAD, not the tree"
  echo "Fix: commit or stash, then run scripts/release-check.sh $tag again."
  exit 1
fi
head=$(git rev-parse HEAD)
# First-parent, not ancestor: a merged pull request's own commit is an ancestor of main too, and the
# release gate (`jtac release --require-ci`) refuses a tag there after the whole build has run.
main=refs/remotes/origin/main
if ! git rev-parse -q --verify "$main^{commit}" >/dev/null; then
  echo "$main is missing, so main's line cannot be checked"
  echo "Fix: git fetch origin, then run scripts/release-check.sh $tag again."
  exit 1
fi
if ! git rev-list --first-parent "$main" | grep -qx "$head"; then
  echo "HEAD $head is not on main's first-parent line"
  echo "Fix: git fetch origin, check out the release pull request's merge commit on main, then run scripts/release-check.sh $tag."
  exit 1
fi
if target=$(git rev-parse -q --verify "refs/tags/$tag^{commit}") && [ "$target" != "$head" ]; then
  echo "tag $tag already exists at $target"
  echo "Fix: tags are never reused; bump to the next patch in a pull request and tag that merge commit."
  exit 1
fi
manifest=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
if [ "$version" != "$manifest" ]; then
  echo "tag $tag does not match workspace version $manifest"
  echo "Fix: bump Cargo.toml to $version in a pull request, then tag the merge commit."
  exit 1
fi
if ! grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md has no '## [$version]' section"
  echo "Fix: merge the rel/$tag pull request first (docs/RELEASING.md step 2), then tag its merge commit."
  exit 1
fi
sh scripts/action-version.sh
echo "release-check: $tag may be tagged at $head"
