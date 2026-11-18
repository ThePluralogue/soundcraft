//! FX Chain and Scene menus: save a channel's inserts as a chain and load chains onto channels;
//! store and recall whole-console scenes.
use super::{SoundApp, Tokens, Ui, json};
use soundcraft_model::TrackId;

/// The folder FX Chains are saved to and listed from: the chosen one, else "FX Chains" beside the
/// session's folder, else in the home folder.
fn chain_dir(app: &SoundApp) -> String {
    if !app.ui.fx_chain_dir.trim().is_empty() {
        return app.ui.fx_chain_dir.clone();
    }
    // A session lives at <sessions>/<name>/<name>.scraft: chains go in <sessions>/FX Chains.
    let beside = app
        .engine
        .path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).parent())
        .and_then(std::path::Path::parent)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join("FX Chains").display().to_string());
    beside.unwrap_or_else(|| {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
        format!("{home}/SoundCraft FX Chains")
    })
}

/// A name usable as a file name on every platform.
fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.len() <= 120 && !n.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
}

fn dim(ui: &mut Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).color(Tokens::current().text_dim).small());
}

/// The FX Chain menu of one channel: save its inserts, load a saved chain (replacing or after the
/// current inserts), or copy its chain to the other selected channels.
pub(crate) fn chain_menu(app: &mut SoundApp, ui: &mut Ui, tid: TrackId) {
    let dir = chain_dir(app);
    let (has_inserts, selected) = {
        let s = app.engine.session();
        let has = s.track(tid).is_some_and(|t| t.mixer.inserts.iter().any(Option::is_some));
        let sel: Vec<u64> = s.edit.selected_tracks.iter().filter(|t| **t != tid).map(|t| t.0).collect();
        (has, sel)
    };
    ui.set_min_width(240.0);
    dim(ui, "Save this channel's inserts as a chain");
    ui.horizontal(|ui| {
        let edit = ui.add(egui::TextEdit::singleline(&mut app.ui.chain_name).hint_text("Chain name").desired_width(150.0));
        let ok = has_inserts && valid_name(&app.ui.chain_name);
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let save = ui.add_enabled(ok, egui::Button::new("Save"));
        let save = if has_inserts { save } else { save.on_disabled_hover_text("This channel has no inserts") };
        if ok && (save.clicked() || enter) {
            let name = app.ui.chain_name.trim().to_string();
            let path = format!("{dir}/{name}.{}", soundcraft_engine::cmd::CHAIN_EXTENSION);
            if let Ok(r) = app.run("mix.chain_save", json!({"track": tid.0, "path": path, "name": name})) {
                app.ui.status = format!("Saved FX Chain “{name}” ({} inserts)", r["inserts"]);
                app.ui.chain_name.clear();
            }
            ui.close();
        }
    });
    ui.separator();
    let chains: Vec<(String, String)> = app
        .engine
        .execute("mix.chain_list", &json!({"dir": dir}))
        .ok()
        .and_then(|v| v["chains"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| Some((c["name"].as_str()?.to_string(), c["path"].as_str()?.to_string())))
        .collect();
    if chains.is_empty() {
        dim(ui, "No saved chains yet");
    } else {
        for (label, mode, tip) in [
            ("Load Chain", "replace", "Replace this channel's inserts with the chain"),
            ("Add Chain After Inserts", "append", "Put the chain's plugins in the free slots after the current inserts"),
        ] {
            ui.menu_button(label, |ui| {
                dim(ui, tip);
                for (name, path) in &chains {
                    if ui.button(name).clicked() {
                        load(app, tid, name, path, mode);
                        ui.close();
                    }
                }
            });
        }
    }
    if !selected.is_empty() {
        ui.separator();
        let label = format!("Copy Chain to {} Selected Track{}", selected.len(), if selected.len() == 1 { "" } else { "s" });
        if ui.add_enabled(has_inserts, egui::Button::new(label)).clicked() {
            if let Ok(r) = app.run("mix.chain_copy", json!({"from": tid.0, "tracks": selected})) {
                app.ui.status = format!("Copied {} inserts", r["placed"]);
            }
            ui.close();
        }
    }
    ui.separator();
    dim(ui, format!("Folder: {dir}"));
}

fn load(app: &mut SoundApp, tid: TrackId, name: &str, path: &str, mode: &str) {
    if let Ok(r) = app.run("mix.chain_load", json!({"tracks": [tid.0], "path": path, "mode": mode})) {
        let skipped: Vec<&str> = r["skipped"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
        app.ui.status = if skipped.is_empty() {
            format!("Loaded FX Chain “{name}”")
        } else {
            format!("Loaded FX Chain “{name}”; not available here: {}", skipped.join(", "))
        };
        crate::panels::open_chain(app, tid, 0);
    }
}

/// The Scenes menu: recall, update or delete stored console states, and store a new one.
pub(crate) fn scenes_menu(app: &mut SoundApp, ui: &mut Ui) {
    ui.set_min_width(260.0);
    let scenes: Vec<(usize, String)> = app.engine.session().scenes.iter().enumerate().map(|(i, s)| (i, s.name.clone())).collect();
    if scenes.is_empty() {
        dim(ui, "No scenes yet. A scene stores every channel's levels, mutes, inserts and sends.");
    }
    let mut parts = vec!["levels", "mutes", "inserts", "sends"];
    if app.ui.scene_routing {
        parts.push("routing");
    }
    for (i, name) in scenes {
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(&name).min_size(egui::vec2(130.0, 0.0))).on_hover_text("Recall this scene").clicked() {
                if app.run("mix.scene_recall", json!({"index": i, "parts": parts})).is_ok() {
                    app.ui.status = format!("Recalled scene “{name}”");
                }
                ui.close();
            }
            if ui.small_button("Update").on_hover_text("Store the console as it is now into this scene").clicked() {
                if app.run("mix.scene_store", json!({"index": i})).is_ok() {
                    app.ui.status = format!("Updated scene “{name}”");
                }
                ui.close();
            }
            let new_name = app.ui.scene_name.trim().to_string();
            let rename = ui.add_enabled(!new_name.is_empty(), egui::Button::new("Rename").small());
            if rename.on_hover_text("Rename this scene to the name typed below").on_disabled_hover_text("Type a name below first").clicked() {
                if app.run("mix.scene_rename", json!({"index": i, "name": new_name})).is_ok() {
                    app.ui.status = format!("Renamed scene “{name}” to “{new_name}”");
                    app.ui.scene_name.clear();
                }
                ui.close();
            }
            if ui.small_button("Delete").on_hover_text("Delete this scene (Undo brings it back)").clicked() {
                let _ = app.run("mix.scene_delete", json!({"index": i}));
                ui.close();
            }
        });
    }
    ui.separator();
    ui.horizontal(|ui| {
        let edit = ui.add(egui::TextEdit::singleline(&mut app.ui.scene_name).hint_text("New scene name").desired_width(150.0));
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Store").on_hover_text("Store the console as a new scene").clicked() || enter {
            let name = app.ui.scene_name.trim().to_string();
            if let Ok(r) = app.run("mix.scene_store", json!({"name": name})) {
                app.ui.status = format!("Stored scene “{}”", r["name"].as_str().unwrap_or_default());
                app.ui.scene_name.clear();
            }
            ui.close();
        }
    });
    ui.checkbox(&mut app.ui.scene_routing, "Recall also changes inputs and outputs");
}
