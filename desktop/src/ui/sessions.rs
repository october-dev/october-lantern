//! SessionViews.swift: the source form order, agent chips, model search and launch footer.
//! No capture, engine, folder creation or launch effects happen here; those are UiActions.
use super::{
    Ctx, SessionRequest, UiAction, components as c,
    icons::Icon,
    theme::{self as t, Weight},
};
use crate::model::{AgentKind, AppContext, Capture, ModelList};
use eframe::egui::{self, Align, Layout, Margin, Rect, Response, Sense, Stroke, Ui};
use std::path::{Path, PathBuf};

/// Dynamic screenshots are decoded once per changed path, including failed decodes. Not part of
/// fixture state; a fixture can name an existing local capture, but never triggers a capture.
#[derive(Default)]
pub struct CaptureTexture {
    path: Option<PathBuf>,
    image: Option<egui::TextureHandle>,
    error: Option<String>,
}

impl std::fmt::Debug for CaptureTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureTexture").field("path", &self.path).field("error", &self.error).finish_non_exhaustive()
    }
}

impl CaptureTexture {
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> Result<&egui::TextureHandle, &str> {
        if self.path.as_deref() != Some(path) {
            self.path = Some(path.to_owned());
            self.image = None;
            self.error = None;
            match image::open(path) {
                Ok(image) => {
                    let image = image.to_rgba8();
                    let size = [image.width() as usize, image.height() as usize];
                    self.image = Some(ctx.load_texture(
                        "capture-preview",
                        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                Err(_) => self.error = Some("The screenshot preview couldn't be opened. Retake it, or turn it off.".into()),
            }
        }
        self.image.as_ref().ok_or_else(|| self.error.as_deref().unwrap_or("The screenshot isn't ready."))
    }
}

pub fn panel_title(ui: &mut Ui, cx: &mut Ctx, title: &str) {
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 10 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(egui::text::LayoutJob::single_section(
                    title.to_uppercase(),
                    egui::TextFormat {
                        font_id: t::font(11.0, Weight::Semibold),
                        color: t::INK,
                        extra_letter_spacing: 0.8,
                        ..Default::default()
                    },
                ))
                .extend(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if c::icon_button(ui, ("close", title), Icon::Close, 22.0, t::MUTED, false).on_hover_text("Close").clicked() {
                    cx.emit(UiAction::SetPanel(None));
                }
            });
        });
    });
}

pub fn render(ui: &mut Ui, cx: &mut Ctx, max_height: f32) {
    panel_title(ui, cx, "New session");
    let kind = selected_kind(cx).cloned();
    if cx.view.model_search_kind != kind {
        cx.view.model_search_kind = kind.clone();
        cx.view.searching_models = false;
        cx.view.model_query.clear();
    }
    egui::ScrollArea::vertical()
        .id_salt("new-session-form")
        .max_height((max_height - 150.0).max(220.0))
        .min_scrolled_height(0.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 4 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                prompt(ui, cx);
                section(ui, "Agent", |ui| agents(ui, cx, kind.as_ref()));
                if let Some(kind) = &kind {
                    section(ui, "Model", |ui| model(ui, cx, kind));
                }
                section(ui, "Folder", |ui| folder(ui, cx));
                section(ui, "Context", |ui| context(ui, cx));
                section(ui, "Open in", |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let width = (ui.available_width() - 8.0) / 2.0;
                        let terminal = cx.inputs.new_session.open_in_terminal || !cx.model.new_session.in_lantern_available;
                        if choice(ui, "Terminal window", Icon::Window, terminal, true, width).clicked() {
                            cx.emit(UiAction::SetOpenInTerminal(true));
                        }
                        if choice(ui, "In Lantern", Icon::Chat, !terminal, cx.model.new_session.in_lantern_available, width).clicked() {
                            cx.emit(UiAction::SetOpenInTerminal(false));
                        }
                    })
                    .response
                    .on_hover_text(if cx.model.new_session.in_lantern_available {
                        "In Lantern: the session runs in the background and its chat opens here; click Open to see it in its terminal."
                    } else {
                        "Background sessions are unavailable. You can still open a Terminal window."
                    });
                });
            });
        });
    ui.add_space(10.0);
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 0, bottom: 16 }).show(ui, |ui| footer(ui, cx));
}

fn section(ui: &mut Ui, title: &str, draw: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        c::label(ui, title, 11.0, Weight::Medium, t::MUTED);
        draw(ui);
    });
}

