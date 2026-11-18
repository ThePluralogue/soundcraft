//! Quantize, the Studio One way: a stored set of quantize settings (grid or groove, note value
//! and grouping, swing, and Start / End / Range / Velocity amounts) that Quantize, Quantize 50%
//! and Quantize Note Ends use, five presets, Humanize, and a groove taken from a clip. MIDI
//! notes and audio clips both quantize; the first quantize of a MIDI clip keeps its original
//! notes so Restore Performance puts the timing back.
use super::{CommandSpec, always, bad, bool_or, clip_ids_param, f64_or, has_selection, i64_or, str_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_midi::ops::{QuantizeOptions, humanize, quantize, quantize_to_groove};
use soundcraft_model::{ClipContent, ClipId, Groove, Session};
use soundcraft_time::{GridValue, Samples, TICKS_PER_QUARTER};

/// Stored settings: (key, default). Values are kept in the session's edit values.
const GRID: (&str, f64) = ("quantize.grid", 0.0);
const GROUPING: (&str, f64) = ("quantize.grouping", 0.0);
const SWING: (&str, f64) = ("quantize.swing", 0.0);
const START: (&str, f64) = ("quantize.start", 100.0);
const END: (&str, f64) = ("quantize.end", 0.0);
const RANGE: (&str, f64) = ("quantize.range", 100.0);
const VELOCITY: (&str, f64) = ("quantize.velocity", 0.0);
const MODE: (&str, f64) = ("quantize.mode", 0.0);
const SETTINGS: [(&str, f64); 8] = [GRID, GROUPING, SWING, START, END, RANGE, VELOCITY, MODE];
/// Number of quantize presets.
pub const PRESETS: usize = 5;
const GROUPINGS: [&str; 5] = ["straight", "triplet", "quintuplet", "septuplet", "dotted"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "event.quantize",
            "Quantize...",
            ["Event", "MIDI Operations"],
            Some("Alt+0"),
            "{clips?, mode?: grid|groove, grid?: '1/16'|'1/8t'|'edit'|ticks, grouping?: straight|triplet|quintuplet|septuplet|dotted, swing?: 0..100, start?|strength?: 0..100, end?: 0..100, range?: 0..100, velocity?: 0..100} — omitted values come from the quantize settings",
            has_selection,
            |e, p| run_quantize(e, p, None)
        ),
        cmd!("event.quantize_50", "Quantize 50%", [], None, "{clips?} — quantize with Start at 50%", has_selection, |e, p| run_quantize(
            e,
            p,
            Some((50.0, None))
        )),
        cmd!("event.quantize_ends", "Quantize Note Ends", [], None, "{clips?} — moves only note ends to the grid", has_selection, |e, p| {
            run_quantize(e, p, Some((0.0, Some(100.0))))
        }),
        cmd!("event.humanize", "Humanize", [], None, "{clips?, timing?: ticks (20), velocity?: 0..127 (10), seed?}", has_selection, run_humanize),
        cmd!(
            noundo "event.quantize_settings",
            "Quantize Settings",
            [],
            None,
            "{mode?, grid?, grouping?, swing?, start?, end?, range?, velocity?} — sets (or reads) what Quantize uses",
            always,
            settings
        ),
        cmd!(noundo "event.quantize_preset", "Quantize Preset", [], None, "{slot: 1..5, store?: false} — recalls (or stores) a set of quantize settings", always, preset),
        cmd!(
            "event.groove_extract",
            "Extract Groove",
            [],
            None,
            "{clip?} — the selected clip's notes or hits become the groove; switches to groove mode",
            has_selection,
            groove_extract
        ),
        cmd!("event.groove_clear", "Clear Groove", [], None, "{}", always, |e, _| {
            e.session_mut().groove = None;
            set(e.session_mut(), MODE, 0.0);
            Ok(state(e.session()))
        }),
    ]
}

fn get(s: &Session, (key, default): (&str, f64)) -> f64 {
    let v = s.edit.value(key, default);
    if v.is_finite() { v } else { default }
}

fn set(s: &mut Session, (key, _): (&str, f64), v: f64) {
    s.edit.values.insert(key.to_string(), v);
}

