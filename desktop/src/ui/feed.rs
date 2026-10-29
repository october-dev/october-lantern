//! PanelView.swift: Waiting / Agents, cards, permission affordances and engine banner.
use super::{
    Ctx, UiAction,
    components::{self as c, MarkdownStyle},
    icons::Icon,
    theme::{self as t, Weight},
};
use crate::model::{Agent, EngineHealth, PanelMode};
use eframe::egui::{self, Align, Layout, Margin, Sense, Stroke, Ui};

pub fn engine_banner(ui: &mut Ui, cx: &mut Ctx) {
    let (body, icon, color) = match &cx.model.engine {
        EngineHealth::Ready => return,
        EngineHealth::Starting => ("Starting…", Icon::Hourglass, t::MUTED),
        EngineHealth::Restarting => ("Lantern's engine stopped. Restarting…", Icon::Warning, t::AMBER),
        EngineHealth::Failed(message) => (message.as_str(), Icon::Warning, t::AMBER),
    };
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 0 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
            c::paint_icon(ui.painter(), rect, icon, color);
            c::label(ui, body, 11.5, Weight::Medium, color);
        });
    });
}

pub fn render(ui: &mut Ui, cx: &mut Ctx, max_height: f32) {
    header(ui, cx);
    egui::ScrollArea::vertical()
        .id_salt("agent-feed")
        .max_height((max_height - 200.0).max(240.0))
        .min_scrolled_height(0.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 8 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.set_width(ui.available_width());
                if cx.model.panel == Some(PanelMode::Agents) {
                    if cx.model.agents.is_empty() {
                        empty(ui, "No agents running", "Start Claude Code, Codex, OpenCode, Pi, Gemini or another agent in any terminal.");
                        connect_phone(ui, cx);
                    }
                    for agent in &cx.model.agents {
                        row(ui, cx, agent);
                    }
                    ended(ui, cx);
                } else {
                    if cx.model.inbox.new.is_empty() && cx.model.inbox.earlier.is_empty() {
                        let detail = if cx.model.agents.is_empty() {
                            "No agents running.".into()
                        } else {
                            format!("{} agent{} running.", cx.model.agents.len(), if cx.model.agents.len() == 1 { "" } else { "s" })
                        };
                        empty(ui, "Nothing waiting on you", &detail);
                        connect_phone(ui, cx);
                    }
                    for id in &cx.model.inbox.new {
                        if let Some(agent) = cx.model.agent(id) {
                            card(ui, cx, agent, true);
                        }
                    }
                    if !cx.model.inbox.earlier.is_empty() {
                        egui::Frame::new()
                            .inner_margin(Margin { left: 4, right: 4, top: if cx.model.inbox.new.is_empty() { 0 } else { 6 }, bottom: 0 })
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    tracking_label(ui, "EARLIER", 10.5, t::MUTED);
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if c::text_button(ui, "Clear all", 11.0, Weight::Medium, t::MUTED)
                                            .on_hover_text("Dismiss all of these until they have something new")
                                            .clicked()
                                        {
                                            cx.emit(UiAction::DismissTurns {
                                                turn_keys: cx
                                                    .model
                                                    .inbox
                                                    .earlier
                                                    .iter()
                                                    .filter_map(|id| cx.model.agent(id))
                                                    .map(Agent::turn_key)
                                                    .collect(),
                                            });
                                        }
                                    });
                                });
                            });
                        for id in &cx.model.inbox.earlier {
                            if let Some(agent) = cx.model.agent(id) {
                                card(ui, cx, agent, false);
                            }
                        }
                    }
                }
            });
        });
}

fn header(ui: &mut Ui, cx: &mut Ctx) {
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 10 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            tab(
                ui,
                cx,
                "WAITING",
                cx.model.inbox.new.len() + cx.model.inbox.earlier.len(),
                cx.model.panel != Some(PanelMode::Agents),
                PanelMode::Inbox,
            );
            tab(ui, cx, "AGENTS", cx.model.agents.len(), cx.model.panel == Some(PanelMode::Agents), PanelMode::Agents);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if c::icon_button(ui, "close-feed", Icon::Close, 22.0, t::MUTED, false).on_hover_text("Close").clicked() {
                    cx.emit(UiAction::SetPanel(None));
                }
            });
        });
    });
}

