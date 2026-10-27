//! The Mac app's surfaces, drawn with egui (CONTRACT §4–5). Views read `&Model`, edit `Inputs`
//! text in place, keep view-only state in `UiState`, and report what the person did as
//! `UiAction`s; `app::Lantern::apply` turns those into effects.

pub mod assets;
mod assistant;
mod cards;
mod chat;
pub mod components;
mod composer;
mod feed;
pub mod icons;
mod markdown;
mod october;
mod point_ask;
mod question;
mod sessions;
mod settings;
pub mod theme;
pub mod welcome;

use crate::model::{AgentId, AgentKind, AppContext, Edge, Inputs, Model, PanelMode};
use eframe::egui;
use std::path::PathBuf;

pub use assets::Assets;

/// What a view needs from outside the model.
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// Milliseconds since the epoch; fixed in fixtures.
    pub now_ms: f64,
    pub reduce_motion: bool,
    /// The home folder, for `~` paths and "Home folder".
    pub home: String,
}

/// View-local state (Atlas): expanded cards, More toggles, request detail, menus, welcome step,
/// settings tab, model search, email/password, focus tokens, scroll anchors.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct UiState {
    pub expanded_cards: std::collections::HashSet<String>,
    /// Options ticked so far on a question card, by prompt id, then question.
    #[serde(skip)]
    pub question_picks: std::collections::HashMap<String, std::collections::BTreeMap<usize, std::collections::BTreeSet<usize>>>,
    pub request_details: std::collections::HashSet<String>,
    pub expanded_bubbles: std::collections::HashSet<String>,
    pub welcome_step: usize,
    pub settings_tab: usize,
    pub settings_uninstall: bool,
    /// Initial scroll position for deterministic Settings fixtures; normal windows leave it unset.
    pub settings_scroll: Option<f32>,
    #[serde(skip)]
    pub composer_focus: Option<(String, u64)>,
    pub searching_models: bool,
    pub model_query: String,
    pub session_error: Option<String>,
    #[serde(skip)]
    pub model_search_kind: Option<AgentKind>,
    #[serde(skip)]
    pub capture_texture: sessions::CaptureTexture,
    #[serde(skip)]
    pub task_icon: sessions::CaptureTexture,
    #[serde(skip)]
    pub assistant_focus: Option<u64>,
    /// Ctrl+V is held (so holding it adds the clipboard's image once).
    #[serde(skip)]
    pub paste_held: bool,
    #[serde(skip)]
    pub thumbnails: assistant::Thumbnails,
    #[serde(skip)]
    pub assistant_scroll_anchor: Option<(u64, u64)>,
    /// Ask Lantern's Recent shows every saved conversation, not just the last three.
    pub assistant_show_all: bool,
    pub account_use_email: bool,
    pub account_email: String,
    #[serde(skip)]
    pub account_password: String,
    #[serde(skip)]
    pub desktop_flow: Option<(bool, f64)>,
    #[serde(skip)]
    pub point_ask_capture: sessions::CaptureTexture,
    #[serde(skip)]
    pub point_ask_focus: Option<u64>,
    #[serde(skip)]
    pub point_ask_scroll_anchor: Option<(usize, u64)>,
}

pub struct Ctx<'a> {
    pub model: &'a Model,
    pub inputs: &'a mut Inputs,
    pub view: &'a mut UiState,
    pub out: &'a mut Vec<UiAction>,
    pub assets: &'a Assets,
    pub env: &'a Env,
}

impl Ctx<'_> {
    pub fn emit(&mut self, action: UiAction) {
        self.out.push(action);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Google,
    GitHub,
    Apple,
}

/// Whose draft dictation writes into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DictationOwner {
    Reply(AgentId),
    Assistant,
    PointAsk,
}

/// A New Session start. Defaults, persistence and saving the screenshot first belong to `apply`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionRequest {
    pub kind: AgentKind,
    pub folder: String,
    pub prompt: String,
    pub model: Option<String>,
    pub screenshot: Option<PathBuf>,
    /// Images added to an Ask Lantern task (in the screenshots folder).
    pub images: Vec<PathBuf>,
    pub context: Option<AppContext>,
    pub toolkit: bool,
    pub bus: bool,
    /// In Lantern (true) or a Terminal window (false).
    pub background: bool,
    /// The app a Task is for (its id).
    pub task_app: Option<String>,
    /// Ask Lantern's task for this turn: it stays in that conversation, so the panel doesn't
    /// switch to it and how the launch went is reported back (Swift `launchTask(report:)`).
    pub for_turn: Option<u64>,
}