fn prompt(ui: &mut Ui, cx: &mut Ctx) {
    let hint = if task(cx) {
        format!("What should it do in {}?", cx.model.new_session.task_app.as_ref().map_or("this app", |app| app.name.as_str()))
    } else {
        "What should it work on? (optional)".into()
    };
    egui::Frame::new().fill(t::FAINT).stroke(Stroke::new(1.0, t::HAIRLINE)).corner_radius(10).inner_margin(Margin::symmetric(11, 8)).show(
        ui,
        |ui| {
            egui::ScrollArea::vertical().id_salt("session-prompt-scroll").max_height(6.0 * 16.0).min_scrolled_height(0.0).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut cx.inputs.new_session.prompt)
                        .id_salt("session-prompt")
                        .font(t::font(13.0, Weight::Regular))
                        .text_color(t::INK)
                        .hint_text(c::text(hint, 13.0, Weight::Regular, t::MUTED))
                        .desired_width(ui.available_width())
                        .desired_rows(3)
                        .frame(egui::Frame::NONE)
                        .margin(0)
                        .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)),
                );
            });
        },
    );
}

fn selected_kind<'a>(cx: &'a Ctx) -> Option<&'a AgentKind> {
    cx.inputs.new_session.kind.as_ref().or_else(|| cx.model.new_session.installed.as_ref()?.first())
}

fn agents(ui: &mut Ui, cx: &mut Ctx, selected: Option<&AgentKind>) {
    let Some(installed) = &cx.model.new_session.installed else {
        loading(ui, "Looking for installed agents…", 12.0, cx.env);
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if installed.is_empty() {
            c::label(ui, "No agents found on this computer yet.", 12.0, Weight::Regular, t::MUTED);
        }
        for kind in installed.iter().take(5) {
            if agent_chip(ui, cx, kind, selected == Some(kind)).clicked() {
                cx.emit(UiAction::ChooseSessionAgent { kind: kind.clone() });
            }
        }
        let others: Vec<_> = AgentKind::SUPPORTED
            .iter()
            .map(|kind| AgentKind::new(kind))
            .filter(|kind| !installed.contains(kind) && kind.website().is_some())
            .collect();
        if installed.len() > 5 || !others.is_empty() {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 28.0), Sense::click());
            ui.painter().rect_filled(rect, 14.0, t::FAINT);
            for x in [-3.5, 0.0, 3.5] {
                ui.painter().circle_filled(rect.center() + egui::vec2(x, 0.0), 0.9, t::MUTED);
            }
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "More agents"));
            let response = response.on_hover_text("More agents");
            egui::Popup::menu(&response).show(|ui| {
                for kind in installed.iter().skip(5) {
                    if ui.button(c::text(kind.display_name(), 12.0, Weight::Regular, t::INK)).clicked() {
                        cx.emit(UiAction::ChooseSessionAgent { kind: kind.clone() });
                    }
                }
                if installed.len() > 5 && !others.is_empty() {
                    ui.separator();
                }
                for kind in others {
                    if ui.button(c::text(format!("Get {}…", kind.display_name()), 12.0, Weight::Regular, t::INK)).clicked() {
                        cx.emit(UiAction::GetAgent { kind });
                    }
                }
            });
        }
    });
}

fn agent_chip(ui: &mut Ui, cx: &Ctx, kind: &AgentKind, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(kind.display_name(), t::font(11.5, Weight::Semibold), t::INK);
    let width = if selected { 18.0 + 5.0 + galley.size().x + 18.0 } else { 30.0 };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 28.0), Sense::click());
    ui.painter().rect(
        rect,
        14.0,
        t::FAINT,
        Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { t::opacity(t::AMBER, 0.7) } else { t::HAIRLINE }),
        egui::StrokeKind::Inside,
    );
    let icon = Rect::from_min_size(egui::pos2(rect.left() + if selected { 9.0 } else { 6.0 }, rect.top() + 5.0), egui::Vec2::splat(18.0));
    if let Some(texture) = cx.assets.harness(kind) {
        egui::Image::new(texture).corner_radius(4).paint_at(ui, icon);
    } else {
        c::paint_icon(ui.painter(), icon, Icon::Terminal, t::INK);
    }
    if selected {
        ui.painter().galley(egui::pos2(icon.right() + 5.0, rect.center().y - galley.size().y / 2.0), galley, t::INK);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, kind.display_name()));
    response.on_hover_text(kind.display_name())
}

