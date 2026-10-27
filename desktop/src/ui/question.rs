//! QuestionCard.swift: an agent's question, answered right here. A permission (Yes / Always / No),
//! Claude's own questions (their options, numbered as in the terminal) or its plan (start / keep
//! planning). With `keys`, pressing a number picks too, unless something is being typed in a box.
use super::{
    Ctx, UiAction,
    components::{self as c, MarkdownStyle},
    feed,
    icons::Icon,
    theme::{self as t, Weight},
};
use crate::model::{Agent, AgentAnswer};
use eframe::egui::{self, Align, Layout, Margin, Sense, Stroke, Ui};

pub fn question_card(ui: &mut Ui, cx: &mut Ctx, agent: &Agent, keys: bool) {
    let Some(prompt_id) = agent.prompt_id.clone() else {
        return;
    };
    let sent = cx.model.answered.get(&prompt_id).cloned();
    let delivery = cx.model.answer_delivery.get(&prompt_id).copied();
    // A text box has the keyboard (empty or not): plain digits are text there; Ctrl+1–9 still answer.
    let typing = ui.ctx().egui_wants_keyboard_input();
    let pressed = if keys && sent.is_none() { number_pressed(ui, typing) } else { None };
    let answer = egui::Frame::new()
        .fill(t::opacity(t::AMBER, 0.10))
        .stroke(Stroke::new(1.0, t::opacity(t::AMBER, 0.22)))
        .corner_radius(12)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            let answer = match agent.question_kind.as_deref() {
                Some("choice") => choice_questions(ui, cx, agent, &prompt_id, sent.as_ref(), pressed),
                Some("plan") => plan(ui, cx, agent, sent.as_ref(), pressed),
                _ => permission(ui, cx, agent, sent.as_ref(), pressed),
            };
            if sent.is_some() {
                // Sent only once the keys went in; until then, or when that's unclear, it says so.
                let (text, color) = match delivery {
                    Some(crate::model::AnswerDelivery::Sent) => ("Sent", t::GREEN),
                    Some(crate::model::AnswerDelivery::Unconfirmed) => ("Not sure it went in. Check its terminal.", t::AMBER),
                    _ => ("Sending…", t::MUTED),
                };
                c::label(ui, text, 11.0, Weight::Semibold, color);
            }
            answer
        })
        .inner;
    // Answered already: its options still take clicks (so they don't open the chat behind), but
    // nothing more is sent.
    if let Some(answer) = answer.filter(|_| sent.is_none()) {
        cx.view.question_picks.remove(&prompt_id);
        cx.emit(UiAction::AnswerQuestion { agent_id: agent.id.clone(), prompt_id, answer });
    }
}

fn permission(ui: &mut Ui, cx: &mut Ctx, agent: &Agent, sent: Option<&AgentAnswer>, pressed: Option<usize>) -> Option<AgentAnswer> {
    heading(ui, agent.question.as_deref().unwrap_or("Permission to continue?"), sent.is_some());
    feed::request_detail(ui, cx, agent);
    let rows = permission_rows(agent);
    let mut answer = pressed.and_then(|i| rows.get(i)).map(|row| row.2.clone());
    for (i, (label, detail, row)) in rows.into_iter().enumerate() {
        let id = ui.id().with(("permission", &agent.id, i));
        if option_row(ui, id, i + 1, &label, detail.as_deref(), sent == Some(&row), sent.is_some(), false).clicked() {
            answer = Some(row);
        }
    }
    answer
}

fn permission_rows(agent: &Agent) -> Vec<(String, Option<String>, AgentAnswer)> {
    let mut rows = vec![("Yes".to_string(), None, AgentAnswer::Yes)];
    if let Some(always) = agent.choices.as_ref().and_then(|c| c.always.clone()) {
        rows.push((always, None, AgentAnswer::Always));
    }
    rows.push(("No".into(), Some(format!("Then tell @{} what to do instead", agent.handle)), AgentAnswer::No));
    rows
}

