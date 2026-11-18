//! The MIDI editor's Quantize panel: settings, Quantize 50% and Restore Timing.
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;
use soundcraft_ui_egui::{Services, SoundApp};

fn first_start(h: &mut Harness<'_, SoundApp>, clip: &serde_json::Value) -> i64 {
    let notes = h.state_mut().run("midi.notes", json!({"clip": clip})).unwrap();
    notes["notes"][0]["start"].as_i64().unwrap()
}

#[test]
fn the_quantize_panel_quantizes_half_way_and_restores_the_timing() {
    let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
    app.run("track.new", json!({"name": "Keys", "kind": "midi"})).unwrap();
    let clip = app.run("midi.new_clip", json!({"track": "Keys", "start": 0, "end": 96_000})).unwrap()["clip"].clone();
    app.run("midi.note_add", json!({"clip": clip, "pitch": 60, "start_ticks": 300, "length_ticks": 240})).unwrap();
    app.run("edit.select", json!({"clips": [clip]})).unwrap();
    app.run("event.quantize_settings", json!({"grid": "1/8"})).unwrap();
    app.ui.show_midi_editor = true;
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(5);
    h.get_by_label("Q 1/8 v").click();
    h.run_steps(3);
    assert!(h.query_all_by_label("Swing").next().is_some(), "the panel has a Swing slider");
    h.get_by_label("Quantize 50%").click();
    h.run_steps(3);
    assert_eq!(first_start(&mut h, &clip), 390);
    h.get_by_label("Q 1/8 v").click();
    h.run_steps(3);
    h.get_by_label("Restore Timing").click();
    h.run_steps(3);
    assert_eq!(first_start(&mut h, &clip), 300);
}
