//! SettingsView.swift: four tabs in an ordinary dark, fixed-size window, laid out like the Mac's
//! grouped Form (metrics measured from the Mac settings-0..3 snapshots); not panel glass.
use super::{
    Ctx, SettingChange, UiAction, cards, components as c,
    icons::Icon,
    theme::{self as t, Weight},
};
use crate::model::{AgentKind, Edge, HookState, MESSAGE_HOTKEYS, POINT_ASK_HOTKEYS, Platform, Uninstall};
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Stroke, Ui, vec2};

const SIZE: egui::Vec2 = vec2(520.0, 480.0);
const BACKGROUND: Color32 = Color32::from_gray(30);
const GROUP: Color32 = Color32::from_gray(37);
// SettingsView.swift's help predates area selection and signed-in Ask routing.
const POINT_ASK_HELP: &str = "Hold the shortcut, select an area and say what you want. A card shows a screenshot of it; Enter asks October, or sends it to the agent.";

pub fn render(ui: &mut Ui, cx: &mut Ctx) -> f32 {
    let rect = Rect::from_min_size(ui.cursor().min, SIZE);
    ui.painter().rect_filled(rect, 0, BACKGROUND);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)), |ui| {
        ui.set_width(SIZE.x);
        ui.set_height(SIZE.y);
        ui.set_clip_rect(ui.clip_rect().intersect(rect));
        // These are native settings controls, not the app's amber panel controls.
        ui.visuals_mut().selection.bg_fill = t::LINK;
        ui.visuals_mut().hyperlink_color = t::LINK;
        ui.visuals_mut().widgets.active.bg_fill = t::LINK;
        ui.visuals_mut().widgets.active.bg_stroke = Stroke::new(1.0, t::LINK);
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        // Mac push buttons in a Form: a soft gray fill, no border.
        ui.spacing_mut().button_padding = vec2(11.0, 4.0);
        let widgets = &mut ui.visuals_mut().widgets;
        for (state, alpha) in [(&mut widgets.inactive, 0.1), (&mut widgets.hovered, 0.14), (&mut widgets.active, 0.2)] {
            state.weak_bg_fill = t::white(alpha);
            state.bg_stroke = Stroke::NONE;
            state.corner_radius = egui::CornerRadius::same(5);
            state.expansion = 0.0;
        }
        tabs(ui, cx);
        let tab = cx.view.settings_tab.min(3);
        let mut scroll = egui::ScrollArea::vertical().id_salt(("settings-page", tab)).max_height(424.0).auto_shrink([false, false]);
        if let Some(offset) = cx.view.settings_scroll.take() {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        scroll.show(ui, |ui| {
            egui::Frame::new().inner_margin(20).show(ui, |ui| {
                ui.set_width(480.0);
                ui.spacing_mut().item_spacing.y = 14.0;
                match tab {
                    0 => general(ui, cx),
                    1 => agents(ui, cx),
                    2 => october(ui, cx),
                    _ => about(ui, cx),
                }
            });
        });
    });
    confirmation(ui, cx);
    SIZE.y
}

/// The window's segmented control (SwiftUI TabView in Settings): one rounded track, text-only
/// segments, the selected one filled, thin dividers between unselected neighbours.
fn tabs(ui: &mut Ui, cx: &mut Ctx) {
    const TITLES: [&str; 4] = ["General", "Agents", "October", "About"];
    let (row, _) = ui.allocate_exact_size(vec2(SIZE.x, 48.0), Sense::hover());
    let font = t::font(13.0, Weight::Regular);
    let galleys: Vec<_> = TITLES.iter().map(|title| ui.painter().layout_no_wrap((*title).into(), font.clone(), t::INK)).collect();
    let widths: Vec<f32> = galleys.iter().map(|galley| galley.size().x + 24.0).collect();
    let height = 24.0;
    let track = Rect::from_center_size(egui::pos2(row.center().x, row.top() + 22.0), vec2(widths.iter().sum::<f32>() + 4.0, height));
    ui.painter().rect_filled(track, 7, t::white(0.07));
    let selected = cx.view.settings_tab.min(3);
    let mut left = track.left() + 2.0;
    for (index, (title, galley)) in TITLES.into_iter().zip(galleys).enumerate() {
        let rect = Rect::from_min_size(egui::pos2(left, track.top() + 2.0), vec2(widths[index], height - 4.0));
        left += widths[index];
        let on = selected == index;
        let response = ui.interact(rect, egui::Id::new(("settings-tab", index)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, title));
        if on {
            ui.painter().rect_filled(rect, 5, t::white(0.2));
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 5, t::white(0.05));
        }
        // A divider sits between two unselected segments only.
        if index > 0 && !on && selected != index - 1 {
            ui.painter().vline(rect.left(), rect.center().y - 6.0..=rect.center().y + 6.0, Stroke::new(1.0, t::white(0.14)));
        }
        if response.has_focus() {
            ui.painter().rect_stroke(rect, 5, Stroke::new(1.0, t::LINK), egui::StrokeKind::Inside);
        }
        let color = if on { t::INK } else { t::opacity(t::INK, 0.85) };
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
        if response.clicked() {
            cx.view.settings_tab = index;
        }
    }
    ui.painter().hline(row.x_range(), row.bottom(), Stroke::new(1.0, t::HAIRLINE));
}

/// Grouped Form metrics (Mac settings-0..3): rows 38pt tall with 10pt side padding, hairlines
/// between rows (captions included) inset to that padding, bold ink section headings.
const ROW: f32 = 38.0;
const ROW_PAD: f32 = 10.0;

fn group_top_id() -> egui::Id {
    egui::Id::new("settings-group-top")
}

