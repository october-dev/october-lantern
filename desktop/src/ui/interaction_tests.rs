//! Real panel hit tests: plain Swift Buttons must not become selectable child labels.
use super::*;
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect, Shape};

fn fixture(name: &str) -> crate::app::fixture::Fixture {
    serde_json::from_str(match name {
        "agents" => include_str!("../../fixtures/panel-agents.json"),
        "inbox" => include_str!("../../fixtures/panel-inbox.json"),
        "chat" => include_str!("../../fixtures/panel-chat.json"),
        "permission" => include_str!("../../fixtures/panel-permission.json"),
        _ => unreachable!(),
    })
    .unwrap()
}

fn render(
    ctx: &egui::Context,
    assets: &Assets,
    f: &mut crate::app::fixture::Fixture,
    events: Vec<Event>,
) -> (egui::FullOutput, Vec<UiAction>) {
    let mut actions = Vec::new();
    let mut output = ctx.run_ui(
        egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(380.0, 900.0))), events, ..Default::default() },
        |ui| {
            render_panel(
                ui,
                &mut Ctx {
                    model: &f.model,
                    inputs: &mut f.inputs,
                    view: &mut f.view,
                    out: &mut actions,
                    assets,
                    env: &Env { home: f.home.clone(), ..Default::default() },
                },
                900.0,
            );
        },
    );
    output.textures_delta.clear();
    (output, actions)
}

fn text_rect(output: &egui::FullOutput, value: &str) -> Rect {
    find_text(output, value).unwrap_or_else(|| panic!("missing text: {value}"))
}

fn find_text(output: &egui::FullOutput, value: &str) -> Option<Rect> {
    fn find(shape: &Shape, value: &str) -> Option<Rect> {
        match shape {
            Shape::Text(text) if text.galley.job.text == value => Some(Rect::from_min_size(text.pos, text.galley.size())),
            Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, value)),
            _ => None,
        }
    }
    output.shapes.iter().find_map(|shape| find(&shape.shape, value))
}

fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
    vec![Event::PointerMoved(pos), Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]
}

fn click(ctx: &egui::Context, assets: &Assets, f: &mut crate::app::fixture::Fixture, pos: Pos2) -> Vec<UiAction> {
    let (_, mut actions) = render(ctx, assets, f, pointer(pos, true));
    actions.extend(render(ctx, assets, f, pointer(pos, false)).1);
    actions
}

fn key(key: egui::Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
}

#[test]
fn feed_tabs_accept_title_gap_and_count_clicks_selected_or_not() {
    for source in ["agents", "inbox"] {
        for (title, panel) in [("WAITING", PanelMode::Inbox), ("AGENTS", PanelMode::Agents)] {
            for hit in ["title", "gap", "count"] {
                let ctx = egui::Context::default();
                theme::install(&ctx);
                let assets = Assets::load(&ctx);
                let mut f = fixture(source);
                // Both tabs have a badge even in the Agents fixture.
                f.model.inbox.new = vec![f.model.agents[0].id.clone()];
                f.model.inbox.earlier.clear();
                let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
                let label = text_rect(&output, title);
                let count = text_rect(&output, &if title == "WAITING" { 1 } else { f.model.agents.len() }.to_string());
                let pos = match hit {
                    "title" => label.center(),
                    "gap" => egui::pos2(label.right() + 3.0, label.center().y),
                    _ => count.center(),
                };
                assert_eq!(click(&ctx, &assets, &mut f, pos), vec![UiAction::SetPanel(Some(panel))], "{source}: {title} {hit}");
            }
        }
    }
}

#[test]
fn zero_count_tabs_remain_clickable_and_keyboard_activatable() {
    for (title, panel) in [("WAITING", PanelMode::Inbox), ("AGENTS", PanelMode::Agents)] {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut f = fixture("agents");
        f.model.agents.clear();
        f.model.inbox = Default::default();
        let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
        assert_eq!(click(&ctx, &assets, &mut f, text_rect(&output, title).center()), vec![UiAction::SetPanel(Some(panel))]);
        // Like ordinary egui buttons, mouse activation need not take keyboard focus.
        // Reach the real control through the panel's tab order instead.
        for _ in 0..if title == "WAITING" { 1 } else { 2 } {
            render(&ctx, &assets, &mut f, vec![key(egui::Key::Tab, true)]);
            render(&ctx, &assets, &mut f, vec![key(egui::Key::Tab, false)]);
        }
        for k in [egui::Key::Space, egui::Key::Enter] {
            let (_, mut actions) = render(&ctx, &assets, &mut f, vec![key(k, true)]);
            actions.extend(render(&ctx, &assets, &mut f, vec![key(k, false)]).1);
            assert_eq!(actions, vec![UiAction::SetPanel(Some(panel))], "{title}: {k:?}");
        }
    }
}

