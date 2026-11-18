//! The docked console's Cues menu: a new cue mix plays on Out 3-4 and the faders flip to it.
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use soundcraft_model::Route;
use soundcraft_ui_egui::{MainWindow, Services, SoundApp};

fn step(h: &mut Harness<'_, SoundApp>) {
    let mut raw = std::mem::take(h.input_mut());
    let ctx = h.ctx.clone();
    h.state_mut().raw_input_hook(&ctx, &mut raw);
    *h.input_mut() = raw;
    h.step();
}

fn click(h: &mut Harness<'_, SoundApp>, pos: Pos2) {
    let button = PointerButton::Primary;
    let m = Modifiers::NONE;
    h.state_mut().synthetic.extend([
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button, pressed: true, modifiers: m },
        Event::PointerButton { pos, button, pressed: false, modifiers: m },
    ]);
    for _ in 0..8 {
        step(h);
    }
}

#[test]
fn new_cue_mix_from_the_console_bar_flips_the_faders_to_it() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.ui.window = MainWindow::Edit;
    app.ui.show_mix_dock = true;
    let mut h = Harness::builder().with_size(vec2(1800.0, 1100.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        step(&mut h);
    }
    let cues = h.get_by_label("Cues v").rect().center();
    click(&mut h, cues);
    let new = h.get_by_label("New Cue Mix").rect().center();
    click(&mut h, new);
    let s = h.state().engine.session();
    let bus = s.bus_by_name("Cue 1").expect("a cue bus");
    assert!(bus.cue.is_some());
    let bid = bus.id;
    let aux = s.track_by_name("Cue 1").expect("its aux channel");
    assert_eq!(aux.mixer.output, Route::Hardware("Out 3-4".into()));
    assert_eq!(h.state().ui.fader_flip, Some(bid.0));
    for _ in 0..4 {
        step(&mut h);
    }
    assert!(h.query_by_label("Kick send to Cue 1").is_some(), "Kick's fader now sets its cue send");
    assert!(h.query_by_label("Cues (1) v").is_some());
    h.state_mut().run("mix.cue_follow", json!({"cue": "Cue 1"})).unwrap();
    assert!(h.state().engine.session().bus(bid).and_then(|b| b.cue).is_some_and(|c| c.follow_main));
}
