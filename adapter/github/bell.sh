#!/usr/bin/env bash
# The doorbell (DESIGN.md section 15.2, decision 67): a listed workflow completed. It rings
# for a push run of the branch whose commit the fetched branch holds on its first-parent
# line, and for nothing else, so no step that holds the CI key starts for a pull request's
# run, a fork's, a tag's or another branch's. This step is never given the key.
set -euo pipefail

say() { printf '%s\n' "$@" | tee -a "$GITHUB_STEP_SUMMARY" >&2; }
quiet() {
  say "$1" \
      "The merge job holds the CI key only for a commit on $BRANCH's first-parent line, which it reads from the branch it fetched and never from the event." \
      "Nothing to do."
  echo "ring=false" >> "$GITHUB_OUTPUT"
  exit 0
}

if [ "$RUN_EVENT" != push ] || [ "$RUN_BRANCH" != "$BRANCH" ]; then
  quiet "jtac: nothing judged: the completed run was not for a push to $BRANCH."
fi
# One name of one commit: grep reads each line of a pattern as a pattern of its own.
case "$RUN_SHA" in
  "" | *[!0-9a-f]*) quiet "jtac: nothing judged: the completed run names no commit." ;;
esac
git fetch -q "$REMOTE" "+refs/heads/$BRANCH:refs/remotes/$REMOTE/$BRANCH"
line="$RUNNER_TEMP/jtac-line"
git rev-list --first-parent "refs/remotes/$REMOTE/$BRANCH" > "$line"
if ! grep -qxF -- "$RUN_SHA" "$line"; then
  quiet "jtac: nothing judged: the completed run's commit is not on $BRANCH's first-parent line."
fi
echo "ring=true" >> "$GITHUB_OUTPUT"