fn model(ui: &mut Ui, cx: &mut Ctx, kind: &AgentKind) {
    let Some(list) = cx.model.new_session.models.get(kind.as_str()) else {
        loading(ui, &format!("Loading {}'s models…", kind.display_name()), 11.5, cx.env);
        return;
    };
    if !list.choosable {
        c::label(ui, format!("{} starts with its own default model.", kind.display_name()), 11.5, Weight::Regular, t::MUTED);
        return;
    }
    let label = cx
        .inputs
        .new_session
        .model
        .as_ref()
        .map(|chosen| {
            list.models
                .iter()
                .find(|m| &m.id == chosen)
                .map_or_else(|| chosen.clone(), |m| m.group.as_ref().map_or_else(|| m.label.clone(), |g| format!("{} · {g}", m.label)))
        })
        .unwrap_or_else(|| "Default".into());
    let response = field(ui, "session-model", Icon::Cpu, &label).on_hover_text(format!("Model: {label}"));
    egui::Popup::menu(&response).show(|ui| {
        if ui.button("Default").clicked() {
            pick_model(cx, kind, None);
        }
        if !list.models.is_empty() && list.models.len() <= 25 {
            ui.separator();
            for option in &list.models {
                if ui.button(&option.label).clicked() {
                    pick_model(cx, kind, Some(option.id.clone()));
                }
            }
        }
        ui.separator();
        let label = if list.models.len() > 25 { format!("Search {} models…", list.models.len()) } else { "Other model…".into() };
        if ui.button(label).clicked() {
            cx.view.searching_models = true;
        }
    });
    if cx.view.searching_models {
        model_search(ui, cx, kind, list);
    }
}

fn model_search(ui: &mut Ui, cx: &mut Ctx, kind: &AgentKind, list: &ModelList) {
    ui.spacing_mut().item_spacing.y = 2.0;
    egui::Frame::new().fill(t::FAINT).corner_radius(8).inner_margin(Margin::symmetric(10, 7)).show(ui, |ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut cx.view.model_query)
                .id_salt("model-query")
                .font(t::font(12.5, Weight::Regular))
                .hint_text(c::text("Model name or id", 12.5, Weight::Regular, t::MUTED))
                .frame(egui::Frame::NONE)
                .margin(0)
                .desired_width(ui.available_width()),
        );
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let query = cx.view.model_query.trim().to_owned();
            if !query.is_empty() {
                pick_model(cx, kind, Some(query));
            }
        }
    });
    let query = cx.view.model_query.trim().to_owned();
    for option in model_matches(list, &query) {
        let response = egui::Frame::new()
            .inner_margin(Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    c::label(ui, &option.label, 12.0, Weight::Regular, t::white(0.9));
                    if let Some(group) = &option.group {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            c::label(ui, group, 12.0, Weight::Regular, t::MUTED);
                        });
                    }
                });
            })
            .response
            .interact(Sense::click());
        if response.clicked() {
            pick_model(cx, kind, Some(option.id.clone()));
        }
    }
    if !query.is_empty() && !list.models.iter().any(|m| m.id == query) {
        egui::Frame::new().inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
            if c::text_button(ui, &format!("Use “{query}”"), 12.0, Weight::Medium, t::AMBER).clicked() {
                pick_model(cx, kind, Some(query));
            }
        });
    }
}

fn model_matches<'a>(list: &'a ModelList, query: &str) -> Vec<&'a crate::model::ModelOption> {
    let lower = query.to_lowercase();
    list.models
        .iter()
        .filter(|m| lower.is_empty() || m.id.to_lowercase().contains(&lower) || m.label.to_lowercase().contains(&lower))
        .take(if lower.is_empty() { 6 } else { 8 })
        .collect()
}

fn pick_model(cx: &mut Ctx, kind: &AgentKind, model: Option<String>) {
    cx.emit(UiAction::ChooseSessionModel { kind: kind.clone(), model });
    cx.view.searching_models = false;
    cx.view.model_query.clear();
}

/// SessionViews.swift's model and folder menus: `.menuStyle(.borderlessButton)` with the
/// indicator hidden, so AppKit draws a plain row, just the symbol and the text (panel-new.png).
pub fn field(ui: &mut Ui, id: &str, icon: Icon, value: &str) -> Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 24.0), Sense::hover());
    let response = ui.interact(rect, ui.make_persistent_id(id), Sense::click());
    let ink = if response.is_pointer_button_down_on() { t::white(0.7) } else { t::INK };
    c::paint_icon(
        ui.painter(),
        Rect::from_center_size(egui::pos2(rect.left() + 11.0, rect.center().y), egui::Vec2::splat(14.0)),
        icon,
        ink,
    );
    let display = middle_truncate(ui, value, 13.0, Weight::Regular, width - 28.0);
    let galley = ui.painter().layout_no_wrap(display, t::font(13.0, Weight::Regular), ink);
    let text = Rect::from_min_size(egui::pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0), galley.size());
    ui.painter().galley(text.min, galley, ink);
    if response.has_focus() {
        let hit = Rect::from_min_max(rect.min, egui::pos2(text.right() + 4.0, rect.bottom()));
        ui.painter().rect_stroke(hit, 6.0, Stroke::new(1.0, t::opacity(t::AMBER, 0.6)), egui::StrokeKind::Inside);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), value));
    response
}

