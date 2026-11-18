//! FX Chains (a channel's whole insert chain, saved to a file and loaded onto any channel) and
//! console Scenes (stored mixer states recalled in one step).
use super::{CommandSpec, always, bad, has_tracks, str_param, track_param, tracks_required};

use crate::{Engine, EngineError, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{INSERT_SLOTS, Insert, MAX_SCENES, MixScene, Mixer, TrackId, truncate_chars};

/// `format` tag of an FX Chain file.
pub const CHAIN_FORMAT: &str = "soundcraft-fx-chain";
/// File extension of FX Chain files.
pub const CHAIN_EXTENSION: &str = "scchain";

/// The parts of a scene `mix.scene_recall` applies when `parts` is not given. Routing is left out:
/// recalling a mix balance must not silently re-patch inputs and outputs.
const DEFAULT_PARTS: &[&str] = &["levels", "mutes", "inserts", "sends"];
const ALL_PARTS: &[&str] = &["levels", "mutes", "inserts", "sends", "routing"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "mix.chain_save", "Save FX Chain...", [], None, "{track?, path, name?} — writes the track's inserts as an FX Chain file", has_tracks, chain_save),
        cmd!(
            "mix.chain_load",
            "Load FX Chain",
            [],
            None,
            "{tracks?, path, mode?: replace|append} — unknown plugins are skipped and listed",
            has_tracks,
            chain_load
        ),
        cmd!(query "mix.chain_list", "List FX Chains", [], None, "{dir} — FX Chain files in a folder", always, chain_list),
        cmd!("mix.chain_copy", "Copy FX Chain", [], None, "{from, tracks, mode?: replace|append}", has_tracks, chain_copy),
        cmd!(
            "mix.insert_copy",
            "Copy Insert",
            [],
            None,
            "{from: track, from_slot, track, slot} — copies one insert (settings and state), replacing what is there",
            has_tracks,
            insert_copy
        ),
        cmd!(
            "mix.scene_store",
            "Store Scene",
            [],
            None,
            "{name?, index?} — stores every track's mixer; `index` overwrites a scene",
            has_tracks,
            scene_store
        ),
        cmd!(
            "mix.scene_recall",
            "Recall Scene",
            [],
            None,
            "{index, parts?: [levels|mutes|inserts|sends|routing]} (default: all but routing)",
            has_tracks,
            scene_recall
        ),
        cmd!("mix.scene_rename", "Rename Scene", [], None, "{index, name}", always, scene_rename),
        cmd!("mix.scene_delete", "Delete Scene", [], None, "{index}", always, scene_delete),
        cmd!(query "mix.scene_list", "List Scenes", [], None, "{}", always, |e, _| {
            let list: Vec<Value> =
                e.session().scenes.iter().enumerate().map(|(i, s)| json!({"index": i, "name": s.name, "tracks": s.tracks.len()})).collect();
            Ok(json!({"scenes": list}))
        }),
    ]
}

// ---- FX Chains ---------------------------------------------------------------------------

fn used_inserts(m: &Mixer) -> Vec<Insert> {
    m.inserts.iter().flatten().cloned().collect()
}

/// Put `chain` on a mixer. `replace` clears every slot first; append fills free slots. Plugins
/// this build can't host are skipped. Returns (placed, skipped plugin ids).
fn apply_chain(m: &mut Mixer, chain: &[Insert], replace: bool) -> (usize, Vec<String>) {
    m.inserts.resize(INSERT_SLOTS, None);
    if replace {
        m.inserts.iter_mut().for_each(|s| *s = None);
    }
    let mut placed = 0;
    let mut skipped = Vec::new();
    for ins in chain {
        let Some(info) = super::clap::plugin_info(&ins.plugin) else {
            skipped.push(ins.plugin.clone());
            continue;
        };
        let Some(slot) = m.inserts.iter_mut().find(|s| s.is_none()) else {
            skipped.push(ins.plugin.clone());
            continue;
        };
        let mut ins = ins.clone();
        // Clamp values from the file into the plugin's ranges and drop ids it doesn't have.
        ins.params.retain(|k, _| info.params.iter().any(|p| p.id == k));
        for p in info.params {
            let v = ins.params.get(p.id).copied().filter(|v| v.is_finite()).unwrap_or(p.default);
            ins.params.insert(p.id.to_string(), v.clamp(p.min, p.max));
        }
        truncate_chars(&mut ins.preset, soundcraft_model::MAX_NAME_CHARS);
        *slot = Some(ins);
        placed += 1;
    }
    (placed, skipped)
}