fn group(ui: &mut Ui, title: Option<&str>, body: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if let Some(title) = title {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(ROW_PAD);
                c::label(ui, title, 13.0, Weight::Semibold, t::INK);
            });
            ui.add_space(8.0);
        }
        egui::Frame::new().fill(GROUP).corner_radius(10).inner_margin(egui::Margin::symmetric(ROW_PAD as i8, 0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let top = ui.cursor().top();
            ui.ctx().data_mut(|data| data.insert_temp(group_top_id(), top));
            body(ui);
            ui.ctx().data_mut(|data| data.remove::<f32>(group_top_id()));
        });
    });
}

/// A hairline above every row but a group's first, captions included.
fn divider(ui: &mut Ui) {
    let top = ui.ctx().data(|data| data.get_temp::<f32>(group_top_id()));
    let y = ui.cursor().top();
    if top.is_some_and(|top| y > top + 0.5) {
        let width = 1.0 / ui.ctx().pixels_per_point();
        ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(width, t::white(0.06)));
    }
}

/// One row of a group: at least `ROW` tall, contents vertically centred.
fn row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    divider(ui);
    ui.allocate_ui_with_layout(vec2(ui.available_width(), ROW), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_height(ROW);
        ui.spacing_mut().item_spacing.x = 8.0;
        add(ui)
    })
    .inner
}

/// A Form footnote row (Mac settings-0/3): its own block below a hairline, 10pt above and below.
fn caption(ui: &mut Ui, text: &str) {
    divider(ui);
    ui.add_space(10.0);
    c::label(ui, text, 11.0, Weight::Regular, t::MUTED);
    ui.add_space(10.0);
}

/// What "Share usage counts" sends (Swift SettingsView).
pub(super) const USAGE_DETAIL: &str = "How often Lantern and its features are used, so we know what to improve. Never your code, messages, commands or folder names. If you sign in to October, counts are linked to your October account and email.";

fn toggle(ui: &mut Ui, cx: &mut Ctx, title: &str, current: bool, change: impl FnOnce(bool) -> SettingChange) {
    if let Some(on) = setting_switch(ui, title, current, None) {
        cx.emit(UiAction::ChangeSetting(change(on)));
    }
}

/// GroupedFormStyle puts labels first and a switch at the trailing edge (Mac settings-0: a 36x16
/// track, a 20x14 capsule knob, gray when off).
fn setting_switch(ui: &mut Ui, title: &str, current: bool, agent: Option<(&super::Assets, &AgentKind)>) -> Option<bool> {
    divider(ui);
    let enabled = ui.is_enabled();
    let ink = if enabled { t::INK } else { t::opacity(t::INK, 0.45) };
    let width = ui.available_width();
    let gap = ui.spacing().item_spacing.x;
    let logo_width = if agent.is_some() { 18.0 + gap } else { 0.0 };
    let label = ui.painter().layout(title.into(), t::font(13.0, Weight::Regular), ink, (width - 36.0 - gap - logo_width).max(1.0));
    // Measure once: wrapped labels, logos and the switch share the same row center.
    // A zero-height child layout centers text relative to a different baseline.
    let (row, _) = ui.allocate_exact_size(vec2(width, (label.rect.height() + 16.0).max(ROW)), Sense::hover());
    if let Some((assets, kind)) = agent {
        let rect = Rect::from_center_size(egui::pos2(row.left() + 9.0, row.center().y), vec2(18.0, 18.0));
        let mut logo =
            ui.new_child(egui::UiBuilder::new().id_salt(("settings-logo", title)).max_rect(rect).layout(Layout::top_down(Align::Min)));
        c::harness_logo(&mut logo, assets, kind, 18.0);
    }
    ui.painter().galley(egui::pos2(row.left() + logo_width, row.center().y - label.rect.center().y), label, ink);
    let response = ui.interact(row, egui::Id::new(("settings-switch", title)), Sense::click());
    let on = if response.clicked() { !current } else { current };
    let track = Rect::from_center_size(egui::pos2(row.right() - 18.0, row.center().y), vec2(36.0, 16.0));
    let fill = if on { t::LINK } else { t::white(0.1) };
    ui.painter().rect_filled(track, 8.0, if enabled { fill } else { t::opacity(fill, 0.35) });
    let knob =
        Rect::from_center_size(egui::pos2(if on { track.right() - 11.0 } else { track.left() + 11.0 }, track.center().y), vec2(20.0, 14.0));
    let knob_fill = if on { Color32::WHITE } else { Color32::from_gray(226) };
    ui.painter().rect_filled(knob, 7.0, if enabled { knob_fill } else { t::opacity(knob_fill, 0.45) });
    if response.has_focus() {
        ui.painter().rect_stroke(track.expand(2.0), 10.0, Stroke::new(1.0, t::LINK), egui::StrokeKind::Inside);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, title));
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_role(egui::accesskit::Role::Switch));
    response.clicked().then_some(on)
}

