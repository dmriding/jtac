#!/bin/sh
# The action installs the release whose version the workspace carries, and signs as the
# principal `jtac init --ci github` writes into .jtac/signers; release.yml refuses a tag where
# either pair differs. Usage: action-version.sh [root]
set -eu
root=${1:-.}
manifest=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
action=$(sed -n 's/^ *VERSION: v\(.*\)$/\1/p' "$root/action.yml" | head -1)
principal_action=$(sed -n 's/^ *default: \(jtac-ci@[^ ]*\)$/\1/p' "$root/action.yml" | head -1)
principal_src=$(sed -n 's/^pub const CI_PRINCIPAL: &str = "\(.*\)";$/\1/p' "$root/crates/jtac/src/ciwf.rs")
if [ "$manifest" != "$action" ]; then
  echo "action.yml installs v$action; Cargo.toml is $manifest"
  echo "Fix: set VERSION in action.yml to v$manifest, in the pull request that bumps the version."
  exit 1
fi
if [ "$principal_action" != "$principal_src" ]; then
  echo "action.yml signs as '$principal_action'; ciwf.rs writes '$principal_src' into .jtac/signers"
  echo "Fix: make the principal input's default in action.yml equal CI_PRINCIPAL in crates/jtac/src/ciwf.rs."
  exit 1
fi
echo "action.yml: v$action, principal $principal_action"
