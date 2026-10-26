//! The always-on button (Swift `PillView`): collapsed, just the lantern, lit with a count when
//! agents are waiting on you; hovered (or with a panel open) it shows its four actions.
//! Metrics from PillView.swift: padding 6, lantern 34 (+2), sections 8 apart, actions 32 square
//! and 6 apart, a 22x1 divider with 2 below; 50 x 50 collapsed, 50 x 213 expanded.
use crate::model::{EngineHealth, Model, PanelMode};
use crate::ui::components::{material_tint, paint_glass};
use crate::ui::icons::{Icon, paint_icon};
use crate::ui::{Assets, UiAction, theme};
use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

const PAD: f32 = 6.0;
const LANTERN: f32 = 38.0;
const ICON: f32 = 32.0;
/// Agent dots (Swift `AgentDots`): at most 8, one 14pt row each, then a "+N" row.
const DOTS: usize = 8;
const DOT_ROW: f32 = 14.0;

/// The room the agent dots take under the lantern (with the 8 between sections).
pub fn dots_height(count: usize) -> f32 {
    match count {
        0 => 0.0,
        n => 8.0 + DOT_ROW * (n.min(DOTS) + usize::from(n > DOTS)) as f32,
    }
}
/// GlassContainer tint for the pill (Windows.swift). Without backdrop blur the glass is denser
/// (`material_tint`), so the wallpaper behind doesn't show through sharp.
const TINT: f32 = 0.45;

/// What the person did with the lantern this frame (the window moves with drags).
#[derive(Default)]
pub struct PillInput {
    pub drag_started: bool,
    pub dragging: bool,
    pub drag_stopped: bool,
    /// What's under the pointer: its help text, its rect in the pill, and the text's size. A 50pt
    /// window would clip an egui tooltip, so the shell shows it in a window of its own.
    pub help: Option<(String, Rect, Vec2)>,
    /// Right-clicked on the lantern, at this point in the pill: show its menu.
    pub menu_at: Option<Pos2>,
}

/// Help text style (the shell's tooltip window uses the same).
pub fn help_font() -> egui::FontId {
    theme::font(12.0, theme::Weight::Regular)
}

