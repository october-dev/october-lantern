//! PointAsk.swift's card only. Selection, capture, placement, recipient pinning, streaming
//! and dictation belong to the coordinator; this view emits intents and never performs them.
use super::{
    Ctx, DictationOwner, UiAction,
    components::{self as c, MarkdownStyle},
    icons::Icon,
    theme::{self as t, Weight},
};
use crate::model::{Capture, PanelMode};
use eframe::egui::{self, Align, Color32, Layout, Margin, Rect, Response, Sense, Stroke, Ui, Vec2, vec2};
use std::hash::{Hash, Hasher};

pub fn render(ui: &mut Ui, cx: &mut Ctx, max_height: f32) -> f32 {
    if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        // Esc answers "Turn on dictation?" with Not now before it closes the card.
        cx.emit(if cx.model.dictation_offer.as_ref().is_some_and(|offer| offer.point_ask) {
            UiAction::AnswerDictationOffer { download: false }
        } else {
            UiAction::CancelPointAsk
        });
    }
    let start = ui.cursor().min;
    let background = ui.painter().add(egui::Shape::Noop);
    let outer = Rect::from_min_size(start, vec2(t::POINT_WIDTH, max_height));
    let result = ui.scope_builder(egui::UiBuilder::new().max_rect(outer).layout(Layout::top_down(Align::Min)), |ui| {
        ui.set_width(t::POINT_WIDTH);
        ui.set_clip_rect(ui.clip_rect().intersect(outer));
        egui::Frame::new().inner_margin(16).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            header(ui, cx);
            screenshot(ui, cx);
            context(ui, cx);
            if !cx.model.point_ask.turns.is_empty() {
                conversation(ui, cx);
            }
            if recipient_missing(cx) && cx.model.agents.iter().any(|agent| agent.is_live()) {
                c::label(ui, "That agent has exited. Choose another.", 11.5, Weight::Regular, t::MUTED);
            }
            if cx.model.dictation_offer.as_ref().is_some_and(|offer| offer.point_ask) {
                super::dictation_offer(ui, cx);
            }
            input(ui, cx);
            footer(ui, cx);
        });
    });
    let height = result.response.rect.height().min(max_height).max(1.0);
    let rect = Rect::from_min_size(start, vec2(t::POINT_WIDTH, height));
    // The source has gray .08 at opacity .88 over GlassContainer's .62 black tint,
    // NOT glassSurface's .35 black overlay. Flatten those tint layers for our renderer:
    // alpha = .88 + outer_tint * .12; premultiplied gray = .08 * .88 = .0704.
    // This deterministic tint is not a substitute for native live backdrop blur.
    let tint = c::material_tint(0.62, cx.model.caps.dense_glass);
    let fill = Color32::from_rgba_premultiplied(18, 18, 18, ((0.88 + tint * 0.12) * 255.0).round() as u8);
    ui.painter().set(background, c::rimmed_shape(rect, t::POINT_RADIUS as f32, fill));
    height
}

fn header(ui: &mut Ui, cx: &mut Ctx) {
    c::centered_row(ui, 22.0, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        glyph(ui, Icon::PointAsk, 13.0, t::AMBER);
        c::label(ui, "Point & Ask", 13.0, Weight::Semibold, t::INK);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
            ui.painter().circle_filled(rect.center(), 11.0, t::FAINT);
            c::paint_icon(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(10.0)), Icon::Close, t::MUTED);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
            focus_ring(ui, &response, rect, 11.0);
            if response.on_hover_text("Close (Esc)").clicked() {
                cx.emit(UiAction::CancelPointAsk);
            }
            if cx.model.point_ask.listening {
                c::label(ui, "Listening", 11.0, Weight::Medium, t::RED);
                glyph(ui, Icon::Waveform, 11.0, t::RED);
            }
        });
    });
}