fn tab(ui: &mut Ui, cx: &mut Ctx, title: &str, count: usize, selected: bool, panel: PanelMode) {
    let response = ui
        .scope_builder(egui::UiBuilder::new().id_salt(("feed-tab", title)).sense(Sense::click()), |ui| {
            // Swift's TabLabel is one plain Button, not independently selectable text.
            ui.style_mut().interaction.selectable_labels = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                tracking_label(ui, title, 11.0, if selected { t::INK } else { t::MUTED });
                if count > 0 {
                    let waiting = selected && panel == PanelMode::Inbox;
                    let color = if waiting { t::AMBER } else { t::MUTED };
                    let galley = ui.painter().layout_no_wrap(count.to_string(), t::font(10.0, Weight::Rounded), color);
                    let (rect, _) = ui.allocate_exact_size(galley.size() + egui::vec2(12.0, 2.0), Sense::hover());
                    ui.painter().rect_filled(rect, rect.height() / 2.0, if waiting { t::opacity(t::AMBER, 0.25) } else { t::FAINT });
                    ui.painter().galley(rect.min + egui::vec2(6.0, 1.0), galley, color);
                }
            });
        })
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, format!("{title}, {count}"))
    });
    if response.has_focus() {
        ui.painter().rect_stroke(response.rect.expand(2.0), 3.0, Stroke::new(1.0, t::AMBER), egui::StrokeKind::Outside);
    }
    if response.clicked() {
        cx.emit(UiAction::SetPanel(Some(panel)));
    }
}

fn tracking_label(ui: &mut Ui, value: &str, size: f32, color: egui::Color32) {
    let job = egui::text::LayoutJob::single_section(
        value.into(),
        egui::TextFormat { font_id: t::font(size, Weight::Semibold), color, extra_letter_spacing: 0.8, ..Default::default() },
    );
    ui.add(egui::Label::new(job).extend());
}

fn empty(ui: &mut Ui, title: &str, detail: &str) {
    egui::Frame::new().inner_margin(Margin::symmetric(0, 28)).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            c::label(ui, title, 13.0, Weight::Medium, t::white(0.85));
            c::label(ui, detail, 12.0, Weight::Regular, t::MUTED);
        });
    });
}

fn card(ui: &mut Ui, cx: &mut Ctx, agent: &Agent, is_new: bool) {
    let selected = cx.inputs.drafts.recipient() == Some(agent.id.as_str());
    let border = if selected {
        t::opacity(t::AMBER, 0.6)
    } else if is_new {
        t::opacity(t::AMBER, 0.3)
    } else {
        t::HAIRLINE
    };
    let before = cx.out.len();
    // Register the card below its child widgets, so Reply/Allow/More keep their own clicks.
    let response = ui
        .scope_builder(egui::UiBuilder::new().id_salt(("inbox-card", &agent.id)).sense(Sense::click()), |ui| {
            egui::Frame::new()
                .fill(t::FAINT)
                .stroke(Stroke::new(1.0, border))
                .corner_radius(14)
                // Add the 1pt stroke to this margin to recover Swift's 12pt content inset.
                .inner_margin(11)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        c::agent_badge(ui, cx.assets, agent, 28.0);
                        let right = if is_new { 104.0 } else { 75.0 };
                        let width = (ui.available_width() - right - 10.0).max(40.0);
                        ui.allocate_ui_with_layout(egui::vec2(width, 30.0), Layout::top_down(Align::Min), |ui| {
                            ui.set_min_width(width);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add(egui::Label::new(c::text(format!("@{}", agent.handle), 13.0, Weight::Semibold, t::INK)).truncate());
                            ui.add(
                                egui::Label::new(c::text(
                                    [agent.project.clone(), agent.location()].into_iter().flatten().collect::<Vec<_>>().join(" · "),
                                    11.0,
                                    Weight::Regular,
                                    t::MUTED,
                                ))
                                .truncate(),
                            );
                        });
                        let (rect, status) = ui.allocate_exact_size(egui::vec2(right, 30.0), Sense::hover());
                        status.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Label,
                                ui.is_enabled(),
                                format!(
                                    "{}{}, {}",
                                    if is_new { "New, " } else { "" },
                                    if agent.is_live() { t::state_label(agent.state) } else { "Recent" },
                                    c::time_ago(agent.state_since, cx.env.now_ms)
                                ),
                            )
                        });
                        card_status(ui, cx, agent, is_new, rect);
                    });
                    if let Some(title) = &agent.title {
                        ui.add(egui::Label::new(c::text(title, 12.0, Weight::Medium, t::white(0.75))).truncate());
                    }
                    let keys = cx.inputs.drafts.recipient() == Some(agent.id.as_str());
                    if let Some(launch) = cx.model.launch_for(&agent.id) {
                        super::task_cards::launch_cards(ui, cx, launch, keys && !agent.is_answerable());
                    }
                    if agent.is_answerable() {
                        super::question::question_card(ui, cx, agent, keys);
                    } else if let Some(question) = &agent.question {
                        egui::Frame::new().fill(t::opacity(t::AMBER, 0.12)).corner_radius(10).inner_margin(10).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 6.0;
                            ui.add(egui::Label::new(c::text(question, 13.0, Weight::Medium, t::AMBER)).wrap().selectable(true));
                            request_detail(ui, cx, agent);
                        });
                    }
                    if let Some(message) = &agent.last_message
                        && !message.is_empty()
                    {
                        let expanded = cx.view.expanded_cards.contains(&agent.id);
                        if let Some(link) = c::markdown_linked(
                            ui,
                            message,
                            MarkdownStyle { max_rows: if expanded { 40 } else { 5 }, ..Default::default() },
                            0.0,
                        )
                        .1
                        {
                            cx.emit(UiAction::OpenUrl(link));
                        }
                        if message.chars().count() > 280
                            && c::text_button(ui, if expanded { "Less" } else { "More" }, 11.0, Weight::Medium, t::MUTED).clicked()
                        {
                            toggle(&mut cx.view.expanded_cards, &agent.id);
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if c::small_button(ui, "Reply", Icon::Reply).clicked() {
                            cx.emit(UiAction::Compose { agent_id: Some(agent.id.clone()) });
                        }
                        if c::small_button(ui, "Open", Icon::Open).clicked() {
                            cx.emit(UiAction::OpenAgent { agent_id: agent.id.clone() });
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if c::small_button(ui, "Dismiss", Icon::Dismiss)
                                .on_hover_text(format!("Hide this until @{} has something new", agent.handle))
                                .clicked()
                            {
                                cx.emit(UiAction::DismissTurns { turn_keys: vec![agent.turn_key()] });
                            }
                        });
                    });
                });
        })
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("Open conversation with @{}", agent.handle))
    });
    if response.clicked() && cx.out.len() == before {
        cx.emit(UiAction::OpenChat { agent_id: agent.id.clone() });
    }
}