fn plan(ui: &mut Ui, cx: &mut Ctx, agent: &Agent, sent: Option<&AgentAnswer>, pressed: Option<usize>) -> Option<AgentAnswer> {
    heading(ui, agent.question.as_deref().unwrap_or("Its plan is ready. Start on it?"), sent.is_some());
    if let Some(text) = agent.question_detail.as_deref().filter(|t| !t.is_empty()) {
        egui::Frame::new().fill(t::black(0.18)).corner_radius(8).inner_margin(8).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt(("plan", &agent.id)).max_height(180.0).min_scrolled_height(0.0).show(ui, |ui| {
                if let Some(link) = c::markdown_linked(ui, text, MarkdownStyle { max_rows: 400, ..Default::default() }, 0.0).1 {
                    cx.emit(UiAction::OpenUrl(link));
                }
            });
        });
    }
    let rows = [
        ("Yes, start on it".to_string(), None, AgentAnswer::Yes),
        ("No, keep planning".to_string(), Some(format!("Then tell @{} what to change", agent.handle)), AgentAnswer::No),
    ];
    let mut answer = pressed.and_then(|i| rows.get(i)).map(|row| row.2.clone());
    for (i, (label, detail, row)) in rows.into_iter().enumerate() {
        let id = ui.id().with(("plan-option", &agent.id, i));
        if option_row(ui, id, i + 1, &label, detail.as_deref(), sent == Some(&row), sent.is_some(), false).clicked() {
            answer = Some(row);
        }
    }
    answer
}

fn choice_questions(
    ui: &mut Ui,
    cx: &mut Ctx,
    agent: &Agent,
    prompt_id: &str,
    sent: Option<&AgentAnswer>,
    pressed: Option<usize>,
) -> Option<AgentAnswer> {
    let questions = agent.choices.as_ref().map(|c| c.questions.clone()).unwrap_or_default();
    let one_click = questions.len() == 1 && !questions[0].multi_select;
    let sent_picks = match sent {
        Some(AgentAnswer::Picks { picks }) => Some(picks.clone()),
        _ => None,
    };
    let mut answer = None;
    if one_click && let Some(i) = pressed.filter(|i| *i < questions[0].options.len()) {
        answer = Some(AgentAnswer::Picks { picks: vec![vec![i]] });
    }
    ui.spacing_mut().item_spacing.y = 6.0;
    for (qi, q) in questions.iter().enumerate() {
        if qi > 0 {
            ui.add_space(4.0);
        }
        if questions.len() > 1 || q.multi_select {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if questions.len() > 1
                    && let Some(header) = &q.header
                {
                    egui::Frame::new().fill(t::FAINT).corner_radius(8).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
                        c::label(ui, header, 10.0, Weight::Semibold, t::MUTED);
                    });
                }
                if q.multi_select {
                    c::label(ui, "Pick any", 10.0, Weight::Medium, t::MUTED);
                }
            });
        }
        heading(ui, &q.question, qi == 0 && sent.is_some());
        let picked: Vec<usize> = match &sent_picks {
            Some(picks) => picks.get(qi).cloned().unwrap_or_default(),
            None => cx.view.question_picks.get(prompt_id).and_then(|p| p.get(&qi)).map(|s| s.iter().copied().collect()).unwrap_or_default(),
        };
        for (oi, option) in q.options.iter().enumerate() {
            let id = ui.id().with(("choice", prompt_id, qi, oi));
            let row = option_row(
                ui,
                id,
                oi + 1,
                &option.label,
                option.description.as_deref(),
                picked.contains(&oi),
                sent.is_some(),
                q.multi_select,
            );
            if row.clicked() && sent.is_none() {
                if one_click {
                    answer = Some(AgentAnswer::Picks { picks: vec![vec![oi]] });
                } else {
                    let picks = cx.view.question_picks.entry(prompt_id.to_string()).or_default().entry(qi).or_default();
                    if q.multi_select {
                        if !picks.remove(&oi) {
                            picks.insert(oi);
                        }
                    } else {
                        *picks = [oi].into();
                    }
                }
            }
        }
    }
    if sent.is_none() {
        if !one_click {
            let picks = cx.view.question_picks.get(prompt_id);
            let complete = (0..questions.len()).all(|qi| picks.and_then(|p| p.get(&qi)).is_some_and(|s| !s.is_empty()));
            // Looks disabled until every question has an answer, but still takes the click.
            let send = ui
                .scope(|ui| {
                    if !complete {
                        ui.multiply_opacity(0.5);
                    }
                    c::amber_button(ui, "Send answers")
                })
                .inner;
            if send.clicked()
                && complete
                && let Some(picks) = picks
            {
                answer = Some(AgentAnswer::Picks {
                    picks: (0..questions.len()).map(|qi| picks.get(&qi).map(|s| s.iter().copied().collect()).unwrap_or_default()).collect(),
                });
            }
        }
        if c::text_button(ui, "Don't answer", 11.0, Weight::Medium, t::MUTED)
            .on_hover_text(format!("Close the question in the terminal; @{} is told you didn't answer", agent.handle))
            .clicked()
        {
            answer = Some(AgentAnswer::No);
        }
    }
    answer
}

