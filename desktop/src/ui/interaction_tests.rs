//! Real panel hit tests: plain Swift Buttons must not become selectable child labels.
use super::*;
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect, Shape};

fn fixture(name: &str) -> crate::app::fixture::Fixture {
    serde_json::from_str(match name {
        "agents" => include_str!("../../fixtures/panel-agents.json"),
        "inbox" => include_str!("../../fixtures/panel-inbox.json"),
        "chat" => include_str!("../../fixtures/panel-chat.json"),
        "permission" => include_str!("../../fixtures/panel-permission.json"),
        "question" => include_str!("../../fixtures/panel-question.json"),
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

fn answer(answer: crate::model::AgentAnswer, prompt: &str) -> Vec<UiAction> {
    vec![UiAction::AnswerQuestion { agent_id: "claude:1:1".into(), prompt_id: prompt.into(), answer }]
}

#[test]
fn a_permission_card_offers_the_terminals_options() {
    use crate::model::AgentAnswer;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let assets = Assets::load(&ctx);
    let mut f = fixture("permission");
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    text_rect(&output, "Then tell @claude-1 what to do instead");
    assert!(find_text(&output, "Always allow cargo test:*").is_none(), "only when the terminal offers it");
    let no = text_rect(&output, "No").center();
    assert_eq!(click(&ctx, &assets, &mut f, no), answer(AgentAnswer::No, "fixture-prompt-1"));

    let mut f = fixture("permission");
    f.model.agents[0].choices = Some(crate::model::Choices { always: Some("Always allow cargo test:*".into()), ..Default::default() });
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    let always = text_rect(&output, "Always allow cargo test:*").center();
    assert_eq!(click(&ctx, &assets, &mut f, always), answer(AgentAnswer::Always, "fixture-prompt-1"));
}

#[test]
fn claudes_question_is_answered_by_its_option_or_number_key_but_not_while_typing() {
    use crate::model::AgentAnswer;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let assets = Assets::load(&ctx);
    let mut f = fixture("question");
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    text_rect(&output, "Free, Pro and Max side by side");
    let one_plan = text_rect(&output, "One plan").center();
    assert_eq!(click(&ctx, &assets, &mut f, one_plan), answer(AgentAnswer::Picks { picks: vec![vec![1]] }, "t-fixture-1"));

    // Digits require deliberate focus on this exact card, including modified digits.
    let ctrl = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
    let mut f = fixture("question");
    f.inputs.drafts.address(Some("claude:1:1"));
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    let (_, actions) = render(&ctx, &assets, &mut f, vec![ctrl(egui::Key::Num3)]);
    assert!(actions.is_empty(), "a newly arrived question never steals a shortcut");
    // Keep Num3 held while deliberately focusing the card; egui derives repeat from held-key state.
    assert!(click(&ctx, &assets, &mut f, text_rect(&output, "Use number keys").center()).is_empty());
    let repeated =
        egui::Event::Key { key: egui::Key::Num3, physical_key: None, pressed: true, repeat: true, modifiers: egui::Modifiers::COMMAND };
    let (_, actions) = render(&ctx, &assets, &mut f, vec![repeated]);
    assert!(actions.is_empty(), "holding a key never answers");
    render(&ctx, &assets, &mut f, vec![key(egui::Key::Num3, false)]);
    let (_, actions) = render(&ctx, &assets, &mut f, vec![ctrl(egui::Key::Num3)]);
    assert_eq!(actions, answer(AgentAnswer::Picks { picks: vec![vec![2]] }, "t-fixture-1"));
    let (_, actions) = render(&ctx, &assets, &mut f, vec![ctrl(egui::Key::Num9)]);
    assert!(actions.is_empty(), "no ninth option");
    f.model.agents[0].prompt_id = Some("replacement-question".into());
    render(&ctx, &assets, &mut f, vec![key(egui::Key::Num1, false)]);
    let (_, actions) = render(&ctx, &assets, &mut f, vec![ctrl(egui::Key::Num1)]);
    assert!(actions.is_empty(), "replacement questions require their own deliberate focus");

    // Once answered, it says Sending… until the keys went in, then Sent, and can't be answered again.
    let mut f = fixture("question");
    f.model.answered.insert("t-fixture-1".into(), AgentAnswer::Picks { picks: vec![vec![1]] });
    f.model.answer_delivery.insert("t-fixture-1".into(), crate::model::AnswerDelivery::Sending);
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    text_rect(&output, "Sending…");
    f.model.answer_delivery.insert("t-fixture-1".into(), crate::model::AnswerDelivery::Sent);
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    text_rect(&output, "Sent");
    let three = text_rect(&output, "Three tiers").center();
    assert!(click(&ctx, &assets, &mut f, three).is_empty());
}

#[test]
fn several_questions_are_sent_together_once_each_has_an_answer() {
    use crate::model::{AgentAnswer, ChoiceOption, ChoiceQuestion};
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let assets = Assets::load(&ctx);
    let mut f = fixture("question");
    let option = |label: &str| ChoiceOption { label: label.into(), description: None };
    let choices = f.model.agents[0].choices.as_mut().unwrap();
    choices.questions.push(ChoiceQuestion {
        question: "Which checks?".into(),
        header: Some("Checks".into()),
        multi_select: true,
        options: vec![option("Unit tests"), option("Screenshots"), option("Accessibility")],
    });
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    text_rect(&output, "Pick any");
    let send = text_rect(&output, "Send answers").center();
    assert!(click(&ctx, &assets, &mut f, send).is_empty(), "not until every question has an answer");
    for label in ["A table", "Unit tests", "Accessibility", "Screenshots", "Screenshots"] {
        let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
        let pos = text_rect(&output, label).center();
        assert!(click(&ctx, &assets, &mut f, pos).is_empty(), "{label} only ticks");
    }
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    let send = text_rect(&output, "Send answers").center();
    assert_eq!(click(&ctx, &assets, &mut f, send), answer(AgentAnswer::Picks { picks: vec![vec![2], vec![0, 2]] }, "t-fixture-1"));
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

fn phone_fixture() -> crate::app::fixture::Fixture {
    let mut f = fixture("inbox");
    f.model.panel = Some(PanelMode::October);
    f.model.account.signed_in = Some(crate::model::Account { email: "person@example.com".into(), ..Default::default() });
    f.model.phone = Some(
        serde_json::from_value(serde_json::json!({"status":"connected", "pairing":{
            "intentId":"intent", "bind":"phone", "code":"123456", "label":"Test iPhone", "platform":"ios", "expiresAt":9999999999999_f64
        }}))
        .unwrap(),
    );
    f
}

#[test]
fn phone_approval_carries_the_exact_comparison_and_busy_buttons_do_nothing() {
    for busy in [false, true] {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut f = phone_fixture();
        f.model.phone.as_mut().unwrap().busy = busy;
        render(&ctx, &assets, &mut f, Vec::new());
        let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
        assert!(find_text(&output, "Make sure this matches your phone").is_some());
        let pos = text_rect(&output, "Approve").center();
        let actions = click(&ctx, &assets, &mut f, pos);
        assert_eq!(
            actions,
            if busy {
                vec![]
            } else {
                vec![UiAction::Phone(crate::model::phone::Action::Decide {
                    intent_id: "intent".into(),
                    bind: "phone".into(),
                    code: "123456".into(),
                    allow: true,
                })]
            }
        );
    }
}

#[test]
fn expired_phone_code_is_not_copied_and_signed_out_users_get_account_guidance() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let assets = Assets::load(&ctx);
    let mut f = phone_fixture();
    let p = f.model.phone.as_mut().unwrap().pairing.as_mut().unwrap();
    p.code = None;
    p.qr = Some("synthetic-test-link".into());
    p.expires_at = 0.0;
    render(&ctx, &assets, &mut f, Vec::new());
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    assert!(find_text(&output, "This pairing code has expired.").is_some());
    assert!(find_text(&output, "Copy pairing link").is_none());
    assert!(find_text(&output, "New code").is_some());
    f.model.account.signed_in = None;
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    assert!(find_text(&output, "Phone").is_none());
    assert!(find_text(&output, "Your phone uses the same October account. Sign in above to connect it.").is_some());
}

#[test]
fn getting_the_app_and_showing_its_pairing_code_are_distinct_actions() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let assets = Assets::load(&ctx);
    let mut f = phone_fixture();
    let phone = f.model.phone.as_mut().unwrap();
    phone.pairing = None;
    phone.install_url = Some("https://example.com/phone-install".into());
    f.view.phone_setup = true;
    render(&ctx, &assets, &mut f, Vec::new());
    let (output, _) = render(&ctx, &assets, &mut f, Vec::new());
    assert!(find_text(&output, "Get the October app").is_some());
    assert!(find_text(&output, "Open your iPhone Camera and point it here").is_none());
    let pos = text_rect(&output, "Show pairing code").center();
    assert_eq!(click(&ctx, &assets, &mut f, pos), vec![UiAction::Phone(crate::model::phone::Action::Pair)]);
}