fn folder(ui: &mut Ui, cx: &mut Ctx) {
    let selected = selected_folder(cx);
    let response =
        field(ui, "session-folder", Icon::Folder, &selected.as_ref().map_or_else(|| "Choose a folder…".into(), |f| short(f, &cx.env.home)));
    let response = response
        .on_hover_text(selected.as_ref().map_or_else(|| "Choose a folder".into(), |f| format!("Runs in {}", short(f, &cx.env.home))));
    egui::Popup::menu(&response).show(|ui| {
        if task(cx) {
            if let Some(doc) = cx
                .model
                .new_session
                .task_app
                .as_ref()
                .and_then(|app| app.document.as_ref())
                .and_then(|p| parent_folder(&p.to_string_lossy()))
                && ui.button(format!("{} (the document's folder)", short(&doc, &cx.env.home))).clicked()
            {
                cx.inputs.new_session.folder = Some(doc);
            }
            if let Some(own) = tasks_folder(cx)
                && ui.button(format!("Lantern Tasks/{}", own.rsplit(['/', '\\']).next().unwrap_or("App"))).clicked()
            {
                cx.inputs.new_session.folder = Some(own);
            }
            ui.separator();
        }
        for folder in &cx.model.new_session.folders {
            if ui.button(short(folder, &cx.env.home)).clicked() {
                cx.inputs.new_session.folder = Some(folder.clone());
            }
        }
        if !cx.model.new_session.folders.is_empty() {
            ui.separator();
        }
        if ui.button("Choose Folder…").clicked() {
            cx.emit(UiAction::ChooseFolder);
        }
    });
}

fn context(ui: &mut Ui, cx: &mut Ctx) {
    if let Some(app) = cx.model.new_session.task_app.as_ref().filter(|app| !app.is_terminal) {
        let on = task(cx);
        let response = egui::Frame::new()
            .fill(if on { t::opacity(t::AMBER, 0.12) } else { t::FAINT })
            .stroke(Stroke::new(1.0, if on { t::opacity(t::AMBER, 0.45) } else { t::HAIRLINE }))
            .corner_radius(10)
            .inner_margin(Margin::symmetric(9, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if let Some(texture) = app.icon.as_deref().and_then(|path| cx.view.task_icon.get(ui.ctx(), path).ok()).cloned() {
                        ui.add(egui::Image::new(&texture).fit_to_exact_size(egui::Vec2::splat(20.0)));
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        c::label(ui, format!("Work on {}", app.name), 12.5, Weight::Semibold, if on { t::INK } else { t::MUTED });
                        if let Some(detail) = app
                            .document
                            .as_ref()
                            .map(|p| p.to_string_lossy().rsplit(['/', '\\']).next().unwrap_or("").to_owned())
                            .or_else(|| app.window.clone())
                            .filter(|v| !v.is_empty())
                        {
                            ui.add(egui::Label::new(c::text(detail, 10.5, Weight::Regular, t::MUTED)).truncate());
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(15.0), Sense::hover());
                        ui.painter().circle(
                            rect.center(),
                            7.0,
                            if on { t::AMBER } else { egui::Color32::TRANSPARENT },
                            Stroke::new(1.0, if on { t::AMBER } else { t::MUTED }),
                        );
                        if on {
                            c::paint_icon(ui.painter(), rect.shrink(3.0), Icon::Check, t::black(0.85));
                        }
                    });
                });
            })
            .response
            .interact(Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, format!("Work on {}", app.name)));
        if response
            .on_hover_text(if on {
                "The agent gets context about this app and a screenshot. Click for a plain session."
            } else {
                "Click to start this as a task for the app."
            })
            .clicked()
        {
            cx.inputs.new_session.task = !on;
            cx.emit(UiAction::SetSessionScreenshot(!on || cx.model.prefs.screenshot_new_sessions));
        }
    }
    let mut on = cx.inputs.new_session.screenshot;
    if switch(ui, "Include a screenshot of my screen", &mut on).changed() {
        cx.emit(UiAction::SetSessionScreenshot(on));
    }
    if cx.inputs.new_session.screenshot {
        match &cx.model.new_session.screenshot {
            Capture::Ready { path } => match cx.view.capture_texture.get(ui.ctx(), path) {
                Ok(texture) => {
                    let texture = texture.clone();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let image = ui.add(
                            egui::Image::new(&texture)
                                .fit_to_exact_size(egui::vec2(120.0, 120.0 / texture.aspect_ratio()))
                                .corner_radius(6),
                        );
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 6.0;
                            c::label(
                                ui,
                                "Saved on this computer and given to the agent with its first message.",
                                11.0,
                                Weight::Regular,
                                t::MUTED,
                            );
                            ui.painter().rect_stroke(image.rect, 6.0, Stroke::new(1.0, t::HAIRLINE), egui::StrokeKind::Inside);
                            if c::text_button(ui, "Retake", 11.0, Weight::Medium, t::AMBER).clicked() {
                                cx.emit(UiAction::RetakeSessionScreenshot);
                            }
                        });
                    });
                }
                Err(error) => {
                    c::label(ui, error, 11.0, Weight::Regular, t::MUTED);
                    if c::text_button(ui, "Retake", 11.0, Weight::Medium, t::AMBER).clicked() {
                        cx.emit(UiAction::RetakeSessionScreenshot);
                    }
                }
            },
            Capture::Capturing => loading(ui, "Taking a screenshot…", 11.0, cx.env),
            Capture::Failed { message } => {
                ui.horizontal_wrapped(|ui| {
                    c::label(ui, message, 11.0, Weight::Regular, t::MUTED);
                    if c::text_button(ui, "Try again", 11.0, Weight::Medium, t::AMBER).clicked() {
                        cx.emit(UiAction::RetakeSessionScreenshot);
                    }
                });
            }
            Capture::PermissionNeeded => {
                ui.horizontal_wrapped(|ui| {
                    c::label(ui, "To include a screenshot, allow screen capture.", 11.0, Weight::Regular, t::MUTED);
                    if c::text_button(ui, "Allow…", 11.0, Weight::Medium, t::AMBER).clicked() {
                        cx.emit(UiAction::RequestCapturePermission);
                    }
                });
            }
            Capture::None => {}
        }
    }
}

