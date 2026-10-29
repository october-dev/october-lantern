//! October panel → Phone → Pair. Installation, QR and code comparison share one section.
use super::{
    Ctx, UiAction, components as c,
    theme::{self as t, Weight},
};
use crate::model::phone::{Action, Pairing, recovery};
use eframe::egui::{self, Margin, Stroke, Ui};

fn action(cx: &mut Ctx, action: Action) {
    cx.emit(UiAction::Phone(action));
}

pub fn render(ui: &mut Ui, cx: &mut Ctx) {
    if std::mem::take(&mut cx.view.phone_reveal) {
        ui.scroll_to_cursor(Some(egui::Align::Min));
    }
    let model = cx.model;
    let Some(account) = &model.account.signed_in else {
        c::label(ui, "Your phone uses the same October account. Sign in above to connect it.", 11.5, Weight::Regular, t::MUTED);
        return;
    };
    let email = if account.email.is_empty() { "the same October account as this computer" } else { &account.email };
    let phone = model.phone.as_ref();
    let busy = cx.view.phone_sending || phone.is_some_and(|s| s.busy);
    egui::Frame::new().fill(t::FAINT).stroke(Stroke::new(1.0, t::HAIRLINE)).corner_radius(14).inner_margin(Margin::same(12)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                c::label(ui, "Phone", 14.0, Weight::Semibold, t::INK);
                if phone.is_none_or(|s| s.pairing.is_none() && s.outcome.as_deref() != Some("denied"))
                    && ui.add_enabled_ui(!busy, |ui| c::secondary_button(ui, "Pair")).inner.clicked()
                {
                    action(cx, Action::Pair);
                }
            });
            c::label(ui, "See waiting agents and reply from your phone.", 12.0, Weight::Regular, t::MUTED);
            let plan_required = account.mobile_allowed == Some(false) || phone.is_some_and(|s| s.status == "plan-required");
            if plan_required {
                c::label(ui, "Your plan does not currently include mobile access.", 11.5, Weight::Regular, t::AMBER);
                if c::secondary_button(ui, "Upgrade").clicked() {
                    cx.emit(UiAction::OpenUrl("https://www.october.dev/pricing".into()));
                }
                if c::secondary_button(ui, "Refresh plan").clicked() {
                    cx.emit(UiAction::RefreshPlan);
                }
            } else if let Some(p) = phone.and_then(|s| s.pairing.as_ref()) {
                if p.code.is_some() {
                    confirm(ui, cx, p, busy);
                } else {
                    scan(ui, cx, p, email, busy);
                }
            } else if phone.is_some_and(|s| s.outcome.as_deref() == Some("paired")) {
                c::label(ui, "iPhone connected", 13.0, Weight::Semibold, t::GREEN);
                c::label(
                    ui,
                    "You'll get agent alerts and can reply from your phone. Allow notifications in October on your iPhone.",
                    11.5,
                    Weight::Regular,
                    t::MUTED,
                );
            }
            if busy {
                c::label(ui, "Working…", 11.5, Weight::Regular, t::MUTED);
            }
            if let Some(message) = cx.view.phone_error.as_deref().or(phone.and_then(|s| s.message.as_deref())) {
                c::label(ui, message, 11.5, Weight::Regular, t::AMBER);
            }
            if !plan_required
                && let Some(s) = phone
                && (s.error_code.is_some() || s.outcome.as_deref() == Some("denied"))
            {
                if s.outcome.as_deref() == Some("denied") {
                    c::label(ui, "You denied this phone. It has not been connected.", 11.5, Weight::Regular, t::MUTED);
                }
                let title = recovery(s.error_code.as_deref(), s.outcome.as_deref());
                if ui.add_enabled_ui(!busy, |ui| c::secondary_button(ui, title)).inner.clicked() {
                    match title {
                        "Sign in again" => cx.emit(UiAction::SignOut),
                        "New code" | "Start again" => action(cx, Action::Pair),
                        _ => action(cx, Action::Retry),
                    }
                }
            }
            if let Some(s) = phone
                && !s.devices.is_empty()
            {
                ui.separator();
                c::label(ui, "Paired phones", 12.0, Weight::Semibold, t::INK);
                for d in &s.devices {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            c::label(ui, &d.label, 12.0, Weight::Medium, t::INK);
                            let detail = if d.removal_pending {
                                "Blocked here. Retry to finish removing it from October.".into()
                            } else if let Some(seen) = d.last_seen_at {
                                format!("Last seen {}", relative(seen, cx.env.now_ms))
                            } else {
                                "Waiting for the phone to connect".into()
                            };
                            c::label(ui, detail, 11.0, Weight::Regular, if d.removal_pending { t::AMBER } else { t::MUTED });
                        });
                        if ui
                            .add_enabled_ui(!busy, |ui| c::secondary_button(ui, if d.removal_pending { "Retry removal" } else { "Remove" }))
                            .inner
                            .clicked()
                        {
                            if d.removal_pending {
                                action(cx, Action::Remove(d.bind.clone()));
                            } else {
                                cx.view.phone_remove = Some(d.bind.clone());
                            }
                        }
                    });
                    if cx.view.phone_remove.as_deref() == Some(&d.bind) {
                        c::label(
                            ui,
                            format!("Remove {}? This phone will lose access to your agents and notifications.", d.label),
                            11.5,
                            Weight::Regular,
                            t::MUTED,
                        );
                        ui.horizontal(|ui| {
                            if ui.add_enabled_ui(!busy, |ui| c::secondary_button(ui, "Confirm removal")).inner.clicked() {
                                action(cx, Action::Remove(d.bind.clone()));
                                cx.view.phone_remove = None;
                            }
                            if c::secondary_button(ui, "Cancel removal").clicked() {
                                cx.view.phone_remove = None;
                            }
                        });
                    }
                }
            }
            c::label(ui, "End-to-end encrypted. Only phones you approve can connect.", 11.0, Weight::Regular, t::MUTED);
            c::label(ui, "Keep this computer awake with Lantern open for phone access and agent alerts.", 11.0, Weight::Regular, t::MUTED);
        },
    );
}

