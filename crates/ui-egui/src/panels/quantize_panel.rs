//! The MIDI editor's Quantize panel: what Quantize uses (grid or groove, note value, grouping,
//! swing, Start / End / Range / Velocity), five presets, and the quantize actions.
use crate::SoundApp;
use crate::theme::Tokens;
use egui::Ui;
use serde_json::json;
use soundcraft_model::ClipId;

/// Quantize presets (Studio One keeps five).
const PRESETS: usize = 5;
const NOTE_VALUES: [(&str, i64); 8] =
    [("Edit grid", 0), ("1/1", 3840), ("1/2", 1920), ("1/4", 960), ("1/8", 480), ("1/16", 240), ("1/32", 120), ("1/64", 60)];
const GROUPINGS: [(&str, &str); 5] =
    [("Straight", "straight"), ("Triplet", "triplet"), ("Quintuplet", "quintuplet"), ("Septuplet", "septuplet"), ("Dotted", "dotted")];

/// One stored quantize setting.
fn value(app: &SoundApp, key: &str, default: f64) -> f64 {
    let v = app.engine.session().edit.value(&format!("quantize.{key}"), default);
    if v.is_finite() { v } else { default }
}

/// The button label: "Q 1/16 v", "Q 1/8T v", "Q Groove v".
pub(crate) fn quantize_label(app: &SoundApp) -> String {
    if value(app, "mode", 0.0) >= 0.5 && app.engine.session().groove.is_some() {
        return "Q Groove v".into();
    }
    let grid = value(app, "grid", 0.0) as i64;
    let note = NOTE_VALUES.iter().find(|(_, t)| *t == grid).map_or("1/16", |(n, _)| *n);
    let note = if grid == 0 { "Grid" } else { note };
    let mark = match value(app, "grouping", 0.0) as usize {
        1 => "T",
        2 => "Q5",
        3 => "S7",
        4 => ".",
        _ => "",
    };
    format!("Q {note}{mark} v")
}