pub fn switch(ui: &mut Ui, title: &str, value: &mut bool) -> Response {
    let galley = ui.painter().layout_no_wrap(title.into(), t::font(12.5, Weight::Regular), t::white(0.9));
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2((galley.size().x + 8.0 + 44.0).min(ui.available_width()), 20.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let track = Rect::from_center_size(egui::pos2(rect.right() - 22.0, rect.center().y), egui::vec2(44.0, 20.0));
    ui.painter().rect_filled(track, 10.0, if *value { egui::Color32::from_rgb(0, 122, 255) } else { t::white(0.2) });
    let knob = Rect::from_center_size(
        egui::pos2(if *value { track.right() - 14.0 } else { track.left() + 14.0 }, track.center().y),
        egui::vec2(24.0, 16.0),
    );
    ui.painter().rect_filled(knob, 8.0, t::INK);
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, t::INK);
    if response.has_focus() {
        ui.painter().rect_stroke(track.expand(2.0), 9.0, Stroke::new(1.0, t::AMBER), egui::StrokeKind::Inside);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, title));
    response
}

pub fn choice(ui: &mut Ui, title: &str, icon: Icon, selected: bool, enabled: bool, width: f32) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 31.0), Sense::click());
        let color = if !enabled {
            t::white(0.22)
        } else if selected {
            t::INK
        } else {
            t::MUTED
        };
        ui.painter().rect(
            rect,
            10.0,
            if selected { t::FAINT } else { egui::Color32::TRANSPARENT },
            Stroke::new(1.0, if selected { t::opacity(t::AMBER, if enabled { 0.7 } else { 0.28 }) } else { t::HAIRLINE }),
            egui::StrokeKind::Inside,
        );
        let galley = ui.painter().layout_no_wrap(title.into(), t::font(12.0, Weight::Medium), color);
        let x = rect.center().x - (galley.size().x + 18.0) / 2.0;
        c::paint_icon(ui.painter(), Rect::from_center_size(egui::pos2(x + 6.0, rect.center().y), egui::Vec2::splat(12.0)), icon, color);
        ui.painter().galley(egui::pos2(x + 18.0, rect.center().y - galley.size().y / 2.0), galley, color);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, title));
        response
    })
    .inner
}

