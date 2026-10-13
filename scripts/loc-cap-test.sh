#!/bin/sh
# Table for loc-cap.sh, bypasses first. A row is: expected exit, claim, file body on stdin.
set -u
here=$(cd "$(dirname "$0")" && pwd); t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
row() {
  mkdir -p "$t/$2/src"; cat > "$t/$2/src/lib.rs"
  if LOC_CAP=10 sh "$here/loc-cap.sh" "$t/$2" >/dev/null 2>&1; then got=0; else got=1; fi
  [ "$got" = "$1" ] || { echo "FAIL $2: expected exit $1, got $got"; echo "$2" >> "$t/failed"; }
}
fns() { i=0; while [ "$i" -lt "$1" ]; do echo "pub fn f$i() {}"; i=$((i + 1)); done; }
{ echo '#[cfg(test)]'; echo 'use std::fmt;'; fns 9; } | row 1 cfg_test_import_hides_nothing
{ echo '#[cfg(test)]'; echo 'mod tests;'; fns 9; } | row 1 sibling_test_mod_hides_nothing
{ echo 'const S: &str = r#"'; echo '#[cfg(test)]'; echo 'mod x {'; echo '"#;'; fns 9; } | row 1 raw_string_tail_ends_at_column_zero
{ echo '#[cfg(test)]'; echo 'mod tests {'; echo '    fn t() {}'; echo '}'; fns 20; } | row 1 item_after_test_mod_is_production
{ echo '// jtac-loc-exempt:  '; fns 20; } | row 1 whitespace_reason_is_no_marker
{ echo '//jtac-loc-exempt: x'; fns 20; } | row 1 marker_needs_the_space
{ echo '//! doc'; echo '// jtac-loc-exempt: x'; fns 20; } | row 1 marker_on_line_two_is_no_marker
{ fns 5; echo '// a'; echo '// b'; echo '// c'; echo '// d'; echo '// e'; } | row 1 comments_at_or_above_code
{ fns 1; echo '//// a'; echo '//// b'; } | row 1 four_slashes_are_a_comment
{ echo '// a'; echo '// b'; } | row 1 comments_with_no_code
{ fns 11; } | row 1 one_past_cap
{ fns 10; } | row 0 exactly_cap
{ fns 10; echo '#[cfg(test)]'; echo 'pub(crate) mod tests {'; fns 30 | sed 's/^/    /'; echo '}'; } | row 0 tail_test_mod_uncounted
{ echo '// jtac-loc-exempt: one table, defended in the PR'; fns 20; } | row 0 marker_with_reason
{ echo '#![cfg(test)]'; fns 20; } | row 0 inner_cfg_test_file_is_test
{ echo '//! stub'; echo '//! stub'; echo '//! stub'; } | row 0 doc_only_stub
{ fns 3; echo '/// a'; echo '/// b'; echo '/// c'; echo '/// d'; } | row 0 doc_comments_are_not_narration
mkdir -p "$t/named/src/tests" "$t/named/tests"; fns 11 > "$t/named/src/tests.rs"; fns 11 > "$t/named/src/tests/mod.rs"; fns 11 > "$t/named/tests/it.rs"; : > "$t/named/Cargo.toml"
LOC_CAP=10 sh "$here/loc-cap.sh" "$t/named" > "$t/named.out" 2>&1 && echo named_test_files_are_production >> "$t/failed"
for want in src/tests.rs src/tests/mod.rs; do grep -q "$want" "$t/named.out" || echo "named_test_files_are_production: $want" >> "$t/failed"; done
grep -q 'tests/it.rs' "$t/named.out" && echo crate_tests_dir_is_skipped >> "$t/failed"
mkdir "$t/empty"; sh "$here/loc-cap.sh" "$t/empty" >/dev/null 2>&1 && echo no_files_fails_closed >> "$t/failed"
[ ! -e "$t/failed" ] && { echo "loc-cap-test: all rows pass"; exit 0; }
cat "$t/failed"; exit 1
