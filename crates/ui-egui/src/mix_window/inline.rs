//! Inline insert controls (a used insert's parameters as compact bars under its slot) and the
//! Channel Editor (one channel's routing, its whole insert chain with every control, and sends).
use super::expand::{heading, io_controls, send_rows};
use super::{Align2, Rect, Sense, SoundApp, Stroke, Tokens, Track, TrackId, Ui, insert_slot, json, plugin_info, pos2, regular, shade, vec2};
use serde_json::Value;
use soundcraft_dsp::{ParamInfo, Unit};
use soundcraft_model::{INSERT_SLOTS, Insert};

/// Height of one parameter bar.
const BAR_H: f32 = 15.0;
/// The Expand panel shows at most this many of a plugin's controls; the Channel Editor has the rest.
const INLINE_MAX: usize = 12;
/// The Channel Editor shows at most this many per plugin; a hosted plugin's own window has all.
const EDITOR_MAX: usize = 48;
/// Width of one control in the Channel Editor.
const EDITOR_BAR_W: f32 = 172.0;

/// Are `slot`'s controls shown under it in `id`'s Expand panel?
pub(super) fn inline_open(app: &SoundApp, id: TrackId, slot: usize) -> bool {
    app.ui.inline_inserts.contains(&(id, slot))
}