/// The quantize settings, as `event.quantize_settings` reports them.
fn state(s: &Session) -> Value {
    let grid = get(s, GRID) as i64;
    let grouping = GROUPINGS.get(get(s, GROUPING) as usize).copied().unwrap_or("straight");
    json!({
        "mode": if get(s, MODE) >= 0.5 { "groove" } else { "grid" },
        "grid": if grid <= 0 { json!("edit") } else { json!(grid) },
        "grouping": grouping,
        "grid_ticks": grid_ticks(s, grid, get(s, GROUPING) as usize),
        "swing": get(s, SWING),
        "start": get(s, START),
        "end": get(s, END),
        "range": get(s, RANGE),
        "velocity": get(s, VELOCITY),
        "groove": s.groove.as_ref().map(|g| json!({"name": g.name, "length": g.length, "points": g.points.len()})),
        "presets": (1..=PRESETS).filter(|n| s.edit.values.contains_key(&format!("quantize.preset.{n}.quantize.start"))).collect::<Vec<_>>(),
    })
}

/// The base note value in ticks: "1/16", "1/8t", "1/4.", "edit" (0: the Edit window's grid) or ticks.
fn parse_grid(id: &str, v: &Value) -> Result<(i64, Option<usize>)> {
    if let Some(n) = v.as_i64() {
        return Ok((n.clamp(0, TICKS_PER_QUARTER * 16), None));
    }
    let raw = v.as_str().ok_or_else(|| bad(id, "`grid` must be a note value or ticks"))?.trim().to_ascii_lowercase();
    if raw == "edit" || raw == "grid" {
        return Ok((0, None));
    }
    let (body, grouping) = if let Some(b) = raw.strip_suffix('t') {
        (b, Some(1))
    } else if let Some(b) = raw.strip_suffix('.') {
        (b, Some(4))
    } else {
        (raw.as_str(), None)
    };
    let den: i64 = match body.trim() {
        "1/1" | "1" | "bar" => 1,
        "1/2" => 2,
        "1/4" => 4,
        "1/8" => 8,
        "1/16" => 16,
        "1/32" => 32,
        "1/64" => 64,
        other => return Err(bad(id, format!("unknown note value `{other}`"))),
    };
    Ok((TICKS_PER_QUARTER * 4 / den, grouping))
}

fn parse_grouping(id: &str, v: &Value) -> Result<usize> {
    if let Some(n) = v.as_u64() {
        return usize::try_from(n).ok().filter(|n| *n < GROUPINGS.len()).ok_or_else(|| bad(id, "`grouping` 0..4"));
    }
    let g = v.as_str().unwrap_or_default().trim().to_ascii_lowercase();
    GROUPINGS.iter().position(|x| *x == g).ok_or_else(|| bad(id, format!("unknown grouping `{g}`")))
}

/// The grid step in ticks: the note value (or the Edit window's grid) with its grouping.
fn grid_ticks(s: &Session, base: i64, grouping: usize) -> i64 {
    let base = if base > 0 {
        base
    } else {
        match s.edit.grid {
            GridValue::Note { value, dotted, triplet } => {
                let g = value.ticks();
                let g = if dotted { g * 3 / 2 } else { g };
                if triplet { g * 2 / 3 } else { g }
            }
            _ => TICKS_PER_QUARTER / 4,
        }
    };
    let g = match grouping {
        1 => base * 2 / 3,
        2 => base * 4 / 5,
        3 => base * 8 / 7,
        4 => base * 3 / 2,
        _ => base,
    };
    g.clamp(1, TICKS_PER_QUARTER * 16)
}

/// Applies the settings named in `p` to the session; returns whether any was given.
fn apply_settings(s: &mut Session, id: &str, p: &Value) -> Result<()> {
    if let Some(m) = str_param(p, "mode") {
        let groove = match m {
            "grid" => false,
            "groove" => true,
            other => return Err(bad(id, format!("unknown mode `{other}`"))),
        };
        if groove && s.groove.is_none() {
            return Err(bad(id, "no groove yet: extract one from a clip first"));
        }
        set(s, MODE, if groove { 1.0 } else { 0.0 });
    }
    if let Some(v) = p.get("grid") {
        let (ticks, grouping) = parse_grid(id, v)?;
        set(s, GRID, ticks as f64);
        if let Some(g) = grouping {
            set(s, GROUPING, g as f64);
        } else if p.get("grouping").is_none() {
            set(s, GROUPING, 0.0);
        }
    }
    if let Some(v) = p.get("grouping") {
        set(s, GROUPING, parse_grouping(id, v)? as f64);
    }
    for (key, setting) in [("swing", SWING), ("start", START), ("strength", START), ("end", END), ("range", RANGE), ("velocity", VELOCITY)] {
        if let Some(v) = p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()) {
            set(s, setting, v.clamp(0.0, 100.0));
        }
    }
    Ok(())
}

