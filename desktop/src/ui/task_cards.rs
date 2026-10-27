//! Cards.swift: a session's cards (engine `cards`): its status and steps, what it's waiting for
//! you on (a question, a step only you can do), and its result, kept after the agent exits.
use super::{
    Ctx, UiAction,
    components::{self as c, MarkdownStyle},
    icons::Icon,
    question,
    theme::{self as t, Weight},
};
use crate::model::{Card, CardAnswer, CardLaunch};
use eframe::egui::{self, Stroke, Ui};

/// A session's cards. With `keys`, a waiting question can explicitly claim number-key focus.
pub fn launch_cards(ui: &mut Ui, cx: &mut Ctx, launch: &CardLaunch, keys: bool) {
    ui.spacing_mut().item_spacing.y = 8.0;
    if launch.ended_at.is_none() {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !cx.model.control_sending,
                    egui::Button::new(if cx.model.control_held { "Continue app access" } else { "Pause app access" }).small(),
                )
                .clicked()
            {
                cx.emit(UiAction::Control { action: if cx.model.control_held { "continue" } else { "pause" }.into(), launch: None });
            }
            if ui.add_enabled(!cx.model.control_sending, egui::Button::new("Stop session").small()).clicked() {
                cx.emit(UiAction::Control { action: "stop".into(), launch: Some(launch.id.clone()) });
            }
        });
        ui.small(
            cx.model
                .control_message
                .as_deref()
                .unwrap_or("App access controls affect all Lantern-owned agents. Background commands are not isolated."),
        );
    }

    if launch.ended_at.is_none()
        && let Some(status) = &launch.status
    {
        c::label(ui, format!("… {status}"), 12.0, Weight::Regular, t::white(0.85));
    }
    for step in &launch.steps {
        let (mark, color) = if step.done { ("✓", t::GREEN) } else { ("○", t::MUTED) };
        c::label(ui, format!("{mark} {}", step.title), 12.0, Weight::Regular, color);
    }
    // The last couple of closed questions, what's waiting, then the latest result.
    let closed: Vec<&Card> = launch.cards.iter().filter(|card| card.kind != "result" && !card.is_waiting()).collect();
    let first_waiting = launch.waiting().next().map(|card| card.id.clone());
    let shown = closed[closed.len().saturating_sub(2)..]
        .iter()
        .copied()
        .chain(launch.waiting())
        .chain(launch.cards.iter().rev().find(|card| card.kind == "result"));
    for card in shown.collect::<Vec<_>>() {
        match card.kind.as_str() {
            "routine" => {
                if let Some(draft) = &card.routine {
                    ui.label(draft["name"].as_str().unwrap_or("Suggested routine"));
                    if ui.button("Review routine…").clicked() {
                        cx.emit(UiAction::ReviewRoutine(draft.clone()));
                    }
                    ui.small("Nothing is scheduled until you confirm.");
                }
            }
            "result" => result_card(ui, cx, card),
            "handOff" => hand_off_card(ui, cx, card),
            _ => ask_card(ui, cx, card, keys && first_waiting.as_deref() == Some(card.id.as_str())),
        }
    }
}

fn boxed<R>(ui: &mut Ui, waiting: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let (fill, stroke) = if waiting { (t::opacity(t::AMBER, 0.10), t::opacity(t::AMBER, 0.22)) } else { (t::white(0.07), t::HAIRLINE) };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(12)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner
}

/// Where an answered or closed card stands, in words (Swift `CardStateLabel`).
fn state_label(ui: &mut Ui, cx: &Ctx, card: &Card) {
    let (text, color) = match card.state.as_str() {
        "waiting" if cx.model.card_sending.contains(&card.id) => ("Sending…", t::MUTED),
        "answered" => ("Giving it to the agent…", t::MUTED),
        "delivered" => ("Sent", t::GREEN),
        "unconfirmed" => ("Not sure the agent got it: it stopped waiting before it took your answer", t::AMBER),
        "expired" => ("Closed: nobody answered in time", t::MUTED),
        "stopped" => ("Closed: the agent stopped waiting", t::MUTED),
        "cancelled" => ("Closed by the agent", t::MUTED),
        _ => return,
    };
    c::label(ui, text, 11.0, Weight::Semibold, color);
}

