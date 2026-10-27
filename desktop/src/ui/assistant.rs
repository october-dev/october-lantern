//! Assistant.swift: home (with Recent), conversation, the task cards, pending task and the large
//! AskBox.
use super::{
    Ctx, DictationOwner, UiAction, chat,
    components::{self as c, MarkdownStyle},
    icons::Icon,
    theme::{self as t, Weight},
};
use crate::model::{AgentKind, AgentState, AssistantTurn, ChatMessage, Conversation, HarnessState, Role, TaskLink};
use eframe::egui::{self, Align, Layout, Margin, Rect, Sense, Stroke, Ui};

pub fn render(ui: &mut Ui, cx: &mut Ctx, max_height: f32) {
    if cx.model.assistant.turns.is_empty() {
        home(ui, cx);
    } else {
        thread(ui, cx, max_height);
    }
    ask_box(ui, cx);
}

fn close(ui: &mut Ui, cx: &mut Ctx) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if c::icon_button(ui, "assistant-close", Icon::Close, 22.0, t::MUTED, false).on_hover_text("Close").clicked() {
            cx.emit(UiAction::SetPanel(None));
        }
    });
}

fn home(ui: &mut Ui, cx: &mut Ctx) {
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 16, bottom: 8 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            c::label(ui, "Ask Lantern", 17.0, Weight::Bold, t::INK);
            close(ui, cx);
        });
        if cx.model.account.signed_in.is_some() {
            c::label(ui, "Ask anything, or tell Lantern what to do and an agent gets on it.", 13.0, Weight::Regular, t::MUTED);
            ui.add_space(2.0);
            for (icon, title) in [(Icon::Wifi, "Fix the Wi-Fi."), (Icon::Video, "Join the call."), (Icon::Photo, "Find that photo.")] {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 37.0), Sense::click());
                ui.painter().rect_filled(rect, 11.0, t::FAINT);
                c::paint_icon(
                    ui.painter(),
                    Rect::from_center_size(egui::pos2(rect.left() + 23.0, rect.center().y), egui::Vec2::splat(14.0)),
                    icon,
                    t::AMBER,
                );
                let galley = ui.painter().layout_no_wrap(title.into(), t::font(14.0, Weight::Medium), t::white(0.9));
                ui.painter().galley(egui::pos2(rect.left() + 44.0, rect.center().y - galley.size().y / 2.0), galley, t::white(0.9));
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
                if response.clicked() {
                    cx.inputs.assistant = title.into();
                    cx.view.assistant_focus = None;
                }
            }
            if !cx.model.assistant.conversations.is_empty() {
                recent(ui, cx);
            }
        }
    });
}

/// Earlier chats: the last three, or all of them (scrolling) after Show all.
fn recent(ui: &mut Ui, cx: &mut Ctx) {
    let model = cx.model;
    let conversations = &model.assistant.conversations;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            c::label(ui, "Recent", 11.5, Weight::Semibold, t::MUTED);
            if conversations.len() > 3 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let title = if cx.view.assistant_show_all { "Show less" } else { "Show all" };
                    if c::plain_button(ui, title, 11.5, Weight::Medium, t::MUTED).clicked() {
                        cx.view.assistant_show_all = !cx.view.assistant_show_all;
                    }
                });
            }
        });
        ui.add_space(2.0);
        if cx.view.assistant_show_all {
            egui::ScrollArea::vertical()
                .id_salt("assistant-recent")
                .max_height(260.0)
                .min_scrolled_height(0.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for conversation in conversations {
                        conversation_row(ui, cx, conversation);
                    }
                });
        } else {
            for conversation in conversations.iter().take(3) {
                conversation_row(ui, cx, conversation);
            }
        }
    });
}

fn conversation_row(ui: &mut Ui, cx: &mut Ctx, conversation: &Conversation) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 8.0, t::FAINT);
    }
    let icon = if conversation.turns.iter().any(|turn| turn.task.is_some()) { Icon::PlayCircle } else { Icon::Bubble };
    c::paint_icon(
        ui.painter(),
        Rect::from_center_size(egui::pos2(rect.left() + 14.0, rect.center().y), egui::Vec2::splat(11.0)),
        icon,
        t::MUTED,
    );
    let when = crate::model::parse_iso8601(&conversation.updated).map_or(String::new(), |ms| ago(ms, cx.env.now_ms));
    let age = ui.painter().layout_no_wrap(when, t::font(11.0, Weight::Regular), t::MUTED);
    let age_left = rect.right() - 6.0 - age.size().x;
    ui.painter().galley(egui::pos2(age_left, rect.center().y - age.size().y / 2.0), age, t::MUTED);
    let mut title = egui::text::LayoutJob::single_section(
        conversation.title.clone(),
        egui::TextFormat { font_id: t::font(12.5, Weight::Regular), color: t::white(0.9), ..Default::default() },
    );
    title.wrap = egui::text::TextWrapping::truncate_at_width((age_left - 8.0 - (rect.left() + 30.0)).max(10.0));
    let title = ui.painter().layout_job(title);
    ui.painter().galley(egui::pos2(rect.left() + 30.0, rect.center().y - title.size().y / 2.0), title, t::white(0.9));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &conversation.title));
    if response.clicked() {
        cx.emit(UiAction::OpenConversation { id: conversation.id.clone() });
    }
    response.context_menu(|ui| {
        if ui.button("Delete").clicked() {
            cx.emit(UiAction::DeleteConversation { id: conversation.id.clone() });
            ui.close();
        }
    });
}