fn screenshot(ui: &mut Ui, cx: &mut Ctx) {
    let capture = &cx.model.point_ask.screenshot;
    let mut preview_error = None;
    let texture = if let Capture::Ready { path } = capture {
        match cx.view.point_ask_capture.get(ui.ctx(), path) {
            Ok(texture) => Some(texture.clone()),
            Err(_) => {
                preview_error = Some("The screenshot preview couldn't be opened. Reselect to try again.");
                None
            }
        }
    } else {
        None
    };
    let problem = cx.model.point_ask.problem.as_deref().or(preview_error).or(match capture {
        Capture::Failed { message } => Some(message.as_str()),
        Capture::PermissionNeeded => Some("Screen capture permission is needed. Reselect to try again."),
        _ => None,
    });
    let width = ui.available_width();
    let reselect_size = capsule_size(ui, "Reselect", 10.5, vec2(8.0, 4.0), true, false);
    let cap = if cx.model.point_ask.turns.is_empty() { 190.0 } else { 110.0 };
    let top = ui.cursor().min;
    let background = ui.painter().add(egui::Shape::Noop);
    if let Some(texture) = &texture {
        let size = texture.size_vec2();
        let fitted = size * (width / size.x).min(cap / size.y);
        let (rect, _) = ui.allocate_exact_size(vec2(width, fitted.y), Sense::hover());
        egui::Image::new(texture).corner_radius(10).paint_at(ui, Rect::from_center_size(rect.center(), fitted));
    } else if let Some(problem) = problem {
        egui::Frame::new().inner_margin(12).show(ui, |ui| {
            // Intentional source-bug fix: Swift overlays Reselect on full-width error
            // copy. Reserve its measured width only here; ready captures stay unchanged.
            ui.set_width((width - 24.0 - reselect_size.x - 6.0).max(1.0));
            c::label(ui, problem, 11.5, Weight::Regular, t::MUTED);
        });
    } else {
        let galley = ui.painter().layout_no_wrap("Capturing…".into(), t::font(11.5, Weight::Regular), t::MUTED);
        let (rect, _) = ui.allocate_exact_size(vec2(width, 80.0), Sense::hover());
        let size = vec2(18.0 + galley.size().x, galley.size().y.max(12.0));
        let mut row = ui.new_child(
            egui::UiBuilder::new().max_rect(Rect::from_center_size(rect.center(), size)).layout(Layout::left_to_right(Align::Center)),
        );
        row.spacing_mut().item_spacing.x = 6.0;
        c::spinner(&mut row, 12.0, t::MUTED, cx.env);
        c::label(&mut row, "Capturing…", 11.5, Weight::Regular, t::MUTED);
    }
    let rect = Rect::from_min_max(top, egui::pos2(top.x + width, ui.cursor().top() - ui.spacing().item_spacing.y));
    ui.painter().set(background, egui::Shape::rect_filled(rect, 10.0, t::black(0.35)));
    ui.painter().rect_stroke(rect, 10.0, Stroke::new(1.0, t::STROKE), egui::StrokeKind::Inside);
    let size = reselect_size;
    let button = Rect::from_min_size(egui::pos2(rect.right() - 6.0 - size.x, rect.top() + 6.0), size);
    let response =
        capsule_at(ui, button, "point-reselect", "Reselect", 10.5, Some(Icon::Reselect), false, t::INK, t::black(0.7), t::STROKE);
    if response.on_hover_text("Select a different area").clicked() {
        cx.emit(UiAction::ReselectPointAsk);
    }
    if texture.is_some()
        && let Some(problem) = cx.model.point_ask.problem.as_deref()
    {
        c::label(ui, problem, 11.5, Weight::Regular, t::RED);
    }
}

fn context(ui: &mut Ui, cx: &Ctx) {
    let Some(context) = &cx.model.point_ask.context else { return };
    if context.name.is_empty() && context.url.is_none() && context.selected_text.as_deref().is_none_or(str::is_empty) {
        return;
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        if !context.name.is_empty() {
            let title = context.window.as_ref().map_or_else(|| context.name.clone(), |window| format!("{} · {window}", context.name));
            context_line(ui, Icon::Window, &title, 1, false);
        }
        if let Some(url) = &context.url {
            context_line(ui, Icon::Link, url, 1, true);
        }
        if let Some(selected) = context.selected_text.as_deref().filter(|text| !text.is_empty()) {
            context_line(ui, Icon::TextCursor, &format!("“{selected}”"), 2, false);
        }
    });
}

fn context_line(ui: &mut Ui, icon: Icon, text: &str, rows: usize, middle: bool) {
    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        glyph(ui, icon, 11.0, t::MUTED);
        let width = ui.available_width();
        let text = if middle { middle_truncate(ui, text, width) } else { text.to_owned() };
        let mut job = egui::text::LayoutJob::single_section(
            text,
            egui::TextFormat { font_id: t::font(11.0, Weight::Regular), color: t::MUTED, ..Default::default() },
        );
        job.wrap.max_width = width;
        job.wrap.max_rows = rows;
        ui.add(egui::Label::new(job).wrap());
    });
}

fn middle_truncate(ui: &Ui, text: &str, width: f32) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.into(), t::font(11.0, Weight::Regular), t::MUTED).size().x <= width;
    if fits(text) {
        return text.into();
    }
    let chars: Vec<_> = text.chars().collect();
    let candidate = |n: usize| {
        chars[..n.div_ceil(2)].iter().chain(std::iter::once(&'…')).chain(chars[chars.len() - n / 2..].iter()).collect::<String>()
    };
    let (mut low, mut high) = (0, chars.len());
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