fn error(ui: &mut Ui, cx: &Ctx, card: &Card) {
    if let Some(error) = cx.model.card_errors.get(&card.id) {
        c::label(ui, error, 11.0, Weight::Regular, t::RED);
    }
}

fn answer(cx: &mut Ctx, card: &Card, answer: CardAnswer) {
    cx.emit(UiAction::AnswerCard { card_id: card.id.clone(), revision: card.revision, answer });
}

/// A question from the agent: its options, or your own words.
fn ask_card(ui: &mut Ui, cx: &mut Ctx, card: &Card, keys: bool) {
    let open = card.is_waiting() && !cx.model.card_sending.contains(&card.id);
    let options = card.options.clone().unwrap_or_default();
    let focused = keys && open && !options.is_empty() && question::answer_focus(ui, &format!("{}:{}", card.id, card.revision));
    let pressed = if focused { question::number_pressed(ui) } else { None };
    boxed(ui, card.is_waiting(), |ui| {
        ui.add(egui::Label::new(c::text(card.question.clone().unwrap_or_default(), 13.0, Weight::Medium, t::INK)).wrap().selectable(true));
        if let Some(details) = &card.details {
            ui.add(egui::Label::new(c::text(details, 11.5, Weight::Regular, t::white(0.8))).wrap().selectable(true));
        }
        let mut chosen = pressed.and_then(|i| options.get(i)).cloned();
        for (i, option) in options.iter().enumerate() {
            let picked = card.answer.as_ref().and_then(|a| a.choice.as_deref()) == Some(option.as_str());
            let id = ui.id().with(("card-option", &card.id, i));
            if question::option_row(ui, id, i + 1, option, None, picked, !open, false).clicked() && open {
                chosen = Some(option.clone());
            }
        }
        if let Some(choice) = chosen.filter(|_| open) {
            answer(cx, card, CardAnswer { choice: Some(choice), ..Default::default() });
        }
        if let Some(text) = card.answer.as_ref().and_then(|a| a.text.as_deref()) {
            c::label(ui, format!("You answered: {text}"), 12.0, Weight::Regular, t::white(0.8));
        } else if card.is_waiting() {
            let hint = if options.is_empty() { "Your answer" } else { "Or answer in your own words" };
            let mut words = cx.view.card_words.get(&card.id).cloned().unwrap_or_default();
            let mut send = false;
            ui.horizontal(|ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut words).hint_text(hint).desired_width(ui.available_width() - 70.0));
                send = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                send |= c::amber_button(ui, "Send").clicked();
            });
            let text = words.trim().to_string();
            cx.view.card_words.insert(card.id.clone(), words);
            if send && open && !text.is_empty() {
                cx.view.card_words.remove(&card.id);
                answer(cx, card, CardAnswer { text: Some(text), ..Default::default() });
            }
        }
        state_label(ui, cx, card);
        error(ui, cx, card);
    });
}

/// Lantern's own words for why a step is yours; the agent's reason is shown as its own.
fn rule(reason: Option<&str>) -> &'static str {
    match reason {
        Some("signIn") => "Sign in yourself, in your own browser. Never send your password to the agent.",
        Some("verification") => "Only you can do this check. Never send codes to the agent.",
        Some("payment") => "Only you can approve a payment.",
        Some("identity") => "Only you can prove who you are.",
        Some("physical") => "This needs you in person.",
        _ => "Only you can do this step.",
    }
}

