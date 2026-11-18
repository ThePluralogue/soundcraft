use super::plugin_menu::{MenuSearch, menu_row};
use super::{Route, SoundApp, Track, TrackId, Ui, json};

pub(crate) fn route_name(app: &SoundApp, r: &Route) -> String {
    match r {
        Route::None => "no output".into(),
        Route::Main => app.engine.session().outputs.first().map_or_else(|| "Out 1-2".into(), |o| o.name.clone()),
        Route::Bus(b) => app.engine.session().bus(*b).map_or_else(|| "bus?".into(), |b| b.name.clone()),
        Route::Hardware(h) => h.clone(),
    }
}

pub(crate) fn input_name(app: &SoundApp, track: &Track) -> String {
    if matches!(track.kind, soundcraft_model::TrackKind::Instrument | soundcraft_model::TrackKind::Midi) {
        // An instrument track plays from the keyboards (MIDI input devices).
        return match crate::app::picked_keyboards(app.engine.session()) {
            None => "All keyboards".into(),
            Some(p) if p.len() == 1 && p.first().is_some_and(|x| x == NO_KEYBOARD) => "No keyboard".into(),
            Some(p) if p.len() == 1 => p.first().cloned().unwrap_or_default(),
            Some(p) => format!("{} keyboards", p.len()),
        };
    }
    if track.kind.has_playlist() && track.mixer.input == Route::None {
        let channels = track.format.channels();
        if channels == 1 { "In 1".to_string() } else { format!("In 1-{channels}") }
    } else {
        route_name(app, &track.mixer.input)
    }
}

/// Hardware outputs a channel can play out of: the main output ("main"), the session's other
/// output paths, and the usual pairs after it (a cue mix goes to Out 3-4).
pub(super) fn output_choices(app: &SoundApp) -> Vec<String> {
    let mut v = vec!["main".to_string()];
    for o in app.engine.session().outputs.iter().skip(1) {
        v.push(o.name.clone());
    }
    for pair in ["Out 3-4", "Out 5-6", "Out 7-8"] {
        if !v.iter().any(|x| x.eq_ignore_ascii_case(pair)) {
            v.push(pair.to_string());
        }
    }
    v
}

/// The input or output menu under `resp`. It closes only on a click outside, so its bus search
/// field can be typed in.
pub(crate) fn route_popup(app: &mut SoundApp, resp: &egui::Response, id: TrackId, input: bool) {
    egui::Popup::menu(resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| route_menu(app, ui, id, input));
}

/// Input or output menu: no route and buses on both, plus the main output (outputs) or inputs 1-16
/// (inputs). Its search field takes typing at once and finds any of them, not just the buses.
fn route_menu(app: &mut SoundApp, ui: &mut Ui, id: TrackId, input: bool) {
    let plays_instrument =
        app.engine.session().track(id).is_some_and(|t| matches!(t.kind, soundcraft_model::TrackKind::Instrument | soundcraft_model::TrackKind::Midi));
    if input && plays_instrument {
        keyboard_menu(app, ui);
        return;
    }
    let key = if input { "input" } else { "output" };
    let cmd = if input { "track.input" } else { "track.output" };
    let hint = if input { "Search inputs…" } else { "Search outputs…" };
    let search = MenuSearch::show(ui, egui::Id::unique(("route-search", id.0, input)), hint);
    let busses: Vec<String> = app.engine.session().busses.iter().map(|b| b.name.clone()).collect();
    let main_name = app.engine.session().outputs.first().map_or_else(|| "Out 1-2".to_string(), |o| o.name.clone());
    // (label, the route the command is given)
    let mut fixed = vec![(if input { "no input" } else { "no output" }.to_string(), "none".to_string())];
    if input {
        fixed.extend((1..=16).map(|i| (format!("In {i}"), format!("In {i}"))));
    } else {
        fixed.push((main_name, "main".to_string()));
        fixed.extend(output_choices(app).into_iter().filter(|o| o != "main").map(|o| (o.clone(), o)));
    }
    if search.typing() {
        let visible: Vec<(&String, &String)> =
            fixed.iter().map(|(l, v)| (l, v)).chain(busses.iter().map(|b| (b, b))).filter(|(l, _)| search.matches(l)).collect();
        let picked = search.rows(ui, visible.len(), 420.0, |ui, i, highlighted| visible.get(i).is_some_and(|(l, _)| menu_row(ui, highlighted, l)));
        if let Some((_, value)) = picked.and_then(|i| visible.get(i)) {
            let _ = app.run(cmd, json!({"track": id.0, key: value}));
            ui.close();
        }
        return;
    }
    for (label, value) in &fixed {
        if ui.button(label).clicked() {
            let _ = app.run(cmd, json!({"track": id.0, key: value}));
            ui.close();
        }
    }
    ui.separator();
    ui.strong("Bus");
    if let Some(bus) = search
        .rows(ui, busses.len(), 420.0, |ui, i, highlighted| busses.get(i).is_some_and(|b| menu_row(ui, highlighted, b)))
        .and_then(|i| busses.get(i))
    {
        let _ = app.run(cmd, json!({"track": id.0, key: bus}));
        ui.close();
    }
    if ui.button("new bus...").clicked() {
        let mut n = busses.len() + 1;
        while busses.contains(&format!("Bus {n}")) || app.engine.session().track_by_name(&format!("Bus {n}")).is_some() {
            n += 1;
        }
        let _ = app.run(cmd, json!({"track": id.0, key: format!("Bus {n}")}));
        ui.close();
    }
}

