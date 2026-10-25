//! ChatView.swift and StartingChatView: content-sized history and immediate launch preview.
use super::{
    Ctx, UiAction,
    components::{self as c},
    icons::Icon,
    theme::{self as t, Weight},
};
use crate::model::{Agent, AgentState, ChatMessage, HarnessState, LaunchPreview, Role};
use eframe::egui::{self, Align, Layout, Margin, Sense, Stroke, Ui};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

pub fn render(ui: &mut Ui, cx: &mut Ctx, agent: &Agent, max_height: f32) {
    egui::Frame::new().inner_margin(Margin { left: 12, right: 12, top: 12, bottom: 10 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if c::icon_button(ui, "chat-back", Icon::Back, 22.0, t::MUTED, false).on_hover_text("Back (Esc)").clicked() {
                cx.emit(UiAction::CloseChat);
            }
            c::agent_badge(ui, cx.assets, agent, 24.0);
            let open_width = ui.painter().layout_no_wrap("Open".into(), t::font(11.0, Weight::Medium), t::MUTED).size().x + 31.0;
            let width = (ui.available_width() - c::state_chip_width(ui, agent) - open_width - 20.0).max(30.0);
            ui.allocate_ui_with_layout(egui::vec2(width, 28.0), Layout::top_down(Align::Min), |ui| {
                ui.set_min_width(width);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(egui::Label::new(c::text(format!("@{}", agent.handle), 13.0, Weight::Semibold, t::INK)).truncate());
                let subtitle = agent
                    .title
                    .clone()
                    .unwrap_or_else(|| [agent.project.clone(), agent.location()].into_iter().flatten().collect::<Vec<_>>().join(" · "));
                ui.add(egui::Label::new(c::text(subtitle, 11.0, Weight::Regular, t::MUTED)).truncate());
            });
            c::state_chip(ui, agent);
            if c::small_button(ui, "Open", Icon::Open).clicked() {
                cx.emit(UiAction::OpenAgent { agent_id: agent.id.clone() });
            }
        });
    });
    if agent.is_answerable() {
        egui::Frame::new().inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 10 }).show(ui, |ui| {
            super::question::question_card(ui, cx, agent, true);
        });
    }
    divider(ui);
    egui::ScrollArea::vertical().id_salt(("chat",&agent.id)).max_height((max_height-240.0).clamp(180.0,480.0)).min_scrolled_height(0.0)
        .auto_shrink([false,true]).stick_to_bottom(true).show(ui,|ui|{
            egui::Frame::new().inner_margin(Margin::symmetric(12,10)).show(ui,|ui|{
                ui.spacing_mut().item_spacing.y=8.0;
                let Some(chat)=&cx.model.chat else{return;};
                if agent.session_ambiguous(){
                    note(ui,&format!("Another {} session works in this folder, so Lantern can't tell which conversation is this one. Turn on hooks (Claude Code) or start it with --resume <id> for an exact match.",agent.kind.display_name()));
                }else if !chat.supported{
                    note(ui,&format!("Lantern can't read {} conversations yet. Click Open to see it in its terminal.",agent.kind.display_name()));
                }else if chat.messages.is_empty(){
                    if chat.loading{
                        egui::Frame::new().inner_margin(Margin::symmetric(0,24)).show(ui,|ui|{ui.horizontal(|ui|{
                            ui.add_space(60.0);c::spinner(ui,12.0,t::MUTED,cx.env);c::label(ui,"Loading the conversation…",12.0,Weight::Regular,t::MUTED);
                        });});
                    }else{note(ui,"No messages yet.");}
                }else{
                    if chat.messages.len()>=120{
                        ui.horizontal_wrapped(|ui|{
                            c::label(ui,"Recent messages only. The whole conversation is in its terminal.",11.0,Weight::Regular,t::MUTED);
                            if c::text_button(ui,"Open",11.0,Weight::Semibold,t::AMBER).clicked(){cx.emit(UiAction::OpenAgent{agent_id:agent.id.clone()});}
                        });ui.add_space(4.0);
                    }
                    for (key,message) in keyed(&chat.messages){bubble(ui,cx,message,&key);}
                    if agent.state==AgentState::Working{ui.horizontal(|ui|{ui.add_space(4.0);c::spinner(ui,10.0,t::MUTED,cx.env);c::label(ui,"Working…",11.0,Weight::Regular,t::MUTED);});}
                }
            });
        });
}