fn replace_mode(cmd: &str, p: &Value) -> Result<bool> {
    match str_param(p, "mode").unwrap_or("replace") {
        "replace" => Ok(true),
        "append" => Ok(false),
        other => Err(bad(cmd, format!("`mode` must be replace or append, not `{other}`"))),
    }
}

fn chain_save(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.chain_save";
    let path = str_param(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(id, "`path` required"))?.to_string();
    let t = *tracks_required(e, id, p)?.first().ok_or_else(|| bad(id, "no track"))?;
    let tr = e.session().track(t).ok_or_else(|| bad(id, "no track"))?;
    let inserts = used_inserts(&tr.mixer);
    if inserts.is_empty() {
        return Err(bad(id, format!("`{}` has no inserts to save", tr.name)));
    }
    let stem = std::path::Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("FX Chain").to_string();
    let mut name = str_param(p, "name").map_or(stem, str::to_string);
    truncate_chars(&mut name, soundcraft_model::MAX_NAME_CHARS);
    let doc = json!({"format": CHAIN_FORMAT, "version": 1, "name": name, "inserts": inserts});
    let text = serde_json::to_string_pretty(&doc).map_err(|err| EngineError::Io(err.to_string()))?;
    if let Some(dir) = std::path::Path::new(&path).parent() {
        crate::io::make_dir(dir)?;
    }
    crate::io::write_file(&path, text.as_bytes())?;
    Ok(json!({"path": path, "name": name, "inserts": inserts.len()}))
}

fn read_chain(cmd: &str, path: &str) -> Result<(String, Vec<Insert>)> {
    let text = crate::io::read_text(path)?;
    let v: Value = serde_json::from_str(&text).map_err(|err| EngineError::Io(format!("{path}: {err}")))?;
    if v.get("format").and_then(Value::as_str) != Some(CHAIN_FORMAT) {
        return Err(bad(cmd, format!("{path}: not a SoundCraft FX Chain")));
    }
    let name = v.get("name").and_then(Value::as_str).unwrap_or("FX Chain").to_string();
    let list = v.get("inserts").and_then(Value::as_array).cloned().unwrap_or_default();
    // A malformed entry is skipped rather than failing the whole chain.
    let inserts = list.into_iter().take(INSERT_SLOTS).filter_map(|x| serde_json::from_value::<Insert>(x).ok()).collect();
    Ok((name, inserts))
}

fn chain_load(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.chain_load";
    let path = str_param(p, "path").ok_or_else(|| bad(id, "`path` required"))?.to_string();
    let replace = replace_mode(id, p)?;
    let tracks = tracks_required(e, id, p)?;
    let (name, chain) = read_chain(id, &path)?;
    let (placed, skipped) = apply_to(e, &tracks, &chain, replace);
    Ok(json!({"name": name, "tracks": tracks.iter().map(|t| t.0).collect::<Vec<_>>(), "placed": placed, "skipped": skipped}))
}

fn apply_to(e: &mut Engine, tracks: &[TrackId], chain: &[Insert], replace: bool) -> (usize, Vec<String>) {
    let s = e.session_mut();
    let mut placed = 0;
    let mut skipped = Vec::new();
    for t in tracks {
        if let Some(tr) = s.track_mut(*t) {
            let (n, mut sk) = apply_chain(&mut tr.mixer, chain, replace);
            placed += n;
            skipped.append(&mut sk);
        }
    }
    skipped.sort();
    skipped.dedup();
    (placed, skipped)
}

fn chain_copy(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.chain_copy";
    let from = track_param(e, id, p, "from")?.ok_or_else(|| bad(id, "`from` (track) required"))?;
    let replace = replace_mode(id, p)?;
    let tracks: Vec<TrackId> = tracks_required(e, id, p)?.into_iter().filter(|t| *t != from).collect();
    if tracks.is_empty() {
        return Err(bad(id, "choose at least one other track"));
    }
    let chain = e.session().track(from).map(|t| used_inserts(&t.mixer)).unwrap_or_default();
    let (placed, skipped) = apply_to(e, &tracks, &chain, replace);
    Ok(json!({"placed": placed, "skipped": skipped}))
}

fn insert_slot_param(cmd: &str, p: &Value, key: &str) -> Result<usize> {
    p.get(key)
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v < INSERT_SLOTS)
        .ok_or_else(|| bad(cmd, format!("`{key}` must be an insert slot 0..{}", INSERT_SLOTS - 1)))
}

