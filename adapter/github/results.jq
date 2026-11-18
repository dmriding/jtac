# The results file, version 1 (DESIGN.md section 15.2), from GitHub's listing of a commit's
# runs ($runs) and of each run's jobs at its newest attempt ($jobs). A word the forge has
# not given is left out, never null, and a run it lists with no ref has the empty one,
# which no branch matches. A run or job a second page repeats is written once.
def word(k): if .[k] == null then {} else {(k): .[k]} end;
{
  v: 1,
  forge: "github",
  runs: [
    $runs | unique_by(.id)[] | . as $run | {
      workflow: .path,
      commit: ("sha1:" + .head_sha),
      event,
      ref: (.head_branch // ""),
      id,
      attempt: (.run_attempt // 1),
      status: (.status // "")
    } + word("conclusion") + {
      jobs: [$jobs | unique_by(.id)[] | select(.run_id == $run.id) | {name} + word("conclusion")]
    }
  ]
}
