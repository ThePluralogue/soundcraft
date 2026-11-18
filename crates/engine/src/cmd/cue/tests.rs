use super::*;
use soundcraft_model::{ChannelFormat, Session};

fn engine() -> (Engine, TrackId, TrackId) {
    let mut s = Session::default();
    let vox = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Vox"));
    let gtr = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Gtr"));
    let t = s.track_mut(vox).unwrap();
    t.mixer.volume_db = -6.0;
    t.mixer.pan = vec![-0.5];
    (Engine::new(s), vox, gtr)
}

fn cue_send(e: &Engine, t: TrackId, bus: BusId) -> SendSlot {
    let tr = e.session().track(t).unwrap();
    tr.mixer.sends[send_slot(tr, bus).unwrap()].clone().unwrap()
}

#[test]
fn a_new_cue_mix_copies_the_main_mix_onto_pre_fader_sends_and_undoes_in_one_step() {
    let (mut e, vox, gtr) = engine();
    let r = e.execute("mix.cue_new", &json!({})).unwrap();
    assert_eq!(r["name"], "Cue 1");
    assert_eq!(r["sends"], 2);
    let s = e.session();
    let bus = s.bus_by_name("Cue 1").unwrap();
    assert_eq!(bus.cue, Some(CueMix { follow_main: false }));
    let bid = bus.id;
    let aux = cue_aux(s, bid).unwrap();
    assert_eq!(aux.mixer.output, Route::Hardware("Out 3-4".into()));
    assert!(send_slot(aux, bid).is_none(), "the cue's own aux must not feed itself");
    let v = cue_send(&e, vox, bid);
    assert_eq!((v.level_db, v.pan, v.pre_fader), (-6.0, -0.5, true));
    assert_eq!(cue_send(&e, gtr, bid).pan, 0.0);
    e.execute("edit.undo", &json!({})).unwrap();
    let s = e.session();
    assert!(s.busses.is_empty() && s.tracks.len() == 2, "one undo removes the cue mix");
}

#[test]
fn following_the_main_mix_zeroes_the_offsets_and_leaving_copies_the_main_mix_back() {
    let (mut e, vox, _) = engine();
    e.execute("mix.cue_new", &json!({"name": "Drummer", "output": "Out 3-4"})).unwrap();
    let bid = e.session().bus_by_name("Drummer").unwrap().id;
    e.execute("mix.send_level", &json!({"track": vox.0, "slot": 0, "db": -20.0})).unwrap();
    assert_eq!(e.execute("mix.cue_follow", &json!({"cue": "drummer"})).unwrap()["follow_main"], true);
    assert_eq!(cue_send(&e, vox, bid).level_db, 0.0);
    e.execute("mix.cue_follow", &json!({"cue": "Drummer", "value": false})).unwrap();
    assert_eq!(cue_send(&e, vox, bid).level_db, -6.0);
    e.execute("mix.send_level", &json!({"track": vox.0, "slot": 0, "db": -30.0, "pan": 1.0})).unwrap();
    e.execute("mix.cue_copy_main", &json!({"cue": "Drummer"})).unwrap();
    let v = cue_send(&e, vox, bid);
    assert_eq!((v.level_db, v.pan), (-6.0, -0.5));
}

#[test]
fn new_channels_join_and_removing_a_cue_mix_clears_its_sends_and_aux() {
    let (mut e, vox, _) = engine();
    e.execute("mix.cue_new", &json!({})).unwrap();
    let bid = e.session().bus_by_name("Cue 1").unwrap().id;
    let keys = e.session_mut().add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Keys"));
    assert_eq!(e.execute("mix.cue_add_tracks", &json!({"cue": "Cue 1"})).unwrap()["sends"], 1);
    assert!(send_slot(e.session().track(keys).unwrap(), bid).is_some());
    let list = e.execute("mix.cue_list", &json!({})).unwrap();
    assert_eq!(list["cues"][0]["sends"], 3);
    assert_eq!(list["cues"][0]["output"], "Out 3-4");
    e.execute("mix.cue_remove", &json!({"cue": "Cue 1"})).unwrap();
    let s = e.session();
    assert!(s.busses.is_empty());
    assert!(s.track_by_name("Cue 1").is_none());
    assert!(s.track(vox).unwrap().mixer.sends.iter().all(Option::is_none));
}

#[test]
fn a_cue_name_already_in_use_is_refused() {
    let (mut e, _, _) = engine();
    assert!(e.execute("mix.cue_new", &json!({"name": "Vox"})).is_err());
    assert!(e.execute("mix.cue_follow", &json!({"cue": "Nope"})).is_err());
}

#[test]
fn routing_a_channel_to_out_3_4_goes_to_that_hardware_output() {
    let (mut e, vox, _) = engine();
    e.execute("track.output", &json!({"tracks": [vox.0], "output": "Out 3-4"})).unwrap();
    let s = e.session();
    assert_eq!(s.track(vox).unwrap().mixer.output, Route::Hardware("Out 3-4".into()));
    assert!(s.busses.is_empty(), "no bus named after the output");
}
