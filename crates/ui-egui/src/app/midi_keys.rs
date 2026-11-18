//! Live keyboards (MIDI input devices): which ones are open, what they played during a record
//! pass, and the MIDI clips that recording becomes.
use super::{SoundApp, json};
use serde_json::Value;
use soundcraft_model::{Session, TrackId};
use soundcraft_playback::PlayedMidi;
use soundcraft_time::{Range, Samples, TICKS_PER_QUARTER};

/// Most played messages one record pass keeps.
const MAX_PLAYED: usize = 200_000;

/// The keyboards the session picked (Setup › MIDI › MIDI Input Devices), or `None` for every
/// keyboard: a session that never picked any uses them all.
pub(crate) fn picked_keyboards(s: &Session) -> Option<Vec<String>> {
    let picked: Vec<String> = s.edit.flags.iter().filter_map(|f| f.strip_prefix("midi.input.")).map(str::to_string).collect();
    (!picked.is_empty()).then_some(picked)
}

/// Does the session use keyboard `name`?
pub(crate) fn keyboard_enabled(s: &Session, name: &str) -> bool {
    picked_keyboards(s).is_none_or(|p| p.iter().any(|x| x == name.trim()))
}

/// The record-enabled tracks a keyboard records onto.
pub(crate) fn midi_armed(s: &Session) -> Vec<TrackId> {
    s.tracks.iter().filter(|t| t.mixer.record_arm && soundcraft_mix::hears_live_midi(s, t)).map(|t| t.id).collect()
}

/// Notes from what was played (note-ons paired with their note-offs; a key still down at `end`
/// ends there), as `event.retrospective_record` takes them.
pub(crate) fn played_notes(s: &Session, played: &[PlayedMidi], end: Samples) -> Vec<Value> {
    let sr = s.sample_rate;
    let ticks = |at: Samples| s.tempo.samples_to_ticks(at.max(0), sr);
    // A note shorter than the block it was played in still gets a 1/64.
    let min_len = TICKS_PER_QUARTER / 16;
    let mut down: [Option<(Samples, u8)>; 128] = [None; 128];
    let mut out = Vec::new();
    let mut note = |key: u8, (start, vel): (Samples, u8), stop: Samples| {
        let len = ticks(stop).saturating_sub(ticks(start)).max(min_len);
        out.push(json!({"pitch": key, "start_ticks": ticks(start), "length_ticks": len, "velocity": vel}));
    };
    for p in played {
        let (status, key, vel) = (p.msg[0] & 0xF0, p.msg[1] & 0x7F, p.msg[2] & 0x7F);
        let Some(slot) = down.get_mut(usize::from(key)) else { continue };
        match status {
            0x90 if vel > 0 => {
                if let Some(held) = slot.replace((p.at, vel)) {
                    note(key, held, p.at);
                }
            }
            0x80 | 0x90 => {
                if let Some(held) = slot.take() {
                    note(key, held, p.at);
                }
            }
            _ => {}
        }
    }
    for (key, held) in down.iter().enumerate() {
        if let (Some(held), Ok(key)) = (held, u8::try_from(key)) {
            note(key, *held, end);
        }
    }
    out
}

impl SoundApp {
    /// Opens the session's keyboards when the player is new or the pick changed. Every frame.
    pub(super) fn sync_midi_inputs(&mut self) {
        let Some(p) = &self.player else { return };
        let want = picked_keyboards(self.engine.session());
        if self.midi_key.as_ref() == Some(&want) {
            return;
        }
        let s = self.engine.session_arc();
        let opened = p.open_midi_inputs(&|name| keyboard_enabled(&s, name));
        if !opened.is_empty() {
            log::info!("keyboards: {}", opened.join(", "));
        }
        self.midi_key = Some(want);
    }

    /// Logs when the keyboards first send something and when the engine first takes it, so a
    /// keyboard that plays nothing shows where its notes stop. Every frame.
    pub(super) fn log_midi_activity(&mut self) {
        let Some(p) = &self.player else { return };
        let now = p.midi_activity();
        let last = self.midi_logged;
        // Counts went down: the keyboards reopened and counting started again.
        let last = if now.0 < last.0 || now.1 < last.1 { (0, 0) } else { last };
        if now.0 > 0 && last.0 == 0 {
            log::info!("keyboards: first message received");
        }
        if now.1 > 0 && last.1 == 0 {
            log::info!("keyboards: the audio engine is taking their messages");
        }
        if now.0 >= last.0.saturating_add(50) {
            log::info!("keyboards: {} messages received, {} taken by the audio engine", now.0, now.1);
            self.midi_logged = now;
        } else {
            self.midi_logged = (last.0.max(u64::from(now.0 > 0)), last.1.max(u64::from(now.1 > 0)));
        }
    }

    /// Keeps what the keyboards played during a record pass. Every frame.
    pub(super) fn collect_played_midi(&mut self) {
        let Some(p) = &self.player else { return };
        for ev in p.take_played_midi() {
            if ev.recording && self.played_midi.len() < MAX_PLAYED {
                self.played_midi.push(ev);
            }
        }
    }

