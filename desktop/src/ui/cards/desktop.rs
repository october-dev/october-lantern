//! DesktopCard.swift, including the full card reused by Settings › October.
use super::super::{
    Ctx, UiAction, components as c,
    theme::{self as t, Weight},
};
use crate::model::{AgentKind, OctoberLink};
use eframe::egui::{self, Align, Layout, Margin, Rect, Sense, Stroke, Ui};

pub fn render(ui: &mut Ui, cx: &mut Ctx, compact: bool) {
    let link = cx.model.october.as_ref();
    let paired = link.is_some_and(|link| link.paired);
    egui::Frame::new().fill(t::FAINT).stroke(Stroke::new(1.0, if paired { t::GREEN.gamma_multiply(0.35) } else { t::HAIRLINE }))
        .corner_radius(14).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            if !compact { artwork(ui, cx, paired, status(link) == "pairing"); }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if compact { ui.add(egui::Image::new(&cx.assets.october).fit_to_exact_size(egui::Vec2::splat(30.0)).corner_radius(7)); }
                let (title, action) = action(link);
                let button_width = ui.painter().layout_no_wrap(title.into(), t::font(12.0, Weight::Semibold), t::INK).size().x + 24.0;
                let width = (ui.available_width() - button_width - 8.0).max(1.0);
                let height = ui.fonts_mut(|fonts| fonts.row_height(&t::font(13.5, Weight::Semibold)) + fonts.row_height(&t::font(10.5, Weight::Semibold))) + 8.0;
                ui.allocate_ui_with_layout(egui::vec2(width, height), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_width(width); ui.set_max_width(width);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add(egui::Label::new(c::text("October Desktop", 13.5, Weight::Semibold, t::INK)).truncate());
                    status_pill(ui, link);
                });
                let response = c::secondary_button(ui, title);
                let clicked = response.clicked();
                if status(link) == "connected" {
                    response.on_hover_text("Lantern forgets October's permission and goes back to only listing October's terminals. Remove Lantern in October's Connected apps too.");
                }
                if clicked { cx.emit(action); }
            });
            if !compact { c::label(ui, "October Desktop is October's app for running teams of agents on a shared canvas. Connect it and the agents on your canvas show up in Lantern with their names and questions, and your replies reach them through October.", 11.5, Weight::Regular, t::white(0.8)); }
            if compact { c::label(ui, "Bring your canvas agents into Lantern.", 11.5, Weight::Regular, t::white(0.85)); }
            c::label(ui, detail(link), 11.5, Weight::Regular, t::MUTED);
            if let Some(code) = link.and_then(|link| link.pairing_code.as_deref()) {
                egui::Frame::new().fill(t::AMBER.gamma_multiply(0.1)).corner_radius(10).inner_margin(Margin::same(10)).show(ui, |ui| {
                    {
                        let job = egui::text::LayoutJob::single_section(code.into(), egui::TextFormat { font_id:t::font(20.0, Weight::MonoBold), color:t::AMBER, extra_letter_spacing:2.0, ..Default::default() });
                        let galley = ui.painter().layout_job(job);
                        let width = (ui.available_width() - 12.0 - galley.size().x - 20.0).max(1.0);
                        let height = ui.painter().layout("Allow Lantern in October".into(), t::font(12.0, Weight::Semibold), t::INK, width).size().y
                            + 2.0 + ui.painter().layout("Check October shows this code:".into(), t::font(11.5, Weight::Regular), t::MUTED, width).size().y;
                        c::centered_row(ui, height.max(galley.size().y).max(12.0), |ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            c::spinner(ui, 12.0, t::MUTED, cx.env);
                            ui.allocate_ui_with_layout(egui::vec2(width, height), Layout::top_down(Align::Min), |ui| {
                            ui.set_min_width(width); ui.set_max_width(width); ui.spacing_mut().item_spacing.y = 2.0;
                            c::label(ui, "Allow Lantern in October", 12.0, Weight::Semibold, t::INK);
                            c::label(ui, "Check October shows this code:", 11.5, Weight::Regular, t::MUTED);
                        });
                        ui.add(egui::Label::new(galley).extend());
                        });
                    }
                });
            }
            if let Some(message) = link.and_then(|link| link.message.as_deref()) { c::label(ui, message, 11.5, Weight::Regular, t::RED); }
        });
}

fn status(link: Option<&OctoberLink>) -> &str {
    link.map_or("notInstalled", |link| link.status.as_str())
}

fn action(link: Option<&OctoberLink>) -> (&'static str, UiAction) {
    match status(link) {
        "connected" => ("Disconnect", UiAction::DesktopDisconnect),
        "readOnly" | "error" => ("Connect", UiAction::DesktopConnect),
        "pairing" => ("Cancel", UiAction::DesktopCancelPairing),
        "notRunning" => ("Open October", UiAction::OpenOctober),
        _ => ("Get October", UiAction::GetOctober),
    }
}

fn detail(link: Option<&OctoberLink>) -> String {
    match status(link) {
        "connected" => {
            let count = link.map_or(0, |link| link.agent_count);
            format!("Connected: {count} agent{} from October. Open on one shows it on the canvas.", if count == 1 { "" } else { "s" })
        }
        "readOnly" => "October is running. Click Connect, then allow Lantern in October and check the 6-digit code matches.".into(),
        "pairing" => "Waiting for you to allow Lantern in October…".into(),
        "notRunning" => "Open October Desktop, then connect from here.".into(),
        "error" => link
            .and_then(|link| link.message.as_deref())
            .map_or_else(|| "Couldn't talk to October.".into(), |message| format!("Couldn't talk to October: {message}")),
        // Only the platform noun changes; do not tell Windows/Linux users they are on a Mac.
        _ => "October Desktop isn't on this computer yet. Get it from october.dev.".into(),
    }
}

