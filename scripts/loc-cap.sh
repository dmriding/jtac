#!/bin/sh
# AGENTS.md size rule. Usage: loc-cap.sh [root]. LOC_CAP overrides 800 for loc-cap-test.sh.
# Only a crate's own tests/ dir and a line-1 #![cfg(test)] mark a test file; names prove nothing.
set -euf
root=${1:-.}; cap=${LOC_CAP:-800}; status=0; n=0
IFS='
'
for f in $(find "$root" -path "$root/target" -prune -o -name '*.rs' -print | sort); do
  d=$(dirname "$f")
  [ "$(basename "$d")" = tests ] && [ -f "$d/../Cargo.toml" ] && continue
  n=$((n + 1))
  awk -v cap="$cap" -v f="$f" '
    NR == 1 && /^#!\[cfg\(test\)\]/ { exit }
    NR == 1 && /^\/\/ jtac-loc-exempt: [^[:space:]]/ { exempt = 1 }
    pending { pending = 0; if ($0 ~ /^(pub(\([a-z]+\))? )?mod [a-z0-9_]+ \{$/) { skipping = 1; next } prod++ }
    /^#\[cfg\(test\)\]$/ { pending = 1; next }
    skipping && /^[^[:space:]}]/ { skipping = 0 }
    skipping { next }
    { prod++ }
    /^[[:space:]]*\/\/(\/\/|[^\/!]|$)/ { comment++; next }
    /[^[:space:]]/ { code++ }
    END {
      if (prod > cap && !exempt) {
        print f ": " prod " production lines, cap " cap
        print "  one concern per file; a file this size holds several (AGENTS.md)"
        print "  split it by concern, or put `// jtac-loc-exempt: <reason>` on line 1 and defend it in the PR"
        exit 1
      }
      if (comment > 0 && comment >= code) {
        print f ": " comment " comment lines, " code " code lines"
        print "  a comment is a grep pointer at a block head or tail; explanation goes in the PR body (AGENTS.md)"
        print "  cut // lines in " f " below " code "; do not turn them into /// doc comments"
        exit 1
      }
    }' "$f" || status=1
done
if [ "$n" = 0 ]; then
  echo "loc-cap: no .rs files under $root"
  echo "  a check that finds nothing must not read as a pass (AGENTS.md: fail closed)"
  echo "  run it from the repository root: sh scripts/loc-cap.sh"
  exit 1
fi
exit $status
