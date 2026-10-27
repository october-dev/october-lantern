use super::{
    Ctx, UiAction, components as c,
    theme::{self as t, Weight},
};
use eframe::egui::{self, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Form {
    pub name: String,
    pub prompt: String,
    pub cwd: String,
    pub kind: String,
    pub model: Option<String>,
    pub timezone: String,
    pub cadence: String,
    pub hour: u32,
    pub minute: u32,
    pub weekday: u32,
    pub preferences: String,
    #[serde(skip)]
    pub open: bool,
    #[serde(skip)]
    pub id: Option<String>,
    #[serde(skip)]
    pub revision: Option<i64>,
}
impl Default for Form {
    fn default() -> Self {
        Self {name:String::new(),prompt:String::new(),cwd:String::new(),kind:"october".into(),model:None,timezone:"UTC".into(),cadence:"daily".into(),hour:9,minute:0,weekday:0,preferences:"Ask before spending money, sending messages, deleting files outside the project, installing apps or changing system settings.".into(),open:false,id:None,revision:None}
    }
}

fn action(cx: &mut Ctx, action: &str, r: &Value, change: Option<&str>) {
    let mut value = json!({"action":action,"id":r["id"],"revision":r["revision"]});
    if let Some(change) = change {
        value["change"] = json!(change);
    }
    cx.emit(UiAction::Routine(value));
}

pub fn render(ui: &mut Ui, cx: &mut Ctx) {
    egui::ScrollArea::vertical().max_height(560.0).show(ui,|ui| {
        c::label(ui,"Routines",18.0,Weight::Medium,t::INK);
        ui.label("Run tasks on this computer at a time you choose.");
        ui.small("Lantern and this computer must stay available. Missed runs are not caught up. Pausing or deleting a schedule leaves existing work running.");
        if let Some(error)=&cx.model.routine_error {ui.colored_label(t::AMBER,error);}
        for routine in cx.model.routines.clone() {
            egui::Frame::group(ui.style()).show(ui,|ui| {
                let draft=&routine["draft"];
                ui.strong(draft["name"].as_str().unwrap_or("Routine"));
                ui.label(draft["prompt"].as_str().unwrap_or_default());
                ui.small(format!("{} · {} at {:02}:{:02} · {}",if routine["enabled"]==true {"On"} else {"Paused"},draft["cadence"].as_str().unwrap_or_default(),draft["hour"].as_u64().unwrap_or(0),draft["minute"].as_u64().unwrap_or(0),draft["timezone"].as_str().unwrap_or_default()));
                if let Some(next)=routine["nextRun"].as_str(){ui.small(format!("Next: {next}"));}
                if let Some(state)=routine["lastRun"]["state"].as_str(){ui.small(format!("Last run: {state}"));}
                if let Some(message)=routine["lastRun"]["message"].as_str(){ui.small(message);}
                ui.horizontal(|ui| {
                    if ui.button("Run now").clicked(){action(cx,"run",&routine,None);}
                    if ui.button(if routine["enabled"]==true {"Pause"}else{"Enable"}).clicked(){action(cx,"change",&routine,Some(if routine["enabled"]==true {"pause"}else{"enable"}));}
                    if ui.button("Edit").clicked() && let Ok(mut form)=serde_json::from_value::<Form>(draft.clone()) {
                            form.id=routine["id"].as_str().map(str::to_string);form.revision=routine["revision"].as_i64();form.open=true;cx.view.routine_form=form;
                    }
                    if ui.button("Delete").clicked(){action(cx,"change",&routine,Some("delete"));}
                });
            });
        }
        if !cx.view.routine_form.open {
            if ui.button("Create a routine").clicked(){cx.view.routine_form=Form{open:true,cwd:cx.env.home.clone(),..Default::default()};}
            return;
        }
        let mut form=cx.view.routine_form.clone();
        ui.separator();ui.strong(if form.id.is_some(){"Edit routine"}else{"New routine"});
        ui.add(egui::TextEdit::singleline(&mut form.name).hint_text("Name"));
        ui.add(egui::TextEdit::multiline(&mut form.prompt).hint_text("What should the agent do?").desired_rows(3));
        ui.add(egui::TextEdit::singleline(&mut form.cwd).hint_text("Full project folder path"));
        ui.horizontal(|ui|{ui.label("Agent");for kind in ["october","claude","codex"]{ui.selectable_value(&mut form.kind,kind.into(),kind);}});
        ui.horizontal(|ui|{ui.label("Repeat");for cadence in ["daily","weekdays","weekly"]{ui.selectable_value(&mut form.cadence,cadence.into(),cadence);}});
        if form.cadence=="weekly" {egui::ComboBox::from_label("Day").selected_text(["Monday","Tuesday","Wednesday","Thursday","Friday","Saturday","Sunday"][form.weekday.min(6) as usize]).show_ui(ui,|ui|{for (i,day) in ["Monday","Tuesday","Wednesday","Thursday","Friday","Saturday","Sunday"].iter().enumerate(){ui.selectable_value(&mut form.weekday,i as u32,*day);}});}
        ui.horizontal(|ui|{ui.label("Time");ui.add(egui::DragValue::new(&mut form.hour).range(0..=23));ui.label(":");ui.add(egui::DragValue::new(&mut form.minute).range(0..=59));});
        ui.add(egui::TextEdit::singleline(&mut form.timezone).hint_text("IANA timezone, e.g. America/Los_Angeles"));
        ui.add(egui::TextEdit::multiline(&mut form.preferences).hint_text("Preferences").desired_rows(3));
        ui.small("Preferences are instructions, not enforced restrictions. Runs use the selected agent's account and credits. No overlapping runs. Repeated DST times run once; nonexistent times are skipped.");
        ui.horizontal(|ui|{
            if ui.add_enabled(!form.name.trim().is_empty()&&!form.prompt.trim().is_empty(),egui::Button::new("Confirm and save")).clicked(){
                let draft=serde_json::to_value(&form).unwrap_or_default();
                cx.emit(UiAction::Routine(json!({"action":"save","id":form.id,"revision":form.revision,"draft":draft})));
                form.open=false;
            }
            if ui.button("Cancel").clicked(){form.open=false;}
        });
        cx.view.routine_form=form;
    });
}
