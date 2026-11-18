//! Console metering modes (Peak or Peak + RMS, pre- or post-fader) and dragging inserts between
//! slots and channels.
use super::{Rect, SoundApp, Stroke, Tokens, Track, TrackId, Ui, json, plugin_info};

/// The console bar's Meters menu.
pub(crate) fn meters_menu(app: &mut SoundApp, ui: &mut Ui) {
    ui.set_min_width(240.0);
    if ui.radio(!app.ui.meter_rms, "Peak").on_hover_text("The fastest level, with a 2 second peak hold").clicked() {
        app.ui.meter_rms = false;
        ui.close();
    }
    if ui.radio(app.ui.meter_rms, "Peak + RMS").on_hover_text("Adds the average (RMS) level as a bright inner bar: how loud it sounds").clicked() {
        app.ui.meter_rms = true;
        ui.close();
    }
    ui.separator();
    let mut pre = app.engine.session().edit.pre_fader_metering;
    let resp = ui.checkbox(&mut pre, "Pre-fader (before the fader and mute)");
    if resp.on_hover_text("Meters show the level coming in, whatever the fader is set to").changed() {
        let _ = app.run("options.pre_fader_metering", json!({"value": pre}));
    }
}

/// The label for the console bar's Meters button.
pub(crate) fn meters_label(app: &SoundApp) -> String {
    let mode = if app.ui.meter_rms { "Peak+RMS" } else { "Peak" };
    let pre = if app.engine.session().edit.pre_fader_metering { ", Pre" } else { "" };
    format!("Meters: {mode}{pre} v")
}

/// An insert being dragged: its channel and slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InsertDrag {
    pub track: TrackId,
    pub slot: usize,
}

/// Drag and drop on an insert slot. Dragging a used slot picks it up; dropping it on another slot
/// of the same channel moves it (swapping with what is there), on another channel copies it.
/// Alt while dropping copies within a channel too.
pub(crate) fn insert_drag_and_drop(app: &mut SoundApp, ui: &Ui, resp: &egui::Response, track: &Track, slot: usize, r: Rect) {
    let used = track.mixer.inserts.get(slot).is_some_and(Option::is_some);
    let me = InsertDrag { track: track.id, slot };
    // The slot keeps plain click sensing (a quick click opens the plugin). egui drops a click-only
    // widget's press once the pointer moves past the click distance, so the drag is recognised
    // from where the press began: on this slot, on this layer, with the button still held.
    if used && !egui::DragAndDrop::has_any_payload(ui.ctx()) {
        let (down, origin, now) = ui.input(|i| (i.pointer.primary_down(), i.pointer.press_origin(), i.pointer.interact_pos()));
        if let (true, Some(origin), Some(now)) = (down, origin, now)
            && r.contains(origin)
            && ui.ctx().layer_id_at(origin) == Some(ui.layer_id())
            && origin.distance(now) > 8.0
        {
            egui::DragAndDrop::set_payload(ui.ctx(), me);
        }
    }
    // While dragging, the plugin's name follows the pointer.
    if egui::DragAndDrop::payload::<InsertDrag>(ui.ctx()).is_some_and(|p| *p == me)
        && let Some(at) = ui.ctx().pointer_interact_pos()
    {
        let name =
            track.mixer.inserts.get(slot).cloned().flatten().map(|i| plugin_info(&i.plugin).map_or(i.plugin.clone(), |p| p.short_name.to_string()));
        if let Some(name) = name {
            let t = Tokens::current();
            let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("insert-drag"));
            let p = ui.ctx().layer_painter(layer);
            let copy = ui.input(|i| i.modifiers.alt);
            let text = if copy { format!("+ {name}") } else { name };
            let g = p.layout_no_wrap(text, egui::FontId::proportional(11.0), t.text);
            let box_r = Rect::from_min_size(at + egui::vec2(12.0, 6.0), g.size() + egui::vec2(10.0, 4.0));
            p.rect_filled(box_r, 3.0, t.strip_section);
            p.rect_stroke(box_r, 3.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            p.galley(box_r.min + egui::vec2(5.0, 2.0), g, t.text);
        }
    }
    if let Some(src) = resp.dnd_hover_payload::<InsertDrag>()
        && *src != me
    {
        ui.painter().rect_stroke(r.expand(1.0), 2.0, Stroke::new(2.0, Tokens::current().accent), egui::StrokeKind::Outside);
    }
    if let Some(src) = resp.dnd_release_payload::<InsertDrag>() {
        let src = *src;
        if src == me {
            return;
        }
        let copy = src.track != track.id || ui.input(|i| i.modifiers.alt);
        let result = if copy {
            app.run("mix.insert_copy", json!({"from": src.track.0, "from_slot": src.slot, "track": track.id.0, "slot": slot}))
        } else {
            app.run("mix.insert_move", json!({"track": track.id.0, "from": src.slot, "to": slot}))
        };
        if result.is_ok() {
            let what = if copy { "Copied" } else { "Moved" };
            app.ui.status = format!("{what} the insert to {} slot {}", track.name, (b'A' + slot as u8) as char);
        }
    }
}