fn conversation(ui: &mut Ui, cx: &mut Ctx) {
    let last = cx.model.point_ask.turns.last().unwrap();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    last.answer.hash(&mut hash);
    let anchor = Some((cx.model.point_ask.turns.len(), hash.finish()));
    let changed = anchor != cx.view.point_ask_scroll_anchor;
    cx.view.point_ask_scroll_anchor = anchor;
    egui::Frame::new().fill(t::FAINT).corner_radius(12).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("point-conversation")
            .max_height(320.0)
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Frame::new().inner_margin(12).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    for (i, turn) in cx.model.point_ask.turns.iter().enumerate() {
                        ui.push_id(("point-turn", i), |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                c::label(ui, &turn.question, 12.0, Weight::Semibold, t::INK);
                                if turn.answer.is_empty() {
                                    c::centered_row(ui, 16.0, |ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        c::spinner(ui, 12.0, t::MUTED, cx.env);
                                        c::label(ui, "Thinking…", 12.0, Weight::Regular, t::MUTED);
                                    });
                                } else if let Some(url) = c::markdown_linked(
                                    ui,
                                    &turn.answer,
                                    MarkdownStyle {
                                        color: super::assistant::answer_color(turn.failed, turn.upgrade.is_some()),
                                        ..Default::default()
                                    },
                                    2.0,
                                )
                                .1
                                {
                                    cx.emit(UiAction::OpenUrl(url));
                                }
                                if let Some(note) = &turn.note {
                                    c::label(ui, note, 11.0, Weight::Regular, t::MUTED);
                                }
                                if let Some(url) = &turn.upgrade {
                                    super::assistant::upgrade_button(ui, cx, url, "point_ask");
                                }
                            });
                        });
                    }
                    if changed {
                        ui.scroll_to_cursor(Some(Align::BOTTOM));
                    }
                });
            });
    });
}

fn input(ui: &mut Ui, cx: &mut Ctx) {
    let listening = cx.model.point_ask.listening;
    let placeholder = if listening {
        "Listening… say what you want"
    } else if cx.model.point_ask.turns.is_empty() {
        "Ask about this, or tell an agent what to do"
    } else {
        "Ask a follow-up"
    };
    let input_id = ui.id().with("point-input");
    let focused = ui.memory(|memory| memory.has_focus(input_id));
    egui::Frame::new()
        .fill(t::black(0.3))
        .stroke(Stroke::new(1.0, if focused { t::opacity(t::AMBER, 0.5) } else { t::STROKE }))
        .corner_radius(11)
        .inner_margin(Margin { left: 11, right: 6, top: 7, bottom: 7 })
        .show(ui, |ui| {
            let width = (ui.available_width() - 28.0).max(1.0);
            let line_height = ui.painter().layout_no_wrap("A".into(), t::font(13.0, Weight::Regular), t::INK).size().y;
            let words = if cx.inputs.point_ask.is_empty() { placeholder } else { &cx.inputs.point_ask };
            let text_height = ui.painter().layout(words.into(), t::font(13.0, Weight::Regular), t::INK, width).size().y;
            let height = text_height.clamp(line_height, line_height * 5.0).max(22.0);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::left_to_right(Align::BOTTOM), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let edit = egui::ScrollArea::vertical()
                    .id_salt("point-input-scroll")
                    .max_width(width)
                    .max_height(line_height * 5.0)
                    .min_scrolled_height(0.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut cx.inputs.point_ask)
                                .id(input_id)
                                .font(t::font(13.0, Weight::Regular))
                                .text_color(t::INK)
                                .hint_text(c::text(placeholder, 13.0, Weight::Regular, t::MUTED))
                                .frame(egui::Frame::NONE)
                                .margin(0)
                                .desired_width(width)
                                .desired_rows(1)
                                .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)),
                        )
                    })
                    .inner;
                ui.ctx().accesskit_node_builder(edit.id, |node| node.set_label("Question or instructions"));
                if cx.view.point_ask_focus != Some(cx.model.compose_focus) {
                    edit.request_focus();
                    cx.view.point_ask_focus = Some(cx.model.compose_focus);
                }
                let enter = edit.has_focus() && c::consume_plain_key(ui, egui::Key::Enter);
                let signed_in = cx.model.account.signed_in.is_some();
                if enter && can_send_selection(cx) && (signed_in || !recipient_missing(cx)) && (!signed_in || !cx.model.point_ask.asking) {
                    cx.emit(UiAction::SubmitPointAsk);
                }
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                let mic_label = if listening { "Stop listening" } else { "Speak" };
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, mic_label));
                c::paint_icon(
                    ui.painter(),
                    Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
                    if listening { Icon::MicFill } else { Icon::Mic },
                    if listening { t::RED } else { t::MUTED },
                );
                if response.has_focus() {
                    ui.painter().rect_stroke(rect, 5, Stroke::new(1.0, t::AMBER), egui::StrokeKind::Inside);
                }
                if response.on_hover_text(mic_label).clicked() {
                    cx.emit(UiAction::ToggleDictation { owner: DictationOwner::PointAsk });
                }
            });
        });
}

fn can_send_selection(cx: &Ctx) -> bool {
    !matches!(cx.model.point_ask.screenshot, Capture::Capturing) && !cx.model.point_ask.sending
}