/// A Mac popup button in a Form row: the value right-aligned, then a small up/down chevron
/// badge at the trailing edge, no box. Clicking opens the option list.
fn picker(ui: &mut Ui, title: &str, current: &str, options: &[&str], help: Option<&str>, label: impl Fn(&str) -> String) -> Option<String> {
    let mut selected = current.to_owned();
    row(ui, |ui| {
        let value = ui.painter().layout_no_wrap(label(current), t::font(13.0, Weight::Regular), t::INK);
        let control_width = value.size().x + 8.0 + 19.0 + 3.0;
        let label_width = (ui.available_width() - control_width - 12.0).max(1.0);
        let label_response = ui
            .allocate_ui_with_layout(vec2(label_width, ROW), Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_width(label_width);
                c::label(ui, title, 13.0, Weight::Regular, t::INK)
            })
            .inner;
        let label_response = if let Some(help) = help { label_response.on_hover_text(help) } else { label_response };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(control_width, 24.0), Sense::hover());
            let response = ui.interact(rect, egui::Id::new(("settings-picker", title)), Sense::click()).labelled_by(label_response.id);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), label(current)));
            let badge = Rect::from_center_size(egui::pos2(rect.right() - 3.0 - 9.5, rect.center().y), vec2(19.0, 20.0));
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
            ui.painter().rect_filled(badge, 7.0, t::white(if open || response.is_pointer_button_down_on() { 0.16 } else { 0.08 }));
            c::paint_icon(
                ui.painter(),
                Rect::from_center_size(badge.center(), vec2(16.0, 16.0)),
                Icon::ChevronUpDown,
                t::opacity(t::INK, 0.88),
            );
            ui.painter().galley(egui::pos2(badge.left() - 8.0 - value.size().x, rect.center().y - value.size().y / 2.0), value, t::INK);
            if response.has_focus() {
                ui.painter().rect_stroke(rect.expand(2.0), 6.0, Stroke::new(1.0, t::LINK), egui::StrokeKind::Inside);
            }
            egui::Popup::menu(&response).align(egui::RectAlign::BOTTOM_END).gap(4.0).show(|ui| {
                ui.set_min_width(170.0);
                for value in options {
                    ui.selectable_value(&mut selected, (*value).to_owned(), label(value));
                }
            });
            if let Some(help) = help {
                response.on_hover_text(help);
            }
        });
    });
    (selected != current).then_some(selected)
}

/// A bordered push button on its own Form row (Mac About: Report a Problem…, Uninstall…).
fn button_row(ui: &mut Ui, title: &str) -> bool {
    row(ui, |ui| ui.button(title).clicked())
}

fn general(ui: &mut Ui, cx: &mut Ctx) {
    group(ui, None, |ui| {
        toggle(ui, cx, "Open at login", cx.model.prefs.open_at_login, SettingChange::OpenAtLogin);
        let edge = if cx.model.prefs.edge == Edge::Left { "left" } else { "right" };
        if let Some(value) = picker(ui, "Lantern position", edge, &["right", "left"], None, |value| {
            if value == "left" { "Left edge".into() } else { "Right edge".into() }
        }) {
            cx.emit(UiAction::ChangeSetting(SettingChange::Edge(if value == "left" { Edge::Left } else { Edge::Right })));
        }
        if let Some(value) =
            picker(ui, "Shortcut to message an agent", &cx.model.prefs.message_hotkey, &MESSAGE_HOTKEYS, None, crate::model::hotkey_label)
        {
            cx.emit(UiAction::ChangeSetting(SettingChange::MessageHotkey(value)));
        }
        if let Some(value) = picker(
            ui,
            "Point & Ask (hold, point, speak)",
            &cx.model.prefs.point_ask_hotkey,
            &POINT_ASK_HOTKEYS,
            Some(POINT_ASK_HELP),
            crate::model::hotkey_label,
        ) {
            cx.emit(UiAction::ChangeSetting(SettingChange::PointAskHotkey(value)));
        }
        toggle(ui, cx, "Check for updates automatically", cx.model.prefs.automatic_updates, SettingChange::AutomaticUpdates);
    });
    group(ui, Some("New sessions and tasks"), |ui| {
        toggle(
            ui,
            cx,
            "Include a screenshot of my screen when starting a session",
            cx.model.prefs.screenshot_new_sessions,
            SettingChange::ScreenshotNewSessions,
        );
        caption(
            ui,
            "The new agent gets a screenshot of the screen you're on (without Lantern) with its first message, for context. It stays on this computer. Needs screen capture permission; you can also switch it per session.",
        );
        toggle(ui, cx, "Show me on screen", cx.model.prefs.show_me, SettingChange::ShowMe);
        toggle(ui, cx, "Let agents use this computer's apps", cx.model.prefs.computer_use, SettingChange::ComputerUse);
        let status = match cx.model.computer.as_ref().filter(|_| cx.model.prefs.computer_use) {
            Some(s) if s.status == "installing" => " Downloading Cua Driver…".to_string(),
            Some(s) if s.status == "failed" => {
                format!(" Cua Driver didn't start: {}. Switch it off and on to try again.", s.message.as_deref().unwrap_or("unknown error"))
            }
            Some(s) if s.status == "ready" => format!(" Ready (Cua Driver {}).", s.version.as_deref().unwrap_or("")),
            _ => String::new(),
        };
        caption(
            ui,
            &format!(
                "Agents Lantern starts can see and work your apps (your browser, forms, desktop apps) in the background, with their own cursor, without taking over your mouse. Uses Cua Driver (open source).{status}"
            ),
        );
        toggle(ui, cx, "Ask before agents act", cx.model.prefs.ask_first, SettingChange::AskFirst);
        caption(
            ui,
            "Off: agents Lantern starts work without stopping to ask (Claude Code, Codex and others in their auto-approve mode), so tasks finish on their own. On: each agent asks you before it runs commands or changes files.",
        );
        toggle(ui, cx, "Connect new sessions to October Bus", cx.model.prefs.bus_for_sessions, SettingChange::BusForSessions);
        caption(
            ui,
            "Agents Lantern starts (Claude Code, Codex, OpenCode, October) can see and message each other and share tasks. Everything stays on this computer.",
        );
        toggle(
            ui,
            cx,
            "Also give new sessions the list of tools on this computer",
            cx.model.prefs.toolkit_in_sessions,
            SettingChange::ToolkitInSessions,
        );
        // The toolkit's caption sits beside its two buttons, in a caption block of its own.
        divider(ui);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let buttons_width = ["Open List", "Refresh"].into_iter().map(|title| {
                ui.painter().layout_no_wrap(title.into(), t::font(12.0, Weight::Medium), t::INK).size().x
                    + 2.0 * ui.spacing().button_padding.x + 4.0 // Atom-frame pixel rounding allowance.
            }).sum::<f32>() + ui.spacing().item_spacing.x;
            ui.allocate_ui_with_layout(vec2(ui.available_width() - buttons_width - 8.0, 0.0), Layout::top_down(Align::Min), |ui| {
                caption(ui, "Lantern keeps a short list of what this computer has (tools, apps, local models) for agents it starts. Tasks always get it. You can add your own notes at the end of the list.");
            });
            if ui.button("Open List").clicked() {
                cx.emit(UiAction::OpenToolkit);
            }
            if ui.button("Refresh").clicked() {
                cx.emit(UiAction::RefreshToolkit);
            }
        });
        ui.add_space(10.0);
    });
    group(ui, Some("Exact status from agent hooks"), |ui| hooks(ui, cx));
    group(ui, Some("Notifications"), |ui| {
        toggle(ui, cx, "Notify me when an agent needs me", cx.model.prefs.notifications, SettingChange::NotificationsEnabled);
        if cx.model.prefs.notifications && cx.model.system.notifications_allowed == Some(false) {
            row(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                c::paint_icon(ui.painter(), rect, Icon::Warning, t::AMBER);
                c::label(ui, "Off in System Settings", 12.0, Weight::Medium, t::AMBER);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Open System Settings…").clicked() {
                        cx.emit(UiAction::OpenNotificationSettings);
                    }
                });
            });
            caption(
                ui,
                "Your desktop is blocking Lantern's notifications. Allow them for October Lantern in your system's notification settings.",
            );
        }
        ui.add_enabled_ui(cx.model.prefs.notifications, |ui| {
            toggle(ui, cx, "Also when an agent finishes its turn", cx.model.prefs.notify_finished, SettingChange::NotifyOnFinish)
        });
        caption(
            ui,
            "Questions and permission prompts always notify (when notifications are on). Notifications are skipped for agents you've muted in Agents.",
        );
        if cx.model.system.notifications_allowed != Some(false) && button_row(ui, "Open Notification Settings…") {
            cx.emit(UiAction::OpenNotificationSettings);
        }
    });
}

