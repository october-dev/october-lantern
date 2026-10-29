//! AccountCard.swift. Credentials are view-local; authentication is exclusively an intent.
use super::super::{
    Ctx, Provider, UiAction, components as c,
    icons::Icon,
    theme::{self as t, Weight},
};
use eframe::egui::{self, Align, Layout, Margin, Rect, Sense, Stroke, Ui};

pub fn render(ui: &mut Ui, cx: &mut Ctx) {
    render_with(ui, cx, true);
}

/// `show_header` off where the context already says why (Ask Lantern's sign-in): just the
/// buttons. Signed in, the header (with Sign Out) always shows.
pub fn render_with(ui: &mut Ui, cx: &mut Ctx, show_header: bool) {
    let signed_in = cx.model.account.signed_in.is_some();
    if signed_in {
        cx.view.account_password.clear();
        cx.view.account_use_email = false;
    }
    egui::Frame::new()
        .fill(t::FAINT)
        .stroke(Stroke::new(1.0, if signed_in { t::GREEN.gamma_multiply(0.35) } else { t::HAIRLINE }))
        .corner_radius(14)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            if show_header || signed_in {
                header(ui, cx, signed_in);
            }
            if signed_in {
                lantern(ui, cx);
            }
            if !signed_in {
                if cx.model.account.pending && !cx.view.account_use_email {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        c::spinner(ui, 12.0, t::MUTED, cx.env);
                        let cancel_width =
                            ui.painter().layout_no_wrap("Cancel".into(), t::font(12.0, Weight::Regular), t::MUTED).size().x + 8.0;
                        let width = (ui.available_width() - cancel_width).max(1.0);
                        let height = ui
                            .painter()
                            .layout("Finish signing in in your browser…".into(), t::font(12.0, Weight::Regular), t::MUTED, width)
                            .size()
                            .y;
                        ui.allocate_ui_with_layout(egui::vec2(width, height), Layout::top_down(Align::Min), |ui| {
                            c::label(ui, "Finish signing in in your browser…", 12.0, Weight::Regular, t::MUTED);
                        });
                        if c::plain_button(ui, "Cancel", 12.0, Weight::Regular, t::MUTED).clicked() {
                            cx.emit(UiAction::CancelSignIn);
                        }
                    });
                } else if cx.view.account_use_email {
                    email_form(ui, cx);
                } else {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let width = (ui.available_width() - 12.0) / 3.0;
                            for (provider, title, texture) in [
                                (Provider::Google, "Google", &cx.assets.google),
                                (Provider::GitHub, "GitHub", &cx.assets.github),
                                (Provider::Apple, "Apple", &cx.assets.apple),
                            ] {
                                if provider_button(ui, title, texture, width).clicked() {
                                    cx.emit(UiAction::SignInProvider(provider));
                                }
                            }
                        });
                        let title = "Use email and password";
                        let galley = ui.painter().layout_no_wrap(title.into(), t::font(11.5, Weight::Medium), t::MUTED);
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(galley.size().x + 19.0, galley.size().y.max(13.0)), Sense::click());
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
                        c::paint_icon(
                            ui.painter(),
                            Rect::from_center_size(egui::pos2(rect.left() + 6.5, rect.center().y), egui::Vec2::splat(13.0)),
                            Icon::Mail,
                            t::MUTED,
                        );
                        ui.painter().galley(egui::pos2(rect.left() + 19.0, rect.center().y - galley.size().y / 2.0), galley, t::MUTED);
                        if response.clicked() {
                            cx.view.account_use_email = true;
                        }
                    });
                }
                if let Some(error) = &cx.model.account.error {
                    c::label(ui, error, 11.5, Weight::Regular, t::RED);
                }
            }
        });
}

/// The icon, "Connect to October" (or who's signed in) and Sign Out.
fn header(ui: &mut Ui, cx: &mut Ctx, signed_in: bool) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 20.0), Sense::hover());
        let glyph = Rect::from_center_size(rect.center(), egui::Vec2::splat(16.0));
        c::paint_icon(ui.painter(), glyph, Icon::AccountCircle, if signed_in { t::GREEN } else { t::MUTED });
        if signed_in {
            let badge = Rect::from_center_size(glyph.right_bottom() - egui::vec2(1.0, 1.0), egui::Vec2::splat(8.0));
            ui.painter().circle_filled(badge.center(), 4.5, t::GREEN);
            c::paint_icon(ui.painter(), badge.shrink(1.5), Icon::Check, t::black(0.85));
        }
        let action_width = if signed_in {
            ui.painter().layout_no_wrap("Sign Out".into(), t::font(12.0, Weight::Semibold), t::INK).size().x + 24.0 + 12.0
        } else {
            0.0
        };
        let width = ui.available_width() - action_width;
        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 2.0;
            c::label(ui, if signed_in { "Connected to October" } else { "Connect to October" }, 13.0, Weight::Semibold, t::INK);
            c::label(ui, detail(cx), 11.5, Weight::Regular, t::MUTED);
        });
        if signed_in && c::secondary_button(ui, "Sign Out").clicked() {
            cx.emit(UiAction::SignOut);
        }
    });
}