pub fn preview(ui: &mut Ui, cx: &mut Ctx, preview: &LaunchPreview) {
    egui::Frame::new().inner_margin(Margin { left: 12, right: 12, top: 12, bottom: 10 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if c::icon_button(ui, "preview-back", Icon::Back, 22.0, t::MUTED, false).on_hover_text("Back (Esc)").clicked() {
                cx.emit(UiAction::CloseLaunchPreview);
            }
            c::harness_logo(ui, cx.assets, &preview.kind, 24.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                c::label(ui, preview.kind.display_name(), 13.0, Weight::Semibold, t::INK);
                c::label(ui, folder_name(&preview.folder, &cx.env.home), 11.0, Weight::Regular, t::MUTED);
            });
        });
    });
    divider(ui);
    egui::Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let prompt = without_lantern_notes(&preview.prompt).trim();
        if !prompt.is_empty() {
            bubble(ui, cx, &ChatMessage { role: Role::User, text: prompt.into(), at: None }, "launch-preview");
        }
        if let Some(error) = &preview.error {
            egui::Frame::new().inner_margin(Margin { left: 4, right: 0, top: 0, bottom: 0 }).show(ui, |ui| {
                c::label(ui, format!("Couldn't start it: {error}"), 12.0, Weight::Regular, t::AMBER);
                ui.horizontal(|ui| {
                    if c::small_button(ui, "Try in Terminal", Icon::Terminal).clicked() {
                        cx.emit(UiAction::RetryPreviewInTerminal);
                    }
                    if c::small_button(ui, "Close", Icon::Close).clicked() {
                        cx.emit(UiAction::CloseLaunchPreview);
                    }
                });
            });
        } else {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                c::spinner(ui, 10.0, t::MUTED, cx.env);
                let status = if preview.kind.as_str() == "october"
                    && cx.model.harness.as_ref().is_some_and(|h| h.status == HarnessState::Installing)
                {
                    "Setting up October Harness…".into()
                } else {
                    format!("Starting {}…", preview.kind.display_name())
                };
                c::label(ui, status, 11.0, Weight::Regular, t::MUTED);
            });
        }
    });
    if preview.error.is_none() {
        egui::Frame::new().inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 12 }).show(ui, |ui| {
            egui::Frame::new().fill(t::FAINT).corner_radius(12).inner_margin(Margin::symmetric(12, 9)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                c::label(ui, "You can reply once it's started.", 12.0, Weight::Regular, t::white(0.55 * 0.7));
            });
        });
    }
}