fn insert_copy(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.insert_copy";
    let from = track_param(e, id, p, "from")?.ok_or_else(|| bad(id, "`from` (track) required"))?;
    let to = track_param(e, id, p, "track")?.ok_or_else(|| bad(id, "`track` required"))?;
    let from_slot = insert_slot_param(id, p, "from_slot")?;
    let slot = insert_slot_param(id, p, "slot")?;
    if from == to && from_slot == slot {
        return Err(bad(id, "the source and destination are the same slot"));
    }
    let ins = e.session().track(from).and_then(|t| t.mixer.inserts.get(from_slot).cloned().flatten()).ok_or_else(|| bad(id, "that slot is empty"))?;
    let plugin = ins.plugin.clone();
    let tr = e.session_mut().track_mut(to).ok_or_else(|| bad(id, "no such track"))?;
    if let Some(x) = tr.mixer.inserts.get_mut(slot) {
        *x = Some(ins);
    }
    Ok(json!({"plugin": plugin, "track": to.0, "slot": slot}))
}

fn chain_list(_e: &mut Engine, p: &Value) -> Result<Value> {
    let dir = str_param(p, "dir").ok_or_else(|| bad("mix.chain_list", "`dir` required"))?;
    // Bounded: a chain folder is browsed in a menu.
    let mut out: Vec<(String, String)> = crate::io::list_files(dir, CHAIN_EXTENSION, 500)
        .into_iter()
        .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), p.display().to_string())))
        .collect();
    out.sort_by_key(|(n, _)| n.to_lowercase());
    let chains: Vec<Value> = out.into_iter().map(|(name, path)| json!({"name": name, "path": path})).collect();
    Ok(json!({"dir": dir, "chains": chains}))
}

// ---- Scenes -------------------------------------------------------------------------------

fn index_param(e: &Engine, cmd: &str, p: &Value) -> Result<usize> {
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`index` required"))?;
    let i = usize::try_from(i).map_err(|_| bad(cmd, "`index` out of range"))?;
    if i < e.session().scenes.len() { Ok(i) } else { Err(bad(cmd, format!("no scene {i}"))) }
}

fn scene_store(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.scene_store";
    let overwrite = if p.get("index").is_some() { Some(index_param(e, id, p)?) } else { None };
    let s = e.session_mut();
    if overwrite.is_none() && s.scenes.len() >= MAX_SCENES {
        return Err(bad(id, format!("a session keeps at most {MAX_SCENES} scenes; delete one first")));
    }
    let tracks: Vec<(u64, Mixer)> = s.tracks.iter().map(|t| (t.id.0, t.mixer.clone())).collect();
    let mut name = match (str_param(p, "name").filter(|n| !n.trim().is_empty()), overwrite) {
        (Some(n), _) => n.to_string(),
        (None, Some(i)) => s.scenes.get(i).map(|x| x.name.clone()).unwrap_or_default(),
        (None, None) => format!("Scene {}", s.scenes.len() + 1),
    };
    truncate_chars(&mut name, soundcraft_model::MAX_NAME_CHARS);
    let scene = MixScene { name: name.clone(), tracks };
    let index = match overwrite.and_then(|i| s.scenes.get_mut(i).map(|x| (i, x))) {
        Some((i, slot)) => {
            *slot = scene;
            i
        }
        None => {
            s.scenes.push(scene);
            s.scenes.len() - 1
        }
    };
    Ok(json!({"index": index, "name": name}))
}

fn scene_recall(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.scene_recall";
    let i = index_param(e, id, p)?;
    let parts: Vec<String> = match p.get("parts") {
        None | Some(Value::Null) => DEFAULT_PARTS.iter().map(|x| x.to_string()).collect(),
        Some(Value::Array(a)) => {
            let mut v = Vec::new();
            for x in a {
                let s = x.as_str().filter(|s| ALL_PARTS.contains(s)).ok_or_else(|| bad(id, format!("unknown part {x}")))?;
                v.push(s.to_string());
            }
            v
        }
        Some(_) => return Err(bad(id, "`parts` must be a list")),
    };
    let has = |k: &str| parts.iter().any(|x| x == k);
    let s = e.session_mut();
    let scene = s.scenes.get(i).cloned().ok_or_else(|| bad(id, format!("no scene {i}")))?;
    let mut applied = 0;
    for (tid, m) in &scene.tracks {
        let Some(tr) = s.track_mut(TrackId(*tid)) else { continue };
        let cur = &mut tr.mixer;
        if has("levels") {
            cur.volume_db = m.volume_db;
            cur.trim_db = m.trim_db;
            // A track whose width changed since the scene keeps its own panners.
            if cur.pan.len() == m.pan.len() {
                cur.pan = m.pan.clone();
            }
            cur.surround = m.surround;
        }
        if has("mutes") {
            cur.mute = m.mute;
            cur.solo = m.solo;
            cur.solo_safe = m.solo_safe;
            cur.phase_invert = m.phase_invert;
        }
        if has("inserts") {
            // Keep slot positions: rebuild slot by slot rather than packing the chain.
            cur.inserts = m.inserts.clone();
            cur.inserts.resize(INSERT_SLOTS, None);
            for slot in cur.inserts.iter_mut() {
                if slot.as_ref().is_some_and(|x| super::clap::plugin_info(&x.plugin).is_none()) {
                    *slot = None;
                }
            }
        }
        if has("sends") {
            cur.sends = m.sends.clone();
        }
        if has("routing") {
            cur.input = m.input.clone();
            cur.output = m.output.clone();
            cur.vca = m.vca;
        }
        cur.sanitize();
        applied += 1;
    }
    Ok(json!({"index": i, "name": scene.name, "tracks": applied, "parts": parts}))
}