fn scan(ui: &mut Ui, cx: &mut Ctx, p: &Pairing, email: &str, busy: bool) {
    c::label(ui, "Scan to pair", 13.0, Weight::Semibold, t::INK);
    let remaining = p.remaining(cx.env.now_ms);
    if remaining > 0 {
        c::label(ui, "Open your iPhone Camera and point it here", 12.5, Weight::Medium, t::INK);
        if let Some(link) = &p.qr {
            qr(ui, link, 240.0);
        }
        c::label(ui, "Or use the scanner in the October app, or paste the pairing link.", 11.5, Weight::Regular, t::MUTED);
        c::label(ui, format!("Expires in {}:{:02}", remaining / 60, remaining % 60), 11.0, Weight::Mono, t::MUTED);
        let title = if cx.view.phone_copied { "Pairing link copied" } else { "Copy pairing link" };
        if ui.add_enabled_ui(!busy, |ui| c::secondary_button(ui, title)).inner.clicked() {
            action(cx, Action::CopyLink);
        }
    } else {
        c::label(ui, "This pairing code has expired.", 11.5, Weight::Regular, t::AMBER);
    }
    c::label(ui, format!("Sign in to the phone with {email}. A different account cannot pair."), 11.5, Weight::Regular, t::MUTED);
    ui.horizontal(|ui| {
        if ui.add_enabled_ui(!busy, |ui| c::secondary_button(ui, "New code")).inner.clicked() {
            action(cx, Action::Pair);
        }
        if ui.add_enabled_ui(!busy, |ui| c::secondary_button(ui, "Cancel")).inner.clicked() {
            action(cx, Action::Cancel);
        }
    });
    c::label(
        ui,
        "Need the app? Install October with TestFlight, sign in, then scan this QR. If it expires, choose New code.",
        11.5,
        Weight::Regular,
        t::MUTED,
    );
    if let Some(url) = cx.model.phone.as_ref().and_then(|s| s.install_url.as_deref())
        && c::secondary_button(ui, "Get the October app").clicked()
    {
        cx.emit(UiAction::OpenUrl(url.into()));
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
}

fn confirm(ui: &mut Ui, cx: &mut Ctx, p: &Pairing, busy: bool) {
    c::label(ui, "Confirm your phone", 13.0, Weight::Semibold, t::INK);
    c::label(ui, p.label.as_deref().unwrap_or("Your phone"), 13.0, Weight::Medium, t::INK);
    c::label(ui, if p.platform.as_deref() == Some("ios") { "iPhone" } else { "Phone" }, 11.5, Weight::Regular, t::MUTED);
    if let Some(code) = &p.code {
        c::label(ui, code, 32.0, Weight::Mono, t::INK);
        c::label(ui, "Make sure this matches your phone", 11.5, Weight::Regular, t::MUTED);
        if p.finishing {
            c::label(ui, "Finishing on your phone…", 11.5, Weight::Regular, t::MUTED);
        } else if let Some(bind) = &p.bind {
            ui.horizontal(|ui| {
                for (title, allow) in [("Approve", true), ("Deny", false)] {
                    if ui.add_enabled_ui(!busy, |ui| c::secondary_button(ui, title)).inner.clicked() {
                        action(cx, Action::Decide { intent_id: p.intent_id.clone(), bind: bind.clone(), code: code.clone(), allow });
                    }
                }
            });
        }
    }
}

fn qr(ui: &mut Ui, text: &str, size: f32) {
    // Use the QR encoder already in the app. Paint modules directly, with a four-module quiet
    // zone; no QR image/secret is cached on disk, sent to a service or exposed as accessibility text.
    let Ok(code) = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M) else {
        c::label(ui, "The QR couldn't be drawn. Use Copy pairing link instead.", 11.5, Weight::Regular, t::AMBER);
        return;
    };
    let size = size.min(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(rect, 8.0, egui::Color32::WHITE);
    let module = size / (code.width() + 8) as f32;
    for y in 0..code.width() {
        for x in 0..code.width() {
            if code[(x, y)] == qrcode::Color::Dark {
                let min = rect.min + egui::vec2((x + 4) as f32 * module, (y + 4) as f32 * module);
                ui.painter().rect_filled(egui::Rect::from_min_size(min, egui::Vec2::splat(module + 0.1)), 0.0, egui::Color32::BLACK);
            }
        }
    }
}

fn relative(at: f64, now: f64) -> String {
    let minutes = ((now - at).max(0.0) / 60_000.0).floor() as u64;
    if minutes == 0 {
        "just now".into()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if minutes < 1440 {
        format!("{}h ago", minutes / 60)
    } else {
        format!("{}d ago", minutes / 1440)
    }
}