pub fn bubble(ui: &mut Ui, cx: &mut Ctx, message: &ChatMessage, key: &str) {
    if message.role == Role::Tool {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            // SF Mono 11 advances ~6.9pt a glyph; JetBrains Mono needs 11.5 for the same line.
            // `chevron.right.2` at 8pt bold: two heavy chevrons, 3.5pt apart.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 10.0), Sense::hover());
            let stroke = Stroke::new(1.5, t::white(0.44));
            for x in [0.75, 4.25] {
                ui.painter().line_segment([rect.min + egui::vec2(x, 2.0), rect.min + egui::vec2(x + 3.0, 5.0)], stroke);
                ui.painter().line_segment([rect.min + egui::vec2(x + 3.0, 5.0), rect.min + egui::vec2(x, 8.0)], stroke);
            }
            ui.add(egui::Label::new(c::text(&message.text, 11.5, Weight::Mono, t::white(0.44))).truncate()).on_hover_text(&message.text);
        });
        return;
    }
    let user = message.role == Role::User;
    let source = if user { without_lantern_notes(&message.text) } else { &message.text };
    let available = ui.available_width();
    let width = if user {
        let natural = source
            .lines()
            .map(|line| ui.painter().layout_no_wrap(line.into(), t::font(12.5, Weight::Regular), t::INK).size().x)
            .fold(0.0, f32::max)
            + 22.0;
        natural.max(42.0).min((available - 40.0).max(30.0))
    } else {
        (available - 24.0).max(30.0)
    };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if user {
            ui.add_space((available - width).max(0.0));
        }
        egui::Frame::new()
            .fill(if user { t::opacity(t::AMBER, 0.22) } else { t::FAINT })
            .stroke(Stroke::new(1.0, if user { t::opacity(t::AMBER, 0.3) } else { t::HAIRLINE }))
            .corner_radius(14)
            // egui's stroke consumes 1pt per side; Swift's strokeBorder does not.
            .inner_margin(Margin::symmetric(10, 7))
            .show(ui, |ui| {
                let inner = (width - 22.0).max(8.0);
                ui.set_width(inner);
                ui.spacing_mut().item_spacing.y = 4.0;
                let expanded = cx.view.expanded_bubbles.contains(key);
                let limit = if expanded {
                    f32::INFINITY
                } else if user {
                    200.0
                } else {
                    280.0
                };
                let start = ui.cursor().min;
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt(("bubble-content", key))
                        .max_rect(egui::Rect::from_min_size(start, egui::vec2(inner, f32::INFINITY)))
                        .layout(Layout::top_down(Align::Min)),
                );
                child.set_clip_rect(ui.clip_rect().intersect(egui::Rect::from_min_size(start, egui::vec2(inner, limit))));
                let actions = c::blocks_linked(&mut child, source, egui::Id::new(key));
                if let Some(code) = actions.copy {
                    cx.emit(UiAction::Copy(code));
                }
                if let Some(link) = actions.link {
                    cx.emit(UiAction::OpenUrl(link));
                }
                let full = child.min_rect().height();
                ui.allocate_exact_size(egui::vec2(inner, full.min(limit)), Sense::hover());
                if full > if user { 201.0 } else { 281.0 }
                    && c::text_button(ui, if expanded { "Less" } else { "More" }, 11.0, Weight::Medium, t::MUTED).clicked()
                    && !cx.view.expanded_bubbles.remove(key)
                {
                    cx.view.expanded_bubbles.insert(key.into());
                }
            });
    });
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0, t::HAIRLINE);
}
fn note(ui: &mut Ui, body: &str) {
    egui::Frame::new().inner_margin(Margin::symmetric(0, 40)).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            c::label(ui, body, 12.0, Weight::Regular, t::MUTED);
        });
    });
}

pub fn without_lantern_notes(source: &str) -> &str {
    const MARKERS: [&str; 5] = [
        "\n\nYou're connected to October Bus",
        "\n\n(Started from October Lantern",
        "\n\nFor context, a screenshot of my screen",
        "\n\nWhat this Mac already has",
        "\n\nWhat this computer already has",
    ];
    &source[..MARKERS.iter().filter_map(|marker| source.find(marker)).min().unwrap_or(source.len())]
}

pub(super) fn folder_name<'a>(folder: &'a str, home: &str) -> &'a str {
    if folder.trim_end_matches(['/', '\\']) == home.trim_end_matches(['/', '\\']) {
        "Home folder"
    } else {
        folder.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(folder)
    }
}

pub(super) fn keyed(messages: &[ChatMessage]) -> Vec<(String, &ChatMessage)> {
    let mut seen = HashMap::<String, usize>::new();
    messages
        .iter()
        .map(|message| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            message.text.hash(&mut hasher);
            let base = format!("{:?}|{}|{}", message.role, message.at.as_deref().unwrap_or(""), hasher.finish());
            let n = seen.entry(base.clone()).or_default();
            let key = if *n == 0 { base } else { format!("{base}#{n}") };
            *n += 1;
            (key, message)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hides_earliest_scaffolding_only_in_the_presented_user_copy() {
        let original = "Find it.\n\nFor context, a screenshot of my screen is /tmp/a\n\nYou're connected to October Bus";
        assert_eq!(without_lantern_notes(original), "Find it.");
        assert!(original.contains("/tmp/a"));
        assert_eq!(without_lantern_notes("A normal message"), "A normal message");
    }
    #[test]
    fn repeated_message_keys_are_unique_and_survive_window_trimming() {
        let a = ChatMessage { role: Role::Agent, text: "a".into(), at: Some("1".into()) };
        let b = ChatMessage { role: Role::Agent, text: "b".into(), at: Some("2".into()) };
        let first = [a, b.clone(), b.clone()];
        let keys = keyed(&first);
        assert_ne!(keys[1].0, keys[2].0);
        assert_eq!(keys[1].0, keyed(&[b])[0].0);
    }
    #[test]
    fn folder_names_support_both_platforms() {
        assert_eq!(folder_name("/home/me/", "/home/me"), "Home folder");
        assert_eq!(folder_name("C:\\Users\\Me\\project", "C:\\Users\\Me"), "project");
    }
}