fn settings(e: &mut Engine, p: &Value) -> Result<Value> {
    apply_settings(e.session_mut(), "event.quantize_settings", p)?;
    Ok(state(e.session()))
}

fn preset(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "event.quantize_preset";
    let n = i64_or(p, "slot", 0);
    if !(1..=PRESETS as i64).contains(&n) {
        return Err(bad(id, "`slot` 1..5"));
    }
    let s = e.session_mut();
    if bool_or(p, "store", false) {
        for setting in SETTINGS {
            let v = get(s, setting);
            s.edit.values.insert(format!("quantize.preset.{n}.{}", setting.0), v);
        }
    } else {
        let mut found = false;
        for setting in SETTINGS {
            if let Some(v) = s.edit.values.get(&format!("quantize.preset.{n}.{}", setting.0)).copied() {
                set(s, setting, v);
                found = true;
            }
        }
        if !found {
            return Err(bad(id, format!("preset {n} is empty")));
        }
        if get(s, MODE) >= 0.5 && s.groove.is_none() {
            set(s, MODE, 0.0);
        }
    }
    Ok(state(s))
}

/// Quantize with the stored settings, overridden by `p`, and by `fixed` (Start, End) for
/// Quantize 50% and Quantize Note Ends.
fn run_quantize(e: &mut Engine, p: &Value, fixed: Option<(f64, Option<f64>)>) -> Result<Value> {
    let id = "event.quantize";
    // Overrides apply to this run only: work on a copy of the settings.
    let mut tmp = Session::default();
    tmp.edit.values = e.session().edit.values.clone();
    tmp.edit.grid = e.session().edit.grid;
    tmp.groove = e.session().groove.clone();
    apply_settings(&mut tmp, id, p)?;
    if let Some((start, end)) = fixed {
        set(&mut tmp, START, start);
        if let Some(end) = end {
            set(&mut tmp, END, end);
        }
    }
    let groove = if get(&tmp, MODE) >= 0.5 { tmp.groove.clone() } else { None };
    let opts = QuantizeOptions {
        grid_ticks: grid_ticks(&tmp, get(&tmp, GRID) as i64, get(&tmp, GROUPING) as usize),
        strength: (get(&tmp, START) / 100.0) as f32,
        swing: (get(&tmp, SWING) / 100.0) as f32,
        include_within: (get(&tmp, RANGE) / 100.0) as f32,
        end_strength: (get(&tmp, END) / 100.0) as f32,
        ..QuantizeOptions::default()
    };
    let velocity = (get(&tmp, VELOCITY) / 100.0) as f32;
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    let (mut notes, mut clips) = (0, 0);
    for cid in ids {
        let Some((_, c)) = s.find_clip(cid) else { continue };
        let offset = s.tempo.samples_to_ticks(c.start, s.sample_rate);
        if let ClipContent::Midi { sequence } = &c.content {
            let original = sequence.clone();
            s.midi_originals.entry(cid.0).or_insert(original);
            let Some(c) = s.find_clip_mut(cid) else { continue };
            let ClipContent::Midi { sequence } = &mut c.content else { continue };
            let opts = QuantizeOptions { offset, ..opts };
            match &groove {
                Some(g) => quantize_to_groove(&mut sequence.notes, &g.points, g.length, &opts, velocity),
                None => quantize(&mut sequence.notes, &opts),
            }
            sequence.sort();
            notes += sequence.notes.len();
        } else if quantize_audio_clip(s, cid, &opts, groove.as_ref()) {
            clips += 1;
        }
    }
    Ok(json!({"notes": notes, "clips": clips}))
}