fn hooks(ui: &mut Ui, cx: &mut Ctx) {
    let mut any = false;
    let mut installed = true;
    let mut outdated = false;
    for (kind, title) in [("claude", "Claude Code"), ("codex", "Codex")] {
        let state = cx.model.system.hooks.get(kind).copied().unwrap_or_default();
        any |= state != HookState::Off;
        installed &= state == HookState::On;
        outdated |= state == HookState::NeedsUpdate;
        row(ui, |ui| {
            c::label(ui, title, 13.0, Weight::Regular, t::INK);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, color, icon) = match state {
                    HookState::On => ("On", t::GREEN, Some(Icon::CheckCircle)),
                    HookState::NeedsUpdate => ("Needs update", t::AMBER, Some(Icon::Refresh)),
                    HookState::Off => ("Off", t::MUTED, None),
                };
                c::label(ui, label, 12.0, Weight::Regular, color);
                if let Some(icon) = icon {
                    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    c::paint_icon(ui.painter(), rect, icon, color);
                }
            });
        });
    }
    row(ui, |ui| {
        if cx.model.system.hooks_busy {
            c::spinner(ui, 12.0, t::MUTED, cx.env);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled_ui(!cx.model.system.hooks_busy, |ui| {
                if !installed
                    && ui
                        .button(if outdated {
                            "Update"
                        } else if any {
                            "Turn On for Both"
                        } else {
                            "Turn On"
                        })
                        .clicked()
                {
                    cx.emit(UiAction::SetHooks { enabled: true });
                }
                if any && ui.button("Turn Off").clicked() {
                    cx.emit(UiAction::SetHooks { enabled: false });
                }
            });
        });
    });
    if let Some(error) = &cx.model.system.hooks_error {
        divider(ui);
        ui.add_space(10.0);
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            c::paint_icon(ui.painter(), rect, Icon::Warning, t::AMBER);
            c::label(ui, error, 11.0, Weight::Regular, t::AMBER);
        });
        ui.add_space(18.0);
    }
    caption(
        ui,
        "Claude Code and Codex tell Lantern the moment they finish or need permission. Lantern backs up ~/.claude/settings.json and ~/.codex/config.toml before changing them, and restores them when you turn this off. Restart running agents afterwards.",
    );
}