/// Draws the pill filling `ui`'s window and reports actions. `expanded` decides whether the four
/// actions show.
pub fn show(ui: &mut egui::Ui, model: &Model, assets: &Assets, expanded: bool, out: &mut Vec<UiAction>) -> PillInput {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    paint_glass(&painter, rect, rect.width() / 2.0, material_tint(TINT, model.caps.dense_glass));

    let lantern = Rect::from_min_size(rect.min + vec2(PAD, PAD), vec2(LANTERN, LANTERN));
    let response = ui.interact(lantern, ui.id().with("lantern"), Sense::click_and_drag());
    lantern_button(&painter, lantern, model, assets);
    engine_problem(&painter, lantern, &model.engine);
    if response.clicked() {
        out.push(UiAction::TogglePanel(PanelMode::Assistant));
    }
    // PillView.swift: the lantern's help leads with what's waiting; engine trouble is the "!"
    // badge's own help.
    let tooltip = match model.badge() {
        0 => "Ask Lantern".to_string(),
        n => format!("{n} new, waiting on you · Ask Lantern"),
    };
    let measure = |text: &str| painter.layout_no_wrap(text.to_string(), help_font(), Color32::WHITE).size();
    let mut input = PillInput {
        drag_started: response.drag_started(),
        dragging: response.dragged(),
        drag_stopped: response.drag_stopped(),
        help: None,
        menu_at: response.secondary_clicked().then(|| response.interact_pointer_pos()).flatten(),
    };
    if response.hovered() && !response.dragged() {
        let badge = problem_badge(lantern);
        let on_badge = response.hover_pos().is_some_and(|pos| badge.expand(1.0).contains(pos));
        input.help = match problem_text(&model.engine) {
            Some(problem) if on_badge => Some((problem.clone(), badge, measure(&problem))),
            _ => Some((tooltip.clone(), lantern, measure(&tooltip))),
        };
    }

    let agents = model.pill_agents();
    let mut y = lantern.max.y + 8.0;
    if !agents.is_empty() {
        for agent in agents.iter().take(DOTS) {
            let row = Rect::from_min_size(pos2(rect.center().x - LANTERN / 2.0, y), vec2(LANTERN, DOT_ROW));
            let response = ui.interact(row, ui.id().with(("dot", &agent.id)), Sense::click());
            let color = theme::state_color(agent.state);
            let radius = if response.hovered() { 5.4 } else { 4.0 };
            if model.inbox.new.contains(&agent.id) {
                painter.circle_stroke(row.center(), 7.0, Stroke::new(1.5, theme::opacity(color, 0.45)));
            }
            painter.circle_filled(row.center(), radius, color);
            let help = [Some(format!("@{}", agent.handle)), Some(theme::state_label(agent.state).to_string()), agent.project.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &help));
            if response.clicked() {
                out.push(UiAction::OpenChat { agent_id: agent.id.clone() });
            }
            if response.hovered() {
                input.help = Some((help.clone(), row, measure(&help)));
            }
            y += DOT_ROW;
        }
        if agents.len() > DOTS {
            let row = Rect::from_min_size(pos2(rect.center().x - LANTERN / 2.0, y), vec2(LANTERN, DOT_ROW));
            let more = agents.len() - DOTS;
            let response = ui.interact(row, ui.id().with("more-dots"), Sense::click());
            painter.text(row.center(), Align2::CENTER_CENTER, format!("+{more}"), theme::font(9.0, theme::Weight::Semibold), theme::MUTED);
            if response.clicked() {
                out.push(UiAction::TogglePanel(PanelMode::Agents));
            }
            if response.hovered() {
                let help = format!("{more} more agents");
                input.help = Some((help.clone(), row, measure(&help)));
            }
            y += DOT_ROW;
        }
        y += 8.0;
    }

    if expanded {
        painter.hline(rect.center().x - 11.0..=rect.center().x + 11.0, y + 0.5, Stroke::new(1.0, theme::STROKE));
        y += 1.0 + 2.0 + 6.0;
        let panel = model.panel;
        // PillView.swift's order: Ask Lantern, Point & Ask, a new agent, your agents.
        for (icon, help, active) in [
            (Icon::Sparkles, "Ask Lantern: ask anything, or tell it what to do", panel == Some(PanelMode::Assistant)),
            (Icon::PointAsk, "Point & Ask: select part of the screen and ask about it", false),
            (Icon::Plus, "Start a new agent", panel == Some(PanelMode::NewSession)),
            (Icon::Layers, "Your agents", panel.is_some_and(PanelMode::is_list)),
        ] {
            let slot = Rect::from_min_size(pos2(rect.center().x - ICON / 2.0, y), vec2(ICON, ICON));
            let response = ui.interact(slot, ui.id().with(help), Sense::click());
            if response.hovered() || active {
                painter.circle_filled(slot.center(), ICON / 2.0, theme::white(0.12));
            }
            // The sparkles turn amber while Ask Lantern is open; the others turn white.
            let color = match icon {
                Icon::Sparkles if active => theme::AMBER,
                _ if active => theme::INK,
                _ => theme::MUTED,
            };
            paint_icon(&painter, Rect::from_center_size(slot.center(), vec2(14.0, 14.0)), icon, color);
            if response.clicked() {
                out.push(match icon {
                    Icon::Sparkles => UiAction::TogglePanel(PanelMode::Assistant),
                    Icon::Layers => UiAction::TogglePanel(PanelMode::Inbox),
                    Icon::Plus => UiAction::StartNewSession,
                    _ => UiAction::StartPointAsk,
                });
            }
            if response.hovered() {
                input.help = Some((help.to_string(), slot, measure(help)));
            }
            y += ICON + 6.0;
        }
    }
    input
}

fn problem_text(engine: &EngineHealth) -> Option<String> {
    match engine {
        EngineHealth::Ready | EngineHealth::Starting => None,
        EngineHealth::Restarting => Some("Lantern's engine stopped and is restarting. Agents may be out of date.".into()),
        EngineHealth::Failed(message) => Some(format!("Lantern's engine isn't running: {message}")),
    }
}

