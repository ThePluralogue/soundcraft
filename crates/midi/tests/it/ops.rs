use soundcraft_midi::ops::*;
use soundcraft_midi::*;

fn n(pitch: u8, start: i64, length: i64) -> Note {
    Note { pitch, velocity: 100, release_velocity: 64, channel: 0, start, length }
}

fn starts(notes: &[Note]) -> Vec<i64> {
    notes.iter().map(|n| n.start).collect()
}

#[test]
fn quantize_full_strength() {
    let mut notes = vec![n(60, 10, 200), n(60, 230, 100), n(60, 365, 50), n(60, -20, 40)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 240, ..Default::default() });
    assert_eq!(starts(&notes), vec![0, 240, 480, 0]);
    // Durations preserved by default.
    assert_eq!(notes.iter().map(|n| n.length).collect::<Vec<_>>(), vec![200, 100, 50, 40]);
}

#[test]
fn quantize_half_strength() {
    let mut notes = vec![n(60, 20, 100), n(60, 220, 100)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 240, strength: 0.5, ..Default::default() });
    assert_eq!(starts(&notes), vec![10, 230]);
}

#[test]
fn quantize_swing() {
    // 100% swing on a 1/8 (480) grid moves odd lines by 480/3 = 160 (triplet feel).
    let mut notes = vec![n(60, 0, 10), n(60, 470, 10), n(60, 650, 10), n(60, 950, 10)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 480, swing: 1.0, ..Default::default() });
    assert_eq!(starts(&notes), vec![0, 640, 640, 960]);
    let mut notes = vec![n(60, 480, 10)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 480, swing: 0.5, ..Default::default() });
    assert_eq!(starts(&notes), vec![560]);
}

#[test]
fn quantize_include_exclude() {
    let opts = QuantizeOptions { grid_ticks: 240, include_within: 0.5, ..Default::default() };
    // Half grid = 120; include within 60 ticks.
    let mut notes = vec![n(60, 50, 10), n(60, 100, 10)];
    quantize(&mut notes, &opts);
    assert_eq!(starts(&notes), vec![0, 100]);

    let opts = QuantizeOptions { grid_ticks: 240, exclude_within: 0.25, ..Default::default() };
    // Exclude closer than 30 ticks.
    let mut notes = vec![n(60, 20, 10), n(60, 50, 10)];
    quantize(&mut notes, &opts);
    assert_eq!(starts(&notes), vec![20, 0]);
}

#[test]
fn quantize_ends_and_duration() {
    let mut notes = vec![n(60, 10, 220)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 240, quantize_ends: true, ..Default::default() });
    assert_eq!((notes[0].start, notes[0].length), (0, 240));

    let mut notes = vec![n(60, 10, 220)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 240, preserve_duration: false, ..Default::default() });
    assert_eq!((notes[0].start, notes[0].length), (0, 230));

    // Ends quantized onto the start line still keep a length of at least 1.
    let mut notes = vec![n(60, 10, 5)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 240, quantize_ends: true, ..Default::default() });
    assert_eq!(notes[0].length, 1);
}

#[test]
fn quantize_random_is_deterministic_and_bounded() {
    let opts = QuantizeOptions { grid_ticks: 240, random: 0.5, ..Default::default() };
    let mut a: Vec<Note> = (0..50).map(|i| n(60, i * 240 + 7, 10)).collect();
    let mut b = a.clone();
    quantize(&mut a, &opts);
    quantize(&mut b, &opts);
    assert_eq!(a, b);
    for (i, note) in a.iter().enumerate() {
        assert!((note.start - i as i64 * 240).abs() <= 60);
    }
    let mut c: Vec<Note> = (0..50).map(|i| n(60, i * 240 + 7, 10)).collect();
    quantize_with_seed(&mut c, &opts, 42);
    assert_ne!(a, c);
}

#[test]
fn quantize_degenerate_options() {
    let mut notes = vec![n(60, 17, 10)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 0, ..Default::default() });
    quantize(&mut notes, &QuantizeOptions { grid_ticks: -5, ..Default::default() });
    assert_eq!(notes[0].start, 17);
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 240, strength: f32::NAN, swing: f32::INFINITY, ..Default::default() });
    assert_eq!(notes[0].start, 17);
    let mut extreme = vec![n(60, i64::MAX - 10, 1), n(60, i64::MIN + 10, 1)];
    quantize(&mut extreme, &QuantizeOptions { grid_ticks: i64::MAX, random: 1.0, ..Default::default() });
}

#[test]
fn transpose_clamps() {
    let mut notes = vec![n(0, 0, 1), n(60, 0, 1), n(127, 0, 1)];
    transpose(&mut notes, 12);
    assert_eq!(notes.iter().map(|n| n.pitch).collect::<Vec<_>>(), vec![12, 72, 127]);
    transpose(&mut notes, -100);
    assert_eq!(notes.iter().map(|n| n.pitch).collect::<Vec<_>>(), vec![0, 0, 27]);
    transpose(&mut notes, i32::MAX);
    assert!(notes.iter().all(|n| n.pitch == 127));
}

#[test]
fn ramp_full_tick_range() {
    let mut notes = vec![n(60, i64::MIN, 1), n(60, 0, 1), n(60, i64::MAX, 1)];
    change_velocity(&mut notes, VelocityOp::Ramp { from: 1, to: 127 });
    assert_eq!(notes.iter().map(|n| n.velocity).collect::<Vec<_>>(), vec![1, 64, 127]);
}