#[test]
fn agent_row_handle_title_location_badge_and_whitespace_open_the_same_chat() {
    for hit in ["handle", "title", "location", "badge", "whitespace"] {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut f = fixture("agents");
        let agent = &f.model.agents[0];
        let location = [agent.short_cwd(&f.home), agent.location()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
        let handle = text_rect(&output, "@claude-1");
        let location = text_rect(&output, &location);
        let pos = match hit {
            "handle" => handle.center(),
            "title" => text_rect(&output, "Review the launch flow").center(),
            "location" => location.center(),
            "badge" => egui::pos2(handle.left() - 23.0, handle.center().y + 7.0),
            _ => egui::pos2(24.0, location.bottom() + 3.0),
        };
        assert_eq!(click(&ctx, &assets, &mut f, pos), vec![UiAction::OpenChat { agent_id: "claude:1:1".into() }], "{hit}");
    }
}

#[test]
fn non_replyable_windows_chat_keeps_the_hint_and_sends_by_button_or_enter() {
    for supported in [true, false] {
        for by_button in [true, false] {
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let assets = Assets::load(&ctx);
            let mut f = fixture("chat");
            f.model.caps.platform = crate::model::Platform::Windows;
            f.model.agents[0].can_reply = false;
            f.model.agents[0].host = Some(crate::model::HostApp { app: "Windows Terminal".into(), ..Default::default() });
            f.model.chat.as_mut().unwrap().supported = supported;
            let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
            text_rect(&output, "Your reply is copied, and Lantern opens Windows Terminal");
            let ticket = f.inputs.drafts.ticket().unwrap();
            let actions = if by_button {
                let send = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        Shape::Circle(circle) if circle.radius == 9.0 && circle.fill == theme::AMBER => Some(circle.center),
                        _ => None,
                    })
                    .expect("enabled amber Send button");
                click(&ctx, &assets, &mut f, send)
            } else {
                render(&ctx, &assets, &mut f, vec![key(egui::Key::Enter, true)]).1
            };
            assert_eq!(actions, vec![UiAction::SendReply { agent_id: "claude:1:1".into() }]);
            assert_eq!(f.inputs.drafts.ticket(), Some(ticket), "only the coordinator acknowledges a send");
        }
    }
}

#[test]
fn permission_buttons_explain_what_they_press() {
    for (title, help) in [
        ("Allow", "Allow once: presses 1 in the agent's terminal, if this is still the prompt on screen"),
        ("Deny", "Decline (presses Escape), then tell the agent what to do instead"),
    ] {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx.global_style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        let assets = Assets::load(&ctx);
        let mut f = fixture("permission");
        let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
        let pos = text_rect(&output, title).center();
        render(&ctx, &assets, &mut f, vec![Event::PointerMoved(pos)]);
        let (output, actions) = render(&ctx, &assets, &mut f, vec![Event::PointerMoved(pos)]);
        assert!(actions.is_empty());
        text_rect(&output, help);
    }
}

#[test]
fn the_dictation_question_answers_by_button_or_escape_in_the_panel() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let assets = Assets::load(&ctx);
    let mut f = fixture("chat");
    f.model.dictation_offer = Some(crate::model::DictationOffer { text: crate::speech::offer(true, true), point_ask: false });
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    text_rect(&output, "Turn on dictation?");
    let download = text_rect(&output, "Download").center();
    let not_now = text_rect(&output, "Not now").center();
    assert_eq!(click(&ctx, &assets, &mut f, download), vec![UiAction::AnswerDictationOffer { download: true }]);
    assert_eq!(click(&ctx, &assets, &mut f, not_now), vec![UiAction::AnswerDictationOffer { download: false }]);
    let (_, actions) = render(&ctx, &assets, &mut f, vec![key(egui::Key::Escape, true)]);
    assert_eq!(actions, vec![UiAction::AnswerDictationOffer { download: false }], "Esc is Not now, not Back");
    // Point & Ask's question isn't shown in the panel.
    f.model.dictation_offer.as_mut().unwrap().point_ask = true;
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    assert!(find_text(&output, "Turn on dictation?").is_none());
}