/// Explicit trailing geometry avoids egui inheriting RTL from a trailing-aligned VStack.
fn card_status(ui: &mut Ui, cx: &Ctx, agent: &Agent, is_new: bool, rect: egui::Rect) {
    let color = if agent.state.wants_you() { t::AMBER } else { t::MUTED };
    let state = if agent.is_live() { t::state_label(agent.state) } else { "Recent" };
    let galley = ui.painter().layout_no_wrap(state.into(), t::font(11.0, Weight::Medium), color);
    let state_left = rect.right() - galley.size().x;
    let center_y = rect.top() + 7.0;
    if !agent.is_live() {
        let recent = ui.painter().layout_no_wrap("Recent".into(), t::font(10.5, Weight::Medium), t::MUTED);
        let size = recent.size() + egui::vec2(14.0, 4.0);
        let capsule = egui::Rect::from_min_size(egui::pos2(rect.right() - size.x, center_y - size.y / 2.0), size);
        ui.painter().rect_filled(capsule, size.y / 2.0, t::FAINT);
        ui.painter().galley(capsule.min + egui::vec2(7.0, 2.0), recent, t::MUTED);
        ui.interact(capsule, ui.id().with(("recent-chip", &agent.id)), Sense::hover()).on_hover_text(c::recent_help(agent));
    } else {
        ui.painter().galley(egui::pos2(state_left, center_y - galley.size().y / 2.0), galley, color);
    }
    let dot = state_left - 8.0;
    if agent.is_live() {
        ui.painter().circle_filled(egui::pos2(dot, center_y), 3.0, t::state_color(agent.state));
    }
    if is_new {
        let job = egui::text::LayoutJob::single_section(
            "NEW".into(),
            egui::TextFormat { font_id: t::font(9.0, Weight::Bold), color: t::black(0.8), extra_letter_spacing: 0.5, ..Default::default() },
        );
        let new = ui.painter().layout_job(job);
        let size = new.size() + egui::vec2(10.0, 2.0);
        let capsule = egui::Rect::from_min_size(egui::pos2(dot - 3.0 - 6.0 - size.x, center_y - size.y / 2.0), size);
        ui.painter().rect_filled(capsule, size.y / 2.0, t::AMBER);
        ui.painter().galley(capsule.min + egui::vec2(5.0, 1.0), new, t::black(0.8));
    }
    let age = ui.painter().layout_no_wrap(c::time_ago(agent.state_since, cx.env.now_ms), t::font(10.0, Weight::Regular), t::MUTED);
    ui.painter().galley(egui::pos2(rect.right() - age.size().x, rect.bottom() - age.size().y), age, t::MUTED);
}