#[test]
fn velocity_ops() {
    let base = vec![n(60, 0, 1), n(60, 100, 1), n(60, 200, 1)];
    let vels = |v: &[Note]| v.iter().map(|n| n.velocity).collect::<Vec<_>>();

    let mut v = base.clone();
    change_velocity(&mut v, VelocityOp::Set(0));
    assert_eq!(vels(&v), vec![1, 1, 1]);
    change_velocity(&mut v, VelocityOp::Set(200));
    assert_eq!(vels(&v), vec![127, 127, 127]);

    let mut v = base.clone();
    change_velocity(&mut v, VelocityOp::Add(50));
    assert_eq!(vels(&v), vec![127; 3]);
    change_velocity(&mut v, VelocityOp::Add(-500));
    assert_eq!(vels(&v), vec![1; 3]);

    let mut v = base.clone();
    change_velocity(&mut v, VelocityOp::Scale(0.5));
    assert_eq!(vels(&v), vec![50; 3]);
    change_velocity(&mut v, VelocityOp::Scale(f32::NAN));
    assert_eq!(vels(&v), vec![50; 3]);

    let mut v = vec![n(60, 0, 1), n(60, 0, 1)];
    v[0].velocity = 10;
    v[1].velocity = 120;
    change_velocity(&mut v, VelocityOp::Limit { min: 100, max: 30 });
    assert_eq!(vels(&v), vec![30, 100]);

    let mut v = base.clone();
    change_velocity(&mut v, VelocityOp::Ramp { from: 20, to: 120 });
    assert_eq!(vels(&v), vec![20, 70, 120]);

    let mut a = base.clone();
    let mut b = base.clone();
    change_velocity(&mut a, VelocityOp::Humanize(10));
    change_velocity(&mut b, VelocityOp::Humanize(10));
    assert_eq!(a, b);
    assert!(a.iter().all(|n| (90..=110).contains(&n.velocity)));
}

#[test]
fn duration_ops() {
    let lens = |v: &[Note]| v.iter().map(|n| n.length).collect::<Vec<_>>();
    let base = vec![n(60, 0, 100), n(62, 480, 100), n(64, 480, 50), n(60, 960, 100)];

    let mut v = base.clone();
    change_duration(&mut v, DurationOp::Set(0));
    assert_eq!(lens(&v), vec![1; 4]);

    let mut v = base.clone();
    change_duration(&mut v, DurationOp::Add(-75));
    assert_eq!(lens(&v), vec![25, 25, 1, 25]);

    let mut v = base.clone();
    change_duration(&mut v, DurationOp::Scale(2.0));
    assert_eq!(lens(&v), vec![200, 200, 100, 200]);

    let mut v = base.clone();
    change_duration(&mut v, DurationOp::Legato { gap: 10 });
    assert_eq!(lens(&v), vec![470, 470, 470, 100]);

    let mut v = vec![n(60, 0, 500), n(60, 200, 500), n(61, 100, 500), Note { channel: 1, ..n(60, 100, 500) }];
    change_duration(&mut v, DurationOp::RemoveOverlap);
    assert_eq!(lens(&v), vec![200, 500, 500, 500]);
}

#[test]
fn split_by_pitch() {
    let notes = vec![n(36, 0, 1), n(60, 1, 1), n(72, 2, 1), n(48, 3, 1)];
    let (inside, outside) = split_notes(&notes, 60, 40);
    assert_eq!(inside.iter().map(|n| n.pitch).collect::<Vec<_>>(), vec![60, 48]);
    assert_eq!(outside.iter().map(|n| n.pitch).collect::<Vec<_>>(), vec![36, 72]);
}

#[test]
fn end_strength_moves_only_the_ends() {
    let mut notes = vec![n(60, 250, 200)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 240, strength: 0.0, end_strength: 1.0, ..Default::default() });
    assert_eq!((notes[0].start, notes[0].end()), (250, 480));
}

#[test]
fn an_offset_quantizes_to_the_song_grid_not_the_clip() {
    // The clip starts 100 ticks into the bar: tick 140 of the clip is tick 240 of the song.
    let mut notes = vec![n(60, 130, 100)];
    quantize(&mut notes, &QuantizeOptions { grid_ticks: 240, offset: 100, ..Default::default() });
    assert_eq!(notes[0].start, 140);
}

#[test]
fn groove_quantize_pulls_to_the_groove_and_its_velocity() {
    // A one-beat groove with a late off-beat: 0 and 560 (a swung eighth), loud then soft.
    let groove = [(0, 120), (560, 40)];
    let mut notes = vec![n(60, 500, 100), n(60, 970, 100)];
    let opts = QuantizeOptions { grid_ticks: 240, ..Default::default() };
    quantize_to_groove(&mut notes, &groove, 960, &opts, 1.0);
    assert_eq!((notes[0].start, notes[0].velocity), (560, 40));
    assert_eq!((notes[1].start, notes[1].velocity), (960, 120));
    let mut half = vec![n(60, 500, 100)];
    quantize_to_groove(&mut half, &groove, 960, &QuantizeOptions { strength: 0.5, ..opts }, 0.5);
    assert_eq!((half[0].start, half[0].velocity), (530, 70));
    let mut none = vec![n(60, 500, 100)];
    quantize_to_groove(&mut none, &[], 960, &opts, 1.0);
    assert_eq!(none[0].start, 500);
}

#[test]
fn humanize_stays_within_its_amounts() {
    let mut notes: Vec<_> = (0..64).map(|i| n(60, 1000 + i * 240, 100)).collect();
    let before = notes.clone();
    humanize(&mut notes, 20, 10, 7);
    assert!(notes.iter().zip(&before).any(|(a, b)| a.start != b.start), "something moved");
    for (a, b) in notes.iter().zip(&before) {
        assert!((a.start - b.start).abs() <= 20 && (i32::from(a.velocity) - i32::from(b.velocity)).abs() <= 10);
        assert_eq!(a.length, b.length);
    }
}