/// Where the engine-problem badge sits: the lantern's top right.
fn problem_badge(lantern: Rect) -> Rect {
    Rect::from_center_size(lantern.right_top() + vec2(-5.0, 5.0), vec2(14.0, 14.0))
}

/// An exclamation badge at the lantern's top right while the engine isn't running normally.
fn engine_problem(painter: &egui::Painter, lantern: Rect, engine: &EngineHealth) {
    let color = match engine {
        EngineHealth::Restarting => theme::AMBER,
        EngineHealth::Failed(_) => theme::RED,
        _ => return,
    };
    let badge = problem_badge(lantern);
    painter.circle_filled(badge.center(), 7.0, color);
    paint_icon(painter, badge.shrink(3.0), Icon::Exclamation, theme::black(0.85));
}

/// The lantern: glowing amber with a count when turns are new, softer when you've seen them,
/// a faint orange while agents work, dimmed when nothing is happening.
fn lantern_button(painter: &egui::Painter, rect: Rect, model: &Model, assets: &Assets) {
    let count = model.badge();
    let waiting = !model.inbox.new.is_empty() || !model.inbox.earlier.is_empty();
    let working = model.any_working();
    let center = rect.center();
    if count > 0 || waiting || working {
        // SwiftUI blurs a circle (radius 9); egui has no blur, so concentric rings fade it out.
        let (color, strength, scale) = match (count > 0, waiting) {
            (true, _) => (theme::AMBER, 0.55, 1.05),
            (false, true) => (theme::AMBER, 0.25, 0.9),
            _ => (Color32::from_rgb(255, 149, 0), 0.25, 0.9),
        };
        for step in 0..8 {
            let t = step as f32 / 8.0;
            painter.circle_filled(center, 17.0 * scale + 9.0 * t, theme::opacity(color, strength * (1.0 - t) * 0.35));
        }
    }
    let lit = count > 0 || waiting || working;
    // Aspect-fit in 34 x 34. Idle: Swift's `.saturation(0.55).opacity(0.8)` (the desaturated art at .8).
    let [w, h] = assets.logo.size().map(|side| side as f32);
    let fit = 34.0 / w.max(h);
    let logo = Rect::from_center_size(center, vec2(w * fit, h * fit));
    let (texture, tint) = if lit { (&assets.logo, Color32::WHITE) } else { (&assets.logo_idle, theme::white(0.8)) };
    painter.image(texture.id(), logo, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), tint);
    if count > 0 {
        let galley = painter.layout_no_wrap(count.to_string(), theme::font(10.0, theme::Weight::Rounded), theme::black(0.85));
        let width = (galley.size().x + 10.0).max(17.0);
        // Inside the pill's circle, offset 3 down (PillView.swift).
        let badge = Rect::from_center_size(pos2(center.x, rect.max.y - 17.0 / 2.0 + 1.0), vec2(width, 17.0));
        painter.rect_filled(badge, CornerRadius::same(9), theme::AMBER);
        painter.rect_stroke(badge, CornerRadius::same(9), Stroke::new(1.0, theme::black(0.5)), StrokeKind::Outside);
        painter.galley(Align2::CENTER_CENTER.align_size_within_rect(galley.size(), badge).min, galley, Color32::BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pill's glass fill alpha, with the Mac's tint or the denser no-blur one.
    fn glass_alpha(dense_glass: bool) -> u8 {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut model = Model::default();
        model.caps.dense_glass = dense_glass;
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(50.0, 50.0))), ..Default::default() },
            |ui| {
                show(ui, &model, &assets, false, &mut Vec::new());
            },
        );
        output.textures_delta.clear();
        let first = output.shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Vec(layers) => match layers.first() {
                Some(egui::Shape::Rect(fill)) => Some(fill.fill.a()),
                _ => None,
            },
            _ => None,
        });
        first.expect("the pill paints its glass first")
    }

    /// The help shown with the pointer at `pos` (two frames, so hover settles).
    fn help_at(model: &Model, expanded: bool, pos: Pos2) -> Option<String> {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let size = crate::shell::pill_size(expanded, model.pill_agents().len());
        let mut help = None;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events: vec![egui::Event::PointerMoved(pos)],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| help = show(ui, model, &assets, expanded, &mut Vec::new()).help.map(|(text, ..)| text));
            output.textures_delta.clear();
        }
        help
    }

    #[test]
    fn tooltips_follow_the_mac_pill() {
        let mut model = Model::default();
        let lantern = pos2(PAD + LANTERN / 2.0, PAD + LANTERN / 2.0);
        assert_eq!(help_at(&model, false, lantern).as_deref(), Some("Ask Lantern"));
        model.inbox.new = vec!["a".into(), "b".into()];
        assert_eq!(help_at(&model, false, lantern).as_deref(), Some("2 new, waiting on you · Ask Lantern"));
        // Engine trouble is the "!" badge's help, not the lantern's.
        model.engine = EngineHealth::Restarting;
        assert_eq!(help_at(&model, false, lantern).as_deref(), Some("2 new, waiting on you · Ask Lantern"));
        let badge = problem_badge(Rect::from_min_size(pos2(PAD, PAD), vec2(LANTERN, LANTERN))).center();
        assert_eq!(help_at(&model, false, badge), problem_text(&model.engine));
        // Expanded, top to bottom: Ask Lantern, Point & Ask, a new agent, your agents.
        let first = PAD + LANTERN + 8.0 + 9.0 + ICON / 2.0;
        let helps: Vec<_> = (0..4).map(|i| help_at(&model, true, pos2(25.0, first + i as f32 * (ICON + 6.0))).unwrap()).collect();
        assert_eq!(
            helps,
            [
                "Ask Lantern: ask anything, or tell it what to do",
                "Point & Ask: select part of the screen and ask about it",
                "Start a new agent",
                "Your agents"
            ]
        );
    }

    #[test]
    fn only_dense_glass_is_denser_than_the_macs() {
        // Swift's 0.45 GlassContainer tint plus glassSurface's 0.35 black: 0.6425, as on the Mac
        // (Windows too); at least 0.85 on GNOME, so a sharp wallpaper doesn't show through.
        assert_eq!(glass_alpha(false), (0.6425_f32 * 255.0).round() as u8);
        assert!(glass_alpha(true) >= 216, "{}", glass_alpha(true));
    }

    #[test]
    fn a_dot_for_every_running_agent_opens_its_chat() {
        use crate::model::{Agent, AgentState};
        let mut model = Model::default();
        let agent = |id: &str, state| Agent {
            id: id.into(),
            handle: id.replace(':', "-"),
            state,
            project: Some("lantern".into()),
            ..Default::default()
        };
        // Needing you first (only 8 show), then by id within a state.
        model.agents = vec![agent("claude:1", AgentState::Working), agent("codex:2", AgentState::NeedsInput)];
        let first = pos2(25.0, PAD + LANTERN + 8.0 + DOT_ROW / 2.0);
        assert_eq!(help_at(&model, false, first).as_deref(), Some("@codex-2 · Needs you · lantern"));
        assert_eq!(help_at(&model, false, first + vec2(0.0, DOT_ROW)).as_deref(), Some("@claude-1 · Working · lantern"));
        // Clicked: its chat.
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let size = crate::shell::pill_size(false, model.pill_agents().len());
        assert_eq!(size, vec2(50.0, 50.0 + 8.0 + 2.0 * DOT_ROW));
        let mut out = Vec::new();
        for pressed in [None, Some(true), Some(false)] {
            let mut events = vec![egui::Event::PointerMoved(first)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: first,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                show(ui, &model, &assets, false, &mut out);
            });
            output.textures_delta.clear();
        }
        assert_eq!(out, vec![UiAction::OpenChat { agent_id: "codex:2".into() }]);
        // Nine or more: eight dots and "+N".
        model.agents = (0..10).map(|i| agent(&format!("claude:{i}"), AgentState::Idle)).collect();
        assert_eq!(dots_height(model.pill_agents().len()), 8.0 + 9.0 * DOT_ROW);
        assert_eq!(help_at(&model, false, first + vec2(0.0, 8.0 * DOT_ROW)).as_deref(), Some("2 more agents"));
    }
}