fn recipient_missing(cx: &Ctx) -> bool {
    cx.model.point_ask.recipient.as_deref().is_some_and(|id| cx.model.agent(id).is_none())
}

fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: f32) {
    if response.has_focus() {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, t::AMBER), egui::StrokeKind::Inside);
    }
}

fn footer(ui: &mut Ui, cx: &mut Ctx) {
    let signed_in = cx.model.account.signed_in.is_some();
    let title = if signed_in { if cx.model.point_ask.asking { "Asking…" } else { "Ask" } } else { "Sign in to Ask" };
    let primary_size = capsule_size(
        ui,
        title,
        12.0,
        if signed_in { vec2(14.0, 6.0) } else { vec2(12.0, 5.0) },
        false,
        signed_in && !cx.model.point_ask.asking,
    );
    let copy = !cx.model.point_ask.turns.is_empty() && !cx.model.point_ask.asking;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), primary_size.y.max(26.0)), Sense::hover());
    let primary = Rect::from_center_size(egui::pos2(row.right() - primary_size.x / 2.0, row.center().y), primary_size);
    let enabled = !signed_in || (!cx.model.point_ask.asking && can_send_selection(cx));
    let fill = if signed_in { t::opacity(t::AMBER, if enabled { 1.0 } else { 0.4 }) } else { Color32::TRANSPARENT };
    let response = {
        let mut button_ui = ui.new_child(egui::UiBuilder::new().max_rect(primary));
        if !enabled {
            button_ui.disable();
        }
        capsule_at(
            &mut button_ui,
            primary,
            "point-ask",
            title,
            12.0,
            None,
            signed_in && !cx.model.point_ask.asking,
            if signed_in { t::black(0.85) } else { t::AMBER },
            fill,
            if signed_in { Color32::TRANSPARENT } else { t::opacity(t::AMBER, 0.6) },
        )
    };
    let response = if signed_in {
        response
    } else {
        response.on_hover_text("Ask answers right here with October's AI. Sign in to October to use it.")
    };
    if response.clicked() {
        if signed_in {
            cx.emit(UiAction::SubmitPointAsk);
        } else {
            cx.emit(UiAction::CancelPointAsk);
            cx.emit(UiAction::SetPanel(Some(PanelMode::October)));
        }
    }
    let available = (primary.left() - row.left() - 8.0 - if copy { 25.0 } else { 0.0 }).max(1.0);
    let agent_width = agent_button(ui, cx, Rect::from_min_size(row.min, vec2(available, row.height())), signed_in);
    if copy {
        let rect = Rect::from_center_size(egui::pos2(row.left() + agent_width + 8.0 + 5.5, row.center().y), Vec2::splat(11.0));
        c::paint_icon(ui.painter(), rect, Icon::Copy, t::MUTED);
        let response = ui.interact(rect.expand(3.0), ui.id().with("point-copy"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Copy the answer"));
        focus_ring(ui, &response, rect.expand(3.0), 4.0);
        if response.on_hover_text("Copy the answer").clicked() {
            cx.emit(UiAction::CopyPointAskAnswer);
        }
    }
}

fn agent_button(ui: &mut Ui, cx: &mut Ctx, area: Rect, signed_in: bool) -> f32 {
    let live: Vec<_> = cx.model.agents.iter().filter(|agent| agent.is_live()).collect();
    let Some(agent) = cx.model.point_ask.recipient.as_deref().and_then(|id| cx.model.agent(id)) else {
        if recipient_missing(cx) && !live.is_empty() {
            let size = capsule_size(ui, "Choose another", 11.5, vec2(10.0, 5.0), false, false);
            let rect = Rect::from_center_size(
                egui::pos2(area.left() + size.x.min(area.width()) / 2.0, area.center().y),
                vec2(size.x.min(area.width()), size.y),
            );
            let response =
                capsule_at(ui, rect, "point-recipient-recovery", "Choose another", 11.5, None, false, t::INK, t::FAINT, t::STROKE);
            recipient_menu(&response, cx, &live);
            return rect.width();
        }
        let galley = ui.painter().layout_no_wrap("No agent running".into(), t::font(11.5, Weight::Regular), t::MUTED);
        ui.painter().galley(egui::pos2(area.left(), area.center().y - galley.size().y / 2.0), galley.clone(), t::MUTED);
        return galley.size().x;
    };
    let menu = live.len() > 1;
    let title = format!("Send to @{}", agent.handle);
    let size = capsule_size(ui, &title, 11.5, vec2(10.0, 5.0), true, !signed_in);
    let width = (size.x + if menu { 17.0 } else { 0.0 }).min(area.width());
    let rect = Rect::from_center_size(egui::pos2(area.left() + width / 2.0, area.center().y), vec2(width, size.y));
    ui.painter().rect_filled(rect, rect.height() / 2.0, t::FAINT);
    ui.painter().rect_stroke(rect, rect.height() / 2.0, Stroke::new(1.0, t::STROKE), egui::StrokeKind::Inside);
    let send_rect = Rect::from_min_max(rect.min, rect.max - vec2(if menu { 17.0 } else { 0.0 }, 0.0));
    let enabled = can_send_selection(cx);
    let response = {
        let mut button_ui = ui.new_child(egui::UiBuilder::new().max_rect(send_rect));
        if !enabled {
            button_ui.disable();
        }
        capsule_at(
            &mut button_ui,
            send_rect,
            "point-send",
            &title,
            11.5,
            Some(Icon::Send),
            !signed_in,
            t::INK,
            Color32::TRANSPARENT,
            Color32::TRANSPARENT,
        )
    };
    if response.on_hover_text("Send the screenshot and your words to this agent instead").clicked() {
        cx.emit(UiAction::SendPointAskToAgent);
    }
    if menu {
        let rect = Rect::from_min_max(egui::pos2(send_rect.right(), rect.top()), rect.max);
        c::paint_icon(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(9.0)), Icon::ChevronDown, t::MUTED);
        let response = ui.interact(rect, ui.id().with("point-recipient-menu"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Choose the agent"));
        focus_ring(ui, &response, rect, 4.0);
        recipient_menu(&response, cx, &live);
    }
    width
}

fn recipient_menu(response: &Response, cx: &mut Ctx, live: &[&crate::model::Agent]) {
    egui::Popup::menu(response).show(|ui| {
        for agent in live {
            if ui.button(format!("@{} · {}", agent.handle, agent.project.as_deref().unwrap_or(&agent.kind.display_name()))).clicked() {
                cx.emit(UiAction::ChoosePointAskRecipient(agent.id.clone()));
            }
        }
    });
}

fn glyph(ui: &mut Ui, icon: Icon, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    c::paint_icon(ui.painter(), rect, icon, color);
}

fn capsule_size(ui: &Ui, title: &str, size: f32, pad: Vec2, leading: bool, enter: bool) -> Vec2 {
    let galley = ui.painter().layout_no_wrap(title.into(), t::font(size, Weight::Semibold), t::INK);
    galley.size() + pad * 2.0 + vec2(if leading { size + 5.0 } else { 0.0 } + if enter { 16.0 } else { 0.0 }, 0.0)
}

#[allow(clippy::too_many_arguments)]
fn capsule_at(
    ui: &mut Ui,
    rect: Rect,
    id: &str,
    title: &str,
    size: f32,
    icon: Option<Icon>,
    enter: bool,
    color: Color32,
    fill: Color32,
    stroke: Color32,
) -> Response {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title));
    ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
    ui.painter().rect_stroke(
        rect,
        rect.height() / 2.0,
        Stroke::new(1.0, if response.has_focus() { t::AMBER } else { stroke }),
        egui::StrokeKind::Inside,
    );
    let leading = icon.map_or(0.0, |_| size + 5.0);
    let trailing = if enter { 16.0 } else { 0.0 };
    let mut job = egui::text::LayoutJob::single_section(
        title.into(),
        egui::TextFormat { font_id: t::font(size, Weight::Semibold), color, ..Default::default() },
    );
    job.wrap.max_width = (rect.width() - 16.0 - leading - trailing).max(1.0);
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(job);
    let start = rect.center() - vec2(galley.size().x + leading + trailing, galley.size().y) / 2.0;
    if let Some(icon) = icon {
        c::paint_icon(
            ui.painter(),
            Rect::from_center_size(egui::pos2(start.x + size / 2.0, rect.center().y), Vec2::splat(size)),
            icon,
            color,
        );
    }
    ui.painter().galley(start + vec2(leading, 0.0), galley.clone(), color);
    if enter {
        c::paint_icon(
            ui.painter(),
            Rect::from_center_size(egui::pos2(start.x + leading + galley.size().x + 10.0, rect.center().y), Vec2::splat(10.5)),
            Icon::Return,
            if icon.is_some() { t::MUTED } else { color },
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Account, Agent, AgentState, AskTurn, Inputs, Model, PointAskPhase};
    use crate::ui::{Assets, Env, UiState};

    fn setup() -> (egui::Context, Assets, Model, Inputs, UiState) {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut model = Model::default();
        model.point_ask.phase = PointAskPhase::Card;
        model.point_ask.screenshot = Capture::PermissionNeeded;
        model.agents.push(Agent { id: "claude:1".into(), handle: "claude-26".into(), state: AgentState::Waiting, ..Default::default() });
        model.point_ask.recipient = Some("claude:1".into());
        (ctx, assets, model, Inputs::default(), UiState::default())
    }

    fn frame(
        ctx: &egui::Context,
        assets: &Assets,
        model: &Model,
        inputs: &mut Inputs,
        view: &mut UiState,
        events: Vec<egui::Event>,
    ) -> (f32, Vec<UiAction>, Vec<(String, Rect)>) {
        fn collect(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => texts.push((text.galley.job.text.clone(), text.galley.rect.translate(text.pos.to_vec2()))),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts)),
                _ => {}
            }
        }
        let (height, actions, output) = render_frame(ctx, assets, model, inputs, view, events);
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        (height, actions, texts)
    }

    fn render_frame(
        ctx: &egui::Context,
        assets: &Assets,
        model: &Model,
        inputs: &mut Inputs,
        view: &mut UiState,
        events: Vec<egui::Event>,
    ) -> (f32, Vec<UiAction>, egui::FullOutput) {
        let mut actions = Vec::new();
        let mut height = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 1000.0))), events, ..Default::default() },
            |ui| {
                let mut cx = Ctx { model, inputs, view, assets, out: &mut actions, env: &Env::default() };
                height = super::super::render_point_ask(ui, &mut cx, 900.0);
                assert!(ui.min_rect().width() <= 400.1, "card width {}", ui.min_rect().width());
            },
        );
        output.textures_delta.clear();
        (height, actions, output)
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn the_dictation_question_shows_in_the_card_and_escape_says_not_now() {
        let (ctx, assets, mut model, mut inputs, mut view) = setup();
        model.dictation_offer = Some(crate::model::DictationOffer { text: crate::speech::offer(true, false), point_ask: true });
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        let at = |title: &str| texts.iter().find(|(text, _)| text == title).unwrap_or_else(|| panic!("missing {title}")).1.center();
        assert!(texts.iter().any(|(text, _)| text == "Turn on dictation?"));
        let click = |pos: egui::Pos2, pressed: bool| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
            ]
        };
        for (title, download) in [("Download", true), ("Not now", false)] {
            let pos = at(title);
            frame(&ctx, &assets, &model, &mut inputs, &mut view, click(pos, true));
            let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, click(pos, false));
            assert_eq!(actions, vec![UiAction::AnswerDictationOffer { download }]);
        }
        let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert_eq!(actions, vec![UiAction::AnswerDictationOffer { download: false }], "Esc doesn't close the card");
        // The panel's question isn't shown here.
        model.dictation_offer.as_mut().unwrap().point_ask = false;
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        assert!(!texts.iter().any(|(text, _)| text == "Turn on dictation?"));
    }

    #[test]
    fn enter_is_primary_escape_cancels_and_shift_enter_is_a_newline() {
        let (ctx, assets, mut model, mut inputs, mut view) = setup();
        inputs.point_ask = "Explain this".into();
        frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(actions, vec![UiAction::SubmitPointAsk]);
        assert_eq!(inputs.point_ask, "Explain this", "the coordinator owns clearing after submission");
        let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert_eq!(actions, vec![UiAction::CancelPointAsk]);
        // Escape closes the real card. Selecting again provides a new focus token.
        model.compose_focus += 1;
        frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)]);
        assert!(actions.is_empty());
        assert!(inputs.point_ask.contains('\n'));
        model.point_ask.screenshot = Capture::Capturing;
        let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(actions.is_empty(), "wait for the current capture");
        model.point_ask.screenshot = Capture::PermissionNeeded;
        model.point_ask.sending = true;
        let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(actions.is_empty(), "wait for screenshot saving");
        model.point_ask.sending = false;
        model.account.signed_in = Some(Account::default());
        model.point_ask.asking = true;
        let (_, actions, _) = frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(actions.is_empty(), "don't issue another AI request midstream");
    }

    fn click(ctx: &egui::Context, assets: &Assets, model: &Model, inputs: &mut Inputs, view: &mut UiState, title: &str) -> Vec<UiAction> {
        let (_, _, texts) = frame(ctx, assets, model, inputs, view, Vec::new());
        let pos = texts.iter().find(|(text, _)| text == title).unwrap_or_else(|| panic!("missing {title}: {texts:?}")).1.center();
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
    fn explicit_send_sign_in_ask_and_reselect_are_distinct_intents() {
        let (ctx, assets, mut model, mut inputs, mut view) = setup();
        inputs.point_ask = "Fix it".into();
        assert_eq!(click(&ctx, &assets, &model, &mut inputs, &mut view, "Send to @claude-26"), vec![UiAction::SendPointAskToAgent]);
        assert_eq!(
            click(&ctx, &assets, &model, &mut inputs, &mut view, "Sign in to Ask"),
            vec![UiAction::CancelPointAsk, UiAction::SetPanel(Some(PanelMode::October))]
        );
        assert_eq!(click(&ctx, &assets, &model, &mut inputs, &mut view, "Reselect"), vec![UiAction::ReselectPointAsk]);
        model.account.signed_in = Some(Account::default());
        assert_eq!(click(&ctx, &assets, &model, &mut inputs, &mut view, "Ask"), vec![UiAction::SubmitPointAsk]);
        model.point_ask.screenshot = Capture::Capturing;
        assert!(click(&ctx, &assets, &model, &mut inputs, &mut view, "Ask").is_empty());
        assert!(click(&ctx, &assets, &model, &mut inputs, &mut view, "Send to @claude-26").is_empty());
    }

    #[test]
    fn vanished_recipient_is_not_replaced_and_focus_token_restores_the_input() {
        let (ctx, assets, mut model, mut inputs, mut view) = setup();
        model.point_ask.recipient = Some("gone".into());
        let (_, actions, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        assert!(texts.iter().any(|(text, _)| text == "That agent has exited. Choose another."));
        assert!(texts.iter().any(|(text, _)| text == "Choose another"));
        assert!(actions.is_empty());
        assert_eq!(model.point_ask.recipient.as_deref(), Some("gone"));
        let other = egui::Id::new("some-other-control");
        ctx.memory_mut(|memory| memory.request_focus(other));
        model.compose_focus += 1;
        frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        assert_eq!(view.point_ask_focus, Some(model.compose_focus));
        assert_ne!(ctx.memory(|memory| memory.focused()), Some(other));
    }

    #[test]
    fn missing_recipient_requires_explicit_selection_with_one_or_several_alternatives() {
        for count in [1, 2] {
            let (ctx, assets, mut model, mut inputs, mut view) = setup();
            model.point_ask.recipient = Some("gone".into());
            inputs.point_ask = "Keep my words".into();
            model.point_ask.turns.push(AskTurn {
                question: "Keep this conversation".into(),
                answer: "Existing answer".into(),
                ..Default::default()
            });
            if count == 2 {
                model.agents.push(Agent {
                    id: "codex:1".into(),
                    handle: "codex-1".into(),
                    state: AgentState::Working,
                    ..Default::default()
                });
            }
            frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
            assert!(frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]).1.is_empty());
            assert!(click(&ctx, &assets, &model, &mut inputs, &mut view, "Choose another").is_empty());
            let (_, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
            let choice = texts
                .iter()
                .find(|(text, _)| text.starts_with("@claude-26 ·"))
                .expect("live agent offered even with one alternative")
                .0
                .clone();
            assert_eq!(
                click(&ctx, &assets, &model, &mut inputs, &mut view, &choice),
                vec![UiAction::ChoosePointAskRecipient("claude:1".into())]
            );
            assert_eq!(model.point_ask.recipient.as_deref(), Some("gone"), "rendering never retargets");
            assert_eq!(inputs.point_ask, "Keep my words");
            assert_eq!(model.point_ask.turns[0].answer, "Existing answer");
            model.account.signed_in = Some(Account::default());
            assert_eq!(click(&ctx, &assets, &model, &mut inputs, &mut view, "Ask"), vec![UiAction::SubmitPointAsk]);
            model.agents.clear();
            let (_, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
            assert!(texts.iter().any(|(text, _)| text == "No agent running"));
            assert!(!texts.iter().any(|(text, _)| text == "Choose another"));
        }
    }

    #[test]
    fn custom_controls_have_visible_keyboard_focus_and_activate_with_space() {
        for target in ["Close", "Copy the answer", "Choose the agent"] {
            let (ctx, assets, mut model, mut inputs, mut view) = setup();
            model.agents.push(Agent { id: "codex:1".into(), handle: "codex-1".into(), state: AgentState::Working, ..Default::default() });
            model.point_ask.turns.push(AskTurn { question: "Question".into(), answer: "Answer".into(), ..Default::default() });
            frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
            let mut found = false;
            for _ in 0..16 {
                let (_, _, output) =
                    render_frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
                if !output
                    .platform_output
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::output::OutputEvent::FocusGained(info) if info.label.as_deref() == Some(target)))
                {
                    continue;
                }
                let expected = if target == "Close" {
                    vec2(22.0, 22.0)
                } else if target == "Copy the answer" {
                    vec2(17.0, 17.0)
                } else {
                    vec2(17.0, 23.5)
                };
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.color == t::AMBER && (rect.rect.width()-expected.x).abs()<1.0 && (rect.rect.height()-expected.y).abs()<2.0)), "visible focus for {target}");
                let (_, actions, texts) =
                    frame(&ctx, &assets, &model, &mut inputs, &mut view, vec![key(egui::Key::Space, egui::Modifiers::NONE)]);
                match target {
                    "Close" => assert_eq!(actions, vec![UiAction::CancelPointAsk]),
                    "Copy the answer" => assert_eq!(actions, vec![UiAction::CopyPointAskAnswer]),
                    _ => {
                        let (_, _, next_texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
                        assert!(
                            texts.iter().chain(&next_texts).any(|(text, _)| text.starts_with("@claude-26 ·")),
                            "chooser opens with Space: {next_texts:?}"
                        );
                    }
                }
                found = true;
                break;
            }
            assert!(found, "tab reaches {target}");
        }
    }

    #[test]
    fn question_and_card_controls_have_useful_accessibility_names() {
        let (ctx, assets, mut model, mut inputs, mut view) = setup();
        ctx.enable_accesskit();
        model.agents.push(Agent { id: "codex:1".into(), handle: "codex-1".into(), state: AgentState::Working, ..Default::default() });
        model.point_ask.turns.push(AskTurn { question: "Question".into(), answer: "Answer".into(), ..Default::default() });
        inputs.point_ask = "These are my instructions".into();
        let (_, _, output) = render_frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        let tree = output.platform_output.accesskit_update.expect("egui emits an accessibility tree when enabled");
        for label in ["Close", "Reselect", "Speak", "Send to @claude-26", "Choose the agent", "Copy the answer", "Sign in to Ask"] {
            assert!(tree.nodes.iter().any(|(_, node)| node.label() == Some(label)), "missing accessible control: {label}");
        }
        assert!(tree.nodes.iter().any(|(_, node)| node.role() == egui::accesskit::Role::MultilineTextInput
            && node.label() == Some("Question or instructions")
            && node.value() == Some("These are my instructions")));
    }

    #[test]
    fn every_fixture_uses_the_real_card_and_fits_its_fixed_width() {
        let (ctx, assets, _, _, _) = setup();
        for raw in [
            include_str!("../../fixtures/point-ask.json"),
            include_str!("../../fixtures/point-ask-signed-out.json"),
            include_str!("../../fixtures/point-ask-capturing.json"),
            include_str!("../../fixtures/point-ask-permission.json"),
            include_str!("../../fixtures/point-ask-answer.json"),
            include_str!("../../fixtures/point-ask-thinking.json"),
            include_str!("../../fixtures/point-ask-error.json"),
            include_str!("../../fixtures/point-ask-no-agent.json"),
            include_str!("../../fixtures/point-ask-saving.json"),
            include_str!("../../fixtures/point-ask-exited-agent.json"),
        ] {
            let mut fixture: crate::app::fixture::Fixture = serde_json::from_str(raw).unwrap();
            if let Capture::Ready { path } = &mut fixture.model.point_ask.screenshot {
                *path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(&*path);
            }
            let (height, actions, texts) = frame(&ctx, &assets, &fixture.model, &mut fixture.inputs, &mut fixture.view, Vec::new());
            assert!((160.0..900.0).contains(&height), "height {height}");
            assert!(actions.is_empty());
            assert!(texts.iter().any(|(text, _)| text == "Point & Ask"));
            assert!(texts.iter().all(|(_, rect)| rect.right() <= 384.5), "{texts:?}");
        }
    }

    #[test]
    fn long_unicode_handles_urls_and_streamed_answers_stay_bounded() {
        let (ctx, assets, mut model, mut inputs, mut view) = setup();
        model.agents[0].handle = "agent-é".repeat(30);
        model.account.signed_in = Some(Account::default());
        model.point_ask.turns = vec![AskTurn {
            question: "What does this mean?".into(),
            answer: "A streamed **answer** with `code`.\n".repeat(100),
            ..Default::default()
        }];
        model.point_ask.context = Some(crate::model::AppContext {
            name: "Browser".into(),
            url: Some(format!("https://example.com/{}", "café/".repeat(40))),
            selected_text: Some("Selected words ".repeat(30)),
            ..Default::default()
        });
        let (height, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        assert!(height < 700.0, "conversation is capped at 320: {height}");
        assert!(texts.iter().all(|(_, rect)| rect.right() <= 384.5), "{texts:?}");
        assert!(texts.iter().any(|(text, _)| text.contains('…') && text.starts_with("https://")));
    }

    #[test]
    fn capture_errors_never_run_under_reselect_and_footer_keeps_its_padding() {
        let (ctx, assets, mut model, mut inputs, mut view) = setup();
        let problem = "Couldn't take the screenshot: the selected display is no longer available. Reselect an area on a connected display and try again.";
        model.point_ask.screenshot = Capture::Failed { message: problem.into() };
        let (height, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        let error = texts.iter().find(|(text, _)| text == problem).unwrap().1;
        let reselect = texts.iter().find(|(text, _)| text == "Reselect").unwrap().1;
        // Text begins after 8pt capsule padding + the 10.5pt icon + 5pt gap.
        let capsule_left = reselect.left() - 8.0 - 10.5 - 5.0;
        assert!(error.right() <= capsule_left, "{error:?} overlaps capsule starting {capsule_left}");
        let primary = texts.iter().find(|(text, _)| text == "Sign in to Ask").unwrap().1;
        assert!((height - primary.bottom() - 5.0 - 16.0).abs() < 2.0, "no empty layout scopes below footer: {height} / {primary:?}");
        model.point_ask.screenshot = Capture::Capturing;
        let (_, _, texts) = frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        let capturing = texts.iter().find(|(text, _)| text == "Capturing…").unwrap().1;
        assert!((capturing.center().x - 9.0 - 200.0).abs() < 1.0, "spinner + gap + label centred: {capturing:?}");
    }
}
