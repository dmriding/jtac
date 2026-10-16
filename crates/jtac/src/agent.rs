//! Whether this process runs inside an agent session (DESIGN.md section 11.6): the marker
//! the first set variable names. Pure over the environment it is handed; a lever pulled under
//! a marker records it, the marker never refuses.

/// Variables known agents export to every child, most specific first.
pub const MARKERS: [&str; 9] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CODEX_SANDBOX",
    "CODEX_THREAD_ID",
    "CURSOR_AGENT",
    "GEMINI_CLI",
    "NYX_AGENT_ACTIVE",
    "CHANNELS_AGENT",
    "AI_AGENT",
];

/// The first marker in `env` with a non-blank value.
pub fn marker<'a>(env: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<&'static str> {
    let set: Vec<&str> = env
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, _)| k)
        .collect();
    MARKERS.into_iter().find(|m| set.contains(m))
}

/// [`marker`] over this process's environment.
pub fn here() -> Option<&'static str> {
    let vars: Vec<(String, String)> = std::env::vars().collect();
    marker(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_marker_set_to_a_non_blank_value_names_the_session() {
        type Env<'a> = &'a [(&'a str, &'a str)];
        let rows: [(Env<'_>, Option<&str>); 5] = [
            (&[("HOME", "/workspace")], None),
            (&[("CLAUDECODE", "1")], Some("CLAUDECODE")),
            (&[("CLAUDECODE", "  ")], None),
            (
                &[("AI_AGENT", "x"), ("CODEX_SANDBOX", "1")],
                Some("CODEX_SANDBOX"),
            ),
            (&[("CHANNELS_AGENT", "opus")], Some("CHANNELS_AGENT")),
        ];
        for (env, want) in rows {
            assert_eq!(marker(env.iter().copied()), want, "{env:?}");
        }
    }
}