/// The panel, shown from the MIDI editor's Quantize button.
pub(crate) fn quantize_panel(app: &mut SoundApp, ui: &mut Ui, clip: ClipId) {
    ui.set_min_width(300.0);
    let set = |app: &mut SoundApp, p: serde_json::Value| {
        let _ = app.run("event.quantize_settings", p);
    };
    let has_groove = app.engine.session().groove.is_some();
    let groove_mode = value(app, "mode", 0.0) >= 0.5 && has_groove;
    ui.horizontal(|ui| {
        if ui.selectable_label(!groove_mode, "Grid").on_hover_text("Quantize to the grid").clicked() {
            set(app, json!({"mode": "grid"}));
        }
        let groove_name = app.engine.session().groove.as_ref().map(|g| g.name.clone());
        let tip = match &groove_name {
            Some(n) => format!("Quantize to the groove taken from {n}"),
            None => "No groove yet: Extract Groove from a clip first".into(),
        };
        if ui.add_enabled(has_groove, egui::Button::selectable(groove_mode, "Groove")).on_hover_text(tip).clicked() {
            set(app, json!({"mode": "groove"}));
        }
    });
    let grid = value(app, "grid", 0.0) as i64;
    let grouping = value(app, "grouping", 0.0) as usize;
    ui.horizontal(|ui| {
        let current = NOTE_VALUES.iter().find(|(_, t)| *t == grid).map_or("1/16", |(n, _)| *n);
        craft_ui::ComboBox::from_id_salt("quantize-note").selected_text(current).width(90.0).show_ui(ui, |ui| {
            for (name, ticks) in NOTE_VALUES {
                if ui.selectable_label(grid == ticks, name).clicked() {
                    let g = if ticks == 0 { json!("edit") } else { json!(ticks) };
                    set(app, json!({"grid": g, "grouping": GROUPINGS.get(grouping).map_or("straight", |g| g.1)}));
                }
            }
        });
        let current = GROUPINGS.get(grouping).map_or("Straight", |g| g.0);
        craft_ui::ComboBox::from_id_salt("quantize-grouping").selected_text(current).width(100.0).show_ui(ui, |ui| {
            for (k, (name, id)) in GROUPINGS.iter().enumerate() {
                if ui.selectable_label(grouping == k, *name).clicked() {
                    set(app, json!({"grouping": id}));
                }
            }
        });
    });
    for (key, label, default, tip) in [
        ("swing", "Swing", 0.0, "Delays every second grid line; 100% is a full triplet swing"),
        ("start", "Start", 100.0, "How far note starts move to the grid"),
        ("end", "End", 0.0, "How far note ends move to the grid"),
        ("range", "Range", 100.0, "Only notes this close to a grid line move (100% = all)"),
        ("velocity", "Velocity", 0.0, "Groove only: how far velocities move to the groove's"),
    ] {
        let mut v = value(app, key, default);
        let resp = ui.add(egui::Slider::new(&mut v, 0.0..=100.0).suffix("%").text(label).integer()).on_hover_text(tip);
        if resp.changed() {
            let mut m = serde_json::Map::new();
            m.insert(key.to_string(), json!(v));
            set(app, serde_json::Value::Object(m));
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Presets");
        for n in 1..=PRESETS {
            let stored = app.engine.session().edit.values.contains_key(&format!("quantize.preset.{n}.quantize.start"));
            let resp = ui.add_enabled(stored, egui::Button::new(n.to_string()).small()).on_hover_text(format!("Recall preset {n}"));
            if resp.clicked() {
                let _ = app.run("event.quantize_preset", json!({"slot": n}));
            }
        }
        ui.menu_button("Store v", |ui| {
            for n in 1..=PRESETS {
                if ui.button(format!("Store as preset {n}")).clicked() {
                    let _ = app.run("event.quantize_preset", json!({"slot": n, "store": true}));
                    ui.close();
                }
            }
        });
    });
    ui.separator();
    let clips = json!([clip.0]);
    let mut op: Option<(&str, serde_json::Value)> = None;
    ui.horizontal_wrapped(|ui| {
        if ui.button("Quantize").on_hover_text("Quantize this clip with the settings above").clicked() {
            op = Some(("event.quantize", json!({})));
        }
        if ui.button("Quantize 50%").on_hover_text("Half way to the grid").clicked() {
            op = Some(("event.quantize_50", json!({})));
        }
        if ui.button("Quantize Ends").on_hover_text("Move only note ends to the grid").clicked() {
            op = Some(("event.quantize_ends", json!({})));
        }
        if ui.button("Humanize").on_hover_text("Nudge timing and velocity a little at random").clicked() {
            op = Some(("event.humanize", json!({})));
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Restore Timing").on_hover_text("Put the notes back where they were before the first quantize").clicked() {
            op = Some(("event.restore_performance", json!({})));
        }
        if ui.button("Freeze Quantize").on_hover_text("Keep the quantized notes as the timing Restore goes back to").clicked() {
            op = Some(("event.flatten_performance", json!({})));
        }
        if ui.button("Extract Groove").on_hover_text("This clip's notes become the groove other clips quantize to").clicked() {
            op = Some(("event.groove_extract", json!({})));
        }
        if has_groove && ui.button("Clear Groove").on_hover_text("Forget the groove and quantize to the grid").clicked() {
            let _ = app.run("event.groove_clear", json!({}));
        }
    });
    if let Some((id, mut p)) = op {
        p["clips"] = clips;
        if let Err(e) = app.run(id, p) {
            app.ui.status = e.to_string();
        }
    }
    ui.separator();
    let mut iq = app.engine.session().edit.flag("midi.input_quantize");
    if ui.checkbox(&mut iq, "Input Quantize").on_hover_text("Quantize notes as they are recorded").changed() {
        let _ = app.run("event.input_quantize", json!({"enabled": iq}));
    }
    if let Some(g) = app.engine.session().groove.as_ref() {
        let text = format!("Groove: {} ({} hits)", g.name, g.points.len());
        ui.label(egui::RichText::new(text).small().color(Tokens::current().text_dim));
    }
}
