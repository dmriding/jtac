#!/usr/bin/env bash
# The default-branch job (DESIGN.md sections 15 and 15.2, decisions 43, 66 and 67). A push
# event judges its own tip. A listed workflow's completion drains the branch: every pushed
# tip the CI gate reads no document for, oldest first, each with a results file, stopping
# at the first whose results the forge still runs. The commits come from the fetched branch
# and the forge's push list; the event's commit is only added to them, and only when the
# branch holds it. Run from a copy outside the work tree: the drain checks commits out.
set -euo pipefail

GH=${GH:-gh}
WINDOW=30
POLICY=.jtac/policy.toml
tmp="$RUNNER_TEMP/jtac-root"
mkdir -p "$tmp"

say() { printf '%s\n' "$@" | tee -a "$GITHUB_STEP_SUMMARY" >&2; }
gate() {
  code=0
  printf '```\n' >> "$GITHUB_STEP_SUMMARY"
  "$JTAC_BIN" "$@" | tee -a "$GITHUB_STEP_SUMMARY" || code=$?
  printf '```\n' >> "$GITHUB_STEP_SUMMARY"
  return "$code"
}

# One pushed tip, checked out: `check --from <its head>` and a root when the forge names it
# a merged pull request's, `jtac ci` from the old tip and no root for a direct push, a
# fast-forward or a tip with the pull request's commits on its first-parent line (decision
# 66). A force push or an old tip the clone cannot fetch is refused: a gate that cannot
# anchor judges nothing. Exits; the drain runs it in a subshell.
judge() {
  local commit=$1 before=$2 forced=$3 results=$4
  if expr "$before" : '0*$' >/dev/null; then
    say "jtac: nothing judged: this push created the branch, so there is no old tip to diff from." \
        "The gate judges the diff from the branch's old tip and the merge root records it; the next push has one." \
        "Nothing to do."
    exit 0
  fi
  if [ "$forced" = true ]; then
    say "jtac: refused: this push rewrote the branch, so the old tip $before is not an ancestor of the new one." \
        "The gate anchors every verdict at the old tip; a force push moves the anchor, and a check that cannot anchor must not pass." \
        "Restore the old tip (git push --force-with-lease $REMOTE $before:$BRANCH) and push the change as a pull request; deny force pushes in the ruleset."
    exit 2
  fi
  git rev-parse -q --verify "$before^{commit}" >/dev/null || git fetch -q "$REMOTE" "$before" || true
  if ! git rev-parse -q --verify "$before^{commit}" >/dev/null; then
    say "jtac: refused: the old tip $before is not in this clone and $REMOTE does not serve it." \
        "The gate anchors every verdict at the old tip; without it nothing can be judged, and a check that did not run must not pass." \
        "Re-run the job once the remote serves the commit, or push the change again as a pull request."
    exit 2
  fi
  pr=$("$GH" api "repos/$GITHUB_REPOSITORY/commits/$commit/pulls" \
    --jq "[.[] | select(.merged_at != null and .merge_commit_sha == \"$commit\")][0] | select(. != null) | \"\(.number) \(.head.sha) \(.head.repo.clone_url // \"\")\"")
  number=""; head=""; head_repo=""
  if [ -n "$pr" ]; then read -r number head head_repo <<< "$pr"; fi
  if [ -z "$pr" ]; then
    say "jtac: no CI root: the forge names no merged pull request for $(git rev-parse --short "$commit"), so this is a direct push; it is judged below and signed over by nothing." \
        "A CI root stands for the pull request head it compared; a direct push has none. Require pull requests in the ruleset." \
        "jtac waive <commit> --ci --reason records a merge made around CI."
    gate ci --base "$before"; exit $?
  fi
  if [ "$head" = "$commit" ]; then
    say "jtac: no CI root: pull request #$number was merged by fast-forward, so the tip is its head and the evidence is the developer's, where it was pushed; it is judged below." \
        "A merge root stands for a commit the forge made; a fast-forward makes none. Require merge commits in the ruleset for CI-signed evidence." \
        "Nothing else to do."
    gate ci --base "$before"; exit $?
  fi
  # The pull request's commits on the tip's first-parent line are a prefix of it from the
  # old tip, so the lowest commit there is the one test (decision 66).
  git fetch -q "$REMOTE" "+refs/pull/$number/head:refs/remotes/pull/$number/head"
  first=$(git rev-list --first-parent "$before..$commit" | tail -n 1)
  if [ -n "$first" ] && git merge-base --is-ancestor "$first" "$head"; then
    say "jtac: no CI root: $(git rev-parse --short "$first"), the first commit this push put on the branch's first-parent line, is pull request #$number's own and not the forge's; the push is judged below." \
        "A merge root speaks for the tip's first-parent line back to the old tip, which every merge the forge makes fills with its own commits; a merge pushed with the pull request's commits as its first parent does not." \
        "Merge pull requests through the forge and deny direct pushes in the ruleset. jtac waive <commit> --ci --reason records a merge made around CI."
    gate ci --base "$before"; exit $?
  fi
  if [ -z "$KEY" ]; then
    say "jtac: the CI key is not set, so the merge root for pull request #$number cannot be signed." \
        "A merge root is signed by the CI key from the environment secret; without it the job proves nothing, and a job that proves nothing must not pass." \
        "Run jtac init --ci github in a clone: it prints the environment and gh secret set commands with the key path filled in; then re-run this job."
    exit 2
  fi
  # The head repository's roots for the pull request's own commits alone, where this clone
  # holds none, and no tags: a fork never supplies the old tip's root, so the merge root's
  # parent stays the branch's own. A root already on the tip is `check --from`'s to judge.
  if [ -n "$head_repo" ]; then
    git fetch -q --no-tags "$head_repo" "+refs/jtac/roots/*:refs/jtac-head/roots/*"
    added=$(git rev-list "$head" --not "$before")
    for c in $added; do
      from="refs/jtac-head/roots/${c:0:2}/$c"
      want="refs/jtac/roots/${c:0:2}/$c"
      if oid=$(git rev-parse -q --verify "$from") && ! git rev-parse -q --verify "$want" >/dev/null; then
        git update-ref "$want" "$oid"
      fi
    done
    git for-each-ref --format='delete %(refname)' 'refs/jtac-head/' | git update-ref --stdin
  fi
  keyfile="$RUNNER_TEMP/jtac-ci-key"
  trap 'rm -f "$keyfile"' EXIT
  umask 077
  printf '%s\n' "$KEY" > "$keyfile"
  git config user.email "$PRINCIPAL"
  git config jtac.signingkey "$keyfile"
  read_ci=()
  if [ -n "$results" ]; then read_ci=(--results "$results" --branch "$BRANCH"); fi
  code=0
  gate check --from "$head" --base "$before" --push "$REMOTE" ${read_ci[@]+"${read_ci[@]}"} || code=$?
  # Exit 3 is the binary's word that a listed result is still running, and no other
  # command's: the drain waits on this file, never on a bare code.
  if [ "$code" -eq 3 ]; then : > "$tmp/wait"; fi
  exit "$code"
}