fn scene_rename(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "mix.scene_rename";
    let i = index_param(e, id, p)?;
    let mut name = str_param(p, "name").filter(|n| !n.trim().is_empty()).ok_or_else(|| bad(id, "`name` required"))?.to_string();
    truncate_chars(&mut name, soundcraft_model::MAX_NAME_CHARS);
    if let Some(s) = e.session_mut().scenes.get_mut(i) {
        s.name = name.clone();
    }
    Ok(json!({"index": i, "name": name}))
}

fn scene_delete(e: &mut Engine, p: &Value) -> Result<Value> {
    let i = index_param(e, "mix.scene_delete", p)?;
    let s = e.session_mut();
    let removed = if i < s.scenes.len() { s.scenes.remove(i).name } else { String::new() };
    Ok(json!({"deleted": removed}))
}

#[cfg(test)]
mod tests {
    use crate::Engine;
    use serde_json::json;
    use soundcraft_model::{ChannelFormat, Session, TrackKind};

    fn two_tracks() -> (Engine, u64, u64) {
        let mut s = Session::default();
        let a = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Vox"));
        let b = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Gtr"));
        (Engine::new(s), a.0, b.0)
    }

    #[test]
    fn chain_saves_loads_and_copies() {
        let (mut e, a, b) = two_tracks();
        e.execute("mix.insert", &json!({"track": a, "plugin": "eq_7band", "slot": 0})).unwrap();
        e.execute("mix.insert", &json!({"track": a, "plugin": "compressor", "slot": 3})).unwrap();
        let dir = std::env::temp_dir().join(format!("sc-chain-{}", std::process::id()));
        let path = dir.join("Vocal.scchain");
        let r = e.execute("mix.chain_save", &json!({"track": a, "path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["inserts"], 2);
        let r = e.execute("mix.chain_load", &json!({"tracks": [b], "path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["placed"], 2);
        let tb = e.session().track(soundcraft_model::TrackId(b)).unwrap();
        // Loaded chains pack into the first slots, in order.
        assert_eq!(tb.mixer.inserts[0].as_ref().map(|i| i.plugin.as_str()), Some("eq_7band"));
        assert_eq!(tb.mixer.inserts[1].as_ref().map(|i| i.plugin.as_str()), Some("compressor"));
        let list = e.execute("mix.chain_list", &json!({"dir": dir.to_string_lossy()})).unwrap();
        assert_eq!(list["chains"][0]["name"], "Vocal");
        e.execute("mix.chain_copy", &json!({"from": a, "tracks": [b], "mode": "append"})).unwrap();
        let tb = e.session().track(soundcraft_model::TrackId(b)).unwrap();
        assert_eq!(tb.mixer.inserts.iter().flatten().count(), 4);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_plugins_are_skipped_not_fatal() {
        let (mut e, a, _) = two_tracks();
        let dir = std::env::temp_dir().join(format!("sc-chain-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.scchain");
        std::fs::write(&path, r#"{"format":"soundcraft-fx-chain","inserts":[{"plugin":"nope"},{"plugin":"eq_7band","params":{"zzz":1}},7]}"#)
            .unwrap();
        let r = e.execute("mix.chain_load", &json!({"tracks": [a], "path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["placed"], 1);
        assert_eq!(r["skipped"], json!(["nope"]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scenes_store_and_recall_parts() {
        let (mut e, a, _) = two_tracks();
        e.execute("mix.volume", &json!({"tracks": [a], "db": -6.0})).unwrap();
        e.execute("mix.insert", &json!({"track": a, "plugin": "eq_7band", "slot": 2})).unwrap();
        e.execute("mix.scene_store", &json!({"name": "Verse"})).unwrap();
        e.execute("mix.volume", &json!({"tracks": [a], "db": 0.0})).unwrap();
        e.execute("mix.mute", &json!({"tracks": [a], "value": true})).unwrap();
        e.execute("mix.insert_remove", &json!({"track": a, "slot": 2})).unwrap();
        e.execute("mix.scene_recall", &json!({"index": 0, "parts": ["levels"]})).unwrap();
        let m = &e.session().track(soundcraft_model::TrackId(a)).unwrap().mixer;
        assert_eq!(m.volume_db, -6.0);
        assert!(m.mute, "mutes were not part of the recall");
        assert!(m.inserts[2].is_none());
        e.execute("mix.scene_recall", &json!({"index": 0})).unwrap();
        let m = &e.session().track(soundcraft_model::TrackId(a)).unwrap().mixer;
        assert!(!m.mute);
        assert_eq!(m.inserts[2].as_ref().map(|i| i.plugin.as_str()), Some("eq_7band"));
        assert!(e.execute("mix.scene_recall", &json!({"index": 5})).is_err());
        assert!(e.execute("mix.scene_recall", &json!({"index": 0, "parts": ["bogus"]})).is_err());
        e.execute("mix.scene_rename", &json!({"index": 0, "name": "Chorus"})).unwrap();
        assert_eq!(e.execute("mix.scene_list", &json!({})).unwrap()["scenes"][0]["name"], "Chorus");
        e.execute("mix.scene_delete", &json!({"index": 0})).unwrap();
        assert!(e.session().scenes.is_empty());
    }

    #[test]
    fn scenes_survive_save_and_reload() {
        let (mut e, a, _) = two_tracks();
        e.execute("mix.volume", &json!({"tracks": [a], "db": -3.0})).unwrap();
        e.execute("mix.scene_store", &json!({})).unwrap();
        let text = serde_json::to_string(e.session()).unwrap();
        let back = Session::from_json(&text).unwrap();
        assert_eq!(back.scenes.len(), 1);
        assert_eq!(back.scenes[0].name, "Scene 1");
    }
}

#[cfg(test)]
mod insert_copy_tests {
    use crate::Engine;
    use serde_json::json;
    use soundcraft_model::{ChannelFormat, Session, TrackId, TrackKind};

    fn two_tracks() -> (Engine, u64, u64) {
        let mut s = Session::default();
        let a = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Vox"));
        let b = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Gtr"));
        (Engine::new(s), a.0, b.0)
    }

    #[test]
    fn an_insert_copies_to_another_track_with_its_settings() {
        let (mut e, a, b) = two_tracks();
        e.execute("mix.insert", &json!({"track": a, "slot": 0, "plugin": "compressor"})).unwrap();
        e.execute("mix.insert_bypass", &json!({"track": a, "slot": 0, "value": true})).unwrap();
        e.execute("mix.insert_copy", &json!({"from": a, "from_slot": 0, "track": b, "slot": 3})).unwrap();
        let src = e.session().track(TrackId(a)).unwrap().mixer.inserts[0].clone();
        let dst = e.session().track(TrackId(b)).unwrap().mixer.inserts[3].clone();
        assert_eq!(src, dst);
        assert!(dst.unwrap().bypass);
        assert!(e.undo());
        assert!(e.session().track(TrackId(b)).unwrap().mixer.inserts[3].is_none());
    }

    #[test]
    fn copying_an_empty_or_out_of_range_slot_is_an_error() {
        let (mut e, a, _) = two_tracks();
        assert!(e.execute("mix.insert_copy", &json!({"from": a, "from_slot": 1, "track": a, "slot": 2})).is_err());
        e.execute("mix.insert", &json!({"track": a, "slot": 1, "plugin": "compressor"})).unwrap();
        assert!(e.execute("mix.insert_copy", &json!({"from": a, "from_slot": 99, "track": a, "slot": 2})).is_err());
        assert!(e.execute("mix.insert_copy", &json!({"from": a, "from_slot": 1, "track": a, "slot": 1})).is_err());
        e.execute("mix.insert_copy", &json!({"from": a, "from_slot": 1, "track": a, "slot": 2})).unwrap();
        assert!(e.session().track(TrackId(a)).unwrap().mixer.inserts[2].is_some());
    }
}