/// How long ago, as the Mac's numeric narrow relative date ("2m ago", "3h ago", "1d ago").
fn ago(since_ms: f64, now_ms: f64) -> String {
    let seconds = ((now_ms - since_ms) / 1000.0).max(0.0) as u64;
    let (n, unit) = match seconds {
        0..60 => (seconds, "s"),
        60..3600 => (seconds / 60, "m"),
        3600..86_400 => (seconds / 3600, "h"),
        86_400..604_800 => (seconds / 86_400, "d"),
        604_800..2_592_000 => (seconds / 604_800, "w"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "mo"),
        _ => (seconds / 31_536_000, "y"),
    };
    format!("{n}{unit} ago")
}

fn thread(ui: &mut Ui, cx: &mut Ctx, max_height: f32) {
    use std::hash::{Hash, Hasher};
    let anchor = cx.model.assistant.turns.last().map(|turn| {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        turn.answer.hash(&mut hash);
        cx.model.assistant.watched.as_ref().map(|watched| watched.messages.len()).hash(&mut hash);
        (turn.id, hash.finish())
    });
    let latest_task = cx.model.assistant.latest_task().map(|(turn, _)| turn);
    let scroll_to_last = anchor != cx.view.assistant_scroll_anchor;
    cx.view.assistant_scroll_anchor = anchor;
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 6 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.horizontal(|ui| {
            let galley = ui.painter().layout_no_wrap("New chat".into(), t::font(12.0, Weight::Semibold), t::MUTED);
            let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + 17.0, 22.0), Sense::click());
            c::paint_icon(
                ui.painter(),
                Rect::from_center_size(egui::pos2(rect.left() + 6.0, rect.center().y), egui::Vec2::splat(12.0)),
                Icon::Back,
                t::MUTED,
            );
            ui.painter().galley(egui::pos2(rect.left() + 17.0, rect.center().y - galley.size().y / 2.0), galley, t::MUTED);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New chat"));
            if response.on_hover_text("Start a new chat. This one stays in Recent.").clicked() {
                cx.emit(UiAction::ClearAssistant);
            }
            close(ui, cx);
        });
        egui::ScrollArea::vertical()
            .id_salt("assistant-thread")
            .max_height((max_height - 220.0).max(200.0))
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                egui::Frame::new().inner_margin(Margin::symmetric(0, 4)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 14.0;
                    let model = cx.model;
                    for turn in &model.assistant.turns {
                        ui.push_id(turn.id, |ui| render_turn(ui, cx, turn));
                        if let Some(link) = &turn.task {
                            ui.push_id(("task", turn.id), |ui| task_card(ui, cx, turn.id, link, latest_task == Some(turn.id)));
                        }
                        if scroll_to_last && anchor.is_some_and(|(id, _)| id == turn.id) {
                            ui.scroll_to_cursor(Some(Align::BOTTOM));
                        }
                    }
                });
            });
    });
}

fn render_turn(ui: &mut Ui, cx: &mut Ctx, turn: &AssistantTurn) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        question(ui, turn);
        if !turn.images.is_empty() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for path in &turn.images {
                    if thumbnail(ui, cx, path, 40.0).on_hover_text("Open this image").clicked() {
                        cx.emit(UiAction::OpenImage(path.clone()));
                    }
                }
            });
        }
        if turn.answer.is_empty() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                c::spinner(ui, 12.0, t::MUTED, cx.env);
                c::label(ui, "Thinking…", 12.0, Weight::Regular, t::MUTED);
            });
        } else {
            if let Some(link) = c::markdown_linked(
                ui,
                &turn.answer,
                MarkdownStyle { size: 12.5, color: answer_color(turn.failed, turn.upgrade.is_some()), ..Default::default() },
                2.0,
            )
            .1
            {
                cx.emit(UiAction::OpenUrl(link));
            }
        }
        if let Some(url) = &turn.upgrade {
            upgrade_button(ui, cx, url, "assistant");
        }
        if let Some(started) = turn.started.as_ref().filter(|_| turn.task.is_none()) {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 6.0, t::GREEN);
                c::paint_icon(ui.painter(), rect.shrink(3.0), Icon::Play, t::black(0.85));
                c::label(ui, started, 11.5, Weight::Medium, t::GREEN);
            });
        }
        if let Some(note) = &turn.note {
            c::label(ui, note, 11.0, Weight::Regular, t::MUTED);
        }
        if let Some(problem) = &turn.task_problem {
            c::label(ui, problem, 11.5, Weight::Regular, t::AMBER);
        }
    });
}

/// An answer's color: amber for a plan limit (not an error), red for a failure.
pub(super) fn answer_color(failed: bool, upgrade: bool) -> egui::Color32 {
    if upgrade {
        t::AMBER
    } else if failed {
        t::RED
    } else {
        t::white(0.92)
    }
}

