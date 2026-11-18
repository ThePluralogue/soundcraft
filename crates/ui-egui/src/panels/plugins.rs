use super::{Color32, Sense, SoundApp, Stroke, Tokens, Ui, json, mono, pos2, vec2};

/// Draw plugin windows for instrument and insert slots that still exist, removing closed ones from the list.
pub(super) fn plugin_windows(app: &mut SoundApp, ctx: &egui::Context) {
    let wins = app.ui.plugin_windows.clone();
    let mut keep = Vec::new();
    for (tid, slot) in wins {
        // The instrument slot (an instrument track's instrument) or an insert slot.
        let instrument = slot == soundcraft_mix::INSTRUMENT_SLOT;
        let Some((tname, ins)) = app.engine.session().track(tid).and_then(|t| {
            let ins = if instrument { t.instrument.clone() } else { t.mixer.inserts.get(slot).cloned().flatten() };
            ins.map(|i| (t.name.clone(), i))
        }) else {
            continue;
        };
        let Some(info) = crate::mix_window::plugin_info(&ins.plugin) else { continue };
        let place = if instrument { "instrument".to_string() } else { char::from(b'a'.saturating_add(slot as u8)).to_string() };
        let mut open = true;
        let shown = egui::Window::new(format!("{tname} · {place} · {}", info.name))
            .id(egui::Id::unique(("plugin", tid.0, slot)))
            .open(&mut open)
            .default_width(340.0)
            .default_pos(egui::pos2(ctx.content_rect().width() - 380.0, 120.0))
            .show(ctx, |ui| {
                plugin_body(app, ui, tid, slot, &ins, info, instrument);
            });
        if let Some(w) = shown
            && instrument
            && ins.plugin == crate::sampler_window::SAMPLER
        {
            crate::sampler_window::drop_target(app, ctx, w.response.rect, tid);
        }
        if open {
            keep.push((tid, slot));
        }
    }
    app.ui.plugin_windows = keep;
}

/// Open the tabbed Inserts window for `track` showing `slot`'s tab (one window per track).
pub(crate) fn open_chain(app: &mut SoundApp, track: soundcraft_model::TrackId, slot: usize) {
    match app.ui.chain_windows.iter_mut().find(|(t, _)| *t == track) {
        Some(w) => w.1 = slot,
        None => app.ui.chain_windows.push((track, slot)),
    }
}

/// The tabbed Inserts windows: a tab per used insert slot, each with a power button that bypasses
/// it, and the chosen insert's controls below.
pub(crate) fn chain_windows(app: &mut SoundApp, ctx: &egui::Context) {
    let t = Tokens::current();
    let wins = app.ui.chain_windows.clone();
    let mut keep = Vec::new();
    for (tid, active) in wins {
        let Some((tname, used)) = app.engine.session().track(tid).map(|tr| {
            let used: Vec<(usize, soundcraft_model::Insert)> =
                tr.mixer.inserts.iter().enumerate().filter_map(|(i, s)| s.clone().map(|ins| (i, ins))).collect();
            (tr.name.clone(), used)
        }) else {
            continue;
        };
        if used.is_empty() {
            continue;
        }
        let active = if used.iter().any(|(i, _)| *i == active) { active } else { used.first().map_or(0, |(i, _)| *i) };
        let mut chosen = active;
        let mut open = true;
        egui::Window::new(format!("{tname} · Inserts"))
            .id(egui::Id::unique(("plugin-chain", tid.0)))
            .open(&mut open)
            .default_width(380.0)
            .default_pos(egui::pos2(ctx.content_rect().width() - 420.0, 110.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.menu_button("FX Chain v", |ui| crate::panels::chain_menu(app, ui, tid))
                        .response
                        .on_hover_text("Save this channel's inserts as a chain, or load a saved chain");
                    ui.label(egui::RichText::new(format!("{} insert{}", used.len(), if used.len() == 1 { "" } else { "s" })).color(t.text_dim));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (slot, ins) in &used {
                        let name = crate::mix_window::plugin_info(&ins.plugin).map_or(ins.plugin.as_str(), |p| p.short_name);
                        let selected = *slot == active;
                        let frame = egui::Frame::new()
                            .fill(if selected { t.panel_bg2 } else { t.toolbar_group })
                            .stroke(Stroke::new(1.0, if selected { t.accent } else { t.border }))
                            .corner_radius(3.0)
                            .inner_margin(egui::Margin::symmetric(4, 2));
                        frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                if power_button(ui, !ins.bypass, ("pwr", tid.0, *slot)).clicked() {
                                    let _ = app.run("mix.insert_bypass", json!({"track": tid.0, "slot": slot}));
                                }
                                let color = if ins.bypass {
                                    t.text_dim
                                } else if selected {
                                    t.text
                                } else {
                                    t.header_text
                                };
                                let label = ui.add(egui::Label::new(egui::RichText::new(name).color(color)).sense(Sense::click()));
                                if label.clicked() {
                                    chosen = *slot;
                                }
                            });
                        });
                    }
                });
                ui.separator();
                if let Some((slot, ins)) = used.iter().find(|(i, _)| *i == active)
                    && let Some(info) = crate::mix_window::plugin_info(&ins.plugin)
                {
                    ui.label(egui::RichText::new(format!("{} · slot {}", info.name, char::from(b'a'.saturating_add(*slot as u8)))).color(t.text_dim));
                    plugin_body(app, ui, tid, *slot, ins, info, false);
                }
            });
        if open {
            keep.push((tid, chosen));
        }
    }
    app.ui.chain_windows = keep;
}

