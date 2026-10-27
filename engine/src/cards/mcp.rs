//! `lantern-engine cards-mcp <launch>`: the MCP server each launch's agent gets (stdio, with the
//! official rmcp SDK). It holds no state: every tool call is a request to the engine (`ipc`), and
//! a call that waits for the person keeps asking the engine until the card is answered or closed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

/// How long to keep trying while the engine is away (it restarts with the app).
const RECONNECT_FOR: Duration = Duration::from_secs(15);

struct Server {
    launch: String,
    key: String,
}

impl Server {
    /// One request to the engine, retried while it can't be reached.
    async fn call(&self, mut op: Value) -> Result<Value> {
        op["operationId"] = json!(uuid::Uuid::new_v4().to_string());
        op["launch"] = json!(self.launch);
        op["key"] = json!(self.key);
        let started = Instant::now();
        loop {
            let request = op.clone();
            match tokio::task::spawn_blocking(move || super::ipc::request(&request)).await? {
                Ok(reply) if reply["ok"] == true => return Ok(reply),
                Ok(reply) => bail!("{}", reply["message"].as_str().unwrap_or("Lantern refused")),
                Err(e) if started.elapsed() >= RECONNECT_FOR => return Err(e),
                Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    }

    /// Waits for the person to answer `card`, or for it to close; the agent cancelling the call
    /// closes it.
    async fn wait(&self, card: &str, context: &RequestContext<RoleServer>) -> Result<(String, Value)> {
        loop {
            let poll = self.call(json!({"op": "wait", "card": card}));
            let reply = tokio::select! {
                reply = poll => reply?,
                _ = context.ct.cancelled() => {
                    let _ = self.call(json!({"op": "cancel", "card": card})).await;
                    bail!("cancelled");
                }
            };
            let state = reply["state"].as_str().unwrap_or("stopped").to_string();
            if state == "waiting" {
                continue;
            }
            if context.ct.is_cancelled() {
                let _ = self.call(json!({"op":"cancel","card":card})).await;
                bail!("cancelled");
            }
            if reply["answer"].is_object() {
                self.call(json!({"op": "ack", "card": card})).await?;
            }
            return Ok((state, reply["answer"].clone()));
        }
    }

    async fn run(&self, name: &str, args: JsonObject, context: &RequestContext<RoleServer>) -> Result<String> {
        let text = |field: &str| args.get(field).and_then(Value::as_str).map(str::to_string);
        let list = |field: &str| args.get(field).cloned().unwrap_or(json!([]));
        match name {
            "suggest_routine" => {
                self.call(json!({"op":"suggestRoutine","name":text("name"),"prompt":text("prompt")})).await?;
                Ok("A draft is shown. Only the person can choose its schedule and create it.".into())
            }
            "show_me" => {
                let reply = self.call(json!({"op":"guide","question":text("question")})).await?;
                let (state, answer) = self.wait(reply["card"].as_str().unwrap_or_default(), context).await?;
                Ok(match answer["done"].as_bool() {
                    Some(true) => "The person says they completed the step; verify the result.".into(),
                    Some(false) => "The person cannot do this now; leave dependent work pending.".into(),
                    None => closed(&state),
                })
            }
            "ask_person" => {
                let reply = self
                    .call(json!({"op": "ask", "question": text("question"), "details": text("details"), "options": list("options")}))
                    .await?;
                let (state, answer) = self.wait(reply["card"].as_str().unwrap_or_default(), context).await?;
                Ok(match (answer["choice"].as_str(), answer["text"].as_str()) {
                    (Some(choice), _) => format!("The person chose: {choice}"),
                    (None, Some(words)) => format!("The person answered in their own words: {words}"),
                    _ => closed(&state),
                })
            }
            "hand_off" | "need_sign_in" => {
                let op = if name == "need_sign_in" {
                    let url = text("url").unwrap_or_default();
                    let site = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or(url.clone());
                    json!({"op": "handOff", "reason": "signIn", "step": format!("Sign in to {site}"), "why": text("why"), "url": url})
                } else {
                    json!({"op": "handOff", "reason": text("reason").unwrap_or("other".into()), "step": text("step"), "why": text("why"),
                        "instructions": list("instructions"), "url": text("url")})
                };
                let reply = self.call(op).await?;
                let (state, answer) = self.wait(reply["card"].as_str().unwrap_or_default(), context).await?;
                let note = answer["text"].as_str().map(|t| format!(" They added: {t}")).unwrap_or_default();
                Ok(match answer["done"].as_bool() {
                    Some(true) => format!("The person says they've done it.{note} Check that it worked before you go on."),
                    Some(false) => format!(
                        "The person can't do this right now.{note} Don't do anything that depends on it. Carry on with anything that \
                         doesn't, then report what's left with show_result (outcome \"partial\")."
                    ),
                    None => closed(&state),
                })
            }
            "set_status" => {
                self.call(json!({"op": "status", "text": text("text")})).await?;
                Ok("Shown.".into())
            }
            "set_steps" => {
                self.call(json!({"op": "steps", "steps": list("steps")})).await?;
                Ok("Shown.".into())
            }
            "step_done" => {
                self.call(json!({"op": "stepDone", "id": text("id")})).await?;
                Ok("Marked done.".into())
            }
            "show_result" => {
                self.call(json!({"op": "result", "summary": text("summary"), "outcome": text("outcome").unwrap_or("done".into()),
                    "files": list("files")}))
                    .await?;
                Ok("Shown to the person.".into())
            }
            other => bail!("no tool named {other}"),
        }
    }
}

/// What the agent hears when a card closed without an answer.
fn closed(state: &str) -> String {
    match state {
        "expired" => format!(
            "Nobody answered within {} minutes, so this was closed. Nothing was decided: don't take it as a yes. Carry on only \
             with work that doesn't depend on it, or ask again later.",
            super::WAIT_LIMIT.as_secs() / 60
        ),
        _ => "This was closed before anyone answered. Nothing was decided: don't take it as a yes.".into(),
    }
}

fn schema(v: Value) -> Arc<JsonObject> {
    Arc::new(v.as_object().cloned().unwrap_or_default())
}

fn tools() -> Vec<Tool> {
    let string = |description: &str| json!({"type": "string", "description": description});
    vec![
        Tool::new(
            "show_me",
            "Offer to point out the next control for the person. They choose Show me and do the step; never request secret text.",
            schema(
                json!({"type":"object","required":["question"],"properties":{"question":string("What to show on this computer, at most 200 characters.")}}),
            ),
        ),
        Tool::new(
            "suggest_routine",
            "Suggest a local routine draft. The person must review the context, choose a schedule and confirm; this never creates unattended work.",
            schema(
                json!({"type":"object","required":["name","prompt"],"properties":{"name":string("Short routine name."),"prompt":string("The complete task for each run; never include secrets.")}}),
            ),
        ),
        Tool::new(
            "ask_person",
            "Ask the person a question and wait for their answer (up to 25 minutes). Use it for decisions only they can make, \
             instead of asking in your reply. Give options when there are clear choices; they can also answer in their own words.",
            schema(json!({"type": "object", "required": ["question"], "properties": {
                "question": string("The question, in one or two plain sentences."),
                "details": string("What they need to know to decide: the exact command, amount, recipient or file."),
                "options": {"type": "array", "maxItems": 6, "items": {"type": "string", "maxLength": 80},
                    "description": "Short choices, if there are any."}
            }})),
        ),
        Tool::new(
            "hand_off",
            "Hand a step to the person when only they can do it, and wait until they've done it or say they can't (up to 25 \
             minutes). For example a verification code, a captcha, approving a payment, proving who they are, or anything \
             physical. Never ask for passwords or codes to be sent to you.",
            schema(json!({"type": "object", "required": ["step", "reason"], "properties": {
                "step": string("What they need to do, in a few words."),
                "reason": {"type": "string", "enum": ["verification", "payment", "identity", "physical", "other"]},
                "why": string("Why it's needed, in one sentence."),
                "instructions": {"type": "array", "maxItems": 3, "items": {"type": "string"}, "description": "Up to three short steps."},
                "url": string("A web page where they do it, if there is one.")
            }})),
        ),
        Tool::new(
            "need_sign_in",
            "Ask the person to sign in to a website themselves (in their own browser), and wait until they have (up to 25 \
             minutes). Never ask for their password.",
            schema(json!({"type": "object", "required": ["url"], "properties": {
                "url": string("The https address of the page to sign in on."),
                "why": string("Why you need them signed in, in one sentence.")
            }})),
        ),
        Tool::new(
            "set_status",
            "Show the person, in a few words, what you're doing now.",
            schema(json!({"type": "object", "required": ["text"], "properties": {"text": string("At most 200 characters.")}})),
        ),
        Tool::new(
            "set_steps",
            "Show the person your plan as a short list of steps (at most 12). Steps already marked done stay done.",
            schema(json!({"type": "object", "required": ["steps"], "properties": {"steps": {"type": "array", "maxItems": 12, "items": {
                "type": "object", "required": ["id", "title"],
                "properties": {"id": string("A short id that stays the same."), "title": string("The step, in a few words.")}
            }}}})),
        ),
        Tool::new(
            "step_done",
            "Mark one of your steps done.",
            schema(json!({"type": "object", "required": ["id"], "properties": {"id": string("The step's id from set_steps.")}})),
        ),
        Tool::new(
            "show_result",
            "Show the person what you did when you finish, with the files they'd want to open. Say honestly whether it's done, \
             partly done or failed.",
            schema(json!({"type": "object", "required": ["summary", "outcome"], "properties": {
                "summary": string("What you did or found, in a few plain sentences."),
                "outcome": {"type": "string", "enum": ["done", "partial", "failed"]},
                "files": {"type": "array", "maxItems": 10, "items": {"type": "string"}, "description": "Full paths of files or folders."}
            }})),
        ),
    ]
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("lantern", env!("CARGO_PKG_VERSION")))
            .with_instructions(super::note())
    }

    async fn list_tools(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let result = match self.run(&request.name, request.arguments.unwrap_or_default(), &context).await {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("{e:#}"))]),
        };
        Ok(result.into())
    }
}

pub fn run(launch: &str) -> Result<()> {
    let key = super::read_key(launch)?;
    let server = Server { launch: launch.to_string(), key };
    tokio::runtime::Builder::new_current_thread().enable_time().build()?.block_on(async move {
        // Tells Lantern this agent has the tools; once, so starting never waits on it.
        let hello = json!({"op": "hello", "launch": server.launch, "key": server.key});
        let _ = tokio::task::spawn_blocking(move || super::ipc::request(&hello)).await;
        let running = server.serve(rmcp::transport::stdio()).await?;
        running.waiting().await?;
        Ok(())
    })
}