/// What the plan gives in Lantern (Lifetime, the trial, October's questions a day), Refresh, and
/// the offers on a free plan.
fn lantern(ui: &mut Ui, cx: &mut Ctx) {
    let Some(account) = &cx.model.account.signed_in else { return };
    let now = cx.env.now_ms;
    let lines = rights_lines(account.lantern.as_ref(), now);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for line in &lines {
                c::label(ui, line, 11.5, Weight::Regular, t::MUTED);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let refreshing = cx.model.account.refreshing;
            let title = if refreshing { "Refreshing…" } else { "Refresh" };
            if ui
                .add_enabled_ui(!refreshing, |ui| c::plain_button(ui, title, 11.5, Weight::Semibold, t::MUTED))
                .inner
                .on_hover_text("Fetch your plan again, after buying in the browser")
                .clicked()
            {
                cx.emit(UiAction::RefreshPlan);
            }
        });
    });
    let rights = account.lantern.as_ref();
    let free = account.plan.as_deref().is_none_or(|plan| plan.eq_ignore_ascii_case("free"));
    if free && !rights.is_some_and(|rights| rights.lifetime || rights.trial_running(now)) {
        crate::ui::assistant::offer_buttons(ui, cx, crate::october_ai::LANTERN, "account");
    }
}

/// "Lantern Lifetime" or "Pro trial · ends Nov 1, 2026", then "Up to 50 questions a day on
/// October's AI".
fn rights_lines(rights: Option<&crate::model::LanternRights>, now_ms: f64) -> Vec<String> {
    let Some(rights) = rights else { return Vec::new() };
    let mut lines = Vec::new();
    if rights.lifetime {
        lines.push("Lantern Lifetime".to_string());
    } else if let Some(ends) = rights.trial_ends_ms.filter(|_| rights.trial_running(now_ms)) {
        lines.push(format!("Pro trial · ends {}", local_date(ends, crate::os::utc_offset())));
    }
    if rights.daily_questions > 0 {
        lines.push(format!("Up to {} questions a day on October's AI", rights.daily_questions));
    }
    lines
}

/// "Nov 1, 2026", in this computer's time zone.
fn local_date(ms: f64, offset: time::UtcOffset) -> String {
    let Ok(format) = time::format_description::parse_borrowed::<2>("[month repr:short] [day padding:none], [year]") else {
        return String::new();
    };
    time::OffsetDateTime::from_unix_timestamp((ms / 1000.0).floor() as i64)
        .map(|at| at.to_offset(offset))
        .ok()
        .and_then(|at| at.format(&format).ok())
        .unwrap_or_default()
}

fn detail(cx: &Ctx) -> String {
    let Some(account) = &cx.model.account.signed_in else {
        return "Sign in with your October account.".into();
    };
    let mut detail = if account.email.is_empty() { "Signed in".into() } else { account.email.clone() };
    if let Some(plan) = &account.plan {
        let plan = plan
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                chars.next().map(|first| first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ");
        detail.push_str(&format!(" · {plan} plan"));
    }
    detail
}

fn provider_button(ui: &mut Ui, title: &str, texture: &egui::TextureHandle, width: f32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(title.into(), t::font(12.0, Weight::Semibold), t::INK);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, galley.size().y + 12.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    ui.painter().rect_filled(rect, 8.0, t::white(if response.is_pointer_button_down_on() { 0.18 } else { 0.1 }));
    let left = rect.center().x - (13.0 + 6.0 + galley.size().x) / 2.0;
    ui.painter().image(
        texture.id(),
        Rect::from_min_size(egui::pos2(left, rect.center().y - 6.5 - if title == "Apple" { 1.0 } else { 0.0 }), egui::Vec2::splat(13.0)),
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        t::INK,
    );
    ui.painter().galley(egui::pos2(left + 19.0, rect.center().y - galley.size().y / 2.0), galley, t::INK);
    if response.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 10.0, Stroke::new(1.0, t::AMBER), egui::StrokeKind::Outside);
    }
    response
}

fn field(ui: &mut Ui, id: &str, hint: &str, text: &mut String, password: bool) -> egui::Response {
    egui::Frame::new()
        .fill(t::white(0.06))
        .stroke(Stroke::new(1.0, t::HAIRLINE))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .id_salt(id)
                    .password(password)
                    .hint_text(hint)
                    .font(t::font(13.0, Weight::Regular))
                    .text_color(t::INK)
                    .margin(egui::Vec2::ZERO)
                    .frame(egui::Frame::NONE)
                    .desired_width(ui.available_width()),
            )
        })
        .inner
}