fn footer(ui: &mut Ui, cx: &mut Ctx) {
    ui.spacing_mut().item_spacing.y = 8.0;
    if let Some(error) = cx.view.session_error.as_ref().or(cx.model.new_session.launch_error.as_ref()) {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0), Sense::hover());
            c::paint_icon(ui.painter(), rect, Icon::Warning, t::RED);
            c::label(ui, error, 11.5, Weight::Regular, t::RED);
        });
    }
    let enabled = can_start(cx);
    let title = if cx.model.new_session.launching {
        "Starting…".into()
    } else if task(cx) {
        "Start task".into()
    } else {
        format!("Start {}", selected_kind(cx).map_or_else(|| "session".into(), AgentKind::display_name))
    };
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::click());
            ui.painter().rect_filled(rect, 10.0, t::opacity(t::AMBER, if enabled { 1.0 } else { 0.35 }));
            let galley = ui.painter().layout_no_wrap(title.clone(), t::font(13.0, Weight::Semibold), t::black(0.85));
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t::black(0.85));
            if response.has_focus() {
                ui.painter().rect_stroke(rect.shrink(2.0), 8.0, Stroke::new(1.0, t::INK), egui::StrokeKind::Inside);
            }
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &title));
            response
        })
        .inner;
    let response = if task(cx) && cx.inputs.new_session.prompt.trim().is_empty() && !cx.model.new_session.launching {
        let app = cx.model.new_session.task_app.as_ref().map_or("this app", |app| app.name.trim());
        response.on_disabled_hover_text(format!("Type what it should do in {app} first."))
    } else {
        response
    };
    let enter = c::consume_plain_key(ui, egui::Key::Enter);
    if enabled && (response.clicked() || enter) {
        cx.view.session_error = None;
        if cx.inputs.new_session.screenshot && !matches!(cx.model.new_session.screenshot, Capture::Ready { .. }) {
            cx.view.session_error = Some(match &cx.model.new_session.screenshot {
                Capture::Failed { message } => message.clone(),
                _ => "The screenshot isn't ready. Retake it, or turn it off.".into(),
            });
        } else if let Some(request) = launch_request(cx) {
            cx.emit(UiAction::Launch(Box::new(request)));
        }
    }
}

fn loading(ui: &mut Ui, title: &str, size: f32, env: &super::Env) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        c::spinner(ui, 12.0, t::MUTED, env);
        c::label(ui, title, size, Weight::Regular, t::MUTED);
    });
}

fn task(cx: &Ctx) -> bool {
    cx.inputs.new_session.task && cx.model.new_session.task_app.as_ref().is_some_and(|app| !app.is_terminal)
}

fn tasks_folder(cx: &Ctx) -> Option<String> {
    let app = cx.model.new_session.task_app.as_ref()?;
    let safe = app.name.replace(['/', ':', '\\'], "-");
    let safe = safe.trim();
    let slash = if cx.env.home.contains('\\') { '\\' } else { '/' };
    Some(format!(
        "{}{slash}Documents{slash}Lantern Tasks{slash}{}",
        cx.env.home.trim_end_matches(['/', '\\']),
        if safe.is_empty() { "App" } else { safe }
    ))
}

fn parent_folder(path: &str) -> Option<String> {
    let index = path.rfind(['/', '\\'])?;
    Some(if index == 0 || (index == 2 && path.as_bytes().get(1) == Some(&b':')) { path[..=index].into() } else { path[..index].into() })
}

fn selected_folder(cx: &Ctx) -> Option<String> {
    cx.inputs
        .new_session
        .folder
        .clone()
        .or_else(|| {
            if !task(cx) {
                return None;
            }
            let app = cx.model.new_session.task_app.as_ref()?;
            app.document
                .as_ref()
                .and_then(|p| parent_folder(&p.to_string_lossy()))
                .or_else(|| cx.model.prefs.task_folder_by_app.get(&app.id).cloned())
                .or_else(|| tasks_folder(cx))
        })
        .or_else(|| cx.model.new_session.folders.first().cloned())
}

fn short(path: &str, home: &str) -> String {
    if !home.is_empty() && (path == home || path.strip_prefix(home).is_some_and(|rest| rest.starts_with(['/', '\\']))) {
        format!("~{}", &path[home.len()..])
    } else {
        path.into()
    }
}