/// Opens October's plans, under an answer that hit a plan limit (Swift `UpgradeButton`).
pub(super) fn upgrade_button(ui: &mut Ui, cx: &mut Ctx, url: &str, from: &'static str) {
    if c::secondary_button(ui, "Upgrade to October Pro ↗").on_hover_text("Opens October's plans in your browser").clicked() {
        cx.emit(UiAction::OpenUpgrade { url: url.to_string(), from });
    }
}

/// A long message (a pasted prompt) shows its first lines in normal weight, with Show more
/// (Swift `TurnView.long`).
fn question(ui: &mut Ui, turn: &AssistantTurn) {
    let long = turn.question.chars().count() > 180 || turn.question.split('\n').count() > 3;
    if !long {
        c::label(ui, &turn.question, 12.5, Weight::Semibold, t::INK);
        return;
    }
    let id = ui.make_persistent_id(("question-expanded", turn.id));
    let expanded = ui.data(|data| data.get_temp::<bool>(id).unwrap_or(false));
    let mut job = egui::text::LayoutJob::single_section(
        turn.question.clone(),
        egui::TextFormat { font_id: t::font(12.5, Weight::Regular), color: t::white(0.85), ..Default::default() },
    );
    job.wrap.max_width = ui.available_width();
    if !expanded {
        job.wrap.max_rows = 4;
    }
    ui.add(egui::Label::new(job));
    if c::plain_button(ui, if expanded { "Show less" } else { "Show more" }, 11.0, Weight::Semibold, t::MUTED).clicked() {
        ui.data_mut(|data| data.insert_temp(id, !expanded));
    }
}

/// The conversation a task card shows: the agent's live one for the latest task, else what it
/// last said, without your first message when it's the question above (Swift `TaskCard.messages`).
fn task_messages<'a>(link: &'a TaskLink, live: Option<&'a [ChatMessage]>) -> &'a [ChatMessage] {
    let all = live.unwrap_or(&link.transcript);
    if link.sent {
        return after_sent(&link.prompt, link.sent_at.as_deref(), all);
    }
    match all.first() {
        Some(first) if first.role == Role::User && chat::without_lantern_notes(&first.text).trim() == link.prompt => &all[1..],
        _ => all,
    }
}

/// What an agent it was sent to said after it (Swift `Assistant.after(sent:since:in:)`): from its
/// last message that is the one sent, or Lantern's nudge to read it on October Bus, made after
/// `since`. A nudge counts only with a time that shows it's this send's: every send's nudge has the
/// same words. Nothing until then.
fn after_sent<'a>(prompt: &str, since: Option<&str>, messages: &'a [ChatMessage]) -> &'a [ChatMessage] {
    use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
    let start: String = prompt.trim().chars().take(60).collect();
    let since = since.and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok());
    // A little slack for the two clocks (the agent's transcript and this app).
    let fresh = |m: &ChatMessage| -> Option<bool> {
        let at = OffsetDateTime::parse(m.at.as_deref()?, &Rfc3339).ok()?;
        Some(at >= since? - Duration::seconds(2))
    };
    let matches = |m: &ChatMessage| {
        if m.text.contains(&start) {
            return fresh(m).unwrap_or(true);
        }
        m.text.contains("message on October Bus") && fresh(m) == Some(true)
    };
    match messages.iter().rposition(|m| m.role == Role::User && matches(m)) {
        Some(i) => &messages[i + 1..],
        None => &[],
    }
}