fn agents(ui: &mut Ui, cx: &mut Ctx) {
    group(ui, Some("Count in Waiting and notify"), |ui| {
        for kind in [
            "claude", "codex", "opencode", "pi", "october", "gemini", "grok", "cursor", "qwen", "goose", "aider", "amp", "copilot", "kimi",
            "droid", "crush", "auggie", "muse",
        ] {
            let kind = AgentKind::new(kind);
            if let Some(counts) = setting_switch(ui, &kind.display_name(), cx.model.prefs.counts(&kind), Some((cx.assets, &kind))) {
                cx.emit(UiAction::ChangeSetting(SettingChange::AgentMuted { kind: kind.clone(), muted: !counts }));
            }
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(ROW_PAD);
        c::label(ui, "Turned-off agents still appear in the Agents list.", 11.0, Weight::Regular, t::MUTED);
    });
}

fn october(ui: &mut Ui, cx: &mut Ctx) {
    ui.painter().rect_filled(ui.max_rect().expand(20.0), 0, Color32::from_gray(28));
    ui.spacing_mut().item_spacing.y = 14.0;
    c::label(
        ui,
        "Lantern works on its own. Signing in to October adds Ask and Point & Ask answers; October Desktop shows its agents here.",
        12.0,
        Weight::Regular,
        t::MUTED,
    );
    cards::account::render(ui, cx);
    own_key(ui, cx);
    cards::desktop::render(ui, cx, false);
}

/// "Your own AI key" (Swift `OwnKeySection`): saved, it shows the provider, its last four
/// characters and the model, never the key; otherwise (or editing) the form.
fn own_key(ui: &mut Ui, cx: &mut Ctx) {
    use crate::model::KeyStatus;
    use crate::own_key::{AiProvider, OwnKey};
    let state = &cx.model.own_key;
    group(ui, Some("Your own AI key"), |ui| {
        match &state.status {
            KeyStatus::Loading => {
                row(ui, |ui| c::label(ui, "Reading the system keyring…", 13.0, Weight::Regular, t::MUTED));
            }
            KeyStatus::Saved { provider, ending, model } if !cx.view.own_key_editing => {
                row(ui, |ui| {
                    let model = if model.is_empty() { String::new() } else { format!(" · {model}") };
                    c::label(ui, format!("{} key {ending}{model}", provider.label()), 13.0, Weight::Regular, t::INK);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(!state.busy, egui::Button::new("Remove key")).clicked() {
                            cx.emit(UiAction::RemoveOwnKey);
                        }
                        if ui.add_enabled(!state.busy, egui::Button::new("Edit")).clicked() {
                            cx.view.own_key_editing = true;
                            cx.view.own_key_provider = *provider;
                            cx.view.own_key_model = model.trim_start_matches(" · ").to_string();
                        }
                    });
                });
            }
            _ => {
                if state.status == KeyStatus::Unavailable {
                    caption(ui, crate::app::UNAVAILABLE_KEYRING);
                }
                row(ui, |ui| {
                    for provider in AiProvider::ALL {
                        ui.selectable_value(&mut cx.view.own_key_provider, provider, provider.label());
                    }
                });
                let provider = cx.view.own_key_provider;
                row(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut cx.view.own_key_input)
                            .id_salt("own-key-input")
                            .password(true)
                            .hint_text(format!("{} API key", provider.label()))
                            .desired_width(ui.available_width()),
                    );
                });
                row(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut cx.view.own_key_model)
                            .id_salt("own-key-model")
                            .hint_text("Model (optional; empty uses October's default)")
                            .desired_width(ui.available_width()),
                    );
                });
                row(ui, |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let key = OwnKey::new(provider, &cx.view.own_key_input, &cx.view.own_key_model);
                        let title = if state.busy { "Saving…" } else { "Save Key" };
                        if ui.add_enabled(key.is_some() && !state.busy, egui::Button::new(title)).clicked()
                            && let Some(key) = key
                        {
                            cx.emit(UiAction::SaveOwnKey(key));
                        }
                        if cx.view.own_key_editing && ui.button("Cancel").clicked() {
                            cx.view.own_key_editing = false;
                            cx.view.own_key_input.clear();
                        }
                    });
                });
            }
        }
        if let Some(error) = &state.error {
            divider(ui);
            ui.add_space(10.0);
            c::label(ui, error, 11.0, Weight::Regular, t::RED);
            ui.add_space(10.0);
        }
        caption(ui, OWN_KEY_DETAIL);
        let signed_in = cx.model.account.signed_in.is_some();
        if signed_in && cx.model.lantern_rights().is_some_and(|rights| !rights.byok) {
            caption(ui, "Your own key needs Lantern Lifetime or the Pro trial; until then October's AI answers.");
            ui.add_space(2.0);
            super::assistant::offer_buttons(ui, cx, "own_key");
            ui.add_space(10.0);
        }
    });
}

/// Under your own key, always.
const OWN_KEY_DETAIL: &str = "Your provider bills you for these questions. Lantern sends your key to October's server only to pass each question on; it's never stored.";

fn about(ui: &mut Ui, cx: &mut Ctx) {
    group(ui, None, |ui| {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            ui.add(egui::Image::new(&cx.assets.logo).fit_to_exact_size(vec2(52.0, 52.0)));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                c::label(ui, "October Lantern", 17.0, Weight::Bold, t::INK);
                c::label(ui, format!("Version {}", cx.model.system.version), 13.0, Weight::Regular, t::MUTED);
            });
        });
        ui.add_space(14.0);
        let newer = cx.model.system.update.as_ref().is_some_and(|update| update.newer);
        let title = if cx.model.system.checking_updates {
            "Checking…"
        } else if newer {
            "Install Update…"
        } else {
            "Check for Updates…"
        };
        if row(ui, |ui| ui.add_enabled(!cx.model.system.checking_updates, egui::Button::new(title)).clicked()) {
            cx.emit(UiAction::CheckUpdates);
        }
        if let Some(update) = &cx.model.system.update {
            caption(ui, &if update.newer { format!("Version {} is available.", update.version) } else { "You're up to date.".into() });
        }
    });
    if crate::analytics::available() {
        group(ui, None, |ui| {
            if let Some(on) = setting_switch(ui, "Share usage counts", crate::analytics::enabled(), None) {
                cx.emit(UiAction::SetShareUsage(on));
            }
            caption(ui, USAGE_DETAIL);
        });
    }
    group(ui, Some("Help"), |ui| {
        if button_row(ui, "Report a Problem…") {
            cx.emit(UiAction::ReportProblem);
        }
        if button_row(ui, "Send Logs…") {
            cx.emit(UiAction::SendLogs);
        }
        if let Some(logs) = &cx.model.system.logs {
            caption(ui, &logs.summary());
        }
        if button_row(ui, "Show Welcome Guide") {
            cx.emit(UiAction::ShowWelcome);
        }
        if row(ui, |ui| ui.link("lantern.october.dev").clicked()) {
            cx.emit(UiAction::OpenUrl("https://lantern.october.dev".into()));
        }
        if row(ui, |ui| ui.link("hey@october.dev").clicked()) {
            cx.emit(UiAction::OpenUrl("mailto:hey@october.dev".into()));
        }
    });
    group(ui, None, |ui| {
        // Neutral like the Mac's Form button; only the confirmation's Uninstall is red.
        if button_row(ui, "Uninstall October Lantern…") {
            cx.view.settings_uninstall = true;
        }
        caption(
            ui,
            &format!(
                "Signs out of October, removes the hooks (restoring your agents' settings), support files and settings, then {}.",
                removal(cx)
            ),
        );
    });
}