/// A small power symbol: lit when the insert is on, dim when bypassed. Click toggles.
fn power_button(ui: &mut Ui, on: bool, id: impl std::hash::Hash + std::fmt::Debug) -> egui::Response {
    let t = Tokens::current();
    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let resp =
        ui.interact(r, ui.scope_id().with(id), Sense::click()).on_hover_text(if on { "On: click to bypass" } else { "Bypassed: click to turn on" });
    let color = if on { t.accent } else { t.text_dim };
    let color = if resp.hovered() { color.gamma_multiply(1.3) } else { color };
    let c = r.center();
    let rad = 5.0;
    let stroke = Stroke::new(1.6, color);
    // An open ring with a gap at the top, and the stroke through it.
    let pts: Vec<egui::Pos2> = (0..=24)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + 0.6 + (i as f32 / 24.0) * (std::f32::consts::TAU - 1.2);
            pos2(c.x + rad * a.cos(), c.y + rad * a.sin())
        })
        .collect();
    ui.painter().add(egui::Shape::line(pts, stroke));
    ui.painter().line_segment([pos2(c.x, c.y - rad - 1.0), pos2(c.x, c.y)], stroke);
    resp
}

/// One plugin's controls: bypass, presets and editor buttons, then its parameters. Shared by the
/// single-plugin windows and the tabbed Inserts window.
fn plugin_body(
    app: &mut SoundApp,
    ui: &mut Ui,
    tid: soundcraft_model::TrackId,
    slot: usize,
    ins: &soundcraft_model::Insert,
    info: &'static soundcraft_dsp::PluginInfo,
    instrument: bool,
) {
    ui.horizontal(|ui| {
        // Bypass and presets act on insert slots.
        if !instrument {
            ui.label(egui::RichText::new(&ins.preset).color(Tokens::current().text_dim));
            let label = if ins.bypass { "BYPASSED" } else { "Bypass" };
            if ui.button(label).clicked() {
                let _ = app.run("mix.insert_bypass", json!({"track": tid.0, "slot": slot}));
            }
            ui.menu_button("Presets v", |ui| preset_menu(app, ui, tid, slot, info, ins));
        }
        if soundcraft_mix::is_third_party(&ins.plugin) {
            let open = app.player.as_ref().is_some_and(|p| p.editor_open(tid, slot));
            let available = app.player.as_ref().is_some_and(|p| p.has_editor(tid, slot));
            let label = if open { "Close Plugin Editor" } else { "Open Plugin Editor" };
            let resp = ui.add_enabled(available, egui::Button::new(label));
            let resp = if available { resp } else { resp.on_disabled_hover_text("This plugin has no editor of its own, or it is not loaded") };
            if resp.clicked()
                && let Some(p) = &app.player
            {
                if open {
                    p.close_editor(tid, slot);
                } else if let Err(e) = p.open_editor(tid, slot) {
                    app.ui.status = format!("{}: {e}", info.name);
                }
            }
        }
    });
    if instrument && soundcraft_mix::is_third_party(&ins.plugin) {
        // A hosted instrument keeps its sounds and settings in its own editor.
        ui.label(egui::RichText::new("Load and edit sounds in the plugin's own editor.").color(Tokens::current().text_dim));
        return;
    }
    if instrument && ins.plugin == crate::sampler_window::SAMPLER {
        crate::sampler_window::sample_strip(app, ui, tid, ins);
        ui.add_space(4.0);
    }
    if info.id == "eq_7band" || info.id == "eq_1band" {
        eq_curve(ui, info.id, ins);
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new(("pg", tid.0, slot)).num_columns(3).show(ui, |ui| {
            for p in info.params {
                let v = ins.params.get(p.id).copied().unwrap_or(p.default);
                let label = ui.label(p.name);
                let mut x = v;
                let resp = if !p.choices.is_empty() {
                    let mut idx = x.round().max(0.0) as usize;
                    let r = craft_ui::ComboBox::from_id_salt(("pc", tid.0, slot, p.id))
                        .selected_text(p.choices.get(idx).copied().unwrap_or("?"))
                        .show_ui(ui, |ui| {
                            for (i, c) in p.choices.iter().enumerate() {
                                ui.selectable_value(&mut idx, i, *c);
                            }
                        });
                    x = idx as f32;
                    r.response
                } else {
                    let mut sl = egui::Slider::new(&mut x, p.min..=p.max).show_value(false);
                    if instrument && crate::sampler_window::whole_number(&ins.plugin, p.id) {
                        sl = sl.integer();
                    }
                    if p.taper == soundcraft_dsp::Taper::Log && p.min > 0.0 {
                        sl = sl.logarithmic(true);
                    }
                    ui.add(sl)
                };
                // The grid cell's painted label is not an accesskit name; tie the
                // combo box or slider to it so screen readers and label queries find it.
                let resp = resp.labelled_by(label.id);
                if (x - v).abs() > f32::EPSILON && instrument {
                    let _ = app.engine.execute_merged(
                        "mix.instrument_param",
                        &json!({"track": tid.0, "param": p.id, "value": x}),
                        &format!("instrument_param:{}:{}", tid.0, p.id),
                    );
                } else if (x - v).abs() > f32::EPSILON {
                    let _ = app.engine.execute_merged(
                        "mix.insert_param",
                        &json!({"track": tid.0, "slot": slot, "param": p.id, "value": x}),
                        &format!("param:{}:{slot}:{}", tid.0, p.id),
                    );
                    if let Some(pl) = &app.player {
                        // Keep the plugin's own editor in step.
                        pl.editor_set_param(tid, slot, p.id, x);
                    }
                }
                let _ = resp;
                let shown = if instrument {
                    crate::sampler_window::format_param(&ins.plugin, p, x)
                } else {
                    app.player.as_ref().and_then(|pl| pl.editor_value_text(tid, slot, p.id, x)).unwrap_or_else(|| p.format(x))
                };
                ui.label(egui::RichText::new(shown).font(mono(11.0)));
                ui.end_row();
            }
        });
    });
}

