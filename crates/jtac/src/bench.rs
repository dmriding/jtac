//! `jtac bench` (DESIGN.md section 13): the public proof. Each control is attacked through
//! its real path in a scratch repository under an isolated home, and every row quotes the
//! gate's own line. No model, no network.

mod cases;
mod scratch;

use std::fmt::Write;

use serde::Serialize;

use crate::Printed;
use crate::refuse::Refusal;

/// One control attacked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    /// The control.
    pub control: &'static str,
    /// What was done to it.
    pub attack: &'static str,
    /// What the gate had to do: `refused`, `noted`, or `passed`.
    pub expected: &'static str,
    /// The gate's own line, or what went wrong.
    pub observed: String,
    /// Whether the control held.
    pub held: bool,
}

/// What `jtac bench` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BenchOut {
    /// Output schema version.
    pub v: u32,
    /// The command.
    pub command: &'static str,
    /// Every control held.
    pub pass: bool,
    /// One row per control, in the order run.
    pub rows: Vec<Row>,
}

/// Run every control; exit 0 when all held, 1 when one did not, 2 when bench could not run.
pub fn bench(json: bool) -> Result<Printed, Refusal> {
    let exe = std::env::current_exe().map_err(|e| Refusal::exe(&e))?;
    let lab = scratch::Lab::new(&exe)?;
    let out = judged(cases::run(&lab)?);
    let text = if json {
        crate::render::json(&out)?
    } else {
        render(&out)
    };
    Ok(Printed::verdict(text, out.pass))
}

/// The verdict over the rows. Pure: every control held, or not.
fn judged(rows: Vec<Row>) -> BenchOut {
    BenchOut {
        v: 1,
        command: "bench",
        pass: rows.iter().all(|r| r.held),
        rows,
    }
}

fn render(out: &BenchOut) -> String {
    let held = out.rows.iter().filter(|r| r.held).count();
    let mut s = format!("jtac bench: {held} of {} controls held\n", out.rows.len());
    for r in &out.rows {
        let mark = if r.held { "held  " } else { "FAILED" };
        let _ = writeln!(
            s,
            "  {mark}  {:<20} {:<8} {}",
            r.control, r.expected, r.observed
        );
    }
    if !out.pass {
        s.push_str(
            "A control that did not hold is a regression in the gate, not in your repository: \
             file the rows above, with `jtac --version`, against jtac.\n",
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(control: &'static str, held: bool) -> Row {
        Row {
            control,
            attack: "x",
            expected: "refused",
            observed: "line".to_owned(),
            held,
        }
    }

    #[test]
    fn one_control_that_did_not_hold_fails_the_bench_and_is_marked() {
        let out = judged(vec![row("a", true), row("b", false)]);
        assert!(!out.pass);
        let text = render(&out);
        assert!(
            text.starts_with("jtac bench: 1 of 2 controls held\n"),
            "{text}"
        );
        assert!(text.contains("  FAILED  b "), "{text}");
        assert!(text.contains("  held    a "), "{text}");
        assert!(text.contains("regression in the gate"), "{text}");
        let all = judged(vec![row("a", true)]);
        assert!(all.pass);
        assert!(!render(&all).contains("regression"));
    }
}