fn email_form(ui: &mut Ui, cx: &mut Ctx) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field(ui, "account-email", "Email", &mut cx.view.account_email, false);
        let password = field(ui, "account-password", "Password", &mut cx.view.account_password, true);
        let enter = password.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            if c::plain_button(ui, "Back", 12.0, Weight::Regular, t::MUTED).clicked() {
                cx.view.account_use_email = false;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let enabled = !cx.view.account_email.is_empty() && !cx.view.account_password.is_empty() && !cx.model.account.pending;
                let click = ui
                    .add_enabled_ui(enabled, |ui| c::amber_button(ui, if cx.model.account.pending { "Signing in…" } else { "Sign In" }))
                    .inner
                    .clicked();
                if (enter || click)
                    && let Some(action) = email_action(cx)
                {
                    cx.emit(action);
                }
            });
        });
    });
}

fn email_action(cx: &Ctx) -> Option<UiAction> {
    (!cx.view.account_email.is_empty() && !cx.view.account_password.is_empty() && !cx.model.account.pending)
        .then(|| UiAction::SignInEmail { email: cx.view.account_email.clone(), password: cx.view.account_password.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{Account, Inputs, Model, PanelMode},
        ui::{Assets, Env, UiState},
    };

    #[test]
    fn successful_sign_in_discards_password_and_email_form_without_side_effects() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let model = Model {
            panel: Some(PanelMode::October),
            account: crate::model::AccountState {
                signed_in: Some(Account { email: "me@example.com".into(), plan: Some("PRO".into()), lantern: None }),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut inputs = Inputs::default();
        let mut view = UiState { account_use_email: true, account_password: "do-not-keep".into(), ..Default::default() };
        let (_, actions, texts) = crate::ui::tests::frame(&ctx, &assets, &model, &mut inputs, &mut view, Vec::new());
        assert!(view.account_password.is_empty());
        assert!(!view.account_use_email);
        assert!(actions.is_empty());
        assert!(texts.iter().any(|text| text == "me@example.com · Pro plan"));
    }

    #[test]
    fn the_plan_says_what_lantern_gets() {
        use crate::model::LanternRights;
        let plan = serde_json::json!({"plan": "free", "lantern": {"lifetime": false, "trialEndsAt": "2026-11-01T19:00:00Z", "byok": true, "dailyQuestions": 50}});
        let rights = LanternRights::from_plan(&plan).unwrap();
        assert!(rights.byok && !rights.lifetime && rights.daily_questions == 50);
        assert_eq!(LanternRights::from_plan(&serde_json::json!({"plan": "free"})), None, "an older server");
        let before = crate::model::parse_iso8601("2026-10-28T00:00:00Z").unwrap();
        let utc = time::UtcOffset::UTC;
        assert_eq!(local_date(rights.trial_ends_ms.unwrap(), utc), "Nov 1, 2026");
        let ist = time::UtcOffset::from_hms(5, 30, 0).unwrap();
        assert_eq!(local_date(rights.trial_ends_ms.unwrap(), ist), "Nov 2, 2026", "local time");
        let lines = rights_lines(Some(&rights), before);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("Pro trial · ends Nov "));
        assert_eq!(lines[1], "Up to 50 questions a day on October's AI");
        let after = crate::model::parse_iso8601("2026-11-02T00:00:00Z").unwrap();
        assert_eq!(rights_lines(Some(&rights), after), ["Up to 50 questions a day on October's AI"], "an ended trial isn't shown");
        let lifetime = LanternRights { lifetime: true, trial_ends_ms: rights.trial_ends_ms, ..Default::default() };
        assert_eq!(rights_lines(Some(&lifetime), before), ["Lantern Lifetime"]);
        assert!(rights_lines(None, before).is_empty());
    }

    #[test]
    fn credentials_are_not_fixture_serialized_and_pending_sign_in_cannot_resubmit() {
        let ctx = egui::Context::default();
        t::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut model = Model::default();
        let mut inputs = Inputs::default();
        let mut view: UiState = serde_json::from_str(r#"{"account_email":"me@example.com","account_password":"not-accepted"}"#).unwrap();
        assert!(view.account_password.is_empty());
        view.account_password = "secret".into();
        let mut out = Vec::new();
        let env = Env::default();
        let cx = Ctx { model: &model, inputs: &mut inputs, view: &mut view, out: &mut out, assets: &assets, env: &env };
        assert_eq!(email_action(&cx), Some(UiAction::SignInEmail { email: "me@example.com".into(), password: "secret".into() }));
        model.account.pending = true;
        let cx = Ctx { model: &model, inputs: &mut inputs, view: &mut view, out: &mut out, assets: &assets, env: &env };
        assert_eq!(email_action(&cx), None);
    }
}