/// A step only you can do: what, why, where, and whether you did it.
fn hand_off_card(ui: &mut Ui, cx: &mut Ctx, card: &Card) {
    let open = card.is_waiting() && !cx.model.card_sending.contains(&card.id);
    boxed(ui, card.is_waiting(), |ui| {
        let step = card.step.clone().unwrap_or_default();
        ui.add(egui::Label::new(c::text(format!("Your turn: {step}"), 13.0, Weight::Medium, t::INK)).wrap().selectable(true));
        if card.guide && ui.button("Show me").clicked() {
            cx.emit(UiAction::StartShow(step.clone()));
        }
        if card.reason.as_deref() == Some("signIn") {
            ui.small("This card does not pause other work or provide protected sign-in.");
        }
        c::label(ui, rule(card.reason.as_deref()), 11.0, Weight::Regular, t::MUTED);
        if let Some(why) = &card.why {
            ui.add(
                egui::Label::new(c::text(format!("The agent says: {why}"), 12.0, Weight::Regular, t::white(0.8))).wrap().selectable(true),
            );
        }
        for (i, line) in card.instructions.iter().flatten().enumerate() {
            ui.add(egui::Label::new(c::text(format!("{}. {line}", i + 1), 12.0, Weight::Regular, t::white(0.85))).wrap().selectable(true));
        }
        // Never opened by itself: the person sees the site first.
        if let Some(url) = &card.url
            && let Ok(parsed) = url::Url::parse(url)
            && let Some(host) = parsed.host_str()
            && c::secondary_button(ui, &format!("Open {host}")).on_hover_text(url).clicked()
        {
            cx.emit(UiAction::OpenUrl(url.clone()));
        }
        match card.answer.as_ref().and_then(|a| a.done) {
            Some(done) => {
                let said = if done { "You said you did it." } else { "You said you can't right now." };
                c::label(ui, said, 12.0, Weight::Regular, t::white(0.8));
            }
            None if card.is_waiting() => {
                let mut note = cx.view.card_words.get(&card.id).cloned().unwrap_or_default();
                ui.add(egui::TextEdit::singleline(&mut note).hint_text("Add a note (optional)").desired_width(ui.available_width()));
                let text = note.trim().to_string();
                cx.view.card_words.insert(card.id.clone(), note);
                let mut done = None;
                ui.horizontal(|ui| {
                    if c::amber_button(ui, "I've done it").clicked() {
                        done = Some(true);
                    }
                    if c::secondary_button(ui, "I can't right now").clicked() {
                        done = Some(false);
                    }
                });
                if let Some(done) = done.filter(|_| open) {
                    cx.view.card_words.remove(&card.id);
                    answer(cx, card, CardAnswer { done: Some(done), text: (!text.is_empty()).then_some(text), ..Default::default() });
                }
            }
            None => {}
        }
        state_label(ui, cx, card);
        error(ui, cx, card);
    });
}

/// What the agent reports it did, with its files. Documents open; folders, programs and scripts
/// are shown in the file manager instead of being run.
fn result_card(ui: &mut Ui, cx: &mut Ctx, card: &Card) {
    boxed(ui, false, |ui| {
        let (outcome, color) = match card.outcome.as_deref() {
            Some("partial") => ("Partly done", t::AMBER),
            Some("failed") => ("Didn't work", t::RED),
            _ => ("Says it's done", t::GREEN),
        };
        ui.horizontal(|ui| {
            c::label(ui, outcome, 11.0, Weight::Semibold, color);
            c::label(ui, "· reported by the agent", 11.0, Weight::Regular, t::MUTED);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if c::small_button(ui, "Copy", Icon::Copy).clicked() {
                    cx.emit(UiAction::Copy(card.summary.clone().unwrap_or_default()));
                }
            });
        });
        if let Some(link) = c::markdown_linked(ui, card.summary.as_deref().unwrap_or_default(), MarkdownStyle::default(), 0.0).1 {
            cx.emit(UiAction::OpenUrl(link));
        }
        for file in card.files.iter().flatten() {
            ui.horizontal(|ui| {
                c::label(ui, if file.folder { "📁" } else { "📄" }, 12.0, Weight::Regular, t::MUTED);
                ui.add(egui::Label::new(c::text(&file.name, 12.0, Weight::Regular, t::INK)).truncate()).on_hover_text(&file.path);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !std::path::Path::new(&file.path).exists() {
                        c::label(ui, "Moved or deleted", 11.0, Weight::Regular, t::MUTED);
                        return;
                    }
                    if c::text_button(ui, "Show", 11.0, Weight::Medium, t::AMBER).clicked() {
                        cx.emit(UiAction::RevealFile(file.path.clone()));
                    }
                    if !file.folder && !runnable(&file.path) && c::text_button(ui, "Open", 11.0, Weight::Medium, t::AMBER).clicked() {
                        cx.emit(UiAction::OpenUrl(file.path.clone()));
                    }
                });
            });
        }
    });
}

/// Programs and scripts, which are shown rather than run.
fn runnable(path: &str) -> bool {
    let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let listed = [
        "exe", "msi", "bat", "cmd", "com", "ps1", "vbs", "js", "jse", "wsf", "scr", "lnk", "sh", "run", "appimage", "desktop", "bin", "py",
    ]
    .contains(&ext.as_str());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let executable = std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        listed || executable
    }
    #[cfg(not(unix))]
    listed
}