/// Visible preferences (Settings).
#[derive(Clone, Debug, PartialEq)]
pub enum SettingChange {
    OpenAtLogin(bool),
    Edge(Edge),
    MessageHotkey(String),
    PointAskHotkey(String),
    AutomaticUpdates(bool),
    ScreenshotNewSessions(bool),
    BusForSessions(bool),
    ComputerUse(bool),
    AskFirst(bool),
    ToolkitInSessions(bool),
    NotificationsEnabled(bool),
    NotifyOnFinish(bool),
    AgentMuted { kind: AgentKind, muted: bool },
}

/// Everything a person can do (CONTRACT §4; Atlas owns this enum, payloads frozen with the skeleton).
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    // Panels and chats
    SetPanel(Option<PanelMode>),
    TogglePanel(PanelMode),
    OpenChat {
        agent_id: AgentId,
    },
    CloseChat,
    /// `None`: Ask Lantern, focused. `Some`: that agent's chat, composer focused.
    Compose {
        agent_id: Option<AgentId>,
    },
    SendReply {
        agent_id: AgentId,
    },
    OpenAgent {
        agent_id: AgentId,
    },
    /// The turns the person saw (`agentId@stateSince`).
    DismissTurns {
        turn_keys: Vec<String>,
    },
    /// Answers an agent's question from its card (a permission, Claude's own question, its plan).
    AnswerQuestion {
        agent_id: AgentId,
        prompt_id: String,
        answer: crate::model::AgentAnswer,
    },
    ToggleDictation {
        owner: DictationOwner,
    },
    /// The answer to "Turn on dictation?": Download, or Not now.
    AnswerDictationOffer {
        download: bool,
    },
    // New Session
    /// Captures the foreground app before the panel takes focus.
    StartNewSession,
    ChooseSessionAgent {
        kind: AgentKind,
    },
    ChooseSessionModel {
        kind: AgentKind,
        model: Option<String>,
    },
    /// The native folder picker.
    ChooseFolder,
    SetSessionScreenshot(bool),
    RetakeSessionScreenshot,
    SetOpenInTerminal(bool),
    /// Opens where to get an agent that isn't installed.
    GetAgent {
        kind: AgentKind,
    },
    RequestCapturePermission,
    /// Boxed: much larger than every other action.
    Launch(Box<SessionRequest>),
    CloseLaunchPreview,
    RetryPreviewInTerminal,
    // Point & Ask
    StartPointAsk,
    ReselectPointAsk,
    CancelPointAsk,
    ChoosePointAskRecipient(AgentId),
    SubmitPointAsk,
    SendPointAskToAgent,
    CopyPointAskAnswer,
    // Ask Lantern
    /// Asks Lantern, or replies to the conversation's running agent.
    SubmitAssistant,
    /// "New chat": the current one stays in Recent.
    ClearAssistant,
    /// Opens October's plans from an answer that hit a plan limit; `from` is "assistant" or
    /// "point_ask" (counted).
    OpenUpgrade {
        url: String,
        from: &'static str,
    },
    /// "Share usage counts".
    SetShareUsage(bool),
    /// Download October Harness again after a failed install (Welcome's Try again).
    PrepareHarness,
    /// Try again: starts the task that couldn't start for this turn.
    RetryAssistantTask {
        turn: u64,
    },
    /// "Ask Lantern instead" ⇄ "Reply to @handle instead".
    ToggleAskLanternInstead,
    OpenConversation {
        id: String,
    },
    DeleteConversation {
        id: String,
    },
    // Account and October Desktop
    SignInProvider(Provider),
    SignInEmail {
        email: String,
        password: String,
    },
    CancelSignIn,
    SignOut,
    DesktopConnect,
    DesktopCancelPairing,
    DesktopDisconnect,
    OpenOctober,
    GetOctober,
    // Settings and app
    ShowSettings,
    ChangeSetting(SettingChange),
    SetHooks {
        enabled: bool,
    },
    RefreshToolkit,
    OpenToolkit,
    OpenNotificationSettings,
    CheckUpdates,
    ReportProblem,
    SendLogs,
    ShowWelcome,
    FinishWelcome,
    ConfirmUninstall,
    /// After partial cleanup, deliberately remove the app despite the listed failures.
    RemoveAppAnyway,
    /// Keep the app after partial cleanup, or dismiss an app-removal error.
    DismissUninstall,
    Copy(String),
    OpenUrl(String),
    /// Ask Lantern's images: the clipboard's (Ctrl+V when it holds no text), dropped files, the
    /// file picker, removing one before sending, and opening one from the conversation.
    PasteAssistantImages,
    AddAssistantImages(Vec<PathBuf>),
    PickAssistantImages,
    RemoveAssistantImage(String),
    OpenImage(String),
    Quit,
}