/// The small arrow beside a used insert that shows or hides its controls under it.
pub(super) fn inline_toggle(app: &mut SoundApp, ui: &mut Ui, r: Rect, track: &Track, slot: usize) {
    let t = Tokens::current();
    let open = inline_open(app, track.id, slot);
    let resp = ui.interact(r, ui.scope_id().with(("inline-toggle", track.id.0, slot)), Sense::click());
    let letter = char::from(b'A'.saturating_add(slot as u8));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Controls for insert {letter} on {}", track.name)));
    let color = if resp.hovered() {
        t.text
    } else if open {
        t.accent
    } else {
        t.text_dim
    };
    let c = r.center();
    let s = 3.5;
    let pts = if open {
        vec![pos2(c.x - s, c.y - s * 0.6), pos2(c.x + s, c.y - s * 0.6), pos2(c.x, c.y + s * 0.8)]
    } else {
        vec![pos2(c.x - s * 0.6, c.y - s), pos2(c.x - s * 0.6, c.y + s), pos2(c.x + s * 0.8, c.y)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    let resp = resp.on_hover_text(if open { "Hide this insert's controls" } else { "Show this insert's controls here" });
    if resp.clicked() {
        if open {
            app.ui.inline_inserts.retain(|x| *x != (track.id, slot));
        } else {
            app.ui.inline_inserts.push((track.id, slot));
        }
    }
}

/// A used insert's first controls as bars, `w` wide, with a way to the rest.
pub(super) fn inline_controls(app: &mut SoundApp, ui: &mut Ui, track: &Track, slot: usize, w: f32) {
    let Some(ins) = track.mixer.inserts.get(slot).cloned().flatten() else { return };
    let Some(info) = plugin_info(&ins.plugin) else {
        ui.label(egui::RichText::new("No controls for this plugin").small().color(Tokens::current().text_dim));
        return;
    };
    for p in info.params.iter().take(INLINE_MAX) {
        param_bar(app, ui, track, slot, &ins, info.short_name, p, w);
    }
    let more = info.params.len().saturating_sub(INLINE_MAX);
    if more > 0 && ui.small_button(format!("{more} more in the Channel Editor")).clicked() {
        open_channel_editor(app, track.id);
    }
}

/// What a parameter's value reads as: the hosted plugin's own text when it has one.
fn value_text(app: &SoundApp, id: TrackId, slot: usize, p: &ParamInfo, v: f32) -> String {
    app.player.as_ref().and_then(|pl| pl.editor_value_text(id, slot, p.id, v)).unwrap_or_else(|| p.format(v))
}

/// The value a bar moves to: a toggle or choice steps on click (wrapping), a continuous control
/// follows a sideways drag across its width (a tenth as far with `fine`), and double-click resets.
pub(super) fn bar_value(p: &ParamInfo, v: f32, clicked: bool, double_clicked: bool, drag_dx: f32, width: f32, fine: bool) -> Option<f32> {
    if matches!(p.unit, Unit::Toggle | Unit::Choice) {
        return clicked.then(|| if v + 1.0 > p.max + 0.5 { p.min } else { p.clamp(v + 1.0) });
    }
    if double_clicked {
        return Some(p.default);
    }
    if drag_dx == 0.0 || !drag_dx.is_finite() {
        return None;
    }
    let k = if fine { 0.1 } else { 1.0 };
    Some(p.from_normalized(p.to_normalized(v) + drag_dx / width.max(1.0) * k))
}

/// One plugin parameter as a bar: its name at the left, its value at the right, filled as far as
/// the control's travel.
#[allow(clippy::too_many_arguments)]
pub(super) fn param_bar(app: &mut SoundApp, ui: &mut Ui, track: &Track, slot: usize, ins: &Insert, plugin: &str, p: &ParamInfo, w: f32) {
    let t = Tokens::current();
    let id = track.id;
    let v = ins.params.get(p.id).copied().unwrap_or(p.default);
    let stepped = matches!(p.unit, Unit::Toggle | Unit::Choice);
    let (r, resp) = ui.allocate_exact_size(vec2(w, BAR_H), if stepped { Sense::click() } else { Sense::click_and_drag() });
    let shown = value_text(app, id, slot, p, v);
    resp.widget_info(|| egui::WidgetInfo::slider(true, f64::from(v), format!("{plugin} {} on {}", p.name, track.name)));
    let dx = if resp.dragged() { resp.drag_delta().x } else { 0.0 };
    let fine = ui.input(|i| i.modifiers.shift);
    if let Some(x) = bar_value(p, v, resp.clicked(), resp.double_clicked(), dx, r.width(), fine).filter(|x| (x - v).abs() > f32::EPSILON) {
        let key = format!("param:{}:{slot}:{}", id.0, p.id);
        let _ = app.engine.execute_merged("mix.insert_param", &json!({"track": id.0, "slot": slot, "param": p.id, "value": x}), &key);
        if let Some(pl) = &app.player {
            // Keep the plugin's own editor in step.
            pl.editor_set_param(id, slot, p.id, x);
        }
        if !resp.dragged() {
            app.engine.end_merge();
        }
    }
    if resp.drag_stopped() {
        app.engine.end_merge();
    }
    let painter = ui.painter();
    let hot = resp.hovered() || resp.dragged();
    painter.rect_filled(r, 2.0, if hot { shade(t.slot_empty, 0.08) } else { t.slot_empty });
    let frac = match p.unit {
        Unit::Choice => 0.0,
        Unit::Toggle => {
            if v >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        _ => p.to_normalized(v),
    };
    if frac > 0.0 {
        let fill = t.accent.gamma_multiply(if ins.bypass { 0.18 } else { 0.42 });
        painter.rect_filled(r.with_max_x(r.min.x + r.width() * frac), 2.0, fill);
    }
    let color = if ins.bypass { t.text_dim } else { t.text };
    let g = painter.layout_no_wrap(shown.clone(), regular(9.5), color);
    let vx = r.max.x - 4.0 - g.size().x;
    painter.with_clip_rect(r.with_max_x(vx - 3.0)).text(pos2(r.min.x + 4.0, r.center().y), Align2::LEFT_CENTER, p.name, regular(9.5), color);
    painter.with_clip_rect(r).galley(pos2(vx, r.center().y - g.size().y / 2.0), g, color);
    let how = if stepped { "click for the next setting" } else { "drag sideways, Shift for fine, double-click for the default" };
    resp.on_hover_text(format!("{}: {shown} · {how}", p.name));
}

// ---- Channel Editor -----------------------------------------------------------------------

/// Open the Channel Editor for `id` (one window per channel).
pub(crate) fn open_channel_editor(app: &mut SoundApp, id: TrackId) {
    if !app.ui.channel_editors.contains(&id) {
        app.ui.channel_editors.push(id);
    }
}

/// UI command `window.channel_editor {tracks?}`: opens the Channel Editor for the named channels
/// (ids or names), or the selected ones.
pub(crate) fn channel_editor_command(app: &mut SoundApp, p: &Value) -> Result<Value, String> {
    let s = app.engine.session();
    let ids: Vec<TrackId> = match p.get("tracks").and_then(Value::as_array) {
        Some(list) => {
            let mut ids = Vec::new();
            for v in list {
                let found = match v {
                    Value::Number(n) => n.as_u64().and_then(|n| s.track(TrackId(n))),
                    Value::String(name) => s.tracks.iter().find(|t| t.name == *name),
                    _ => None,
                };
                ids.push(found.map(|t| t.id).ok_or_else(|| format!("window.channel_editor: no channel {v}"))?);
            }
            ids
        }
        None => s.edit.selected_tracks.clone(),
    };
    if ids.is_empty() {
        return Err("window.channel_editor: select a channel first".into());
    }
    for id in &ids {
        open_channel_editor(app, *id);
    }
    Ok(json!({"tracks": ids.iter().map(|t| t.0).collect::<Vec<_>>()}))
}

/// The open Channel Editor windows.
pub(crate) fn channel_editors(app: &mut SoundApp, ctx: &egui::Context) {
    let mut keep = Vec::new();
    for (n, id) in app.ui.channel_editors.clone().into_iter().enumerate() {
        let Some(track) = app.engine.session().track(id).cloned() else { continue };
        let mut open = true;
        let offset = 24.0 * n as f32;
        egui::Window::new(format!("{} · Channel Editor", track.name))
            .id(egui::Id::unique(("channel-editor", id.0)))
            .open(&mut open)
            .default_width(760.0)
            .default_pos(egui::pos2(120.0 + offset, 90.0 + offset))
            .show(ctx, |ui| editor_body(app, ui, &track));
        if open {
            keep.push(id);
        }
    }
    app.ui.channel_editors = keep;
}

fn editor_body(app: &mut SoundApp, ui: &mut Ui, track: &Track) {
    let t = Tokens::current();
    let id = track.id;
    ui.horizontal(|ui| {
        if ui.selectable_label(track.mixer.mute, "Mute").clicked() {
            let _ = app.run("mix.mute", json!({"tracks": [id.0], "value": !track.mixer.mute}));
        }
        if ui.selectable_label(track.mixer.solo, "Solo").clicked() {
            let _ = app.run("mix.solo", json!({"tracks": [id.0], "value": !track.mixer.solo}));
        }
        ui.separator();
        let label = ui.label("Volume");
        let mut db = track.mixer.volume_db;
        let resp = ui
            .add(
                egui::DragValue::new(&mut db)
                    .range(-144.0..=12.0)
                    .speed(0.1)
                    .max_decimals(1)
                    .custom_formatter(|v, _| if v <= -143.9 { "-inf dB".into() } else { format!("{v:.1} dB") }),
            )
            .labelled_by(label.id);
        if resp.changed() {
            let _ = app.engine.execute_merged("mix.volume", &json!({"tracks": [id.0], "db": db}), &format!("fader:{}", id.0));
        }
        if resp.drag_stopped() || resp.lost_focus() {
            app.engine.end_merge();
        }
        if let Some(&pan) = track.mixer.pan.first() {
            let label = ui.label("Pan");
            let mut x = pan;
            let resp = ui
                .add(egui::DragValue::new(&mut x).range(-1.0..=1.0).speed(0.01).custom_formatter(|v, _| {
                    let p = (v * 100.0).round();
                    if p < 0.0 {
                        format!("L{}", -p)
                    } else if p > 0.0 {
                        format!("R{p}")
                    } else {
                        "C".into()
                    }
                }))
                .labelled_by(label.id);
            if resp.changed() {
                let _ = app.engine.execute_merged("mix.pan", &json!({"tracks": [id.0], "pan": x, "index": 0}), &format!("pan:{}", id.0));
            }
            if resp.drag_stopped() || resp.lost_focus() {
                app.engine.end_merge();
            }
        }
        ui.separator();
        ui.menu_button("FX Chain v", |ui| crate::panels::chain_menu(app, ui, id))
            .response
            .on_hover_text("Save this channel's inserts as a chain, or load a saved chain");
    });
    ui.separator();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(190.0);
            ui.spacing_mut().item_spacing.y = 3.0;
            heading(ui, "INPUT / OUTPUT");
            io_controls(app, ui, track, 190.0);
            heading(ui, "SENDS");
            send_rows(app, ui, track, 190.0);
        });
        ui.separator();
        ui.vertical(|ui| {
            heading(ui, "INSERTS");
            egui::ScrollArea::vertical()
                .id_salt(("channel-editor-inserts", id.0))
                .max_height(620.0)
                .min_scrolled_height(480.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let mut offered_empty = false;
                    for slot in 0..INSERT_SLOTS {
                        match track.mixer.inserts.get(slot).cloned().flatten() {
                            Some(ins) => editor_insert(app, ui, track, slot, &ins),
                            None if !offered_empty => {
                                offered_empty = true;
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(150.0, 15.0), Sense::hover());
                                    insert_slot(app, ui, track, slot, r);
                                    ui.label(egui::RichText::new("Click to add an effect").small().color(t.text_dim));
                                });
                            }
                            None => {}
                        }
                    }
                });
        });
    });
}