fn removal(cx: &Ctx) -> &'static str {
    if cx.model.caps.platform == Platform::Windows { "removes the app" } else { "moves the app to the Trash" }
}

fn confirmation(ui: &mut Ui, cx: &mut Ctx) {
    if cx.model.system.uninstall != Uninstall::Idle {
        cx.view.settings_uninstall = false;
        uninstall_status(ui, cx);
        return;
    }
    if !cx.view.settings_uninstall {
        return;
    }
    let response = egui::Modal::new(ui.id().with("settings-uninstall")).show(ui.ctx(), |ui| {
        ui.set_width(380.0);
        c::label(ui, "Uninstall October Lantern?", 16.0, Weight::Semibold, t::INK);
        c::label(ui, format!("This signs out of October, removes Lantern's hooks from Claude Code and Codex (restoring your previous settings), its support files and its settings, then {}. Your agents and any sessions Lantern started keep running.", removal(cx)), 12.0, Weight::Regular, t::MUTED);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Cancel").clicked() { cx.view.settings_uninstall = false; }
            if ui.button(c::text("Uninstall", 12.0, Weight::Semibold, t::RED)).clicked() { cx.view.settings_uninstall = false; cx.emit(UiAction::ConfirmUninstall); }
        });
    });
    if response.should_close() {
        cx.view.settings_uninstall = false;
    }
}