# The forge's listing of `commit`'s runs and of each run's jobs, as the results file.
results() {
  local commit=$1 out=$2 id
  "$GH" api --paginate "repos/$GITHUB_REPOSITORY/actions/runs?head_sha=$commit&per_page=100" \
    --jq '.workflow_runs[]' > "$tmp/runs.json"
  : > "$tmp/jobs.json"
  for id in $(jq -r '.id' "$tmp/runs.json"); do
    "$GH" api --paginate "repos/$GITHUB_REPOSITORY/actions/runs/$id/jobs?filter=latest&per_page=100" \
      --jq '.jobs[]' >> "$tmp/jobs.json"
  done
  jq -n --slurpfile runs "$tmp/runs.json" --slurpfile jobs "$tmp/jobs.json" \
    -f "$JTAC_ADAPTER/results.jq" > "$out"
}

# How the CI gate reads `commit`: covered, declared_off, failed, or none. The binary's own
# reading under the branch's signers, so a root anyone pushed onto a tip settles nothing.
reading() { "$JTAC_BIN" audit "$1^!" --json | jq -r '.rows[0].ci // "none"'; }

drain() {
  local tip before after kind ci i floor=false seen=false worst=0 todo=()
  if [ -n "$(git ls-remote "$REMOTE" "refs/tags/$BRANCH")" ]; then
    say "jtac: refused: $REMOTE holds a tag named $BRANCH, so no results file is written." \
        "The forge lists a run by its ref's short name: a run for a push to that tag reads as the branch's, and a file can conclude otherwise for a tag than for the branch." \
        "Delete the tag (git push $REMOTE :refs/tags/$BRANCH), add the tag rule jtac init --ci github names, then re-run this job."
    exit 2
  fi
  git fetch -q "$REMOTE" "+refs/heads/$BRANCH:refs/remotes/$REMOTE/$BRANCH"
  tip=$(git rev-parse "refs/remotes/$REMOTE/$BRANCH")
  git rev-list --first-parent "$tip" > "$tmp/line"
  "$GH" api "repos/$GITHUB_REPOSITORY/activity?ref=refs/heads/$BRANCH&direction=desc&per_page=$WINDOW" \
    --jq '.[] | "\(.before) \(.after) \(.activity_type)"' > "$tmp/pushes"
  # Newest first. The commit whose workflow completed is judged unless the CI gate reads it
  # covered or declared off: a failed one may be a re-run's result. Above the floor, the
  # newest tip the gate reads or the first with no policy, so is every merge the forge
  # made and the gate does not read yet; a plain push there bears no root and was judged
  # at its own completion, and skipping it is what bounds the walk on a branch of them.
  while read -r before after kind <&3; do
    grep -qxF -- "$after" "$tmp/line" || continue
    if [ "$after" = "$RUN_SHA" ]; then
      seen=true
      ci=$(reading "$after")
      if [ "$ci" = none ] || [ "$ci" = failed ]; then todo+=("$before $after $kind"); fi
      if [ "$ci" != none ]; then floor=true; fi
      continue
    fi
    if [ "$floor" = true ]; then continue; fi
    case "$kind" in pr_merge | merge_queue_merge) ;; *) continue ;; esac
    if ! git cat-file -e "$after:$POLICY" 2>/dev/null; then floor=true; continue; fi
    ci=$(reading "$after")
    if [ "$ci" = none ]; then todo+=("$before $after $kind"); else floor=true; fi
  done 3< "$tmp/pushes"
  if [ "$seen" = false ]; then
    say "jtac: refused: the forge's last $WINDOW pushes to $BRANCH do not hold $RUN_SHA, so its old tip is not known." \
        "The gate anchors every verdict at the old tip the forge reports; a guessed one would judge another diff under another trust." \
        "Re-run this job; for a push older than that, run jtac check --from <head> --base <old tip> --results <file> --branch $BRANCH --push $REMOTE by hand."
    exit 2
  fi
  if [ "${#todo[@]}" -eq 0 ]; then
    say "jtac: nothing judged: the CI gate already reads the completed commit and every merge above the newest it reads." \
        "A merge root is written once; a completion for a tip that has one changes nothing." \
        "Nothing to do."
    exit 0
  fi
  for ((i = ${#todo[@]} - 1; i >= 0; i--)); do
    read -r before after kind <<< "${todo[i]}"
    say "jtac: judging $(git rev-parse --short "$after"), pushed to $BRANCH over $(git rev-parse --short "$before" 2>/dev/null || echo "$before")."
    results "$after" "$tmp/results-$after.json"
    git checkout -q --detach "$after"
    rm -f "$tmp/wait"
    # errexit is off in a `||` list, subshell and all, so the code is taken this way.
    set +e
    (set -e; judge "$after" "$before" "$([ "$kind" = force_push ] && echo true || echo false)" "$tmp/results-$after.json")
    code=$?
    set -e
    if [ -e "$tmp/wait" ]; then
      say "jtac: no root yet for $(git rev-parse --short "$after"): the forge still runs a workflow its policy reads." \
          "Merge roots are written oldest first, so each names its parent; a later push waits behind this one." \
          "Nothing to do: this job runs again when that workflow completes."
      break
    fi
    if [ "$code" -gt "$worst" ]; then worst=$code; fi
  done
  exit "$worst"
}

case "$GITHUB_EVENT_NAME" in
  push) judge "$GITHUB_SHA" "$BEFORE" "$FORCED" "" ;;
  workflow_run)
    command -v jq >/dev/null || {
      say "jtac: refused: jq is not on this runner's PATH." \
          "The results file is written with jq from the forge's listing; without it no result is read, and a check that was not read must not pass." \
          "Install jq on the runner image, then re-run this job."
      exit 2
    }
    drain ;;
esac