    /// Turns a record pass's playing into MIDI clips on each record-enabled instrument track:
    /// one per loop pass when loop recording (`pass`, from the record start; each pass after the
    /// first on a new playlist, as audio takes are), else one, within the `punch` range when
    /// there is one. Returns how many clips it made.
    pub(super) fn place_played_midi(&mut self, pass: Option<Samples>, punch: Option<Range>) -> usize {
        self.collect_played_midi();
        let played = std::mem::take(&mut self.played_midi);
        let stop = self.engine.transport.position.max(self.record_start);
        let s = self.engine.session();
        if played.is_empty() || s.edit.flag("midi.filter.notes") {
            return 0;
        }
        let targets = midi_armed(s);
        let takes: Vec<(Vec<Value>, Option<Range>)> = match pass.filter(|p| *p > 0) {
            Some(p) => {
                let lp = Range::new(self.record_start, self.record_start.saturating_add(p));
                let passes = split_passes(&played);
                let last = passes.len().saturating_sub(1);
                passes
                    .into_iter()
                    .enumerate()
                    .map(|(i, evs)| {
                        // A key down when the loop wraps ends at the loop end; on the last pass, at Stop.
                        let end = if i == last && lp.contains(stop) { stop } else { lp.end };
                        (played_notes(s, evs, end), Some(lp))
                    })
                    .collect()
            }
            None => vec![(played_notes(s, &played, stop), punch.filter(|r| !r.is_empty()))],
        };
        let mut made = 0;
        for t in targets {
            let mut placed = false;
            for (notes, range) in &takes {
                if notes.is_empty() {
                    continue;
                }
                let mut p = json!({"track": t.0, "notes": notes, "new_playlist": placed});
                if let Some(r) = range {
                    p["range"] = json!({"start": r.start, "end": r.end});
                }
                match self.run("event.retrospective_record", p) {
                    Ok(v) if v["recorded"].as_u64().unwrap_or(0) > 0 => {
                        made += 1;
                        placed = true;
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("recording MIDI: {e}"),
                }
            }
        }
        made
    }
}

/// What was played, split into loop passes: a pass ends where the position jumps back.
pub(crate) fn split_passes(played: &[PlayedMidi]) -> Vec<&[PlayedMidi]> {
    let mut out = Vec::new();
    let mut from = 0;
    for (i, w) in played.windows(2).enumerate() {
        if w[1].at < w[0].at {
            out.push(played.get(from..=i).unwrap_or(&[]));
            from = i + 1;
        }
    }
    out.push(played.get(from..).unwrap_or(&[]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(at: Samples, msg: [u8; 3]) -> PlayedMidi {
        PlayedMidi { at, msg, recording: true }
    }

    #[test]
    fn played_keys_pair_into_notes_and_a_held_key_ends_at_stop() {
        let s = Session::default();
        // 120 bpm at 48 kHz: a quarter note is 24000 samples (960 ticks).
        let played =
            [ev(0, [0x90, 60, 100]), ev(24_000, [0x80, 60, 0]), ev(24_000, [0x90, 64, 90]), ev(30_000, [0x90, 64, 0]), ev(36_000, [0x91, 67, 80])];
        let notes = played_notes(&s, &played, 48_000);
        let n = |i: usize, k: &str| notes[i][k].as_i64().unwrap();
        assert_eq!(notes.len(), 3);
        assert_eq!((n(0, "pitch"), n(0, "start_ticks"), n(0, "length_ticks"), n(0, "velocity")), (60, 0, 960, 100));
        assert_eq!((n(1, "pitch"), n(1, "start_ticks"), n(1, "length_ticks")), (64, 960, 240));
        assert_eq!((n(2, "pitch"), n(2, "start_ticks"), n(2, "length_ticks")), (67, 1440, 480), "held to the stop");
    }

    #[test]
    fn a_loop_recording_splits_into_passes_where_the_position_jumps_back() {
        let played = [ev(100, [0x90, 60, 90]), ev(900, [0x80, 60, 0]), ev(100, [0x90, 62, 90]), ev(500, [0x90, 64, 90]), ev(200, [0x80, 64, 0])];
        let passes: Vec<usize> = split_passes(&played).iter().map(|p| p.len()).collect();
        assert_eq!(passes, vec![2, 2, 1]);
        assert_eq!(split_passes(&[]).len(), 1, "nothing played: one empty pass");
    }

    #[test]
    fn a_session_that_picked_no_keyboard_uses_them_all() {
        let mut s = Session::default();
        assert!(keyboard_enabled(&s, "CASIO USB-MIDI") && picked_keyboards(&s).is_none());
        s.edit.set_flag("midi.input.CASIO USB-MIDI", true);
        assert!(keyboard_enabled(&s, "CASIO USB-MIDI"));
        assert!(!keyboard_enabled(&s, "Other Keys"));
    }
}