/// The pick that means "no keyboard" (a pick of nothing would mean every keyboard).
const NO_KEYBOARD: &str = "(none)";

/// An instrument track's input menu: the keyboards (MIDI input devices) SoundCraft listens to.
/// The pick is the session's (Setup › MIDI › MIDI Input Devices); with none picked, all are used.
fn keyboard_menu(app: &mut SoundApp, ui: &mut Ui) {
    ui.set_min_width(240.0);
    ui.strong("Keyboards (MIDI input)");
    let available = soundcraft_playback::Player::available_midi_inputs();
    let (open, failed) = app.player.as_ref().map(|p| p.midi_inputs()).unwrap_or_default();
    let (received, heard) = app.player.as_ref().map(|p| p.midi_activity()).unwrap_or_default();
    if available.is_empty() {
        ui.label("No keyboard found. Plug one in, then reopen this menu.");
    }
    let picked = crate::app::picked_keyboards(app.engine.session());
    for name in &available {
        let mut on = crate::app::keyboard_enabled(app.engine.session(), name);
        let state = if open.contains(name) {
            "open"
        } else if failed.iter().any(|f| f.starts_with(name.as_str())) {
            "busy: another app has it"
        } else {
            ""
        };
        let label = if state.is_empty() { name.clone() } else { format!("{name}  ({state})") };
        if ui.checkbox(&mut on, label).changed() {
            let mut enable: Vec<String> = Vec::new();
            let mut disable: Vec<String> = vec![NO_KEYBOARD.into()];
            if picked.is_none() {
                // Leaving "every keyboard": pick all the others.
                enable.extend(available.iter().filter(|n| *n != name).cloned());
            }
            if on {
                enable.push(name.clone());
            } else {
                disable.push(name.clone());
            }
            let still = available.iter().any(|n| n != name && (picked.is_none() || crate::app::keyboard_enabled(app.engine.session(), n)));
            if !on && !still {
                enable.push(NO_KEYBOARD.into());
            }
            let _ = app.run("setup.midi_input_devices", json!({"enable": enable, "disable": disable}));
        }
    }
    if !open.is_empty() {
        let text = match (received, heard) {
            (0, _) => "Nothing played yet: press a key".to_string(),
            (r, 0) => format!("{r} messages in, none reached the instrument"),
            (r, h) => format!("{r} messages in, {h} played"),
        };
        ui.label(egui::RichText::new(text).small().color(crate::theme::Tokens::current().text_dim));
    }
    ui.separator();
    if ui.button("Use every keyboard").clicked() {
        let all: Vec<String> = app.engine.session().edit.flags.iter().filter_map(|f| f.strip_prefix("midi.input.")).map(str::to_string).collect();
        let _ = app.run("setup.midi_input_devices", json!({"disable": all}));
        ui.close();
    }
    ui.label(egui::RichText::new("Arm or monitor an instrument track to play it; with none armed, the selected one plays.").small().weak());
}

#[cfg(test)]
mod tests {
    use super::input_name;
    use crate::{Services, SoundApp};
    use serde_json::json;

    #[test]
    fn stereo_track_defaults_to_a_stereo_input_pair() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        app.run("track.new", json!({"format": "stereo"})).unwrap();
        let track = &app.engine.session().tracks[0];

        assert_eq!(input_name(&app, track), "In 1-2");
    }
}
