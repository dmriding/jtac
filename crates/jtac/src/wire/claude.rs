//! The Claude Code adapter (DESIGN.md section 11.3): what its `PreToolUse` hook writes to
//! stdin, the JSON that carries an advisory back, and the settings entry that registers
//! `jtac hook pre-tool claude-code`. A second agent is a sibling file, nothing else.

use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The agent name on the command line.
pub const NAME: &str = "claude-code";

/// Where `jtac init --agent claude-code` writes, under the work tree root: the per-clone
/// file, since the entry names this binary by absolute path.
pub const SETTINGS: &str = ".claude/settings.local.json";

/// The longest `additionalContext` the agent hands its model whole.
pub const MAX_CONTEXT: usize = 10_000;

/// The seconds the agent waits on the hook, above [`crate::pretool::CAP`].
const TIMEOUT: u64 = 2;

#[derive(Deserialize)]
struct Input {
    cwd: Option<String>,
    tool_name: String,
    #[serde(default)]
    tool_input: Map<String, Value>,
}

/// One Bash call as the agent proposed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// The session's directory, when the agent named one.
    pub cwd: Option<String>,
    /// The script.
    pub command: String,
}

/// The call on stdin; `None` for any tool but Bash.
pub fn call(stdin: &str) -> Result<Option<Call>, String> {
    let input: Input = serde_json::from_str(stdin).map_err(|e| format!("stdin: {e}"))?;
    if input.tool_name != "Bash" {
        return Ok(None);
    }
    let command = match input.tool_input.get("command") {
        Some(Value::String(s)) => s.clone(),
        _ => return Err("stdin: `tool_input.command` is not a string".to_owned()),
    };
    Ok(Some(Call {
        cwd: input.cwd,
        command,
    }))
}

/// The stdout that carries `note` to the model: `additionalContext`, no decision, so the
/// call proceeds. Over the cap the agent spills to a file; cut on a line instead.
pub fn output(note: &str) -> String {
    let mut text = note;
    while text.len() > MAX_CONTEXT {
        text = text.rsplit_once('\n').map_or("", |(head, _)| head);
    }
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": text,
        }
    })
    .to_string()
}

/// The shell line the agent runs; single quotes as the git hook spells its path.
pub fn command_line(exe: &str) -> String {
    format!("'{exe}' hook pre-tool {NAME}")
}

/// `{"hooks": {"PreToolUse": [entry]}}`, pretty, for a file that has none.
pub fn fragment(exe: &str) -> String {
    pretty(&json!({"hooks": {"PreToolUse": [entry(exe)]}}))
}

fn entry(exe: &str) -> Value {
    json!({
        "matcher": "Bash",
        "hooks": [{"type": "command", "command": command_line(exe), "timeout": TIMEOUT}],
    })
}

/// What the settings file needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Merge {
    /// It already registers this binary.
    Present,
    /// The whole text to write.
    Written(String),
}