/// Swift's `.truncationMode(.middle)`, at Unicode scalar boundaries (never slice UTF-8 bytes).
fn middle_truncate(ui: &Ui, value: &str, size: f32, weight: Weight, width: f32) -> String {
    let fits = |text: &str| ui.painter().layout_no_wrap(text.into(), t::font(size, weight), t::INK).size().x <= width;
    if fits(value) {
        return value.into();
    }
    let chars: Vec<char> = value.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    let candidate = |kept: usize| -> String {
        chars[..kept.div_ceil(2)].iter().chain(std::iter::once(&'…')).chain(chars[chars.len() - kept / 2..].iter()).collect()
    };
    while low < high {
        let mid = (low + high).div_ceil(2);
        if fits(&candidate(mid)) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    candidate(low)
}

fn can_start(cx: &Ctx) -> bool {
    selected_kind(cx).is_some()
        && selected_folder(cx).is_some()
        && !cx.model.new_session.launching
        && !(cx.inputs.new_session.screenshot && matches!(cx.model.new_session.screenshot, Capture::Capturing))
        && (!task(cx) || !cx.inputs.new_session.prompt.trim().is_empty())
}

fn launch_request(cx: &Ctx) -> Option<SessionRequest> {
    if cx.inputs.new_session.screenshot && !matches!(cx.model.new_session.screenshot, Capture::Ready { .. }) {
        return None;
    }
    let kind = selected_kind(cx)?.clone();
    let form = &cx.inputs.new_session;
    let context: Option<AppContext> = task(cx).then(|| cx.model.new_session.task_app.clone()).flatten();
    Some(SessionRequest {
        model: cx.model.new_session.models.get(kind.as_str()).filter(|m| m.choosable).and(form.model.clone()),
        kind,
        folder: selected_folder(cx)?,
        prompt: form.prompt.clone(),
        screenshot: if form.screenshot {
            match &cx.model.new_session.screenshot {
                Capture::Ready { path } => Some(path.clone()),
                _ => None,
            }
        } else {
            None
        },
        task_app: context.as_ref().map(|app| if app.id.is_empty() { app.name.clone() } else { app.id.clone() }),
        context,
        toolkit: task(cx) || cx.model.prefs.toolkit_in_sessions,
        bus: cx.model.prefs.bus_for_sessions,
        background: cx.model.new_session.in_lantern_available && !form.open_in_terminal,
        for_turn: None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Assets, Env, UiState};
    use super::*;
    use crate::model::{EngineHealth, Inputs, Model, ModelOption, NewSessionModel, PanelMode};

    fn with_cx(model: &Model, inputs: &mut Inputs, f: impl FnOnce(&mut Ctx)) {
        let ctx = egui::Context::default();
        let assets = Assets::load(&ctx);
        f(&mut Ctx {
            model,
            inputs,
            assets: &assets,
            view: &mut UiState::default(),
            out: &mut Vec::new(),
            env: &Env { home: "/home/me".into(), ..Default::default() },
        });
    }

    fn model() -> Model {
        Model {
            panel: Some(PanelMode::NewSession),
            engine: EngineHealth::Ready,
            new_session: NewSessionModel {
                installed: Some(vec![AgentKind::new("claude")]),
                in_lantern_available: true,
                folders: vec!["/home/me/unrelated-project".into()],
                task_app: Some(AppContext { name: " DaVinci/Resolve:Editor ".into(), id: "resolve".into(), ..Default::default() }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn task_folder_never_falls_through_to_an_unrelated_project() {
        let mut model = model();
        let mut inputs = Inputs::default();
        inputs.new_session.task = true;
        with_cx(&model, &mut inputs, |cx| {
            assert_eq!(selected_folder(cx).as_deref(), Some("/home/me/Documents/Lantern Tasks/DaVinci-Resolve-Editor"))
        });
        model.prefs.task_folder_by_app.insert("resolve".into(), "/home/me/last-task".into());
        with_cx(&model, &mut inputs, |cx| assert_eq!(selected_folder(cx).as_deref(), Some("/home/me/last-task")));
        model.new_session.task_app.as_mut().unwrap().document = Some("C:\\Users\\me\\Videos\\clip.drp".into());
        with_cx(&model, &mut inputs, |cx| assert_eq!(selected_folder(cx).as_deref(), Some("C:\\Users\\me\\Videos")));
        inputs.new_session.folder = Some("/chosen".into());
        with_cx(&model, &mut inputs, |cx| assert_eq!(selected_folder(cx).as_deref(), Some("/chosen")));
        inputs.new_session.folder = None;
        inputs.new_session.task = false;
        with_cx(&model, &mut inputs, |cx| assert_eq!(selected_folder(cx).as_deref(), Some("/home/me/unrelated-project")));
        assert_eq!(parent_folder("/clip.drp").as_deref(), Some("/"));
        assert_eq!(parent_folder("C:\\clip.drp").as_deref(), Some("C:\\"));
        assert_eq!(short("/home/me-too/video", "/home/me"), "/home/me-too/video");
    }

    #[test]
    fn task_needs_a_prompt_and_capture_must_be_ready_before_a_launch_payload_exists() {
        let mut model = model();
        let mut inputs = Inputs::default();
        with_cx(&model, &mut inputs, |cx| assert!(can_start(cx)));
        inputs.new_session.task = true;
        inputs.new_session.prompt = " \n ".into();
        with_cx(&model, &mut inputs, |cx| assert!(!can_start(cx)));
        inputs.new_session.prompt = "Cut the pauses".into();
        inputs.new_session.screenshot = true;
        for capture in [Capture::None, Capture::PermissionNeeded, Capture::Failed { message: "Capture failed".into() }] {
            model.new_session.screenshot = capture;
            with_cx(&model, &mut inputs, |cx| {
                assert!(can_start(cx));
                assert!(launch_request(cx).is_none());
            });
        }
        model.new_session.screenshot = Capture::Capturing;
        with_cx(&model, &mut inputs, |cx| assert!(!can_start(cx)));
        model.new_session.screenshot = Capture::Ready { path: "/capture.png".into() };
        with_cx(&model, &mut inputs, |cx| assert!(launch_request(cx).is_some()));
        model.new_session.launching = true;
        with_cx(&model, &mut inputs, |cx| assert!(!can_start(cx)));
    }

    #[test]
    fn payload_preserves_context_and_only_sends_a_choosable_model() {
        let mut model = model();
        model.prefs.toolkit_in_sessions = false;
        model.prefs.bus_for_sessions = false;
        model.new_session.screenshot = Capture::Ready { path: "/capture.png".into() };
        let mut inputs = Inputs::default();
        inputs.new_session.task = true;
        inputs.new_session.screenshot = true;
        inputs.new_session.prompt = "  Cut the pauses  ".into();
        inputs.new_session.model = Some("custom-model".into());
        with_cx(&model, &mut inputs, |cx| {
            let request = launch_request(cx).unwrap();
            assert!(request.background && request.toolkit && !request.bus);
            assert_eq!(request.context, model.new_session.task_app);
            assert_eq!(request.task_app.as_deref(), Some("resolve"));
            assert_eq!(request.prompt, "  Cut the pauses  ");
            assert_eq!(request.screenshot, Some("/capture.png".into()));
            assert_eq!(request.model, None);
        });
        model.new_session.models.insert("claude".into(), ModelList { choosable: true, ..Default::default() });
        model.new_session.in_lantern_available = false;
        inputs.new_session.task = false;
        with_cx(&model, &mut inputs, |cx| {
            let request = launch_request(cx).unwrap();
            assert!(!request.background && !request.toolkit);
            assert_eq!(request.context, None);
            assert_eq!(request.task_app, None);
            assert_eq!(request.model.as_deref(), Some("custom-model"));
        });
    }

    #[test]
    fn search_matches_id_or_label_case_insensitively_and_limits_suggestions() {
        let list = ModelList {
            choosable: true,
            models: (0..30)
                .map(|i| ModelOption { id: format!("provider/MODEL-{i}"), label: format!("Friendly {i}"), group: None })
                .collect(),
        };
        assert_eq!(model_matches(&list, "").len(), 6);
        assert_eq!(model_matches(&list, "model").len(), 8);
        assert_eq!(model_matches(&list, "friendly 12")[0].id, "provider/MODEL-12");
        assert!(model_matches(&list, "missing").is_empty());
    }

    #[test]
    fn empty_task_explains_disabled_start_and_enter_does_not_launch() {
        use super::super::tests::frame;

        for prompt in ["", " \n "] {
            let ctx = egui::Context::default();
            t::install(&ctx);
            ctx.global_style_mut(|style| {
                style.interaction.tooltip_delay = 0.0;
                style.interaction.show_tooltips_only_when_still = false;
            });
            let assets = Assets::load(&ctx);
            let mut model = model();
            let mut inputs = Inputs::default();
            let mut view = UiState::default();
            inputs.new_session.task = true;
            inputs.new_session.prompt = prompt.into();
            frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
            let (height, _, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
            // Footer button: 34pt high, with 16pt padding beneath it.
            let hover = egui::Event::PointerMoved(egui::pos2(190.0, height - 16.0 - 17.0));
            frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![hover.clone()]);
            let (.., actions, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![hover.clone()]);
            let help = "Type what it should do in DaVinci/Resolve:Editor first.";
            assert!(texts.iter().any(|text| text == help), "disabled task button should explain the missing prompt: {texts:?}");
            assert!(actions.is_empty());
            let enter = egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![enter]);
            assert!(!actions.iter().any(|action| matches!(action, UiAction::Launch(_))));
            assert_eq!(inputs.new_session.prompt, prompt);

            model.new_session.launching = true;
            frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![hover.clone()]);
            let (_, actions, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![hover]);
            assert!(!texts.iter().any(|text| text == help), "starting a task should not ask for a new prompt");
            assert!(actions.is_empty());
        }
    }

    #[test]
    fn return_starts_a_plain_session_but_keeps_the_form_and_rejects_missing_capture() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut model = model();
        let mut inputs = Inputs::default();
        let mut view = UiState::default();
        inputs.new_session.prompt = "Find the issue".into();
        let enter = || egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        super::super::tests::frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        let (_, actions, _) = super::super::tests::frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![enter()]);
        assert!(actions.iter().any(|a| matches!(a, UiAction::Launch(r) if r.prompt == "Find the issue" && r.background)));
        assert_eq!(inputs.new_session.prompt, "Find the issue");
        inputs.new_session.screenshot = true;
        model.new_session.screenshot = Capture::PermissionNeeded;
        let (_, actions, _) = super::super::tests::frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![enter()]);
        assert!(!actions.iter().any(|a| matches!(a, UiAction::Launch(_))));
        assert!(view.session_error.as_ref().is_some_and(|error| error.contains("screenshot isn't ready")));
    }
}