/// One used insert in the Channel Editor: its slot, an on/off switch, and its controls.
fn editor_insert(app: &mut SoundApp, ui: &mut Ui, track: &Track, slot: usize, ins: &Insert) {
    let t = Tokens::current();
    let id = track.id;
    let info = plugin_info(&ins.plugin);
    egui::Frame::new().fill(t.strip_section).stroke(Stroke::new(1.0, t.border)).corner_radius(3.0).inner_margin(egui::Margin::same(4)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(150.0, 15.0), Sense::hover());
                insert_slot(app, ui, track, slot, r);
                let label = if ins.bypass { "Off" } else { "On" };
                if ui.selectable_label(!ins.bypass, label).on_hover_text("Turn this insert on or off (bypass)").clicked() {
                    let _ = app.run("mix.insert_bypass", json!({"track": id.0, "slot": slot}));
                }
                if let Some(info) = info {
                    ui.label(egui::RichText::new(info.name).color(t.text_dim));
                }
            });
            let Some(info) = info else { return };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                for p in info.params.iter().take(EDITOR_MAX) {
                    param_bar(app, ui, track, slot, ins, info.short_name, p, EDITOR_BAR_W);
                }
            });
            let more = info.params.len().saturating_sub(EDITOR_MAX);
            if more > 0 && ui.small_button(format!("{more} more: open the plugin")).clicked() {
                crate::panels::open_chain(app, id, slot);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::bar_value;
    use soundcraft_dsp::{ParamInfo, Taper, Unit};

    const GAIN: ParamInfo =
        ParamInfo { id: "gain", name: "Gain", min: -24.0, max: 24.0, default: 0.0, unit: Unit::Db, taper: Taper::Linear, choices: &[] };
    const MODE: ParamInfo =
        ParamInfo { id: "mode", name: "Mode", min: 0.0, max: 2.0, default: 0.0, unit: Unit::Choice, taper: Taper::Linear, choices: &["A", "B", "C"] };

    #[test]
    fn a_bar_follows_the_drag_across_its_width_and_shift_is_fine() {
        // Half the bar's width is half the range: -24..24 is 48 dB, so 24 dB up.
        assert_eq!(bar_value(&GAIN, 0.0, false, false, 50.0, 100.0, false), Some(24.0));
        let fine = bar_value(&GAIN, 0.0, false, false, 50.0, 100.0, true).unwrap_or(f32::NAN);
        assert!((fine - 2.4).abs() < 1e-3, "a tenth as far with Shift, got {fine}");
        assert_eq!(bar_value(&GAIN, 20.0, false, false, 500.0, 100.0, false), Some(24.0), "clamped at the top");
        assert_eq!(bar_value(&GAIN, 6.0, false, false, 0.0, 100.0, false), None, "no movement, no change");
        assert_eq!(bar_value(&GAIN, 6.0, false, false, f32::NAN, 100.0, false), None);
    }

    #[test]
    fn double_click_resets_and_a_choice_steps_and_wraps() {
        assert_eq!(bar_value(&GAIN, 9.0, true, true, 0.0, 100.0, false), Some(0.0));
        assert_eq!(bar_value(&MODE, 0.0, true, false, 0.0, 100.0, false), Some(1.0));
        assert_eq!(bar_value(&MODE, 2.0, true, false, 0.0, 100.0, false), Some(0.0));
        assert_eq!(bar_value(&MODE, 1.0, false, false, 30.0, 100.0, false), None, "a choice ignores drags");
    }
}
