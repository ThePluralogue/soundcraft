//! Live keyboards: MIDI input devices (USB keyboards and controllers) opened with midir. Each
//! device's messages go to the audio thread through a bounded channel, which plays them on the
//! tracks that hear live MIDI (`soundcraft_mix::hears_live_midi`) and hands what was played while
//! the transport ran back to the UI ([`PlayedMidi`]), which records it into clips.

use soundcraft_mix::LiveMidi;
use soundcraft_time::Samples;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;

/// Messages waiting for the audio thread; a flood past this is dropped rather than block a device.
pub(crate) const LIVE_QUEUE: usize = 1024;
/// Played messages waiting for the UI to take them (a few seconds of dense playing).
pub(crate) const PLAYED_QUEUE: usize = 16_384;

/// A message played on a live keyboard while the transport ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayedMidi {
    /// Session position of the block it played in.
    pub at: Samples,
    pub msg: [u8; 3],
    /// Played during a record pass.
    pub recording: bool,
}

/// A channel voice message (note, pressure, controller, program, pitch bend) from raw bytes, or
/// `None` for anything else (system messages, sysex, stray data bytes). midir hands over whole
/// messages, so there is no running status to track.
pub fn parse(bytes: &[u8]) -> Option<LiveMidi> {
    let status = *bytes.first()?;
    if !(0x80..0xF0).contains(&status) {
        return None;
    }
    let data = |i: usize| bytes.get(i).copied().filter(|b| *b < 0x80);
    let msg = match status & 0xF0 {
        0xC0 | 0xD0 => [status, data(1)?, 0],
        _ => [status, data(1)?, data(2)?],
    };
    Some(LiveMidi { msg })
}

/// Open MIDI input connections. Dropping it closes them.
#[derive(Default)]
pub struct MidiInputs {
    #[cfg(not(target_arch = "wasm32"))]
    conns: Vec<midir::MidiInputConnection<()>>,
    /// The devices that opened.
    pub names: Vec<String>,
    /// Devices that would not open, with why.
    pub failed: Vec<String>,
    /// Messages the devices sent since they opened.
    pub received: Arc<AtomicU64>,
    /// Messages the audio engine took since they opened (it plays them on the tracks that hear
    /// live MIDI). Received but not taken means the engine is not listening.
    pub heard: Arc<AtomicU64>,
}

impl MidiInputs {
    /// `(received, heard)`: see the fields.
    pub fn activity(&self) -> (u64, u64) {
        (self.received.load(Ordering::Relaxed), self.heard.load(Ordering::Relaxed))
    }
}

const CLIENT: &str = "SoundCraft";

/// The MIDI input devices on this computer.
#[cfg(not(target_arch = "wasm32"))]
pub fn available() -> Vec<String> {
    let Ok(input) = midir::MidiInput::new(CLIENT) else { return Vec::new() };
    input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
}

#[cfg(target_arch = "wasm32")]
pub fn available() -> Vec<String> {
    Vec::new()
}

/// Opens every input device `enabled` accepts, each sending its messages to `tx`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open(enabled: &dyn Fn(&str) -> bool, tx: &SyncSender<LiveMidi>) -> MidiInputs {
    let mut out = MidiInputs::default();
    let Ok(probe) = midir::MidiInput::new(CLIENT) else { return out };
    let ports: Vec<(midir::MidiInputPort, String)> = probe.ports().into_iter().filter_map(|p| probe.port_name(&p).ok().map(|n| (p, n))).collect();
    for (port, name) in ports {
        if !enabled(&name) {
            continue;
        }
        // One client per connection: connecting consumes it.
        let Ok(mut input) = midir::MidiInput::new(CLIENT) else { continue };
        input.ignore(midir::Ignore::All);
        let tx = tx.clone();
        let received = Arc::clone(&out.received);
        let callback = move |_stamp: u64, bytes: &[u8], _: &mut ()| {
            if let Some(m) = parse(bytes) {
                received.fetch_add(1, Ordering::Relaxed);
                // Full: the audio thread is behind; drop rather than wait on the device thread.
                let _ = tx.try_send(m);
            }
        };
        match input.connect(&port, "SoundCraft In", callback, ()) {
            Ok(c) => {
                log::info!("MIDI input open: {name}");
                out.conns.push(c);
                out.names.push(name);
            }
            Err(e) => {
                log::warn!("MIDI input {name}: {e}");
                out.failed.push(format!("{name}: {e}"));
            }
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn open(_enabled: &dyn Fn(&str) -> bool, _tx: &SyncSender<LiveMidi>) -> MidiInputs {
    MidiInputs::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_messages_parse_and_everything_else_is_ignored() {
        assert_eq!(parse(&[0x90, 60, 100]), Some(LiveMidi { msg: [0x90, 60, 100] }));
        assert_eq!(parse(&[0x83, 60, 0]), Some(LiveMidi { msg: [0x83, 60, 0] }));
        assert_eq!(parse(&[0xC0, 5]), Some(LiveMidi { msg: [0xC0, 5, 0] }));
        assert_eq!(parse(&[0xB0, 64, 127]), Some(LiveMidi { msg: [0xB0, 64, 127] }));
        // System, sysex, truncated and out-of-range data bytes.
        for bad in [&[0xF8][..], &[0xF0, 1, 2, 0xF7], &[0x90, 60], &[0x90, 200, 1], &[], &[60, 100]] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }
}
