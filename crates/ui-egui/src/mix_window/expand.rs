//! A channel's Expand panel (its routing, trim, phase, all ten inserts and sends beside the
//! strip) and Fader Flip (the console's faders set the sends to one bus).
use super::{Rect, Route, Sense, SoundApp, Stroke, Tokens, Track, TrackId, Ui, json, pos2, vec2};
use super::{input_name, insert_slot, route_name, route_popup, send_slot};
use soundcraft_model::{BusId, INSERT_SLOTS, SEND_SLOTS, TrackKind};

/// Width of an expanded channel's extra panel.
pub(super) const EXPAND_W: f32 = 190.0;

/// Is `id`'s strip expanded?
pub(super) fn expanded(app: &SoundApp, id: TrackId) -> bool {
    app.ui.expanded_strips.contains(&id)
}

/// The small arrow that expands or collapses a strip. Points right when collapsed.
pub(super) fn expand_toggle(app: &mut SoundApp, ui: &mut Ui, r: Rect, track: &Track) {
    let t = Tokens::current();
    let open = expanded(app, track.id);
    let resp = ui.interact(r, ui.scope_id().with(("strip-expand", track.id.0)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Expand {}", track.name)));
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
        vec![pos2(c.x + s, c.y - s), pos2(c.x + s, c.y + s), pos2(c.x - s, c.y)]
    } else {
        vec![pos2(c.x - s, c.y - s), pos2(c.x - s, c.y + s), pos2(c.x + s, c.y)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    let resp = resp.on_hover_text(if open { "Collapse this channel" } else { "Expand: routing, all inserts and sends" });
    if resp.clicked() {
        if open {
            app.ui.expanded_strips.retain(|x| *x != track.id);
        } else {
            app.ui.expanded_strips.push(track.id);
        }
    }
}

/// The panel beside an expanded strip.
pub(super) fn expand_panel(app: &mut SoundApp, ui: &mut Ui, track: &Track, h: f32) {
    let t = Tokens::current();
    let id = track.id;
    let (r, _) = ui.allocate_exact_size(vec2(EXPAND_W, h), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.strip_section);
    ui.painter().vline(r.max.x - 0.5, r.y_range(), Stroke::new(1.0, t.border));
    let mut c =
        ui.new_child(egui::UiBuilder::new().id_salt(("expand", id.0)).max_rect(r.shrink(6.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt(("expand-scroll", id.0)).auto_shrink([false, false]).show(&mut c, |ui| {
        // Clear of the floating scroll bar, which would otherwise take clicks on the slots' arrows.
        let w = (ui.available_width() - ui.spacing().scroll.allocated_width() - 8.0).max(60.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&track.name).strong().color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Editor").on_hover_text("Open the Channel Editor: every insert's controls in one window").clicked() {
                    super::open_channel_editor(app, id);
                }
            });
        });
        heading(ui, "INPUT / OUTPUT");
        io_controls(app, ui, track, w);
        heading(ui, "INSERTS");
        for k in 0..INSERT_SLOTS {
            let used = track.mixer.inserts.get(k).is_some_and(Option::is_some);
            let (row, _) = ui.allocate_exact_size(vec2(w, 15.0), Sense::hover());
            let sr = if used { row.with_max_x(row.max.x - 14.0) } else { row };
            insert_slot(app, ui, track, k, sr);
            if used {
                super::inline_toggle(app, ui, Rect::from_center_size(pos2(row.max.x - 5.0, row.center().y), vec2(12.0, 12.0)), track, k);
                if super::inline_open(app, id, k) {
                    super::inline_controls(app, ui, track, k, w);
                    ui.add_space(3.0);
                }
            }
        }
        ui.menu_button("FX Chain v", |ui| crate::panels::chain_menu(app, ui, id));
        heading(ui, "SENDS");
        send_rows(app, ui, track, w);
    });
}

/// A small section heading in the Expand panel and the Channel Editor.
pub(super) fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).small().strong().color(Tokens::current().section_label));
}

/// A channel's input and output pickers, trim and phase invert, `w` wide.
pub(super) fn io_controls(app: &mut SoundApp, ui: &mut Ui, track: &Track, w: f32) {
    let id = track.id;
    let has_input = matches!(track.kind, TrackKind::Audio | TrackKind::Instrument | TrackKind::Midi);
    if has_input {
        let input = input_name(app, track);
        let resp = ui.add_sized(vec2(w, 18.0), egui::Button::new(format!("In: {input}"))).on_hover_text("Input");
        route_popup(app, &resp, id, true);
    }
    let output = route_name(app, &track.mixer.output);
    let resp = ui.add_sized(vec2(w, 18.0), egui::Button::new(format!("Out: {output}"))).on_hover_text("Output");
    route_popup(app, &resp, id, false);
    if has_input {
        ui.horizontal(|ui| {
            let label = ui.label("Trim");
            let mut db = track.mixer.trim_db;
            let resp = ui.add(egui::DragValue::new(&mut db).range(-24.0..=24.0).speed(0.1).suffix(" dB").max_decimals(1)).labelled_by(label.id);
            if resp.changed() {
                let _ = app.engine.execute_merged("mix.trim", &json!({"tracks": [id.0], "db": db}), &format!("trim:{}", id.0));
            }
            if resp.drag_stopped() || resp.lost_focus() {
                app.engine.end_merge();
            }
        });
    }
    let mut phase = track.mixer.phase_invert;
    if ui.checkbox(&mut phase, "Phase invert").changed() {
        let _ = app.run("mix.phase_invert", json!({"tracks": [id.0], "value": phase}));
    }
}

