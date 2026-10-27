//! Show me's own connection to Lantern's Cua Driver.
//!
//! One `cua-driver mcp --socket <Lantern's socket>` child per guide, spoken to with the official
//! MCP SDK (rmcp). The connection owns its driver session: closing it (Stop, a new guide, the
//! engine exiting) ends the session and takes its cursor off the screen, which separate one-shot
//! `cua-driver call` commands wouldn't do.
//!
//! Show me only observes and moves its own cursor overlay: the tools it may call are listed in
//! `ALLOWED`, and the coordinator builds every argument itself.

use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::Value;

/// The only tools Show me calls. None of them clicks, types or moves the real pointer.
const ALLOWED: &[&str] = &["list_windows", "get_window_state", "start_session", "move_cursor", "set_agent_cursor_enabled", "end_session"];

struct Job {
    tool: &'static str,
    args: serde_json::Map<String, Value>,
    reply: mpsc::Sender<Result<Value>>,
}

/// A live connection. Dropping it closes the connection and ends Show me's driver session.
pub struct Cua {
    jobs: tokio::sync::mpsc::UnboundedSender<Job>,
}

impl Cua {
    /// Connects to Lantern's running driver.
    pub fn connect() -> Result<Cua> {
        let (bin, args) = crate::computer::mcp_server().context("Cua Driver isn't installed")?;
        let (jobs, mut incoming) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        std::thread::Builder::new().name("show-cua".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(r) => r,
                Err(e) => {
                    let _ = ready_tx.send(Err(anyhow!(e)));
                    return;
                }
            };
            runtime.block_on(async move {
                let mut cmd = tokio::process::Command::new(&bin);
                cmd.args(&args).env("DO_NOT_TRACK", "1").env("CUA_DRIVER_RS_UPDATE_CHECK", "false");
                if cfg!(target_os = "macos") {
                    cmd.env("CUA_DRIVER_EMBEDDED", "1");
                }
                let transport = match TokioChildProcess::builder(cmd).stderr(std::process::Stdio::null()).spawn() {
                    Ok((t, _)) => t,
                    Err(e) => {
                        let _ = ready_tx.send(Err(anyhow!(e).context("starting Cua Driver's connection")));
                        return;
                    }
                };
                let service = match ().serve(transport).await {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(anyhow!("connecting to Cua Driver: {e}")));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                while let Some(job) = incoming.recv().await {
                    let params = CallToolRequestParams::new(job.tool).with_arguments(job.args);
                    let result = match service.call_tool(params).await {
                        Ok(r) if r.is_error == Some(true) => {
                            Err(anyhow!("{}", text(&r.content).unwrap_or_else(|| "Cua Driver refused".into())))
                        }
                        Ok(r) => r
                            .structured_content
                            .filter(|v| !v.is_null())
                            .or_else(|| text(&r.content).and_then(|t| serde_json::from_str(&t).ok()))
                            .ok_or_else(|| anyhow!("Cua Driver's answer had no data")),
                        Err(e) => Err(anyhow!("Cua Driver: {e}")),
                    };
                    let _ = job.reply.send(result);
                }
                // Every handle is gone: close the connection, which ends Show me's session.
                let _ = service.cancel().await;
            });
        })?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Cua { jobs }),
            Ok(Err(e)) => Err(e),
            Err(_) => bail!("Cua Driver didn't answer"),
        }
    }

    /// Calls one of `ALLOWED` and waits at most `timeout` for its data.
    pub fn call(&self, tool: &'static str, args: Value, timeout: Duration) -> Result<Value> {
        if !ALLOWED.contains(&tool) {
            bail!("Show me doesn't use {tool}");
        }
        let Value::Object(args) = args else { bail!("arguments must be an object") };
        let (reply, answer) = mpsc::channel();
        self.jobs.send(Job { tool, args, reply }).map_err(|_| anyhow!("the connection to Cua Driver closed"))?;
        answer.recv_timeout(timeout).map_err(|_| anyhow!("Cua Driver took too long ({tool})"))?
    }
}

fn text(content: &[rmcp::model::ContentBlock]) -> Option<String> {
    content.iter().find_map(|c| c.as_text().map(|t| t.text.clone()))
}