/// The main panel, 380 wide. Returns the measured outer height.
pub fn render_panel(ui: &mut egui::Ui, cx: &mut Ctx, max_height: f32) -> f32 {
    if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cx.emit(if cx.model.dictation_offer.as_ref().is_some_and(|offer| !offer.point_ask) {
            UiAction::AnswerDictationOffer { download: false }
        } else if cx.model.launch_preview.is_some() && cx.model.panel.is_none_or(PanelMode::is_list) {
            UiAction::CloseLaunchPreview
        } else if cx.model.chat.is_some() && cx.model.panel.is_none_or(PanelMode::is_list) {
            UiAction::CloseChat
        } else {
            UiAction::SetPanel(None)
        });
    }
    let tint = components::material_tint(0.5, cx.model.caps.dense_glass);
    surface(ui, theme::PANEL_WIDTH, max_height, theme::PANEL_RADIUS as f32, tint, |ui| {
        feed::engine_banner(ui, cx);
        match cx.model.panel {
            Some(PanelMode::NewSession) => {
                sessions::render(ui, cx, max_height);
            }
            Some(PanelMode::October) => {
                october::render(ui, cx, max_height);
            }
            Some(PanelMode::Assistant) => {
                assistant::render(ui, cx, max_height);
            }
            _ => {
                if let Some(preview) = &cx.model.launch_preview {
                    chat::preview(ui, cx, preview);
                } else if let Some(agent) = cx.model.chat_agent() {
                    chat::render(ui, cx, agent, max_height);
                } else {
                    feed::render(ui, cx, max_height);
                }
            }
        }
        if cx.model.dictation_offer.as_ref().is_some_and(|offer| !offer.point_ask) {
            egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 6 }).show(ui, |ui| dictation_offer(ui, cx));
        }
        if let Some(toast) = &cx.model.toast {
            egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 0, bottom: 6 }).show(ui, |ui| {
                egui::Frame::new().fill(theme::FAINT).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    components::label(ui, &toast.text, 12.0, theme::Weight::Regular, theme::white(0.9));
                });
            });
        }
        if cx.model.panel.is_none_or(PanelMode::is_list)
            && cx.model.launch_preview.is_none()
            && let Some(agent) = cx.model.chat_agent()
        {
            composer::render(ui, cx, agent);
        }
    })
}

/// "Turn on dictation?": dictation's first use asks before it downloads anything. In the panel
/// (above the composer) or the Point & Ask card (above its box), wherever the mic was pressed.
fn dictation_offer(ui: &mut egui::Ui, cx: &mut Ctx) {
    let Some(offer) = &cx.model.dictation_offer else { return };
    let text = offer.text.clone();
    egui::Frame::new().fill(theme::FAINT).corner_radius(10).inner_margin(12).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
        components::label(ui, crate::speech::OFFER_TITLE, 13.0, theme::Weight::Semibold, theme::INK);
        components::label(ui, text, 12.0, theme::Weight::Regular, theme::MUTED);
        ui.add_space(2.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if components::amber_button(ui, "Download").clicked() {
                cx.emit(UiAction::AnswerDictationOffer { download: true });
            }
            if components::secondary_button(ui, "Not now").clicked() {
                cx.emit(UiAction::AnswerDictationOffer { download: false });
            }
        });
    });
}