pub fn request_detail(ui: &mut Ui, cx: &mut Ctx, agent: &Agent) {
    let Some(detail) = &agent.question_detail else {
        return;
    };
    if Some(detail) == agent.question.as_ref() {
        return;
    }
    let key = agent.prompt_id.as_deref().unwrap_or(&agent.id);
    let shown = cx.view.request_details.contains(key);
    if c::text_button(ui, if shown { "Hide full request" } else { "Show full request" }, 11.0, Weight::Medium, t::MUTED).clicked() {
        toggle(&mut cx.view.request_details, key);
    }
    if shown {
        egui::ScrollArea::vertical().id_salt(("request-detail", key)).max_height(180.0).min_scrolled_height(0.0).show(ui, |ui| {
            ui.add(egui::Label::new(c::text(detail, 11.5, Weight::Mono, t::white(0.9))).wrap().selectable(true));
        });
    }
}

fn row(ui: &mut Ui, cx: &mut Ctx, agent: &Agent) {
    let selected = cx.inputs.drafts.recipient() == Some(agent.id.as_str());
    let background = ui.painter().add(egui::Shape::Noop);
    let response = egui::Frame::new()
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            // AgentRow is also a plain Button on the Mac. Keep selection in chat, not here.
            ui.style_mut().interaction.selectable_labels = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                c::agent_badge(ui, cx.assets, agent, 26.0);
                let width = (ui.available_width() - c::state_chip_width(ui, agent) - 10.0).max(30.0);
                ui.allocate_ui_with_layout(egui::vec2(width, 30.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_width(width);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        c::label(ui, format!("@{}", agent.handle), 13.0, Weight::Semibold, t::INK);
                        if let Some(title) = &agent.title {
                            ui.add(egui::Label::new(c::text(title, 12.0, Weight::Regular, t::MUTED)).truncate());
                        }
                    });
                    ui.add(
                        egui::Label::new(c::text(
                            [agent.short_cwd(&cx.env.home), agent.location()].into_iter().flatten().collect::<Vec<_>>().join(" · "),
                            11.0,
                            Weight::Regular,
                            t::MUTED,
                        ))
                        .truncate(),
                    );
                });
                c::state_chip(ui, agent);
            });
        })
        .response
        .interact(Sense::click());
    if selected || response.hovered() || response.has_focus() {
        ui.painter().set(background, egui::Shape::rect_filled(response.rect, 12, t::FAINT));
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("@{}, {}", agent.handle, agent.state.label()))
    });
    if response.clicked() {
        cx.emit(UiAction::OpenChat { agent_id: agent.id.clone() });
    }
    response.context_menu(|ui| {
        if ui.button(format!("Message @{}", agent.handle)).clicked() {
            cx.emit(UiAction::Compose { agent_id: Some(agent.id.clone()) });
            ui.close();
        }
        if ui.button(format!("Open in {}", agent.host.as_ref().map_or("terminal", |host| host.app.as_str()))).clicked() {
            cx.emit(UiAction::OpenAgent { agent_id: agent.id.clone() });
            ui.close();
        }
    });
}

/// Sessions Lantern started that have ended: what they asked and reported, kept after the agent
/// exits (Swift `PanelView.endedList`).
fn ended(ui: &mut Ui, cx: &mut Ctx) {
    let model = cx.model;
    let mut launches = model.ended_launches().peekable();
    if launches.peek().is_none() {
        return;
    }
    tracking_label(ui, "FINISHED", 10.5, t::MUTED);
    for launch in launches {
        egui::Frame::new().fill(t::FAINT).corner_radius(12).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(c::text(&launch.title, 12.5, Weight::Medium, t::INK)).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    c::label(ui, c::time_ago(launch.ended_at, cx.env.now_ms), 10.0, Weight::Regular, t::MUTED);
                });
            });
            super::task_cards::launch_cards(ui, cx, launch, false);
        });
    }
}

fn toggle(values: &mut std::collections::HashSet<String>, key: &str) {
    if !values.remove(key) {
        values.insert(key.into());
    }
}

fn connect_phone(ui: &mut Ui, cx: &mut Ctx) {
    if c::secondary_button(ui, "Connect your phone").clicked() {
        cx.emit(UiAction::Phone(crate::model::phone::Action::Open));
    }
}
