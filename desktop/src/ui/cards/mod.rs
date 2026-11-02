pub mod account;
pub mod desktop;

#[cfg(test)]
mod tests {
    use crate::{
        model::{EngineHealth, Inputs, Model, PanelMode},
        ui::{Assets, Ctx, Env, Provider, UiAction, UiState, render_panel, theme},
    };
    use eframe::egui;

    fn frame(
        ctx: &egui::Context,
        assets: &Assets,
        model: &Model,
        view: &mut UiState,
        events: Vec<egui::Event>,
    ) -> (Vec<UiAction>, Vec<(String, egui::Rect)>) {
        fn text(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(shape) => {
                    out.push((shape.galley.job.text.clone(), egui::Rect::from_min_size(shape.pos, shape.galley.size())))
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        text(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut actions = Vec::new();
        let mut inputs = Inputs::default();
        let env = Env::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                render_panel(ui, &mut Ctx { model, inputs: &mut inputs, view, out: &mut actions, assets, env: &env }, 900.0);
                assert!(ui.min_rect().width() <= 380.1);
            },
        );
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            text(&clipped.shape, &mut texts);
        }
        output.textures_delta.clear();
        (actions, texts)
    }

    fn click(ctx: &egui::Context, assets: &Assets, model: &Model, view: &mut UiState, title: &str) -> Vec<UiAction> {
        let texts = frame(ctx, assets, model, view, Vec::new()).1;
        let pos = texts.iter().find(|(text, _)| text == title).unwrap_or_else(|| panic!("missing {title}")).1.center();
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(
                frame(
                    ctx,
                    assets,
                    model,
                    view,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                    ],
                )
                .0,
            );
        }
        actions
    }

    #[test]
    fn provider_email_and_cancel_controls_emit_only_the_displayed_intents() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);
        let mut model = Model { panel: Some(PanelMode::October), engine: EngineHealth::Ready, ..Default::default() };
        let mut view = UiState::default();
        for (title, provider) in [("Google", Provider::Google), ("GitHub", Provider::GitHub), ("Apple", Provider::Apple)] {
            assert_eq!(click(&ctx, &assets, &model, &mut view, title), vec![UiAction::SignInProvider(provider)]);
        }
        assert!(click(&ctx, &assets, &model, &mut view, "Use email and password").is_empty());
        assert!(view.account_use_email);
        assert!(click(&ctx, &assets, &model, &mut view, "Sign In").is_empty());
        view.account_email = "me@example.com".into();
        view.account_password = "secret".into();
        assert_eq!(
            click(&ctx, &assets, &model, &mut view, "Sign In"),
            vec![UiAction::SignInEmail { email: "me@example.com".into(), password: "secret".into() }]
        );
        assert_eq!(view.account_password, "secret"); // The view never invents an acknowledgement.
        model.account.pending = true;
        assert!(click(&ctx, &assets, &model, &mut view, "Signing in…").is_empty());
        assert!(click(&ctx, &assets, &model, &mut view, "Back").is_empty());
        assert_eq!(click(&ctx, &assets, &model, &mut view, "Cancel"), vec![UiAction::CancelSignIn]);
    }
}
