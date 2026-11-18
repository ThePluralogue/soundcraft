//! Cue mixes: separate headphone or monitor mixes. A cue mix is a stereo bus every channel
//! sends to (pre-fader, so the main mix can change without moving the performer's balance),
//! with its own aux channel played out of a hardware output such as Out 3-4. "Follow main
//! mix" makes the sends track the channel faders and pans instead.
use super::{CommandSpec, always, bad, bool_or, has_tracks, str_param, tracks_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{BusId, ChannelFormat, CueMix, Route, SEND_SLOTS, SendSlot, Session, Track, TrackId, TrackKind};

/// The hardware output a new cue mix plays out of when none is given.
pub const DEFAULT_CUE_OUTPUT: &str = "Out 3-4";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "mix.cue_new",
            "New Cue Mix",
            [],
            None,
            "{name?, output?: 'Out 3-4', follow_main?: false} — a stereo cue bus with its own aux channel on that output, and a send to it on every channel, starting as a copy of the main mix",
            always,
            cue_new
        ),
        cmd!("mix.cue_follow", "Cue Mix Follows Main Mix", [], None, "{cue, value?: bool} (toggles when omitted)", has_cues, cue_follow),
        cmd!(
            "mix.cue_copy_main",
            "Copy Main Mix to Cue Mix",
            [],
            None,
            "{cue} — sets every cue send to its channel's fader and pan",
            has_cues,
            cue_copy_main
        ),
        cmd!(
            "mix.cue_add_tracks",
            "Add Channels to Cue Mix",
            [],
            None,
            "{cue, tracks?} — gives channels without one a send to the cue mix (all channels when `tracks` is omitted)",
            has_cues,
            cue_add_tracks
        ),
        cmd!("mix.cue_remove", "Remove Cue Mix", [], None, "{cue} — removes the cue mix, its sends and its aux channel", has_cues, cue_remove),
        cmd!(query "mix.cue_list", "List Cue Mixes", [], None, "{}", always, |e, _| Ok(json!({"cues": cue_list(e.session())}))),
    ]
}

fn has_cues(e: &Engine) -> std::result::Result<(), String> {
    has_tracks(e)?;
    if e.session().busses.iter().any(|b| b.cue.is_some()) { Ok(()) } else { Err("no cue mixes".into()) }
}

/// Cue mixes in session order: name, bus, aux channel, output and follow setting.
fn cue_list(s: &Session) -> Vec<Value> {
    s.busses
        .iter()
        .filter_map(|b| {
            let cue = b.cue?;
            let aux = cue_aux(s, b.id);
            Some(json!({
                "name": b.name,
                "bus": b.id,
                "track": aux.map(|t| t.id),
                "output": aux.map(|t| route_label(&t.mixer.output)),
                "follow_main": cue.follow_main,
                "sends": s.tracks.iter().filter(|t| send_slot(t, b.id).is_some()).count(),
            }))
        })
        .collect()
}

fn route_label(r: &Route) -> String {
    match r {
        Route::None => "none".into(),
        Route::Main => "Main".into(),
        Route::Bus(b) => format!("bus {}", b.0),
        Route::Hardware(h) => h.clone(),
    }
}

/// The aux channel that plays a cue bus.
fn cue_aux(s: &Session, bus: BusId) -> Option<&Track> {
    s.tracks.iter().find(|t| t.kind == TrackKind::Aux && t.mixer.input == Route::Bus(bus))
}

/// The cue mix named by `cue` (its bus or aux channel name).
fn cue_param(e: &Engine, id: &str, p: &Value) -> Result<BusId> {
    let name = str_param(p, "cue").or_else(|| str_param(p, "bus")).ok_or_else(|| bad(id, "`cue` required"))?;
    e.session()
        .busses
        .iter()
        .find(|b| b.cue.is_some() && b.name.eq_ignore_ascii_case(name))
        .map(|b| b.id)
        .ok_or_else(|| bad(id, format!("no cue mix named `{name}`")))
}

/// Can `t` feed cue mix `bus`? Audio, instrument and aux channels can, except the cue's own aux
/// and anything that already plays into the bus.
fn can_feed(t: &Track, bus: BusId) -> bool {
    matches!(t.kind, TrackKind::Audio | TrackKind::Instrument | TrackKind::Aux)
        && t.mixer.input != Route::Bus(bus)
        && t.mixer.output != Route::Bus(bus)
        && !t.inactive
}

fn send_slot(t: &Track, bus: BusId) -> Option<usize> {
    t.mixer.sends.iter().position(|x| x.as_ref().is_some_and(|x| x.target == Route::Bus(bus)))
}

/// A cue send set to the channel's main mix: the fader level, and the pan (a stereo channel's
/// balance stays centred).
fn copy_of_main(t: &Track, bus: BusId) -> SendSlot {
    let mut sl = SendSlot::new(Route::Bus(bus));
    sl.level_db = t.mixer.volume_db.clamp(-144.0, 12.0);
    sl.pan = if t.mixer.pan.len() == 1 { t.mixer.pan.first().copied().unwrap_or(0.0).clamp(-1.0, 1.0) } else { 0.0 };
    sl.pre_fader = true;
    sl
}