/// Moves an audio clip's start (its sync point, when set) towards the nearest grid line or
/// groove point. Returns whether it moved.
fn quantize_audio_clip(s: &mut Session, cid: ClipId, opts: &QuantizeOptions, groove: Option<&Groove>) -> bool {
    let Some((_, c)) = s.find_clip(cid) else { return false };
    if c.edit_locked {
        return false;
    }
    let sync = if (0..c.length).contains(&c.sync_point) { c.sync_point } else { 0 };
    let anchor = c.start.saturating_add(sync);
    let at = s.tempo.samples_to_ticks(anchor, s.sample_rate);
    let mut one = [soundcraft_midi::Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start: at, length: 1 }];
    match groove {
        Some(g) => quantize_to_groove(&mut one, &g.points, g.length, opts, 0.0),
        None => quantize(&mut one, &QuantizeOptions { end_strength: 0.0, ..*opts }),
    }
    let target: Samples = s.tempo.tick_to_samples(one[0].start, s.sample_rate);
    let delta = target.saturating_sub(anchor);
    if delta == 0 {
        return false;
    }
    crate::edit::move_clips(s, &[cid], delta, &[]);
    true
}

fn run_humanize(e: &mut Engine, p: &Value) -> Result<Value> {
    let timing = i64_or(p, "timing", 20).clamp(0, TICKS_PER_QUARTER * 4);
    let velocity = i64_or(p, "velocity", 10).clamp(0, 127) as u8;
    let seed = p.get("seed").and_then(Value::as_u64).unwrap_or(soundcraft_midi::ops::DEFAULT_SEED);
    let ids = clip_ids_param(e, p);
    let s = e.session_mut();
    let mut n = 0;
    for (k, cid) in ids.into_iter().enumerate() {
        if let Some(c) = s.find_clip_mut(cid)
            && let ClipContent::Midi { sequence } = &mut c.content
        {
            humanize(&mut sequence.notes, timing, velocity, seed.wrapping_add(k as u64));
            sequence.sort();
            n += sequence.notes.len();
        }
    }
    Ok(json!({"notes": n}))
}

/// The groove from the first selected clip: a MIDI clip's notes, or an audio clip's hits,
/// measured from the bar line at or before the clip and folded into whole bars.
fn groove_extract(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "event.groove_extract";
    let cid = clip_ids_param(e, p).into_iter().next().ok_or_else(|| bad(id, "select a clip"))?;
    let s = e.session();
    let (track, c) = s.find_clip(cid).ok_or_else(|| bad(id, "no such clip"))?;
    let sr = s.sample_rate;
    let start_tick = s.tempo.samples_to_ticks(c.start, sr);
    let end_tick = s.tempo.samples_to_ticks(c.end(), sr);
    let bar = s.tempo.bar_beat_at_tick(start_tick).bar;
    let bar_tick = s.tempo.bar_start_tick(bar);
    let bar_len = s.tempo.meter_at_tick(start_tick).ticks_per_bar().max(1);
    let bars = (end_tick.saturating_sub(bar_tick) + bar_len - 1).div_euclid(bar_len).clamp(1, 16);
    let length = bar_len.saturating_mul(bars);
    let positions: Vec<(i64, u8)> = match &c.content {
        ClipContent::Midi { sequence } => {
            sequence.notes.iter().map(|n| (start_tick.saturating_add(n.start).saturating_sub(bar_tick), n.velocity)).collect()
        }
        _ => {
            let range = soundcraft_time::Range::new(c.start, c.end());
            let sens = f64_or(p, "sensitivity", 0.5).clamp(0.0, 1.0) as f32;
            crate::io::transients_in(s, track, range, sens)
                .into_iter()
                .map(|at| (s.tempo.samples_to_ticks(at, sr).saturating_sub(bar_tick), 100))
                .collect()
        }
    };
    let groove = Groove::from_positions(&c.name, length, positions).ok_or_else(|| bad(id, "the clip has no notes or hits"))?;
    let points = groove.points.len();
    let s = e.session_mut();
    s.groove = Some(groove);
    set(s, MODE, 1.0);
    Ok(json!({"points": points, "length": length, "bars": bars}))
}

#[cfg(test)]
mod tests;
