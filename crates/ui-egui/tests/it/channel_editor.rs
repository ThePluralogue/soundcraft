//! The Channel Editor and the Expand panel's inline insert controls.
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use soundcraft_dsp::Unit;
use soundcraft_ui_egui::{MainWindow, Services, SoundApp};

fn step(h: &mut Harness<'_, SoundApp>) {
    let mut raw = std::mem::take(h.input_mut());
    let ctx = h.ctx.clone();
    h.state_mut().raw_input_hook(&ctx, &mut raw);
    *h.input_mut() = raw;
    h.step();
}

fn steps(h: &mut Harness<'_, SoundApp>, n: usize) {
    for _ in 0..n {
        step(h);
    }
}

fn drag(h: &mut Harness<'_, SoundApp>, from: Pos2, dx: f32) {
    let button = PointerButton::Primary;
    let m = Modifiers::NONE;
    h.state_mut().synthetic.extend([Event::PointerMoved(from), Event::PointerButton { pos: from, button, pressed: true, modifiers: m }]);
    step(h);
    for k in 1..=4 {
        h.state_mut().synthetic.push(Event::PointerMoved(from + vec2(dx * k as f32 / 4.0, 0.0)));
        step(h);
    }
    h.state_mut().synthetic.push(Event::PointerButton { pos: from + vec2(dx, 0.0), button, pressed: false, modifiers: m });
    steps(h, 6);
}

fn click(h: &mut Harness<'_, SoundApp>, pos: Pos2) {
    let button = PointerButton::Primary;
    let m = Modifiers::NONE;
    h.state_mut().synthetic.extend([
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button, pressed: true, modifiers: m },
        Event::PointerButton { pos, button, pressed: false, modifiers: m },
    ]);
    steps(h, 8);
}

fn harness(app: SoundApp) -> Harness<'static, SoundApp> {
    let mut h = Harness::builder().with_size(vec2(1800.0, 1100.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    steps(&mut h, 6);
    h
}

/// Kick's first insert: its slot, plugin short name, and first continuous parameter.
fn kick_insert(app: &SoundApp) -> (usize, &'static str, &'static soundcraft_dsp::ParamInfo) {
    let kick = app.engine.session().track_by_name("Kick").unwrap();
    let (slot, ins) = kick.mixer.inserts.iter().enumerate().find_map(|(i, s)| s.as_ref().map(|s| (i, s))).unwrap();
    let info = soundcraft_dsp::plugins().iter().find(|p| p.id == ins.plugin).unwrap();
    let p = info.params.iter().find(|p| !matches!(p.unit, Unit::Toggle | Unit::Choice) && p.default < p.max).unwrap();
    (slot, info.short_name, p)
}

fn kick_param(h: &Harness<'_, SoundApp>, slot: usize, id: &str) -> Option<f32> {
    let kick = h.state().engine.session().track_by_name("Kick").cloned().unwrap();
    kick.mixer.inserts.get(slot).cloned().flatten().and_then(|i| i.params.get(id).copied())
}

/// `window.channel_editor` opens a window whose controls drag a parameter, and one undo puts it back.
#[test]
fn the_channel_editor_sets_an_insert_parameter_in_one_undo_step() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    assert!(app.run("window.channel_editor", json!({"tracks": ["Nobody"]})).is_err());
    app.run("window.channel_editor", json!({"tracks": ["Kick"]})).unwrap();
    let (slot, short, p) = kick_insert(&app);
    let before = kick_param_of(&app, slot, p.id);
    let mut h = harness(app);
    assert!(h.query_by_label_contains("Channel Editor").is_some(), "the window opened");
    let bar = h.get_by_label(&format!("{short} {} on Kick", p.name)).rect();
    drag(&mut h, bar.center(), bar.width() * 0.25);
    let after = kick_param(&h, slot, p.id).unwrap_or(p.default);
    assert!(after > before.unwrap_or(p.default), "dragging right raised {}: {before:?} -> {after}", p.name);
    h.state_mut().engine.execute("edit.undo", &json!({})).unwrap();
    steps(&mut h, 2);
    assert_eq!(kick_param(&h, slot, p.id), before, "one undo step");
}

fn kick_param_of(app: &SoundApp, slot: usize, id: &str) -> Option<f32> {
    let kick = app.engine.session().track_by_name("Kick").unwrap();
    kick.mixer.inserts.get(slot).cloned().flatten().and_then(|i| i.params.get(id).copied())
}

/// An expanded strip's insert arrow shows that insert's controls under its slot.
#[test]
fn an_expanded_strip_shows_an_inserts_controls_inline() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.ui.window = MainWindow::Mix;
    let kick = app.engine.session().track_by_name("Kick").unwrap().id;
    app.ui.expanded_strips.push(kick);
    app.run("mix.new_bus", json!({"name": "Verb", "format": "stereo"})).unwrap();
    app.run("mix.send", json!({"track": kick.0, "bus": "Verb"})).unwrap();
    let (slot, short, p) = kick_insert(&app);
    let mut h = harness(app);
    let label = format!("{short} {} on Kick", p.name);
    assert!(h.query_by_label(&label).is_none(), "controls start hidden");
    let letter = char::from(b'A' + slot as u8);
    let arrow = h.get_by_label(&format!("Controls for insert {letter} on Kick")).rect().center();
    click(&mut h, arrow);
    assert!(h.state().ui.inline_inserts.contains(&(kick, slot)));
    assert!(h.query_by_label(&label).is_some(), "the insert's controls show under it");
    assert!(h.query_by_label("Send A level value for Kick").is_some(), "the send level box is named");
}
