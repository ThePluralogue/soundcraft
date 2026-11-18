use super::*;
use soundcraft_model::{Clip, TrackKind};

/// A session with one MIDI clip (at `start` samples) holding notes at `starts` ticks.
fn midi_engine(start: Samples, starts: &[i64]) -> (Engine, ClipId) {
    let mut s = Session::default();
    let t = s.add_track(TrackKind::Instrument, soundcraft_model::ChannelFormat::Stereo, Some("Keys"));
    let mut seq = soundcraft_midi::Sequence::default();
    for &at in starts {
        seq.notes.push(soundcraft_midi::Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start: at, length: 100 });
    }
    let cid = s.new_clip_id();
    s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::midi(cid, "Riff", start, 96_000 * 2, seq));
    s.edit.selected_tracks = vec![t];
    let mut e = Engine::new(s);
    e.execute("edit.select", &json!({"clips": [cid.0]})).unwrap();
    (e, cid)
}

fn starts(e: &Engine, cid: ClipId) -> Vec<i64> {
    match &e.session().find_clip(cid).unwrap().1.content {
        ClipContent::Midi { sequence } => sequence.notes.iter().map(|n| n.start).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn quantize_uses_the_stored_settings_and_50_percent_goes_half_way() {
    let (mut e, cid) = midi_engine(0, &[300, 1000]);
    e.execute("event.quantize_settings", &json!({"grid": "1/8"})).unwrap();
    e.execute("event.quantize_50", &json!({})).unwrap();
    assert_eq!(starts(&e, cid), vec![390, 980]);
    e.execute("event.quantize", &json!({})).unwrap();
    assert_eq!(starts(&e, cid), vec![480, 960]);
    // Restore Performance puts back the timing from before the first quantize.
    e.execute("event.restore_performance", &json!({})).unwrap();
    assert_eq!(starts(&e, cid), vec![300, 1000]);
}

#[test]
fn triplets_range_and_the_edit_grid() {
    let (mut e, cid) = midi_engine(0, &[300, 130]);
    // 1/8 triplet = 320 ticks.
    e.execute("event.quantize", &json!({"grid": "1/8t"})).unwrap();
    assert_eq!(starts(&e, cid), vec![0, 320]);
    let (mut e, cid) = midi_engine(0, &[300, 130]);
    // Range 50% of a 1/8 (half step 240): 300 is 180 from 480 — too far; 130 is 130 from 0 — too far too.
    e.execute("event.quantize", &json!({"grid": "1/8", "range": 50})).unwrap();
    assert_eq!(starts(&e, cid), vec![130, 300]);
    let (mut e, cid) = midi_engine(0, &[300]);
    e.execute("view.grid", &json!({"value": "1/4"})).unwrap();
    e.execute("event.quantize", &json!({"grid": "edit"})).unwrap();
    assert_eq!(starts(&e, cid), vec![0]);
}

#[test]
fn a_clip_starting_off_the_bar_quantizes_to_the_song_grid() {
    // 120 bpm at 48 kHz: a quarter note is 24000 samples, so the clip starts 1/8 note in (480 ticks).
    let (mut e, cid) = midi_engine(12_000, &[200]);
    e.execute("event.quantize", &json!({"grid": "1/4"})).unwrap();
    // Song tick 680 is nearest 960: the note moves to clip tick 480.
    assert_eq!(starts(&e, cid), vec![480]);
}

#[test]
fn note_ends_quantize_on_their_own() {
    let (mut e, cid) = midi_engine(0, &[250]);
    e.execute("event.quantize_settings", &json!({"grid": "1/16"})).unwrap();
    e.execute("event.quantize_ends", &json!({})).unwrap();
    let c = e.session().find_clip(cid).unwrap().1.clone();
    let ClipContent::Midi { sequence } = c.content else { panic!("midi") };
    assert_eq!((sequence.notes[0].start, sequence.notes[0].end()), (250, 480));
}

#[test]
fn a_groove_from_one_clip_quantizes_another() {
    // The groove: a late off-beat in every beat.
    let (mut e, groove_clip) = midi_engine(0, &[0, 560, 960, 1520]);
    let r = e.execute("event.groove_extract", &json!({"clip": groove_clip.0})).unwrap();
    assert_eq!(r["bars"], 2, "the clip is two bars long");
    assert_eq!(e.execute("event.quantize_settings", &json!({})).unwrap()["mode"], "groove");
    e.execute("edit.undo", &json!({})).unwrap();
    assert!(e.session().groove.is_none(), "extracting is one undo step");
    e.execute("event.groove_extract", &json!({"clip": groove_clip.0})).unwrap();
    let (mut other, cid) = midi_engine(0, &[480, 1450]);
    other.session_mut().groove = e.session().groove.clone();
    other.execute("event.quantize", &json!({"mode": "groove"})).unwrap();
    assert_eq!(starts(&other, cid), vec![560, 1520]);
}

#[test]
fn presets_store_and_recall_settings() {
    let (mut e, _) = midi_engine(0, &[0]);
    e.execute("event.quantize_settings", &json!({"grid": "1/16", "swing": 40, "start": 80})).unwrap();
    e.execute("event.quantize_preset", &json!({"slot": 2, "store": true})).unwrap();
    e.execute("event.quantize_settings", &json!({"grid": "1/4", "swing": 0, "start": 100})).unwrap();
    let st = e.execute("event.quantize_preset", &json!({"slot": 2})).unwrap();
    assert_eq!((st["grid"].as_i64(), st["swing"].as_f64(), st["start"].as_f64()), (Some(240), Some(40.0), Some(80.0)));
    assert_eq!(st["presets"], json!([2]));
    assert!(e.execute("event.quantize_preset", &json!({"slot": 3})).is_err());
    assert!(e.execute("event.quantize_preset", &json!({"slot": 9})).is_err());
    assert!(e.execute("event.quantize_settings", &json!({"mode": "groove"})).is_err(), "no groove yet");
}

#[test]
fn humanize_moves_notes_a_little() {
    let (mut e, cid) = midi_engine(0, &[960, 1920, 2880, 3840]);
    e.execute("event.humanize", &json!({"timing": 10, "velocity": 0})).unwrap();
    let after = starts(&e, cid);
    assert_ne!(after, vec![960, 1920, 2880, 3840]);
    assert!(after.iter().zip([960, 1920, 2880, 3840]).all(|(a, b)| (a - b).abs() <= 10));
}

#[test]
fn an_audio_clip_quantizes_by_its_start() {
    let mut e = crate::demo::demo_engine();
    let s = e.session();
    let (t, c) = s.tracks.iter().find(|t| t.kind == TrackKind::Audio).and_then(|t| t.clips().first().map(|c| (t.id, c.clone()))).unwrap();
    // Nudge it 1000 samples late, then quantize to the quarter note.
    e.execute("edit.select", &json!({"clips": [c.id.0], "tracks": [t.0]})).unwrap();
    crate::edit::move_clips(e.session_mut(), &[c.id], 1000, &[]);
    let r = e.execute("event.quantize", &json!({"grid": "1/4"})).unwrap();
    assert_eq!(r["clips"], 1);
    let start = e.session().find_clip(c.id).unwrap().1.start;
    let tick = e.session().tempo.samples_to_ticks(start, e.session().sample_rate);
    assert_eq!(tick.rem_euclid(TICKS_PER_QUARTER), 0, "on a quarter note: {start}");
}