/// The question; once answered, a check at its right (what became of the answer is said below).
fn heading(ui: &mut Ui, text: &str, sent: bool) {
    ui.horizontal_top(|ui| {
        let width = (ui.available_width() - if sent { 18.0 } else { 0.0 }).max(40.0);
        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_max_width(width);
            ui.add(egui::Label::new(c::text(text, 13.0, Weight::Medium, t::INK)).wrap().selectable(true));
        });
        if sent {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(11.0, 14.0), Sense::hover());
                c::paint_icon(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(11.0)), Icon::Check, t::GREEN);
            });
        }
    });
}

/// One option: its number (the key that picks it), its label and what it means.
#[allow(clippy::too_many_arguments)]
pub(super) fn option_row(
    ui: &mut Ui,
    id: egui::Id,
    number: usize,
    label: &str,
    detail: Option<&str>,
    chosen: bool,
    faded: bool,
    tick: bool,
) -> egui::Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let inner = ui.scope(|ui| {
        if faded && !chosen {
            ui.multiply_opacity(0.45);
        }
        egui::Frame::new()
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 9.0;
                    let (badge, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add_space(1.0);
                        ui.add(egui::Label::new(c::text(label, 12.5, Weight::Medium, t::INK)).wrap().selectable(false));
                        if let Some(detail) = detail.filter(|d| !d.is_empty()) {
                            ui.add(egui::Label::new(c::text(detail, 11.0, Weight::Regular, t::MUTED)).wrap().selectable(false));
                        }
                    });
                    badge
                })
                .inner
            })
            .inner
    });
    let badge = inner.inner;
    let rect = inner.response.rect;
    let response = ui.interact(rect, id, Sense::click());
    let (fill, stroke) = if chosen {
        (t::opacity(t::GREEN, 0.18), t::opacity(t::GREEN, 0.75))
    } else if response.hovered() && !faded {
        (t::white(0.09), t::HAIRLINE)
    } else {
        (t::white(0.05), t::HAIRLINE)
    };
    let fade = if faded && !chosen { 0.45 } else { 1.0 };
    ui.painter().set(background, egui::Shape::rect_filled(rect, 9, fill.gamma_multiply(fade)));
    ui.painter().rect_stroke(rect, 9, Stroke::new(1.0, stroke.gamma_multiply(fade)), egui::StrokeKind::Inside);
    let painter = ui.painter();
    if chosen {
        painter.rect_filled(badge, 5, t::GREEN);
        c::paint_icon(painter, egui::Rect::from_center_size(badge.center(), egui::Vec2::splat(11.0)), Icon::Check, t::black(0.8));
    } else {
        painter.rect_filled(badge, 5, t::white(0.10).gamma_multiply(fade));
        if tick {
            painter.rect_stroke(badge, 5, Stroke::new(1.0, t::STROKE), egui::StrokeKind::Inside);
        } else {
            let galley = painter.layout_no_wrap(number.to_string(), t::font(11.0, Weight::Semibold), t::white(0.8 * fade));
            painter.galley(badge.center() - galley.size() / 2.0, galley, t::white(0.8 * fade));
        }
    }
    if response.has_focus() {
        painter.rect_stroke(rect.expand(1.0), 10, Stroke::new(1.0, t::AMBER), egui::StrokeKind::Outside);
    }
    let name = if tick { label.to_string() } else { format!("{number}. {label}") };
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, !faded, chosen, &name));
    response
}

/// A number key pressed on its own: 0 for 1. The key and the digit it would type are both taken,
/// so an empty reply box doesn't get it too.
/// Ctrl+1–9 always; a plain digit only when no text box has the keyboard (`typing`).
pub(super) fn number_pressed(ui: &mut Ui, typing: bool) -> Option<usize> {
    const KEYS: [egui::Key; 9] = [
        egui::Key::Num1,
        egui::Key::Num2,
        egui::Key::Num3,
        egui::Key::Num4,
        egui::Key::Num5,
        egui::Key::Num6,
        egui::Key::Num7,
        egui::Key::Num8,
        egui::Key::Num9,
    ];
    if let Some(i) = KEYS.iter().position(|key| ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, *key))) {
        return Some(i);
    }
    if typing {
        return None;
    }
    let i = KEYS.iter().position(|key| c::consume_plain_key(ui, *key))?;
    let digit = (i + 1).to_string();
    ui.input_mut(|input| input.events.retain(|event| !matches!(event, egui::Event::Text(text) if *text == digit)));
    Some(i)
}
