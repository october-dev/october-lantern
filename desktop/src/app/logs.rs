//! Send Logs (Swift `Support.sendLogs`): the engine sends a report with Lantern's logs, secrets
//! taken out, to the October team (`lantern-engine report`) and gives a code to quote. If it
//! can't be sent, it's saved to the Desktop to send some other way.
use super::Lantern;
use crate::os::Event;
use serde_json::Value;
use std::io::Write;

impl Lantern {
    pub(super) fn send_logs(&mut self) {
        if self.model.system.logs == Some(crate::model::LogsReport::Sending) {
            return;
        }
        let details = self.logs_details();
        let token = self.session.as_ref().map(|session| session.access.clone());
        self.model.system.logs = Some(crate::model::LogsReport::Sending);
        self.show("Sending logs…");
        crate::os::task(self.tx.clone(), move || run_report(&details, token.as_deref()), Event::LogsSent);
    }

    /// What the app knows that the engine doesn't: never conversations, folders or code.
    fn logs_details(&self) -> String {
        let model = &self.model;
        let mut kinds = std::collections::BTreeMap::<String, usize>::new();
        for agent in &model.agents {
            *kinds.entry(agent.kind.display_name().to_string()).or_default() += 1;
        }
        let kinds: Vec<String> = kinds.into_iter().map(|(kind, n)| format!("{kind} ×{n}")).collect();
        let agents: Vec<String> = model
            .agents
            .iter()
            .map(|a| {
                let pane = a.tmux.as_ref().map(|t| format!(" pane {}", t.target)).unwrap_or_default();
                let via = a.route.as_ref().map_or("none", |r| r.via.as_str());
                format!(
                    "- {} {:?} via {via}{pane}{}",
                    a.id,
                    a.state,
                    a.route.as_ref().and_then(|r| r.session.as_deref()).map(|s| format!(" session {s}")).unwrap_or_default()
                )
            })
            .collect();
        format!(
            "October Lantern {} ({} {})\nEngine: {:?}\nEngine restarts in the last 5 minutes: {}\nAgents: {}\nComputer use: {}\nSigned in to October: {}\nUpdate: {}\nInstalled at: {}\n",
            env!("CARGO_PKG_VERSION"),
            crate::updates::system_name(),
            std::env::consts::ARCH,
            model.engine,
            self.engine_restarts.iter().filter(|at| at.elapsed().as_secs() < 300).count(),
            if kinds.is_empty() { "none".into() } else { kinds.join(", ") },
            if model.prefs.computer_use { model.computer.as_ref().map_or("starting", |c| c.status.as_str()) } else { "off" },
            if self.session.is_some() { "yes" } else { "no" },
            model.system.update.as_ref().map_or("not checked".into(), |u| format!("{} (newer: {})", u.version, u.newer)),
            std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.display().to_string())).unwrap_or_default(),
        ) + &format!("Ask Lantern tasks, newest first:\n{}\nEach agent:\n{}\n", model.task_report(), agents.join("\n"))
    }

    pub(super) fn logs_sent(&mut self, result: Result<Value, String>) {
        use crate::model::LogsReport;
        let report = match result {
            Ok(value) if value["sent"] == true => {
                let code = value["code"].as_str().unwrap_or_default().to_string();
                self.windows.copy(code.clone());
                LogsReport::Sent { code }
            }
            Ok(value) => LogsReport::Saved {
                path: value["saved"].as_str().unwrap_or_default().to_string(),
                error: value["error"].as_str().unwrap_or("couldn't send them").to_string(),
            },
            Err(error) => LogsReport::Failed { error },
        };
        self.show(report.summary());
        if let LogsReport::Saved { path, .. } = &report
            && let Some(folder) = std::path::Path::new(path).parent()
        {
            let _ = crate::os::open(folder);
        }
        self.model.system.logs = Some(report);
    }
}

fn run_report(details: &str, token: Option<&str>) -> anyhow::Result<Value> {
    let mut command = crate::os::command(crate::sidecar::engine_path()?);
    command.args(["report", "--app", std::env::consts::OS, "--app-version", env!("CARGO_PKG_VERSION")]);
    if let Some(token) = token {
        command.env("LANTERN_REPORT_TOKEN", token);
    }
    let mut child =
        command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(details.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().last().unwrap_or_default();
    serde_json::from_str(line).map_err(|_| anyhow::anyhow!("Lantern's engine didn't say how sending the logs went"))
}