/// `existing` with the entry added, or its command rewritten when it named a jtac binary
/// that moved. A file that is not a JSON object, or whose `hooks` or `PreToolUse` is the
/// wrong shape, is refused with the reason: nothing is written through a guess.
pub fn merge(existing: Option<&str>, exe: &str) -> Result<Merge, String> {
    let Some(text) = existing else {
        return Ok(Merge::Written(fragment(exe)));
    };
    let mut root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Value::Object(top) = &mut root else {
        return Err("it is not a JSON object".to_owned());
    };
    let hooks = top.entry("hooks").or_insert_with(|| json!({}));
    let Value::Object(hooks) = hooks else {
        return Err("`hooks` is not a JSON object".to_owned());
    };
    let pre = hooks.entry("PreToolUse").or_insert_with(|| json!([]));
    let Value::Array(pre) = pre else {
        return Err("`hooks.PreToolUse` is not a JSON array".to_owned());
    };
    let want = command_line(exe);
    let suffix = format!(" hook pre-tool {NAME}");
    let ours = pre
        .iter_mut()
        .filter_map(|e| e.get_mut("hooks").and_then(Value::as_array_mut))
        .flatten()
        .filter_map(|h| h.get_mut("command"))
        .find(|c| c.as_str().is_some_and(|c| c.ends_with(&suffix)));
    match ours {
        Some(c) if c.as_str() == Some(&want) => return Ok(Merge::Present),
        Some(c) => *c = Value::String(want),
        None => pre.push(entry(exe)),
    }
    Ok(Merge::Written(pretty(&root)))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bash_command_and_cwd_are_read_and_other_tools_are_none() {
        let bash = r#"{"session_id":"s","cwd":"/workspace/app","hook_event_name":"PreToolUse",
            "tool_name":"Bash","tool_input":{"command":"git push","description":"d"}}"#;
        assert_eq!(
            call(bash).unwrap(),
            Some(Call {
                cwd: Some("/workspace/app".to_owned()),
                command: "git push".to_owned()
            })
        );
        let edit = r#"{"tool_name":"Edit","tool_input":{"file_path":"x"}}"#;
        assert_eq!(call(edit).unwrap(), None);
        assert!(call("not json").unwrap_err().starts_with("stdin:"));
        let odd = r#"{"tool_name":"Bash","tool_input":{"command":7}}"#;
        assert!(call(odd).unwrap_err().contains("not a string"));
    }

    #[test]
    fn the_output_names_the_event_carries_no_decision_and_cuts_on_a_line_at_the_cap() {
        let v: Value = serde_json::from_str(&output("jtac: x")).unwrap();
        let out = &v["hookSpecificOutput"];
        assert_eq!(out["hookEventName"], "PreToolUse");
        assert_eq!(out["additionalContext"], "jtac: x");
        assert!(out.get("permissionDecision").is_none());
        let long: Vec<String> = (0..200)
            .map(|i| format!("jtac: {i} {}", "x".repeat(60)))
            .collect();
        let v: Value = serde_json::from_str(&output(&long.join("\n"))).unwrap();
        let text = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(
            text.len() <= MAX_CONTEXT && text.ends_with('x'),
            "{}",
            text.len()
        );
    }

    #[test]
    fn the_entry_is_added_kept_or_repointed_and_a_wrong_shape_is_refused() {
        let exe = "/opt/jtac/jtac";
        let Merge::Written(fresh) = merge(None, exe).unwrap() else {
            panic!("no file is a write")
        };
        assert_eq!(fresh, fragment(exe));
        assert_eq!(merge(Some(&fresh), exe).unwrap(), Merge::Present);
        let Merge::Written(moved) = merge(Some(&fresh), "/usr/bin/jtac").unwrap() else {
            panic!("a moved binary is a rewrite")
        };
        assert!(
            moved.contains("'/usr/bin/jtac' hook pre-tool claude-code"),
            "{moved}"
        );
        assert_eq!(moved.matches("pre-tool").count(), 1);
        let theirs = r#"{"permissions":{"allow":["Bash(ls)"]},"hooks":{"PreToolUse":[
            {"matcher":"Edit","hooks":[{"type":"command","command":"lint"}]}]}}"#;
        let Merge::Written(both) = merge(Some(theirs), exe).unwrap() else {
            panic!("another entry is kept beside ours")
        };
        let v: Value = serde_json::from_str(&both).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        assert_eq!(v["permissions"]["allow"][0], "Bash(ls)");
        assert!(
            merge(Some("[]"), exe)
                .unwrap_err()
                .contains("not a JSON object")
        );
        assert!(
            merge(Some(r#"{"hooks":[]}"#), exe)
                .unwrap_err()
                .contains("`hooks`")
        );
        let arr = r#"{"hooks":{"PreToolUse":{}}}"#;
        assert!(merge(Some(arr), exe).unwrap_err().contains("PreToolUse"));
        assert!(merge(Some("{"), exe).is_err());
    }
}