fn uninstall_status(ui: &mut Ui, cx: &mut Ctx) {
    let mut dismiss = false;
    let response = egui::Modal::new(ui.id().with("settings-uninstall-status")).show(ui.ctx(), |ui| {
        ui.set_width(380.0);
        match &cx.model.system.uninstall {
            Uninstall::Idle => {}
            Uninstall::Removing => {
                c::label(ui, "Uninstalling October Lantern…", 16.0, Weight::Semibold, t::INK);
                ui.horizontal(|ui| {
                    c::spinner(ui, 12.0, t::MUTED, cx.env);
                    c::label(ui, "Signing out and removing Lantern's hooks, files and settings…", 12.0, Weight::Regular, t::MUTED);
                });
                caption(ui, "Your agents and running sessions are left alone.");
            }
            Uninstall::Partial { failures } => {
                c::label(ui, "Some of Lantern couldn't be removed", 16.0, Weight::Semibold, t::INK);
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    c::label(ui, failures.join("\n\n"), 12.0, Weight::Regular, t::MUTED);
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Keep the App").clicked() {
                        dismiss = true;
                    }
                    let title = if cx.model.caps.platform == Platform::Windows { "Remove App Anyway" } else { "Move App to Trash Anyway" };
                    if ui.button(title).clicked() {
                        cx.emit(UiAction::RemoveAppAnyway);
                    }
                });
            }
            Uninstall::AppFailed { message } => {
                let title = if cx.model.caps.platform == Platform::Windows {
                    "Couldn't remove the app"
                } else {
                    "Couldn't move the app to the Trash"
                };
                c::label(ui, title, 16.0, Weight::Semibold, t::INK);
                egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                    c::label(ui, message, 12.0, Weight::Regular, t::MUTED);
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("OK").clicked() {
                        dismiss = true;
                    }
                });
            }
        }
    });
    // Closing a failure alert keeps the app. Cleanup already in flight cannot be cancelled
    // merely by clicking the backdrop or pressing Escape.
    if cx.model.system.uninstall != Uninstall::Removing && (dismiss || response.should_close()) {
        cx.emit(UiAction::DismissUninstall);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::fixture::Fixture,
        model::{Inputs, Model},
        ui::{Assets, Env, UiState},
    };

    struct Screen {
        ctx: egui::Context,
        assets: Assets,
        model: Model,
        inputs: Inputs,
        view: UiState,
    }

    impl Screen {
        fn new(tab: usize) -> Self {
            let ctx = egui::Context::default();
            t::install(&ctx);
            let assets = Assets::load(&ctx);
            Self {
                ctx,
                assets,
                model: Model::default(),
                inputs: Inputs::default(),
                view: UiState { settings_tab: tab, ..Default::default() },
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> (Vec<UiAction>, egui::FullOutput) {
            let mut actions = Vec::new();
            let mut width = 0.0;
            let mut output = self.ctx.run_ui(
                egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, SIZE)), events, ..Default::default() },
                |ui| {
                    let mut cx = Ctx {
                        model: &self.model,
                        inputs: &mut self.inputs,
                        view: &mut self.view,
                        out: &mut actions,
                        assets: &self.assets,
                        env: &Env::default(),
                    };
                    assert_eq!(super::super::render_settings(ui, &mut cx), 480.0);
                    width = ui.min_rect().width();
                },
            );
            output.textures_delta.clear();
            assert!(width <= 520.5, "settings width {width}; texts {:?}", texts(&output));
            (actions, output)
        }

        fn click(&mut self, title: &str) -> Vec<UiAction> {
            // A newly opened popup has one sizing frame before its settled position.
            self.frame(Vec::new());
            let (_, output) = self.frame(Vec::new());
            let texts = texts(&output);
            let pos = texts.iter().find(|(text, _)| text == title).unwrap_or_else(|| panic!("missing {title}: {texts:?}")).1.center();
            let mut actions = Vec::new();
            for pressed in [true, false] {
                actions.extend(
                    self.frame(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                    ])
                    .0,
                );
            }
            actions
        }
    }

    fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
        fn collect(shape: &egui::Shape, clip: Rect, out: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    if clip.contains(rect.center()) {
                        out.push((text.galley.job.text.clone(), rect));
                    }
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, clip, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for shape in &output.shapes {
            collect(&shape.shape, shape.clip_rect, &mut out);
        }
        out
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn tabs_keep_the_four_source_surfaces_and_keyboard_activation() {
        let mut screen = Screen::new(0);
        for (index, title, content) in [
            (0, "General", "Open at login"),
            (1, "Agents", "Count in Waiting and notify"),
            (2, "October", "Connect to October"),
            (3, "About", "October Lantern"),
        ] {
            assert!(screen.click(title).is_empty());
            assert_eq!(screen.view.settings_tab, index);
            assert!(texts(&screen.frame(Vec::new()).1).iter().any(|(text, _)| text == content));
        }
        screen.ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(("settings-tab", 0usize))));
        screen.frame(vec![key(egui::Key::Space)]);
        assert_eq!(screen.view.settings_tab, 0);
        screen.frame(vec![key(egui::Key::Tab)]);
        screen.frame(vec![key(egui::Key::Space)]);
        assert_eq!(screen.view.settings_tab, 1);
    }

    #[test]
    fn preferences_emit_intents_without_mutating_the_domain() {
        let mut screen = Screen::new(0);
        let prefs = screen.model.prefs.clone();
        assert_eq!(screen.click("Open at login"), vec![UiAction::ChangeSetting(SettingChange::OpenAtLogin(!prefs.open_at_login))]);
        assert_eq!(screen.model.prefs, prefs);
        screen.ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(("settings-switch", "Open at login"))));
        assert_eq!(
            screen.frame(vec![key(egui::Key::Space)]).0,
            vec![UiAction::ChangeSetting(SettingChange::OpenAtLogin(!prefs.open_at_login))]
        );
        screen.click("Right edge");
        assert_eq!(screen.click("Left edge"), vec![UiAction::ChangeSetting(SettingChange::Edge(Edge::Left))]);
        screen.view.settings_tab = 1;
        assert_eq!(
            screen.click("Claude Code"),
            vec![UiAction::ChangeSetting(SettingChange::AgentMuted { kind: AgentKind::new("claude"), muted: true })]
        );
        assert_eq!(screen.model.prefs, prefs);
    }

    #[test]
    fn hooks_and_disabled_notifications_respect_busy_and_enabled_states() {
        let mut screen = Screen::new(0);
        screen.view.settings_scroll = Some(570.0);
        assert_eq!(screen.click("Turn On"), vec![UiAction::SetHooks { enabled: true }]);
        screen.model.system.hooks_busy = true;
        assert!(screen.click("Turn On").is_empty());
        screen.model.prefs.notifications = false;
        screen.view.settings_scroll = Some(1500.0);
        assert!(screen.click("Also when an agent finishes its turn").is_empty());
        screen.model.prefs.notifications = true;
        assert_eq!(
            screen.click("Also when an agent finishes its turn"),
            vec![UiAction::ChangeSetting(SettingChange::NotifyOnFinish(false))]
        );
    }

    #[test]
    fn uninstall_requires_explicit_confirmation_and_escape_cancels() {
        let mut screen = Screen::new(3);
        assert!(screen.click("Uninstall October Lantern…").is_empty());
        assert!(screen.view.settings_uninstall);
        assert!(screen.frame(vec![key(egui::Key::Enter)]).0.is_empty(), "Return must not implicitly remove the app");
        assert!(screen.frame(vec![key(egui::Key::Escape)]).0.is_empty());
        assert!(!screen.view.settings_uninstall);
        screen.click("Uninstall October Lantern…");
        assert!(screen.click("Cancel").is_empty());
        assert!(!screen.view.settings_uninstall);
        screen.click("Uninstall October Lantern…");
        assert_eq!(screen.click("Uninstall"), vec![UiAction::ConfirmUninstall]);
        assert!(!screen.view.settings_uninstall);
    }

    #[test]
    fn update_button_discloses_installing_and_is_disabled_while_checking() {
        let mut screen = Screen::new(3);
        assert_eq!(screen.click("Check for Updates…"), vec![UiAction::CheckUpdates]);
        screen.model.system.update = Some(crate::model::UpdateInfo { version: "0.8.3".into(), newer: true });
        assert_eq!(screen.click("Install Update…"), vec![UiAction::CheckUpdates]);
        screen.model.system.checking_updates = true;
        assert!(screen.click("Checking…").is_empty());
    }

    #[test]
    fn partial_cleanup_needs_a_second_explicit_remove_intent_and_progress_is_not_cancellable() {
        let mut screen = Screen::new(3);
        screen.model.system.uninstall =
            Uninstall::Partial { failures: vec!["Hooks could not be restored. Restore the saved backup manually.".into()] };
        assert!(screen.frame(vec![key(egui::Key::Enter)]).0.is_empty());
        assert_eq!(screen.click("Keep the App"), vec![UiAction::DismissUninstall]);
        assert_eq!(screen.click("Move App to Trash Anyway"), vec![UiAction::RemoveAppAnyway]);
        screen.model.caps.platform = Platform::Windows;
        assert_eq!(screen.click("Remove App Anyway"), vec![UiAction::RemoveAppAnyway]);
        screen.model.system.uninstall = Uninstall::Removing;
        assert!(screen.frame(vec![key(egui::Key::Escape)]).0.is_empty());
        assert!(texts(&screen.frame(Vec::new()).1).iter().any(|(text, _)| text == "Uninstalling October Lantern…"));
        screen.model.system.uninstall = Uninstall::AppFailed { message: "Development builds are left in place.".into() };
        assert_eq!(screen.click("OK"), vec![UiAction::DismissUninstall]);
    }

    #[test]
    fn point_ask_shortcut_help_describes_selection_and_both_enter_routes() {
        for target in ["Point & Ask (hold, point, speak)", "Ctrl+Alt+P"] {
            let mut screen = Screen::new(0);
            screen.ctx.global_style_mut(|style| {
                style.interaction.tooltip_delay = 0.0;
                style.interaction.show_tooltips_only_when_still = false;
            });
            screen.frame(Vec::new());
            let output = screen.frame(Vec::new()).1;
            let pos = texts(&output).into_iter().find(|(text, _)| text == target).unwrap().1.center();
            screen.frame(vec![egui::Event::PointerMoved(pos)]);
            let (actions, output) = screen.frame(vec![egui::Event::PointerMoved(pos)]);
            assert!(actions.is_empty(), "help must not change the shortcut or submit a question");
            let help =
                texts(&output).into_iter().find(|(text, _)| text == POINT_ASK_HELP).expect("hovering the label or picker shows help").1;
            assert!(Rect::from_min_size(egui::Pos2::ZERO, SIZE).contains_rect(help), "help remains within the Settings window: {help:?}");
        }
    }

    #[test]
    fn your_own_key_shows_only_its_ending_and_saves_what_was_typed() {
        use crate::model::KeyStatus;
        use crate::own_key::{AiProvider, OwnKey};
        let mut screen = Screen::new(2);
        screen.model.own_key.status = KeyStatus::Missing;
        let shown = texts(&screen.frame(Vec::new()).1);
        let has = |texts: &Vec<(String, Rect)>, value: &str| texts.iter().any(|(text, _)| text == value);
        assert!(has(&shown, "Your own AI key") && has(&shown, OWN_KEY_DETAIL));
        assert!(screen.click("Save Key").is_empty(), "nothing to save yet");
        screen.view.own_key_input = "sk-test-abcd".into();
        screen.view.own_key_provider = AiProvider::Anthropic;
        assert_eq!(screen.click("Save Key"), vec![UiAction::SaveOwnKey(OwnKey::new(AiProvider::Anthropic, "sk-test-abcd", "").unwrap())]);
        screen.model.own_key.status = KeyStatus::Saved { provider: AiProvider::OpenAI, ending: "…abcd".into(), model: "gpt-4.1".into() };
        screen.view.own_key_input.clear();
        let shown = texts(&screen.frame(Vec::new()).1);
        assert!(has(&shown, "OpenAI key …abcd · gpt-4.1") && !shown.iter().any(|(text, _)| text.contains("sk-")));
        assert_eq!(screen.click("Remove key"), vec![UiAction::RemoveOwnKey]);
        screen.model.own_key.status = KeyStatus::Unavailable;
        assert!(has(&texts(&screen.frame(Vec::new()).1), crate::app::UNAVAILABLE_KEYRING));
    }

    #[test]
    fn source_tabs_and_agent_switches_have_accessible_names() {
        let mut screen = Screen::new(1);
        screen.ctx.enable_accesskit();
        let tree = screen.frame(Vec::new()).1.platform_output.accesskit_update.unwrap();
        for label in ["General", "Agents", "October", "About", "Claude Code", "Codex", "OpenCode"] {
            assert!(tree.nodes.iter().any(|(_, node)| node.label() == Some(label)), "missing accessible control {label}");
        }
        assert!(tree.nodes.iter().any(|(_, node)| node.label() == Some("Claude Code") && node.role() == egui::accesskit::Role::Switch));
        let row = screen.ctx.read_response(egui::Id::new(("settings-switch", "Claude Code"))).unwrap().rect;
        let label = texts(&screen.frame(Vec::new()).1).into_iter().find(|(text, _)| text == "Claude Code").unwrap().1;
        assert!(label.left() < row.center().x && label.right() < row.right() - 34.0, "label leads the trailing switch");
        assert!((label.center().y - row.center().y).abs() <= 0.5, "agent label and switch share a center: {label:?}, {row:?}");
        screen.view.settings_tab = 0;
        let output = screen.frame(Vec::new()).1;
        let row = screen.ctx.read_response(egui::Id::new(("settings-switch", "Open at login"))).unwrap().rect;
        let label = texts(&output).into_iter().find(|(text, _)| text == "Open at login").unwrap().1;
        assert!((label.center().y - row.center().y).abs() <= 0.5, "general label and switch share a center: {label:?}, {row:?}");
    }

    #[test]
    fn settings_fixtures_use_the_real_fixed_window_without_horizontal_overflow() {
        for raw in [
            include_str!("../../fixtures/settings-general.json"),
            include_str!("../../fixtures/settings-hooks.json"),
            include_str!("../../fixtures/settings-hooks-busy.json"),
            include_str!("../../fixtures/settings-notifications.json"),
            include_str!("../../fixtures/settings-notifications-off.json"),
            include_str!("../../fixtures/settings-agents.json"),
            include_str!("../../fixtures/settings-agents-bottom.json"),
            include_str!("../../fixtures/settings-october.json"),
            include_str!("../../fixtures/settings-october-connected.json"),
            include_str!("../../fixtures/settings-about.json"),
            include_str!("../../fixtures/settings-update.json"),
            include_str!("../../fixtures/settings-checking.json"),
            include_str!("../../fixtures/settings-uninstall.json"),
            include_str!("../../fixtures/settings-uninstall-removing.json"),
            include_str!("../../fixtures/settings-uninstall-partial.json"),
            include_str!("../../fixtures/settings-uninstall-failed.json"),
        ] {
            let fixture: Fixture = serde_json::from_str(raw).unwrap();
            let mut screen = Screen::new(0);
            screen.model = fixture.model;
            screen.view = fixture.view;
            screen.inputs = fixture.inputs;
            for _ in 0..3 {
                let (actions, output) = screen.frame(Vec::new());
                assert!(actions.is_empty(), "render-only fixtures must never cause side effects");
                for (text, rect) in texts(&output) {
                    assert!(rect.right() <= 520.5 && rect.left() >= -0.5, "horizontal overflow {text}: {rect:?}");
                }
            }
        }
    }
}
