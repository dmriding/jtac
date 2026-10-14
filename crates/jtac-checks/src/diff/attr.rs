//! Rust attributes split across lines, read whole, so a marker on an inner line still counts
//! for `JT-SUP-001` and `JT-TEST-002`, and one that read the same before (comments aside,
//! strings kept) is not new; `a_rust_attribute_split_across_lines_is_one_marker` holds the shapes.

use std::collections::BTreeMap;

use super::lang::Lang;
use super::scan::Line;
use super::view::View;

/// Lines an attribute may span.
const LINES: usize = 32;

/// Added lines as (number, as written, probe text). A line inside a Rust attribute probes the
/// whole attribute at its opener's line, once per attribute, and not at all when only an inner
/// line changed and the old file had as many copies of the attribute as the new one; any other
/// line probes itself as `View::added` gives it.
pub(crate) fn probes<'v>(v: &'v View<'_>, comments: bool) -> Vec<(u64, &'v str, String)> {
    let mut out: Vec<(u64, &str, String)> = Vec::new();
    let mut copies: Option<(BTreeMap<String, usize>, BTreeMap<String, usize>)> = None;
    for (n, text, blanked) in v.added(comments) {
        let Some(start) = attribute(v, n) else {
            out.push((n, text, blanked));
            continue;
        };
        let probe = (
            number(start),
            v.new[start].text,
            joined(&v.new, start, code),
        );
        let seen = out.iter().any(|p| p.0 == probe.0);
        let kept = probe.0 != n && {
            let (old, new) = copies.get_or_insert_with(|| (attributes(&v.old), attributes(&v.new)));
            let key = joined(&v.new, start, written);
            old.get(&key) >= new.get(&key)
        };
        if !seen && !kept {
            out.push(probe);
        }
    }
    out
}

/// Every attribute in `lines` as written, with its number of copies.
fn attributes(lines: &[Line]) -> BTreeMap<String, usize> {
    let mut copies = BTreeMap::new();
    for i in (0..lines.len()).filter(|&i| opener(&lines[i])) {
        *copies.entry(joined(lines, i, written)).or_insert(0) += 1;
    }
    copies
}

fn code(l: &Line) -> String {
    l.code.clone()
}

/// The line less its comment, strings kept, so an edit inside one reads as new.
fn written(l: &Line) -> String {
    l.text
        .replacen(l.comment.as_str(), "", usize::from(!l.comment.is_empty()))
}

fn opener(l: &Line) -> bool {
    let s = l.code.trim_start();
    s.starts_with("#[") || s.starts_with("#![")
}

/// The opener of the attribute holding line `n`: the nearest `#[` or `#![` above with no item
/// boundary between and a `[` still open at `n`.
fn attribute(v: &View, n: u64) -> Option<usize> {
    if v.new_lang != Some(Lang::Rust) {
        return None;
    }
    let at = usize::try_from(n).ok()?.checked_sub(1)?;
    let lines = v.new.get(..=at)?;
    let boundary = |l: &Line| l.code.trim_end().ends_with(['{', '}', ';', ']']);
    let mut start = at;
    while !opener(&lines[start]) {
        if start == 0 || at - start == LINES || (start < at && boundary(&lines[start])) {
            return None;
        }
        start -= 1;
    }
    (start == at || lines[start..at].iter().map(brackets).sum::<i64>() > 0).then_some(start)
}

/// The attribute opened at `start`, on one line up to its close, each line as `shown` reads it.
fn joined(lines: &[Line], start: usize, shown: fn(&Line) -> String) -> String {
    let mut open = 0;
    let mut text = String::new();
    for line in lines.iter().skip(start).take(LINES) {
        if line.code.trim().is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(shown(line).trim());
        open += brackets(line);
        if open <= 0 {
            break;
        }
    }
    text
}

fn brackets(line: &Line) -> i64 {
    line.code.chars().fold(0, |n, c| match c {
        '[' => n + 1,
        ']' => n - 1,
        _ => n,
    })
}

fn number(index: usize) -> u64 {
    u64::try_from(index + 1).unwrap_or(u64::MAX)
}