fn eq_curve(ui: &mut Ui, id: &str, ins: &soundcraft_model::Insert) {
    let t = Tokens::current();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().max(300.0), 110.0), Sense::hover());
    ui.painter().rect_filled(r, 4.0, Color32::from_rgb(14, 18, 22));
    let n = 160;
    let freqs: Vec<f32> = (0..n).map(|i| 20.0 * 1000f32.powf(i as f32 / (n - 1) as f32)).collect();
    let params: Vec<(&str, f32)> = ins.params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let resp = if id == "eq_7band" {
        soundcraft_dsp::eq7_response(&params, &freqs, 48_000.0)
    } else {
        soundcraft_dsp::eq1_response(&params, &freqs, 48_000.0)
    };
    for db in [-12.0f32, 0.0, 12.0] {
        let y = r.center().y - db / 18.0 * r.height() * 0.5;
        ui.painter().line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(1.0, Color32::from_rgb(40, 50, 60)));
    }
    let pts: Vec<egui::Pos2> = resp
        .iter()
        .enumerate()
        .map(|(i, db)| pos2(r.min.x + r.width() * i as f32 / (n - 1) as f32, r.center().y - db.clamp(-18.0, 18.0) / 18.0 * r.height() * 0.5))
        .collect();
    ui.painter().add(egui::Shape::line(pts, Stroke::new(2.0, t.counter_text)));
}