fn surface(ui: &mut egui::Ui, width: f32, max_height: f32, radius: f32, tint: f32, draw: impl FnOnce(&mut egui::Ui)) -> f32 {
    let background = ui.painter().add(egui::Shape::Noop);
    let start = ui.cursor().min;
    let result = ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(egui::Rect::from_min_size(start, egui::vec2(width, max_height)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            draw(ui);
        },
    );
    let height = result.response.rect.height().min(max_height).max(1.0);
    let rect = egui::Rect::from_min_size(start, egui::vec2(width, height));
    ui.painter().set(background, components::glass_shape(rect, radius, tint));
    height
}

/// The Point & Ask card, 400 wide.
pub fn render_point_ask(ui: &mut egui::Ui, cx: &mut Ctx, max_height: f32) -> f32 {
    point_ask::render(ui, cx, max_height)
}

/// Welcome, 540 x 640.
pub fn render_welcome(ui: &mut egui::Ui, cx: &mut Ctx) -> f32 {
    welcome::render(ui, cx)
}

/// Settings, 520 x 480.
pub fn render_settings(ui: &mut egui::Ui, cx: &mut Ctx) -> f32 {
    settings::render(ui, cx)
}

#[cfg(test)]
mod interaction_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, AgentState, Chat, ChatMessage, EngineHealth, LaunchPreview, Role, Route};

    pub(super) fn frame(
        ctx: &egui::Context,
        assets: &Assets,
        model: &Model,
        inputs: &mut Inputs,
        view: &mut UiState,
        events: Vec<egui::Event>,
    ) -> (f32, Vec<UiAction>, Vec<String>) {
        fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => texts.push(text.galley.job.text.clone()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, texts);
                    }
                }
                _ => {}
            }
        }
        let mut height = 0.0;
        let mut width = 0.0;
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let mut cx = Ctx { model, inputs, view, out: &mut actions, assets, env: &Env::default() };
                height = render_panel(ui, &mut cx, 900.0);
                width = ui.min_rect().width();
            },
        );
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        output.textures_delta.clear();
        assert!(width <= 380.1, "panel overflowed its 380pt width: {width}");
        (height, actions, texts)
    }

    #[test]
    fn all_list_chat_and_preview_fixtures_deserialize_and_fit_the_panel_width() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        for raw in [
            include_str!("../../fixtures/panel-inbox.json"),
            include_str!("../../fixtures/panel-agents.json"),
            include_str!("../../fixtures/panel-chat.json"),
            include_str!("../../fixtures/panel-permission.json"),
            include_str!("../../fixtures/panel-starting.json"),
            include_str!("../../fixtures/panel-starting-failed.json"),
            include_str!("../../fixtures/panel-empty.json"),
            include_str!("../../fixtures/panel-new-session.json"),
            include_str!("../../fixtures/panel-new-session-task.json"),
            include_str!("../../fixtures/panel-new-session-permission.json"),
            include_str!("../../fixtures/panel-new-session-loading.json"),
            include_str!("../../fixtures/panel-new-session-no-agents.json"),
            include_str!("../../fixtures/panel-assistant-home.json"),
            include_str!("../../fixtures/panel-assistant-signed-out.json"),
            include_str!("../../fixtures/panel-assistant-thread.json"),
            include_str!("../../fixtures/panel-assistant-thinking.json"),
            include_str!("../../fixtures/panel-assistant-error.json"),
            include_str!("../../fixtures/panel-assistant-recent.json"),
            include_str!("../../fixtures/panel-assistant-task.json"),
            include_str!("../../fixtures/panel-october.json"),
            include_str!("../../fixtures/panel-october-pending.json"),
            include_str!("../../fixtures/panel-october-email.json"),
            include_str!("../../fixtures/panel-october-pairing.json"),
            include_str!("../../fixtures/panel-october-connected.json"),
            include_str!("../../fixtures/panel-october-error.json"),
        ] {
            let mut fixture: crate::app::fixture::Fixture = serde_json::from_str(raw).unwrap();
            let (height, actions, _) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut fixture.view, Vec::new());
            assert!(height > 60.0 && height <= 900.0);
            assert!(actions.is_empty());
        }
    }

    #[test]
    fn card_status_is_trailing_aligned_and_new_precedes_the_state() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut fixture: crate::app::fixture::Fixture = serde_json::from_str(include_str!("../../fixtures/panel-inbox.json")).unwrap();
        let mut view = UiState::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 1032.0))),
                ..Default::default()
            },
            |ui| {
                render_panel(
                    ui,
                    &mut Ctx {
                        model: &fixture.model,
                        inputs: &mut fixture.inputs,
                        view: &mut view,
                        out: &mut Vec::new(),
                        assets: &assets,
                        env: &Env { now_ms: fixture.now_ms, ..Default::default() },
                    },
                    1032.0,
                );
            },
        );
        output.textures_delta.clear();
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some((&t.galley.job.text, egui::Rect::from_min_size(t.pos, t.galley.size()))),
                _ => None,
            })
            .collect();
        let state = texts.iter().find(|(text, _)| text.as_str() == "Your turn").unwrap().1;
        let new = texts.iter().find(|(text, _)| text.as_str() == "NEW").unwrap().1;
        let age = texts.iter().find(|(text, _)| text.as_str() == "38m").unwrap().1;
        assert!(new.right() < state.left());
        assert!((state.right() - 356.0).abs() < 0.1, "state edge: {}", state.right());
        assert!((age.right() - state.right()).abs() < 0.1);
        output.textures_delta.clear();
    }

    #[test]
    fn empty_and_short_chat_fit_content_and_preview_has_priority() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut inputs = Inputs::default();
        let mut view = UiState::default();
        let agent = Agent {
            id: "a".into(),
            kind: AgentKind::new("claude"),
            handle: "claude-1".into(),
            state: AgentState::Waiting,
            can_reply: true,
            ..Default::default()
        };
        let mut model = Model { panel: Some(PanelMode::Inbox), engine: EngineHealth::Ready, ..Default::default() };
        let (empty, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        assert!(empty < 200.0, "empty panel grew to {empty}");
        assert!(texts.iter().any(|t| t == "Nothing waiting on you"));
        model.agents.push(agent);
        model.chat = Some(Chat {
            agent_id: "a".into(),
            supported: true,
            messages: vec![ChatMessage { role: Role::Agent, text: "Ready.".into(), at: None }],
            ..Default::default()
        });
        inputs.drafts.address(Some("a"));
        let (chat, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        assert!(chat < 350.0, "short chat grew to {chat}");
        assert!(texts.iter().any(|t| t == "Ready."));
        model.launch_preview = Some(LaunchPreview {
            kind: AgentKind::new("codex"),
            prompt: "Find it.\n\n(Started from October Lantern on Linux.)".into(),
            folder: "/home/me".into(),
            error: None,
        });
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        assert!(texts.iter().any(|t| t == "Starting Codex…"));
        assert!(!texts.iter().any(|t| t == "Ready."));
        assert!(!texts.iter().any(|t| t.contains("Started from October Lantern")));
    }

    #[test]
    fn escape_goes_back_in_a_chat_instead_of_closing_the_panel() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let model = Model {
            panel: Some(PanelMode::Agents),
            engine: EngineHealth::Ready,
            chat: Some(Chat { agent_id: "a".into(), ..Default::default() }),
            ..Default::default()
        };
        let event =
            egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let (_, actions, _) = frame(&ctx, &assets, &model, &mut Inputs::default(), &mut UiState::default(), vec![event]);
        assert!(actions.contains(&UiAction::CloseChat));
        assert!(!actions.contains(&UiAction::SetPanel(None)));
    }

    #[test]
    fn permission_click_carries_the_displayed_prompt_identity() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let agent = Agent {
            id: "a".into(),
            kind: AgentKind::new("claude"),
            state: AgentState::NeedsInput,
            question_kind: Some("permission".into()),
            prompt_id: Some("observed-123".into()),
            route: Some(Route { via: "tmux".into(), ..Default::default() }),
            ..Default::default()
        };
        let model = Model::default();
        let mut inputs = Inputs::default();
        let mut view = UiState::default();
        let mut actions = Vec::new();
        fn find(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == "Yes" => Some(text.pos + text.galley.size() * 0.5),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(find),
                _ => None,
            }
        }
        let mut pos = egui::Pos2::ZERO;
        // Hit testing uses the previous frame's widgets. Lay them out before pressing.
        for pressed in [None, Some(true), Some(false)] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 300.0))),
                    events: if let Some(pressed) = pressed {
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                |ui| {
                    question::question_card(
                        ui,
                        &mut Ctx {
                            model: &model,
                            inputs: &mut inputs,
                            view: &mut view,
                            out: &mut actions,
                            assets: &assets,
                            env: &Env::default(),
                        },
                        &agent,
                        false,
                    );
                },
            );
            if pressed.is_none() {
                pos = output.shapes.iter().find_map(|s| find(&s.shape)).expect("Yes is shown");
            }
            output.textures_delta.clear();
        }
        assert_eq!(
            actions,
            vec![UiAction::AnswerQuestion {
                agent_id: "a".into(),
                prompt_id: "observed-123".into(),
                answer: crate::model::AgentAnswer::Yes
            }]
        );
    }

    #[test]
    fn nested_card_permission_button_does_not_open_the_chat() {
        fn find(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == "Yes" => Some(text.pos + text.galley.size() * 0.5),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(find),
                _ => None,
            }
        }
        let mut fixture: crate::app::fixture::Fixture = serde_json::from_str(include_str!("../../fixtures/panel-permission.json")).unwrap();
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut view = UiState::default();
        let mut out = Vec::new();
        let mut click = None;
        for step in 0..3 {
            let events = if step == 0 {
                Vec::new()
            } else {
                let pos = click.unwrap();
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: step == 1,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    render_panel(
                        ui,
                        &mut Ctx {
                            model: &fixture.model,
                            inputs: &mut fixture.inputs,
                            view: &mut view,
                            out: &mut out,
                            assets: &assets,
                            env: &Env::default(),
                        },
                        900.0,
                    );
                },
            );
            if step == 0 {
                click = output.shapes.iter().find_map(|s| find(&s.shape));
            }
            output.textures_delta.clear();
        }
        assert_eq!(
            out,
            vec![UiAction::AnswerQuestion {
                agent_id: "claude:1:1".into(),
                prompt_id: "fixture-prompt-1".into(),
                answer: crate::model::AgentAnswer::Yes
            }]
        );
    }

    #[test]
    fn a_card_link_returns_only_open_url_not_open_chat_or_a_platform_command() {
        fn find(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == "A PR #228 is ready." => {
                    Some(text.pos + text.galley.pos_from_cursor(egui::text::CCursor::new(4)).center().to_vec2() + egui::vec2(1.0, 0.0))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(find),
                _ => None,
            }
        }
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let model = Model {
            panel: Some(PanelMode::Inbox),
            engine: EngineHealth::Ready,
            agents: vec![Agent {
                id: "a".into(),
                handle: "codex-1".into(),
                state: AgentState::Waiting,
                last_message: Some("A [PR #228](https://example.com/pr/228) is ready.".into()),
                ..Default::default()
            }],
            inbox: crate::model::Inbox { earlier: vec!["a".into()], ..Default::default() },
            ..Default::default()
        };
        let mut inputs = Inputs::default();
        let mut view = UiState::default();
        let mut actions = Vec::new();
        let mut click = None;
        for step in 0..3 {
            let events = if step == 0 {
                Vec::new()
            } else {
                let pos = click.unwrap();
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: step == 1,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    render_panel(
                        ui,
                        &mut Ctx {
                            model: &model,
                            inputs: &mut inputs,
                            view: &mut view,
                            out: &mut actions,
                            assets: &assets,
                            env: &Env::default(),
                        },
                        900.0,
                    );
                },
            );
            if step == 0 {
                click = output.shapes.iter().find_map(|shape| find(&shape.shape));
                assert!(click.is_some());
            }
            assert!(!output.platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::OpenUrl(_))));
            output.textures_delta.clear();
        }
        assert_eq!(actions, vec![UiAction::OpenUrl("https://example.com/pr/228".into())]);
    }

    #[test]
    fn enter_sends_to_the_pinned_chat_without_clearing_the_draft_in_the_view() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut view = UiState::default();
        let mut fixture: crate::app::fixture::Fixture = serde_json::from_str(include_str!("../../fixtures/panel-chat.json")).unwrap();
        frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        let event =
            egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let (_, out, _) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, vec![event]);
        assert_eq!(out, vec![UiAction::SendReply { agent_id: "claude:1:1".into() }]);
        assert_eq!(fixture.inputs.drafts.text(), "Thank you!");
    }
}
