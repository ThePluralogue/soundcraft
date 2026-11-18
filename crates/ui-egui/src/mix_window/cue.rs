//! The console bar's Cues menu: cue mixes (headphone or monitor mixes with their own output).
use super::{Route, SoundApp, Tokens, Track, TrackKind, Ui, json, route_name};
use soundcraft_model::BusId;

/// One cue mix as the menu shows it.
struct Cue {
    bus: BusId,
    name: String,
    follow_main: bool,
    aux: Option<soundcraft_model::TrackId>,
    output: String,
}

fn cues(app: &SoundApp) -> Vec<Cue> {
    let s = app.engine.session();
    s.busses
        .iter()
        .filter_map(|b| {
            let cue = b.cue?;
            let aux = s.tracks.iter().find(|t| t.kind == TrackKind::Aux && t.mixer.input == Route::Bus(b.id));
            Some(Cue {
                bus: b.id,
                name: b.name.clone(),
                follow_main: cue.follow_main,
                aux: aux.map(|t| t.id),
                output: aux.map_or_else(|| "no channel".into(), |t| route_name(app, &t.mixer.output)),
            })
        })
        .collect()
}

/// Is `track` the aux channel that plays a cue mix?
pub(super) fn is_cue_aux(app: &SoundApp, track: &Track) -> bool {
    let Route::Bus(b) = track.mixer.input else { return false };
    track.kind == TrackKind::Aux && app.engine.session().bus(b).is_some_and(|b| b.cue.is_some())
}

/// The label for the console bar's Cues button.
pub(crate) fn cue_label(app: &SoundApp) -> String {
    match app.engine.session().busses.iter().filter(|b| b.cue.is_some()).count() {
        0 => "Cues v".into(),
        n => format!("Cues ({n}) v"),
    }
}

/// Add a cue mix and flip the faders to it, so the next fader move sets the performer's mix.
pub(super) fn new_cue_and_flip(app: &mut SoundApp) {
    let Ok(r) = app.run("mix.cue_new", json!({})) else { return };
    let name = r.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string();
    if let Some(b) = app.engine.session().bus_by_name(&name) {
        app.ui.fader_flip = Some(b.id.0);
        app.ui.status = format!("{name} plays on Out 3-4 and starts as a copy of the main mix; the faders now set its sends");
    }
}

/// The Cues menu: each cue mix's settings, and New Cue Mix.
pub(crate) fn cue_menu(app: &mut SoundApp, ui: &mut Ui) {
    ui.set_min_width(240.0);
    let list = cues(app);
    if list.is_empty() {
        ui.label(egui::RichText::new("No cue mixes yet").small().color(Tokens::current().text_dim));
    }
    for cue in list {
        ui.menu_button(format!("{}  ·  {}", cue.name, cue.output), |ui| cue_submenu(app, ui, &cue));
    }
    ui.separator();
    if ui.button("New Cue Mix").on_hover_text("A headphone mix on Out 3-4: every channel gets a send to it, set like the main mix").clicked() {
        new_cue_and_flip(app);
        ui.close();
    }
}

fn cue_submenu(app: &mut SoundApp, ui: &mut Ui, cue: &Cue) {
    ui.set_min_width(220.0);
    let name = cue.name.as_str();
    let flipped = app.ui.fader_flip == Some(cue.bus.0);
    if ui.selectable_label(flipped, format!("Flip Faders to {name}")).on_hover_text("The faders set each channel's send to this cue").clicked() {
        app.ui.fader_flip = if flipped { None } else { Some(cue.bus.0) };
        ui.close();
    }
    let mut follow = cue.follow_main;
    let tip = "The cue hears each channel after its fader and with its pan; its sends become offsets from the main mix";
    if ui.checkbox(&mut follow, "Follow Main Mix").on_hover_text(tip).changed() {
        let _ = app.run("mix.cue_follow", json!({"cue": name, "value": follow}));
    }
    if ui.button("Copy Main Mix").on_hover_text("Set every send to this cue to its channel's fader and pan").clicked() {
        let _ = app.run("mix.cue_copy_main", json!({"cue": name}));
        ui.close();
    }
    if ui.button("Add Missing Channels").on_hover_text("Channels added since the cue was made get a send to it").clicked() {
        if let Ok(r) = app.run("mix.cue_add_tracks", json!({"cue": name})) {
            let n = r.get("sends").and_then(|n| n.as_u64()).unwrap_or(0);
            app.ui.status = format!("{n} channel{} added to {name}", if n == 1 { "" } else { "s" });
        }
        ui.close();
    }
    if let Some(aux) = cue.aux {
        ui.menu_button(format!("Output: {}", cue.output), |ui| {
            for out in super::routing::output_choices(app) {
                let label = if out == "main" { route_name(app, &Route::Main) } else { out.clone() };
                if ui.selectable_label(cue.output == label, &label).clicked() {
                    let _ = app.run("track.output", json!({"track": aux.0, "output": out, "create_bus": false}));
                    ui.close();
                }
            }
        });
    }
    ui.separator();
    if ui.button(format!("Remove {name}")).on_hover_text("Removes the cue mix, its sends and its channel").clicked() {
        if app.run("mix.cue_remove", json!({"cue": name})).is_ok() && flipped {
            app.ui.fader_flip = None;
        }
        ui.close();
    }
}