/// A channel's send slots, each with its level beside it once used, `w` wide.
pub(super) fn send_rows(app: &mut SoundApp, ui: &mut Ui, track: &Track, w: f32) {
    let id = track.id;
    for k in 0..SEND_SLOTS {
        let send = track.mixer.sends.get(k).cloned().flatten();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let lw = if send.is_some() { 58.0 } else { 0.0 };
            let (sr, _) = ui.allocate_exact_size(vec2(w - lw - 3.0, 15.0), Sense::hover());
            send_slot(app, ui, track, k, sr);
            if let Some(s) = send {
                let mut db = s.level_db;
                let resp = ui.add_sized(
                    vec2(lw, 15.0),
                    egui::DragValue::new(&mut db)
                        .range(-144.0..=12.0)
                        .speed(0.2)
                        .max_decimals(1)
                        .custom_formatter(|v, _| if v <= -143.9 { "-inf".into() } else { format!("{v:.1}") }),
                );
                let which = char::from(b'A'.saturating_add(k as u8));
                resp.widget_info(|| egui::WidgetInfo::slider(true, f64::from(db), format!("Send {which} level value for {}", track.name)));
                if resp.changed() {
                    let _ = app.engine.execute_merged("mix.send_level", &json!({"track": id.0, "slot": k, "db": db}), &format!("send:{}:{k}", id.0));
                }
                if resp.drag_stopped() || resp.lost_focus() {
                    app.engine.end_merge();
                }
            }
        });
    }
}

// ---- Fader Flip ---------------------------------------------------------------------------

/// What a strip's fader controls while the console is flipped to a bus.
pub(super) enum Flip {
    /// Not flipped (or a track that keeps its own fader, like the master).
    Off,
    /// The send slot to the flip bus, and the bus's name.
    Send(usize, String),
    /// The track has no send to the flip bus yet.
    NoSend(String),
}

pub(super) fn flip_of(app: &SoundApp, track: &Track) -> Flip {
    let Some(bus) = app.ui.fader_flip else { return Flip::Off };
    if matches!(track.kind, TrackKind::Master | TrackKind::Vca | TrackKind::Folder) {
        return Flip::Off;
    }
    let Some(b) = app.engine.session().bus(BusId(bus)) else { return Flip::Off };
    let target = Route::Bus(b.id);
    // A track that takes this bus as its input (the cue's own aux) has no send to it.
    if track.mixer.input == target {
        return Flip::Off;
    }
    match track.mixer.sends.iter().position(|s| s.as_ref().is_some_and(|s| s.target == target)) {
        Some(slot) => Flip::Send(slot, b.name.clone()),
        None => Flip::NoSend(b.name.clone()),
    }
}

/// The console bar's Flip menu: faders control the main mix, or the sends to one bus.
pub(crate) fn flip_menu(app: &mut SoundApp, ui: &mut Ui) {
    ui.set_min_width(220.0);
    let busses: Vec<(u64, String)> = app.engine.session().busses.iter().map(|b| (b.id.0, b.name.clone())).collect();
    if ui.selectable_label(app.ui.fader_flip.is_none(), "Main mix faders").clicked() {
        app.ui.fader_flip = None;
        ui.close();
    }
    ui.separator();
    if busses.is_empty() {
        ui.label(egui::RichText::new("No busses yet").small().color(Tokens::current().text_dim));
    }
    for (bid, name) in busses {
        if ui.selectable_label(app.ui.fader_flip == Some(bid), format!("Sends to {name}")).clicked() {
            app.ui.fader_flip = Some(bid);
            ui.close();
        }
    }
    ui.separator();
    if ui.button("New Cue Mix and Flip to It").on_hover_text("A headphone mix on Out 3-4, starting as a copy of the main mix").clicked() {
        super::cue::new_cue_and_flip(app);
        ui.close();
    }
}

/// The label for the console bar's Flip button.
pub(crate) fn flip_label(app: &SoundApp) -> String {
    match app.ui.fader_flip.and_then(|b| app.engine.session().bus(BusId(b))) {
        Some(b) => format!("Flip: {} v", b.name),
        None => "Flip v".into(),
    }
}
