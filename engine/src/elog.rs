//! The engine's own log: a line per event worth knowing about afterwards (a launch and how it
//! went, agents appearing and going), on stderr, which the apps keep as logs/engine.log for Send
//! Logs. Never prompts, conversations, folders' names or keys.

pub fn line(text: impl std::fmt::Display) {
    let now = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default();
    eprintln!("{now} {text}");
}

/// A folder as the log names it: home or not, and whether it's there.
pub fn folder(path: &std::path::Path) -> &'static str {
    match (path == crate::platform::home(), path.is_dir()) {
        (true, _) => "the home folder",
        (false, true) => "another folder",
        (false, false) => "a folder that isn't there",
    }
}

/// An agent as the log names it: id, route, its tmux pane or host app, and state.
pub fn agent(a: &crate::model::Agent) -> String {
    let route = serde_json::to_value(&a.route).ok().and_then(|v| v["via"].as_str().map(String::from)).unwrap_or_default();
    let pane =
        a.tmux.as_ref().map(|t| format!(" pane {}{}", t.socket.as_deref().map(|s| format!("-L {s} ")).unwrap_or_default(), t.target));
    let host = a.host.as_ref().map(|h| format!(" in {}", h.app));
    format!("{} via {route}{}{} ({:?})", a.id, pane.unwrap_or_default(), host.unwrap_or_default(), a.state)
}
