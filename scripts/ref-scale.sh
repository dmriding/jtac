#!/bin/sh
# BLUF: builds N root refs under refs/jtac/roots/ in a temp repository and times the git
# operations that walk every ref (DESIGN.md open question 1). Linux only (date +%N).
# Usage: sh scripts/ref-scale.sh 10000
set -eu
n=${1:?usage: ref-scale.sh <count>}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=jtac GIT_AUTHOR_EMAIL=dev@example.com
export GIT_COMMITTER_NAME=jtac GIT_COMMITTER_EMAIL=dev@example.com

t() {
    start=$(date +%s%N)
    "$@" >/dev/null 2>&1
    end=$(date +%s%N)
    printf '%-44s %8.3fs\n' "$label" "$(echo "($end - $start) / 1000000000" | bc -l)"
}

git init -q -b main "$work/repo"
cd "$work/repo"
echo example > README.md
git add README.md
git commit -q --no-gpg-sign -m first

# One parentless commit per ref, each with its own root.json, through one fast-import.
awk -v n="$n" 'BEGIN {
    for (i = 0; i < n; i++) {
        hex = sprintf("%040x", i)
        body = "{\"commit\":\"sha1:" hex "\"}"
        printf "commit refs/jtac/roots/%s/%s\n", substr(hex, 1, 2), hex
        printf "committer jtac <dev@example.com> 1791657000 +0000\n"
        printf "data 7\nsha256:\n"
        printf "M 100644 inline root.json\ndata %d\n%s\n", length(body), body
        printf "M 100644 inline root.sig\ndata 3\nsig\n\n"
    }
}' | git fast-import --quiet
echo "refs: $(git for-each-ref refs/jtac/roots/ | wc -l)"

label="for-each-ref (packed, after fast-import)"; t git for-each-ref refs/jtac/roots/
label="status"; t git status --porcelain
label="log -1 --decorate"; t git log -1 --decorate=full
label="update-ref one more (loose)"; t git update-ref refs/jtac/roots/ff/ffffffffffffffffffffffffffffffffffffffff HEAD
label="for-each-ref (one loose ref)"; t git for-each-ref refs/jtac/roots/
label="rev-parse one ref"; t git rev-parse refs/jtac/roots/00/0000000000000000000000000000000000000001
git init -q --bare -b main "$work/bare"
git remote add bare "$work/bare"
label="push main"; t git push -q bare main
label="push all root refs (first time)"; t git push -q bare 'refs/jtac/*:refs/jtac/*'
label="push one new root ref"; t sh -c 'git update-ref refs/jtac/roots/fe/feffffffffffffffffffffffffffffffffffffff HEAD && git push -q bare refs/jtac/roots/fe/feffffffffffffffffffffffffffffffffffffff:refs/jtac/roots/fe/feffffffffffffffffffffffffffffffffffffff'
label="push with nothing new"; t git push -q bare 'refs/jtac/*:refs/jtac/*'
git clone -q "$work/bare" "$work/clone"
cd "$work/clone"
label="clone: fetch all root refs (first time)"; t git fetch -q origin '+refs/jtac/*:refs/jtac/*'
label="clone: fetch with nothing new"; t git fetch -q origin '+refs/jtac/*:refs/jtac/*'
label="clone: fetch main only"; t git fetch -q origin main
label="clone: status"; t git status --porcelain