/// Folder for a plugin's user presets.
fn preset_dir(app: &SoundApp, plugin: &str) -> Option<std::path::PathBuf> {
    app.preset_dir.as_ref().map(|d| d.join(crate::preset_folder_name(plugin)))
}

/// Factory default plus user presets for a plugin insert; user presets need a desktop preset directory.
fn preset_menu(
    app: &mut SoundApp,
    ui: &mut Ui,
    tid: soundcraft_model::TrackId,
    slot: usize,
    info: &soundcraft_dsp::PluginInfo,
    ins: &soundcraft_model::Insert,
) {
    if ui.button("<factory default>").clicked() {
        let defaults: serde_json::Map<String, serde_json::Value> = info.params.iter().map(|p| (p.id.to_string(), json!(p.default))).collect();
        let _ = app.run("mix.insert_params", json!({"track": tid.0, "slot": slot, "params": defaults, "preset": "<factory default>"}));
        ui.close();
    }
    let Some(dir) = preset_dir(app, info.id) else {
        ui.label("User presets need the desktop app.");
        return;
    };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().filter_map(|e| e.path().file_stem().and_then(std::ffi::OsStr::to_str).map(str::to_string)).collect())
        .unwrap_or_default();
    names.sort();
    for n in &names {
        if ui.button(n).clicked() {
            if let Ok(text) = std::fs::read_to_string(dir.join(format!("{n}.json")))
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
            {
                // Old presets may carry parameters the plugin no longer exposes; drop them so recall succeeds.
                let params: serde_json::Map<String, serde_json::Value> = v
                    .get("params")
                    .and_then(serde_json::Value::as_object)
                    .into_iter()
                    .flatten()
                    .filter(|(k, val)| info.params.iter().any(|pi| pi.id == k.as_str()) && val.as_f64().is_some_and(f64::is_finite))
                    .map(|(k, val)| (k.clone(), val.clone()))
                    .collect();
                let _ = app.run("mix.insert_params", json!({"track": tid.0, "slot": slot, "params": params, "preset": n}));
            }
            ui.close();
        }
    }
    ui.separator();
    let key = egui::Id::unique(("preset_name", tid.0, slot));
    let mut name: String = ui.ctx().memory(|m| m.data.get_temp(key)).unwrap_or_else(|| format!("{} preset", info.name));
    ui.horizontal(|ui| {
        crate::widgets::named_text_field(ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0)), "Preset name");
        if ui.button("Save").clicked() {
            let clean = soundcraft_engine::io::sanitize_name(&name);
            let body = json!({"format": "soundcraft-plugin-preset", "plugin": info.id, "params": ins.params});
            let ok = std::fs::create_dir_all(&dir).is_ok() && std::fs::write(dir.join(format!("{clean}.json")), body.to_string()).is_ok();
            app.ui.status = if ok { format!("Saved preset {clean}") } else { "Could not save the preset".into() };
            if ok {
                let _ = app.run("mix.insert_params", json!({"track": tid.0, "slot": slot, "params": {}, "preset": clean}));
            }
        }
    });
    ui.ctx().memory_mut(|m| m.data.insert_temp(key, name));
}