/// An agent Ask Lantern started, inside the conversation (Swift `TaskCard`): Allow/Deny when it
/// asks, what it has said (live for the latest task, as last seen for earlier ones) and how it's
/// going in plain words. Which agent it is, and where, stays in a small footer with Open agent.
fn task_card(ui: &mut Ui, cx: &mut Ctx, turn: u64, link: &TaskLink, latest: bool) {
    let model = cx.model;
    let agent = model.task_agent(link);
    let live = model
        .assistant
        .watched
        .as_ref()
        .filter(|watched| latest && agent.is_some_and(|agent| agent.id == watched.agent_id) && !watched.messages.is_empty())
        .map(|watched| watched.messages.as_slice());
    let messages = task_messages(link, live);
    egui::Frame::new().fill(t::black(0.18)).stroke(Stroke::new(1.0, t::HAIRLINE)).corner_radius(12).inner_margin(Margin::same(10)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            for (key, message) in chat::keyed(messages) {
                chat::bubble(ui, cx, message, &format!("task-{turn}-{key}"));
            }
            if let Some(agent) = agent.filter(|agent| agent.is_answerable()) {
                super::question::question_card(ui, cx, agent, latest);
            }
            // Kept after the agent exits: its questions, progress and result.
            if let Some(launch) = agent.map(|a| a.id.as_str()).or(link.agent_id.as_deref()).and_then(|id| model.launch_for(id)) {
                super::task_cards::launch_cards(ui, cx, launch, latest && !agent.is_some_and(|a| a.is_answerable()));
            }
            if let Some(error) = &link.error {
                let what = if link.sent { "Couldn't send it" } else { "Couldn't start it" };
                c::label(ui, format!("{what}: {error}"), 11.5, Weight::Regular, t::RED);
                if latest && c::plain_button(ui, "Try again", 11.0, Weight::Semibold, t::INK).clicked() {
                    cx.emit(UiAction::RetryAssistantTask { turn });
                }
            } else if link.sign_in && agent.is_none_or(|agent| agent.state != AgentState::Working) {
                let text = format!(
                    "{} needs you to sign in first. It's open in a terminal window: sign in there and it starts on this.",
                    link.kind.display_name()
                );
                c::label(ui, text, 11.0, Weight::Regular, t::AMBER);
            } else if let Some(agent) = agent {
                // Only what Lantern can see: a finished turn or a closed agent isn't proof the task
                // was done, so nothing here claims it.
                let quiet = |ui: &mut Ui, text: &str| c::label(ui, text, 11.0, Weight::Regular, t::MUTED);
                match agent.state {
                    AgentState::Working => busy(ui, cx.env, "Working on it…"),
                    AgentState::NeedsInput if !agent.is_answerable() => {
                        c::label(ui, "Needs your answer. Reply below.", 11.0, Weight::Regular, t::AMBER);
                    }
                    AgentState::NeedsInput => {}
                    AgentState::Waiting => {
                        quiet(ui, "Finished its turn. Reply below if there's more.");
                    }
                    AgentState::Idle => {
                        quiet(ui, "Open, but not working on anything.");
                    }
                    AgentState::Unknown => {
                        quiet(ui, "Running.");
                    }
                }
            } else if link.in_terminal {
                c::label(ui, "Running in a Terminal window.", 11.0, Weight::Regular, t::MUTED);
            } else if link.agent_id.is_some() || !link.transcript.is_empty() {
                c::label(ui, "Closed. It isn't running anymore.", 11.0, Weight::Regular, t::MUTED);
            } else if latest
                && link.kind.as_str() == AgentKind::OCTOBER
                && model.harness.as_ref().is_some_and(|h| h.status == HarnessState::Installing)
            {
                busy(ui, cx.env, "Setting up October. The first time takes a few minutes…");
            } else if latest {
                busy(ui, cx.env, "Getting started…");
            }
            footer(ui, cx, link, agent);
        },
    );
}

/// Which agent and Open agent, small, under the conversation; the folder is in the tooltip.
fn footer(ui: &mut Ui, cx: &mut Ctx, link: &TaskLink, agent: Option<&crate::model::Agent>) {
    let folder = chat::folder_name(&link.folder, &cx.env.home);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        c::harness_logo(ui, cx.assets, &link.kind, 12.0);
        let name = match agent.filter(|_| link.sent) {
            Some(agent) => format!("@{}", agent.handle),
            None => link.kind.display_name().to_string(),
        };
        c::label(ui, name, 10.5, Weight::Regular, t::MUTED).on_hover_text(format!("{} in {folder}", link.kind.display_name()));
        if link.sent {
            c::label(ui, "·", 10.5, Weight::Regular, t::MUTED);
            c::label(ui, link.delivery.as_deref().unwrap_or("Sending…"), 10.5, Weight::Regular, t::MUTED);
        }
        if let Some(agent) = agent {
            c::label(ui, "·", 10.5, Weight::Regular, t::MUTED);
            if c::plain_button(ui, "Open agent ↗", 10.5, Weight::Regular, t::MUTED)
                .on_hover_text(format!("Open @{}'s full session", agent.handle))
                .clicked()
            {
                cx.emit(UiAction::OpenAgent { agent_id: agent.id.clone() });
            }
        }
    });
}

/// A spinner and what's going on.
fn busy(ui: &mut Ui, env: &super::Env, text: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        c::spinner(ui, 10.0, t::MUTED, env);
        c::label(ui, text, 11.0, Weight::Regular, t::MUTED);
    });
}

/// Small copies of the images you added, decoded once per path (missing files show a picture
/// icon).
#[derive(Default)]
pub struct Thumbnails(std::collections::HashMap<String, Option<egui::TextureHandle>>);

impl std::fmt::Debug for Thumbnails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Thumbnails").field(&self.0.len()).finish()
    }
}

impl Thumbnails {
    fn get(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
        self.0
            .entry(path.to_string())
            .or_insert_with(|| {
                let image = image::open(path).ok()?.thumbnail(160, 160).to_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                Some(ctx.load_texture(
                    format!("thumbnail-{path}"),
                    egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                    egui::TextureOptions::LINEAR,
                ))
            })
            .clone()
    }
}

/// A square preview of an image you added.
fn thumbnail(ui: &mut Ui, cx: &mut Ctx, path: &str, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), Sense::click());
    ui.painter().rect_filled(rect, 7.0, t::FAINT);
    match cx.view.thumbnails.get(ui.ctx(), path) {
        Some(texture) => {
            // Fill the square, cropped to its middle.
            let [w, h] = texture.size().map(|v| v as f32);
            let side = w.min(h);
            let uv = Rect::from_min_size(egui::pos2((w - side) / 2.0 / w, (h - side) / 2.0 / h), egui::vec2(side / w, side / h));
            egui::Image::new(&texture).uv(uv).corner_radius(7).paint_at(ui, rect);
        }
        None => c::paint_icon(ui.painter(), rect.shrink(size * 0.3), Icon::Photo, t::MUTED),
    }
    ui.painter().rect_stroke(rect, 7.0, Stroke::new(1.0, t::HAIRLINE), egui::StrokeKind::Inside);
    response
}