/// Give each of `tracks` without one a send to `bus`; returns how many were added and the
/// names of the channels with no free send slot.
fn add_sends(s: &mut Session, bus: BusId, tracks: &[TrackId]) -> (usize, Vec<String>) {
    let (mut added, mut full) = (0, Vec::new());
    for id in tracks {
        let Some(t) = s.track_mut(*id) else { continue };
        if !can_feed(t, bus) || send_slot(t, bus).is_some() {
            continue;
        }
        let sl = copy_of_main(t, bus);
        match t.mixer.sends.iter_mut().find(|x| x.is_none()) {
            Some(free) => {
                *free = Some(sl);
                added += 1;
            }
            None => full.push(t.name.clone()),
        }
    }
    (added, full)
}

fn cue_new(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.cue_new";
    let s = e.session_mut();
    let name = match str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => {
            if s.bus_by_name(n).is_some() || s.track_by_name(n).is_some() {
                return Err(bad(id, format!("`{n}` is already a bus or channel name")));
            }
            n.to_string()
        }
        None => (1..=SEND_SLOTS * 100)
            .map(|i| format!("Cue {i}"))
            .find(|n| s.bus_by_name(n).is_none() && s.track_by_name(n).is_none())
            .ok_or_else(|| bad(id, "no free cue name"))?,
    };
    let output = str_param(p, "output").map(str::trim).filter(|o| !o.is_empty()).unwrap_or(DEFAULT_CUE_OUTPUT).to_string();
    let output = match output.to_ascii_lowercase().as_str() {
        "main" | "out 1-2" => Route::Main,
        _ => Route::Hardware(output),
    };
    let follow_main = bool_or(p, "follow_main", false);
    let bus = s.add_bus(&name, ChannelFormat::Stereo);
    if let Some(b) = s.busses.iter_mut().find(|b| b.id == bus) {
        b.cue = Some(CueMix { follow_main });
    }
    let feeders: Vec<TrackId> = s.tracks.iter().map(|t| t.id).collect();
    let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some(&name));
    if let Some(t) = s.track_mut(aux) {
        t.mixer.input = Route::Bus(bus);
        t.mixer.output = output;
    }
    let (sends, full) = add_sends(s, bus, &feeders);
    Ok(json!({"name": name, "bus": bus, "track": aux, "sends": sends, "no_free_send": full}))
}

fn cue_follow(e: &mut Engine, p: &Value) -> Result<Value> {
    let bus = cue_param(e, "mix.cue_follow", p)?;
    let s = e.session_mut();
    let b = s.busses.iter_mut().find(|b| b.id == bus).ok_or_else(|| bad("mix.cue_follow", "no cue mix"))?;
    let cue = b.cue.get_or_insert_with(CueMix::default);
    let follow = bool_or(p, "value", !cue.follow_main);
    if follow == cue.follow_main {
        return Ok(json!({"follow_main": follow}));
    }
    cue.follow_main = follow;
    for t in &mut s.tracks {
        let main = copy_of_main(t, bus);
        for sl in t.mixer.sends.iter_mut().flatten().filter(|x| x.target == Route::Bus(bus)) {
            if follow {
                // Following, a send level is an offset from the fader: start with none.
                sl.level_db = 0.0;
            } else {
                // Leaving, the cue keeps what it was playing: the main mix's levels and pans.
                sl.level_db = main.level_db;
                sl.pan = main.pan;
            }
        }
    }
    Ok(json!({"follow_main": follow}))
}

fn cue_copy_main(e: &mut Engine, p: &Value) -> Result<Value> {
    let bus = cue_param(e, "mix.cue_copy_main", p)?;
    let s = e.session_mut();
    let mut n = 0;
    for t in &mut s.tracks {
        let main = copy_of_main(t, bus);
        for sl in t.mixer.sends.iter_mut().flatten().filter(|x| x.target == Route::Bus(bus)) {
            sl.level_db = main.level_db;
            sl.pan = main.pan;
            sl.mute = false;
            n += 1;
        }
    }
    // A copy is a mix of its own: the cue stops following the faders.
    if let Some(cue) = s.busses.iter_mut().find(|b| b.id == bus).and_then(|b| b.cue.as_mut()) {
        cue.follow_main = false;
    }
    Ok(json!({"sends": n}))
}

fn cue_add_tracks(e: &mut Engine, p: &Value) -> Result<Value> {
    let bus = cue_param(e, "mix.cue_add_tracks", p)?;
    let mut tracks = tracks_param(e, "mix.cue_add_tracks", p)?;
    if p.get("tracks").is_none() && p.get("track").is_none() {
        tracks = e.session().tracks.iter().map(|t| t.id).collect();
    }
    let (sends, full) = add_sends(e.session_mut(), bus, &tracks);
    Ok(json!({"sends": sends, "no_free_send": full}))
}

fn cue_remove(e: &mut Engine, p: &Value) -> Result<Value> {
    let bus = cue_param(e, "mix.cue_remove", p)?;
    let s = e.session_mut();
    let aux = cue_aux(s, bus).map(|t| t.id);
    if let Some(aux) = aux {
        s.tracks.retain(|t| t.id != aux);
        s.edit.selected_tracks.retain(|t| *t != aux);
        for g in &mut s.groups {
            g.members.retain(|t| *t != aux);
        }
    }
    for t in &mut s.tracks {
        for sl in &mut t.mixer.sends {
            if sl.as_ref().is_some_and(|x| x.target == Route::Bus(bus)) {
                *sl = None;
            }
        }
        if t.mixer.input == Route::Bus(bus) {
            t.mixer.input = Route::None;
        }
        if t.mixer.output == Route::Bus(bus) {
            t.mixer.output = Route::Main;
        }
    }
    s.busses.retain(|b| b.id != bus);
    Ok(json!({"removed": true}))
}

#[cfg(test)]
mod tests;