fn status_pill(ui: &mut Ui, link: Option<&OctoberLink>) {
    let (title, color) = match status(link) {
        "connected" => ("Connected", t::GREEN),
        "readOnly" => ("Read-only", t::AMBER),
        "pairing" => ("Pairing", t::AMBER),
        "notRunning" => ("Not running", t::MUTED),
        "error" => ("Problem", t::RED),
        _ => ("Not installed", t::MUTED),
    };
    let galley = ui.painter().layout_no_wrap(title.into(), t::font(10.5, Weight::Semibold), color);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0 + galley.size().x, galley.size().y + 4.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title));
    ui.painter().rect_filled(rect, rect.height() / 2.0, color.gamma_multiply(0.14));
    ui.painter().circle_filled(egui::pos2(rect.left() + 10.0, rect.center().y), 3.0, color);
    ui.painter().galley(egui::pos2(rect.left() + 17.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

fn artwork(ui: &mut Ui, cx: &mut Ctx, connected: bool, pairing: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 96.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 10.0, t::black(0.22));
    let mut y = rect.top() + 6.0;
    while y < rect.bottom() {
        let mut x = rect.left() + 6.0;
        while x < rect.right() {
            painter.circle_filled(egui::pos2(x, y), 0.8, t::white(0.08));
            x += 12.0;
        }
        y += 12.0;
    }
    let october = rect.min + egui::vec2(rect.width() * 0.22, 48.0);
    let lantern = rect.min + egui::vec2(rect.width() * 0.8, 48.0);
    for (name, fraction) in [("@claude-2", 0.2), ("@codex-1", 0.8)] {
        let point = rect.min + egui::vec2(rect.width() * 0.47, 96.0 * fraction);
        painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
            [october, egui::pos2((october.x + point.x) / 2.0, point.y), point],
            false,
            egui::Color32::TRANSPARENT,
            Stroke::new(1.0, t::white(0.18)),
        ));
        let galley = painter.layout_no_wrap(name.into(), t::font(9.5, Weight::Mono), t::white(0.75));
        let node = Rect::from_center_size(point, galley.size() + egui::vec2(12.0, 6.0));
        painter.rect(node, 6.0, t::black(0.35), Stroke::new(1.0, t::HAIRLINE), egui::StrokeKind::Inside);
        painter.galley(point - galley.size() / 2.0, galley, t::white(0.75));
    }
    let state = cx.view.desktop_flow.get_or_insert((connected, cx.env.now_ms));
    if state.0 != connected {
        *state = (connected, cx.env.now_ms);
    }
    let flowing = connected && !cx.env.reduce_motion && cx.env.now_ms - state.1 < 4000.0;
    let phase = if flowing { -((cx.env.now_ms / 1000.0 - 978307200.0) * 24.0).rem_euclid(11.0) as f32 } else { 0.0 };
    let color = if connected {
        t::GREEN.gamma_multiply(0.8)
    } else if pairing {
        t::AMBER.gamma_multiply(0.7)
    } else {
        t::white(0.25)
    };
    let start = october.x + 26.0;
    let end = lantern.x - 24.0;
    let dash = if connected { 6.0 } else { 3.0 };
    let mut x = start + phase;
    while x < end {
        if x + dash > start {
            painter
                .line_segment([egui::pos2(x.max(start), october.y), egui::pos2((x + dash).min(end), october.y)], Stroke::new(1.5, color));
        }
        x += dash + 5.0;
    }
    // The source's native October.app icon has no Windows/Linux model equivalent yet. Use its
    // explicit canonical harness fallback rather than claiming installed-app artwork support.
    let october_texture = cx.assets.harness(&AgentKind("october".into())).unwrap_or(&cx.assets.october);
    for (center, texture) in [(october, october_texture), (lantern, &cx.assets.logo)] {
        let image = Rect::from_center_size(center, egui::Vec2::splat(40.0));
        // epaint's blurred rectangle supplies a bounded, downward soft shadow behind the art.
        painter.add(egui::epaint::Shadow { offset: [0, 2], blur: 6, spread: 0, color: t::black(0.35) }.as_shape(image, 8));
        painter.image(texture.id(), image, Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), t::INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_actions_and_connected_counts_follow_the_swift_states() {
        for (state, expected) in [
            ("notInstalled", UiAction::GetOctober),
            ("notRunning", UiAction::OpenOctober),
            ("pairing", UiAction::DesktopCancelPairing),
            ("readOnly", UiAction::DesktopConnect),
            ("error", UiAction::DesktopConnect),
            ("connected", UiAction::DesktopDisconnect),
        ] {
            let link = OctoberLink { status: state.into(), agent_count: 1, ..Default::default() };
            assert_eq!(action(Some(&link)).1, expected);
            if state == "connected" {
                assert!(detail(Some(&link)).contains("1 agent from"));
            }
        }
        assert_eq!(action(None).1, UiAction::GetOctober);
        assert!(!detail(None).contains("Mac"));
        let link = OctoberLink { status: "connected".into(), agent_count: 2, ..Default::default() };
        assert!(detail(Some(&link)).contains("2 agents from"));
    }
}