fn ask_box(ui: &mut Ui, cx: &mut Ctx) {
    egui::Frame::new().inner_margin(Margin { left: 12, right: 12, top: 4, bottom: 12 }).show(ui, |ui| {
        if cx.model.account.signed_in.is_none() {
            // Sign in right here; afterwards Lantern comes back to this box.
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                c::label(ui, "Sign in to ask Lantern", 13.0, Weight::Semibold, t::INK);
                c::label(
                    ui,
                    "Ask anything about your agents, or tell Lantern what to do. Free accounts get 20 questions a day.",
                    11.5,
                    Weight::Regular,
                    t::MUTED,
                );
                super::cards::account::render_with(ui, cx, false);
            });
            return;
        }
        let model = cx.model;
        // With the latest task's agent running, the box replies to it.
        let agent = model.assistant_agent();
        let replying = agent.filter(|_| !model.assistant.ask_lantern_instead);
        let id = ui.make_persistent_id("assistant-input");
        let focused = ui.memory(|mem| mem.has_focus(id));
        // Images dropped anywhere on Ask Lantern go with the next message.
        let (dropping, dropped) = ui.ctx().input(|i| {
            (
                !i.raw.hovered_files.is_empty(),
                i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>(),
            )
        });
        if !dropped.is_empty() {
            cx.emit(UiAction::AddAssistantImages(dropped));
        }
        let stroke = if dropping {
            Stroke::new(2.0, t::AMBER)
        } else {
            Stroke::new(1.0, if focused { t::opacity(t::AMBER, 0.5) } else { t::STROKE })
        };
        egui::Frame::new()
            .fill(t::black(0.3))
            .stroke(stroke)
            .corner_radius(12)
            .inner_margin(Margin { left: 13, right: 7, top: 10, bottom: 10 })
            .show(ui, |ui| {
                if !cx.model.assistant.attachments.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for path in cx.model.assistant.attachments.clone() {
                            let image = thumbnail(ui, cx, &path, 46.0);
                            let remove = Rect::from_center_size(image.rect.right_top() + egui::vec2(-3.0, 3.0), egui::Vec2::splat(16.0));
                            let response = ui.interact(remove, ui.make_persistent_id(("remove-image", &path)), Sense::click());
                            ui.painter().circle_filled(remove.center(), 8.0, t::black(0.75));
                            c::paint_icon(ui.painter(), remove.shrink(4.5), Icon::Close, t::INK);
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Remove this image"));
                            if response.on_hover_text("Remove this image").clicked() {
                                cx.emit(UiAction::RemoveAssistantImage(path.clone()));
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let width = ui.available_width() - 84.0;
                    let placeholder = if cx.model.dictating {
                        "Listening…".to_string()
                    } else if let Some(agent) = replying {
                        format!("Reply to @{}…", agent.handle)
                    } else if cx.model.assistant.turns.is_empty() {
                        "Ask Lantern, or tell it what to do".into()
                    } else {
                        "Ask a follow-up".into()
                    };
                    let edit = egui::ScrollArea::vertical()
                        .id_salt("assistant-input-scroll")
                        .max_height(6.0 * 18.0)
                        .min_scrolled_height(0.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut cx.inputs.assistant)
                                    .id(id)
                                    .font(t::font(15.0, Weight::Regular))
                                    .text_color(t::INK)
                                    .hint_text(c::text(placeholder, 15.0, Weight::Regular, t::MUTED))
                                    .frame(egui::Frame::NONE)
                                    .margin(0)
                                    .desired_width(width)
                                    .desired_rows(2)
                                    .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)),
                            )
                        })
                        .inner;
                    if cx.view.assistant_focus != Some(cx.model.compose_focus) {
                        edit.request_focus();
                        cx.view.assistant_focus = Some(cx.model.compose_focus);
                    }
                    let submit = edit.has_focus() && c::consume_plain_key(ui, egui::Key::Enter);
                    // Ctrl+V with a picture (and no text) on the clipboard: egui pastes only text and
                    // says nothing, so the keys are read from the system.
                    let pasting = edit.has_focus() && ui.input(|i| i.modifiers.command) && crate::os::paste_keys_down();
                    if pasting && !cx.view.paste_held && !ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))) {
                        cx.emit(UiAction::PasteAssistantImages);
                    }
                    cx.view.paste_held = pasting;
                    ui.allocate_ui_with_layout(egui::vec2(78.0, edit.rect.height()), Layout::bottom_up(Align::Min), |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if c::icon_button(ui, "assistant-images", Icon::Paperclip, 22.0, t::MUTED, false)
                                .on_hover_text("Add images (you can also paste or drop them here)")
                                .clicked()
                            {
                                cx.emit(UiAction::PickAssistantImages);
                            }
                            if c::icon_button(
                                ui,
                                "assistant-mic",
                                if cx.model.dictating { Icon::MicFill } else { Icon::Mic },
                                22.0,
                                if cx.model.dictating { t::RED } else { t::MUTED },
                                false,
                            )
                            .on_hover_text(if cx.model.dictating { "Stop listening" } else { "Speak" })
                            .clicked()
                            {
                                cx.emit(UiAction::ToggleDictation { owner: DictationOwner::Assistant });
                            }
                            // A reply can go while Lantern is still answering.
                            let enabled = (!cx.inputs.assistant.trim().is_empty() || !cx.model.assistant.attachments.is_empty())
                                && (replying.is_some() || !cx.model.assistant.asking);
                            let response = ui
                                .add_enabled_ui(enabled, |ui| {
                                    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(22.0), Sense::click());
                                    ui.painter().circle_filled(rect.center(), 11.0, t::opacity(t::AMBER, if enabled { 1.0 } else { 0.35 }));
                                    c::paint_icon(ui.painter(), rect.shrink(5.5), Icon::ArrowUp, t::black(0.85));
                                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Ask"));
                                    response
                                })
                                .inner
                                .on_hover_text("Ask (Return)");
                            if enabled && (submit || response.clicked()) {
                                cx.emit(UiAction::SubmitAssistant);
                            }
                        });
                    });
                });
            });
        if let Some(problem) = &cx.model.assistant.attach_problem {
            ui.add_space(5.0);
            c::label(ui, problem, 11.0, Weight::Regular, t::AMBER);
        }
        if let Some(agent) = agent {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let (title, icon) = if model.assistant.ask_lantern_instead {
                    (format!("Reply to @{} instead", agent.handle), Icon::Reply)
                } else {
                    ("Ask Lantern instead".to_string(), Icon::Sparkles)
                };
                if c::small_button(ui, &title, icon).clicked() {
                    cx.emit(UiAction::ToggleAskLanternInstead);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {

    #[test]
    fn an_older_nudge_doesnt_pass_for_a_new_send() {
        use crate::model::{ChatMessage, Role};
        let msg = |role, text: &str, at: Option<&str>| ChatMessage { role, text: text.into(), at: at.map(String::from) };
        let nudge = "I sent you a message on October Bus. Check your inbox (check_inbox) and do what it asks.";
        let sent = Some("2026-10-26T12:00:00Z");
        let old = [msg(Role::User, nudge, Some("2026-10-26T11:50:00Z")), msg(Role::Agent, "I finished OLD TASK.", None)];
        assert!(super::after_sent("New task", sent, &old).is_empty());
        let untimed = [msg(Role::User, nudge, None), msg(Role::Agent, "I finished OLD TASK.", None)];
        assert!(super::after_sent("New task", sent, &untimed).is_empty());
        let now = [msg(Role::User, nudge, Some("2026-10-26T12:00:01Z")), msg(Role::Agent, "On it.", None)];
        assert_eq!(super::after_sent("New task", sent, &now).len(), 1);
        let typed = [msg(Role::User, "New task", None), msg(Role::Agent, "On it.", None)];
        assert_eq!(super::after_sent("New task", sent, &typed).len(), 1);
    }
    use super::super::{Assets, Env, Provider, UiState};
    use super::*;
    use crate::model::{Inputs, Model};

    fn frame(
        ctx: &egui::Context,
        assets: &Assets,
        model: &Model,
        inputs: &mut Inputs,
        view: &mut UiState,
        events: Vec<egui::Event>,
    ) -> (f32, Vec<UiAction>, Vec<(String, Rect)>) {
        let mut height = 0.0;
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                height = super::super::render_panel(
                    ui,
                    &mut Ctx {
                        model,
                        inputs,
                        view,
                        assets,
                        out: &mut actions,
                        env: &Env { now_ms: 1_790_000_000_000.0, reduce_motion: true, home: "/home/me".into() },
                    },
                    900.0,
                );
            },
        );
        fn collect(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => texts.push((text.galley.job.text.clone(), Rect::from_min_size(text.pos, text.galley.size()))),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        output.textures_delta.clear();
        (height, actions, texts)
    }

    fn click(ctx: &egui::Context, assets: &Assets, model: &Model, inputs: &mut Inputs, view: &mut UiState, title: &str) -> Vec<UiAction> {
        let (_, _, texts) = frame(ctx, assets, model, inputs, view, Vec::new());
        let pos = texts.iter().find(|(text, _)| text == title).unwrap().1.center();
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(
                frame(
                    ctx,
                    assets,
                    model,
                    inputs,
                    view,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                    ],
                )
                .1,
            );
        }
        actions
    }

    #[test]
    fn suggestions_prefill_without_submitting_and_return_keeps_input_until_ack() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut fixture: crate::app::fixture::Fixture =
            serde_json::from_str(include_str!("../../fixtures/panel-assistant-home.json")).unwrap();
        let mut view = UiState::default();
        assert!(click(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, "Join the call.").is_empty());
        assert_eq!(fixture.inputs.assistant, "Join the call.");
        let (height, _, _) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        assert!(height > 250.0 && height < 450.0, "home height: {height}");
        let (_, actions, _) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, vec![enter()]);
        assert_eq!(actions, vec![UiAction::SubmitAssistant]);
        assert_eq!(fixture.inputs.assistant, "Join the call.");
        fixture.model.assistant.asking = true;
        assert!(frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, vec![enter()]).1.is_empty());
    }

    fn enter() -> egui::Event {
        egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    fn setup(name: &str) -> (egui::Context, Assets, crate::app::fixture::Fixture) {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let raw = match name {
            "task" => include_str!("../../fixtures/panel-assistant-task.json"),
            "recent" => include_str!("../../fixtures/panel-assistant-recent.json"),
            _ => unreachable!(),
        };
        (ctx, assets, serde_json::from_str(raw).unwrap())
    }

    #[test]
    fn the_card_drops_the_first_message_when_it_is_the_question() {
        let message = |role, text: &str| ChatMessage { role, text: text.into(), at: None };
        let link = TaskLink {
            prompt: "Find me my thesis document".into(),
            transcript: vec![
                message(Role::User, "Find me my thesis document\n\n(Started from October Lantern on Linux.)"),
                message(Role::Agent, "Found it."),
            ],
            ..Default::default()
        };
        assert_eq!(task_messages(&link, None), &link.transcript[1..]);
        let live = [message(Role::User, "Something else"), message(Role::Agent, "Sure.")];
        assert_eq!(task_messages(&link, Some(&live[..])), &live[..], "a different first message stays");
        assert!(task_messages(&TaskLink::default(), None).is_empty());
    }

    #[test]
    fn a_finished_task_shows_its_saved_messages_in_the_conversation() {
        let (ctx, assets, mut fixture) = setup("task");
        let mut view = UiState::default();
        let (height, actions, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        let has = |value: &str| texts.iter().any(|(text, _)| text == value);
        assert!(actions.is_empty() && height < 900.0);
        assert!(has("Claude Code") && !has("Home folder"), "the agent's name stays small; the folder is in the tooltip");
        assert!(
            has("Closed. It isn't running anymore.") && !texts.iter().any(|(text, _)| text.starts_with("Done")),
            "a closed agent isn't claimed done"
        );
        assert_eq!(texts.iter().filter(|(text, _)| text == "Find me my thesis document").count(), 1, "the question shows once");
        assert!(texts.iter().any(|(text, _)| text.contains("Thesis-final.pdf")));
        assert!(!has("Started Claude Code in me"), "the card replaces the green line");
        assert!(!has("Ask Lantern instead") && has("Ask a follow-up"));
        assert!(!has("Open agent ↗"), "nothing to open once it has ended");
    }

    #[test]
    fn a_task_card_says_only_what_lantern_can_see() {
        let (ctx, assets, mut fixture) = setup("task");
        let cases = [
            (AgentState::Working, "Working on it…"),
            (AgentState::Waiting, "Finished its turn. Reply below if there's more."),
            (AgentState::Idle, "Open, but not working on anything."),
            (AgentState::Unknown, "Running."),
            (AgentState::NeedsInput, "Needs your answer. Reply below."),
        ];
        for (state, expected) in cases {
            fixture.model.agents = vec![crate::model::Agent {
                id: "demo".into(),
                kind: crate::model::AgentKind::new("claude"),
                handle: "claude-1".into(),
                state,
                ..Default::default()
            }];
            let mut view = UiState::default();
            let (_, _, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
            assert!(texts.iter().any(|(text, _)| text == expected), "{state:?} shows {expected:?}");
            assert!(!texts.iter().any(|(text, _)| text.starts_with("Done")), "{state:?} never claims the task is done");
        }
    }

    #[test]
    fn with_its_agent_running_the_box_replies_to_it_unless_asked_to_ask_lantern() {
        let (ctx, assets, mut fixture) = setup("task");
        fixture.model.agents = vec![crate::model::Agent {
            id: "demo".into(),
            kind: crate::model::AgentKind::new("claude"),
            handle: "claude-1".into(),
            state: AgentState::Working,
            can_reply: true,
            ..Default::default()
        }];
        fixture.model.assistant.asking = true;
        fixture.inputs.assistant = "Open it".into();
        let mut view = UiState::default();
        let (_, _, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        let has = |texts: &Vec<(String, Rect)>, value: &str| texts.iter().any(|(text, _)| text == value);
        assert!(has(&texts, "Claude Code") && has(&texts, "Working on it…") && has(&texts, "Open agent ↗"));
        assert!(!has(&texts, "@claude-1") && !has(&texts, "Closed. It isn't running anymore."), "no agent header; it's still working");
        assert!(has(&texts, "Ask Lantern instead"));
        let (_, actions, _) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, vec![enter()]);
        assert_eq!(actions, vec![UiAction::SubmitAssistant], "a reply goes while Lantern is answering");
        assert_eq!(
            click(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, "Ask Lantern instead"),
            vec![UiAction::ToggleAskLanternInstead]
        );
        fixture.model.assistant.ask_lantern_instead = true;
        let (_, actions, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, vec![enter()]);
        assert!(actions.is_empty(), "a question waits for Lantern's answer");
        assert!(has(&texts, "Reply to @claude-1 instead"));
        fixture.inputs.assistant.clear();
        fixture.model.assistant.ask_lantern_instead = false;
        let (_, _, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        assert!(has(&texts, "Reply to @claude-1…"));
    }

    #[test]
    fn recent_chats_open_and_show_all_when_there_are_more_than_three() {
        let (ctx, assets, mut fixture) = setup("recent");
        let mut view = UiState::default();
        let (_, _, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        let has = |texts: &Vec<(String, Rect)>, value: &str| texts.iter().any(|(text, _)| text == value);
        assert!(has(&texts, "Recent") && has(&texts, "2m ago") && has(&texts, "1h ago") && has(&texts, "1d ago"));
        assert!(!has(&texts, "Show all"));
        assert_eq!(
            click(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, "Which agent needs me first?"),
            vec![UiAction::OpenConversation { id: "c2".into() }]
        );
        let mut older = fixture.model.assistant.conversations[1].clone();
        older.id = "c4".into();
        older.title = "Join the standup".into();
        fixture.model.assistant.conversations.push(older);
        let (_, _, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        assert!(has(&texts, "Show all") && !has(&texts, "Join the standup"));
        assert!(click(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, "Show all").is_empty());
        assert!(view.assistant_show_all);
        let (_, _, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        assert!(has(&texts, "Show less") && has(&texts, "Join the standup"));
    }

    #[test]
    fn relative_times_read_like_the_mac() {
        let now = 1_790_000_000_000.0;
        assert_eq!(ago(now - 5_000.0, now), "5s ago");
        assert_eq!(ago(now - 120_000.0, now), "2m ago");
        assert_eq!(ago(now - 3.0 * 3_600_000.0, now), "3h ago");
        assert_eq!(ago(now - 86_400_000.0 * 9.0, now), "1w ago");
    }

    #[test]
    fn signed_out_ask_lantern_signs_in_right_there() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut fixture: crate::app::fixture::Fixture =
            serde_json::from_str(include_str!("../../fixtures/panel-assistant-signed-out.json")).unwrap();
        let mut view = UiState::default();
        let (_, actions, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        let has = |value: &str| texts.iter().any(|(text, _)| text == value);
        assert!(actions.is_empty());
        assert!(has("Sign in to ask Lantern"));
        assert!(has("Ask anything about your agents, or tell Lantern what to do. Free accounts get 20 questions a day."));
        assert!(!has("Connect to October"), "the card's own header stays in the October panel");
        assert!(!has("Sign in to October to ask Lantern anything"));
        for (title, provider) in [("Google", Provider::Google), ("GitHub", Provider::GitHub), ("Apple", Provider::Apple)] {
            let actions = click(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, title);
            assert_eq!(actions, vec![UiAction::SignInProvider(provider)], "{title}");
        }
        assert!(click(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, "Use email and password").is_empty());
        assert!(view.account_use_email, "the email form opens in place");
    }

    /// Clicks the control named `label` (by its accessibility name: icon buttons have no text).
    fn click_control(
        ctx: &egui::Context,
        assets: &Assets,
        model: &Model,
        inputs: &mut Inputs,
        view: &mut UiState,
        label: &str,
    ) -> Vec<UiAction> {
        ctx.enable_accesskit();
        let mut tree = None;
        let mut out = Vec::new();
        let run = |events: Vec<egui::Event>, inputs: &mut Inputs, view: &mut UiState, out: &mut Vec<UiAction>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let env = Env { now_ms: 1_790_000_000_000.0, reduce_motion: true, home: "/home/me".into() };
                    super::super::render_panel(ui, &mut Ctx { model, inputs, view, assets, out, env: &env }, 900.0);
                },
            );
            output.textures_delta.clear();
            output
        };
        if let Some(update) = run(Vec::new(), inputs, view, &mut out).platform_output.accesskit_update {
            tree = Some(update);
        }
        let tree = tree.expect("egui emits an accessibility tree when enabled");
        let bounds = tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .and_then(|(_, node)| node.bounds())
            .unwrap_or_else(|| panic!("no control named {label}"));
        let pos = egui::pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32);
        out.clear();
        for pressed in [true, false] {
            run(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                ],
                inputs,
                view,
                &mut out,
            );
        }
        out
    }

    #[test]
    fn images_can_be_added_and_removed_before_sending() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut fixture: crate::app::fixture::Fixture =
            serde_json::from_str(include_str!("../../fixtures/panel-assistant-thread.json")).unwrap();
        let mut view = UiState::default();
        let (model, inputs) = (&mut fixture.model, &mut fixture.inputs);
        assert_eq!(click_control(&ctx, &assets, model, inputs, &mut view, "Paperclip"), vec![UiAction::PickAssistantImages]);
        model.assistant.attachments = vec!["/nowhere/a.png".into()];
        inputs.assistant.clear();
        assert_eq!(
            click_control(&ctx, &assets, model, inputs, &mut view, "Remove this image"),
            vec![UiAction::RemoveAssistantImage("/nowhere/a.png".into())]
        );
        // An image alone can be sent.
        assert_eq!(click_control(&ctx, &assets, model, inputs, &mut view, "Ask"), vec![UiAction::SubmitAssistant]);
    }

    #[test]
    fn a_task_about_to_start_shows_no_countdown() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut fixture: crate::app::fixture::Fixture =
            serde_json::from_str(include_str!("../../fixtures/panel-assistant-thread.json")).unwrap();
        let mut view = UiState::default();
        let (_, _, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, Vec::new());
        assert!(fixture.model.assistant.pending_task.is_some(), "the fixture has a task about to start");
        assert!(!texts.iter().any(|(text, _)| text.starts_with("Starting in") || text == "Cancel" || text == "Start now"));
        assert_eq!(click(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut view, "New chat"), vec![UiAction::ClearAssistant]);
    }
}
