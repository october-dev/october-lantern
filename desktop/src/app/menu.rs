//! The lantern's right-click menu (Swift `PillView.contextMenu`): a native menu on the Mac, a small
//! popup window here (the 50pt pill window would clip an in-window menu), drawn like a dark menu.
use super::Lantern;
use crate::model::PanelMode;
use crate::ui::{DictationOwner, UiAction, theme};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, ViewportBuilder, ViewportId, pos2, vec2};

const ROW: f32 = 22.0;
const PAD: f32 = 5.0;
const SEPARATOR: f32 = 9.0;
/// The system accent a dark menu highlights with.
const HIGHLIGHT: Color32 = Color32::from_rgb(10, 132, 255);

enum Entry {
    Item(String, UiAction),
    Separator,
}

#[derive(Default)]
pub struct State {
    /// Open, at this screen point (points).
    at: Option<Pos2>,
    was_focused: bool,
}

impl State {
    pub fn open(&mut self, at: Pos2) {
        self.at = Some(at);
        self.was_focused = false;
    }
}

impl Lantern {
    fn menu_entries(&self) -> Vec<Entry> {
        let dictate = if self.model.dictating { "Stop Dictating" } else { "Dictate a Reply" };
        let owner = self.inputs.drafts.recipient().map_or(DictationOwner::Assistant, |id| DictationOwner::Reply(id.to_string()));
        let mut entries = vec![
            Entry::Item("Ask Lantern".into(), UiAction::SetPanel(Some(PanelMode::Assistant))),
            Entry::Item("Your Agents".into(), UiAction::SetPanel(Some(PanelMode::Inbox))),
            Entry::Item("New Session…".into(), UiAction::StartNewSession),
            Entry::Item("Point & Ask".into(), UiAction::StartPointAsk),
            Entry::Item(dictate.into(), UiAction::ToggleDictation { owner }),
            Entry::Separator,
            Entry::Item("October…".into(), UiAction::SetPanel(Some(PanelMode::October))),
            Entry::Item("Settings…".into(), UiAction::ShowSettings),
        ];
        // The Mac quits from its menu bar. Without a tray host (GNOME without AppIndicator
        // support) this menu is the only way out, so it carries Quit too.
        if !self.model.caps.tray {
            entries.extend([Entry::Separator, Entry::Item("Quit October Lantern".into(), UiAction::Quit)]);
        }
        entries
    }

    /// The menu window, while open.
    pub(super) fn menu(&mut self, ctx: &egui::Context) {
        let Some(at) = self.windows.menu.at else { return };
        let entries = self.menu_entries();
        let font = theme::font(13.0, theme::Weight::Regular);
        let widest = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item(title, _) => {
                    Some(ctx.fonts_mut(|fonts| fonts.layout_no_wrap(title.clone(), font.clone(), Color32::WHITE).size().x))
                }
                Entry::Separator => None,
            })
            .fold(0.0, f32::max);
        let height = entries.iter().map(|entry| if matches!(entry, Entry::Item(..)) { ROW } else { SEPARATOR }).sum::<f32>() + 2.0 * PAD;
        let size = vec2(widest + 2.0 * PAD + 36.0, height);
        // Beside the pill, toward the screen's middle, starting at the click.
        let x = match self.model.prefs.edge {
            crate::model::Edge::Right => at.x - size.x - 4.0,
            crate::model::Edge::Left => at.x + 4.0,
        };
        #[allow(unused_mut)]
        let mut builder = ViewportBuilder::default()
            .with_icon(crate::ui::assets::window_icon())
            .with_title("October Lantern menu")
            .with_inner_size(size)
            .with_position(pos2(x, at.y))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false);
        #[cfg(target_os = "linux")]
        {
            builder = builder.with_window_type(egui::X11WindowType::PopupMenu);
        }
        let mut chosen = None;
        let mut close = false;
        let mut focused = None;
        ctx.show_viewport_immediate(ViewportId::from_hash_of("lantern-menu"), builder, |ui, _| {
            let area = ui.max_rect();
            let painter = ui.painter();
            painter.rect_filled(area, 7.0, Color32::from_rgba_unmultiplied(36, 36, 38, 246));
            painter.rect_stroke(area.shrink(0.5), 7.0, Stroke::new(1.0, theme::white(0.14)), StrokeKind::Inside);
            let mut y = area.min.y + PAD;
            for (index, entry) in entries.iter().enumerate() {
                match entry {
                    Entry::Separator => {
                        let line = y + SEPARATOR / 2.0;
                        painter.hline(area.min.x + 10.0..=area.max.x - 10.0, line, Stroke::new(1.0, theme::white(0.12)));
                        y += SEPARATOR;
                    }
                    Entry::Item(title, action) => {
                        let row = Rect::from_min_size(pos2(area.min.x + PAD, y), vec2(area.width() - 2.0 * PAD, ROW));
                        let response = ui.interact(row, ui.id().with(index), Sense::click());
                        if response.hovered() {
                            painter.rect_filled(row, 4.0, HIGHLIGHT);
                        }
                        painter.text(pos2(row.min.x + 10.0, row.center().y), Align2::LEFT_CENTER, title, font.clone(), Color32::WHITE);
                        if response.clicked() {
                            chosen = Some(action.clone());
                        }
                        y += ROW;
                    }
                }
            }
            close = ui.input(|input| input.key_pressed(egui::Key::Escape));
            focused = ui.input(|input| input.viewport().focused);
        });
        let menu = &mut self.windows.menu;
        // Clicking elsewhere (the menu loses focus) closes it, like a native menu.
        let lost_focus = focused == Some(false) && menu.was_focused;
        menu.was_focused |= focused == Some(true);
        if chosen.is_some() || close || lost_focus {
            menu.at = None;
        }
        if let Some(action) = chosen {
            self.actions.push(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(lantern: &Lantern) -> Vec<String> {
        lantern
            .menu_entries()
            .into_iter()
            .map(|entry| match entry {
                Entry::Item(title, _) => title,
                Entry::Separator => "—".into(),
            })
            .collect()
    }

    #[test]
    fn the_menu_matches_the_macs_and_dictation_toggles_its_title() {
        let (engine, _) = crate::sidecar::test_sender();
        let mut lantern = Lantern::new(engine, None);
        lantern.model.caps.tray = true;
        assert_eq!(
            titles(&lantern),
            ["Ask Lantern", "Your Agents", "New Session…", "Point & Ask", "Dictate a Reply", "—", "October…", "Settings…"]
        );
        lantern.model.dictating = true;
        assert_eq!(titles(&lantern)[4], "Stop Dictating");
        lantern.model.caps.tray = false;
        assert_eq!(titles(&lantern).last().map(String::as_str), Some("Quit October Lantern"), "without a tray, the only way to quit");
    }
}
