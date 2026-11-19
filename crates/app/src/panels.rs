//! The docks' lists (`dock.rs`): the parts to add, a heatmap's drops, the changes, the DRC
//! violations and the BOM checklist.

use crate::dock::Tab;
use crate::edit::NO_LIBRARIES;
use crate::sheet_edit::NO_SYMBOLS;
use crate::tr::{n, t, tr, trf};
use crate::{ADD_FIELD, App, around, color32, focus, theme, um_box};
use cuprita_core::diff::ChangeKind;
use cuprita_core::library::{self, LibPart};
use cuprita_render::heat_color;
use eframe::egui;

/// How many matches the add part list shows at most.
const SHOWN: usize = 200;

/// What a part number's row at Digi-Key does.
const DIGIKEY: &str = t!("Digi-Key's part of this number: KiCad's symbol for it, else a resistor's, capacitor's or \
    inductor's, with its value, footprint, datasheet, maker and number set (your own key, free at developer.digikey.com, \
    in Settings: Ctrl+,)");

/// A line of a Checks list: its kind's name, what is wrong, how to fix it (its tooltip) and its colour.
pub(crate) type Check<'a> = (&'static str, String, &'a str, egui::Color32);

/// This many problems read as one list; more come by kind, a kind of more than this folded.
const AT_A_GLANCE: usize = 12;

/// A Checks list, scrolling: `rows` as they come while they are few, else kind by kind in the
/// order the kinds first come, each under its name and how many it has, a long one folded until
/// it is asked for (a complex board's hundreds are a handful of lines). Returns the row clicked.
pub(crate) fn check_rows(ui: &mut egui::Ui, rows: &[Check]) -> Option<usize> {
    let mut pick = None;
    let mut row = |ui: &mut egui::Ui, k: usize| {
        let (_, what, fix, color) = &rows[k];
        if ui.add(egui::Button::selectable(false, egui::RichText::new(what).color(*color)).wrap()).on_hover_text(*fix).clicked() {
            pick = Some(k);
        }
    };
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        if rows.len() <= AT_A_GLANCE {
            return (0..rows.len()).for_each(|k| row(ui, k));
        }
        let mut kinds: Vec<(&str, Vec<usize>)> = vec![];
        for (k, r) in rows.iter().enumerate() {
            match kinds.iter_mut().find(|kind| kind.0 == r.0) {
                Some(kind) => kind.1.push(k),
                None => kinds.push((r.0, vec![k])),
            }
        }
        for (name, of_kind) in kinds {
            let title = egui::RichText::new(format!("{name}  ·  {}", of_kind.len())).family(theme::strong()).color(rows[of_kind[0]].3);
            egui::CollapsingHeader::new(title).id_salt(name).default_open(of_kind.len() <= AT_A_GLANCE).show(ui, |ui| {
                of_kind.iter().for_each(|&k| row(ui, k));
            });
        }
    });
    pick
}

impl App {
    /// Whether the Parts tab is out, to add a part.
    pub(crate) fn adding(&self) -> bool {
        self.left.shows(Tab::Parts)
    }

    /// `A`, or the button: the Parts tab with its field ready to type in, or back to the layers.
    /// A over the canvas while the tab is out readies the field for the next part instead.
    pub(crate) fn toggle_add(&mut self, ctx: &egui::Context) {
        if !(self.adding() && self.pointer.is_some()) {
            self.left.toggle(Tab::Parts, Tab::Layers);
        }
        if self.adding() {
            self.open_parts(ctx);
        }
    }

    /// Puts the keyboard in the Parts tab's field, its last words selected so typing starts anew.
    pub(crate) fn open_parts(&mut self, ctx: &egui::Context) {
        focus(ctx, ADD_FIELD, "a");
        let id = egui::Id::unique(ADD_FIELD);
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&self.part_query)));
        state.store(ctx, id);
    }

    /// The Parts tab (`A`): the parts of KiCad's libraries that hold every word typed (the
    /// symbols in the schematic; and a part number's part at Digi-Key, `lookup.rs`); ↑↓ pick one,
    /// Enter or a click adds it in the middle of the view (a symbol follows the pointer until a
    /// click puts it down), Esc goes back to the layers.
    pub(crate) fn parts_tab(&mut self, ui: &mut egui::Ui) {
        let sch_view = self.sch_view();
        if if sch_view { self.sch.is_none() } else { self.scene.is_none() } {
            ui.label(egui::RichText::new(tr("Open a board or a schematic to add parts to it.")).color(theme::QUIET));
            return;
        }
        let folder = self.folder();
        let (parts, hint, missing) = if sch_view {
            (self.symbols.get_or_insert_with(|| crate::parts::symbols(folder.as_deref())), "R, LED, LM358DR, regulator 3.3…", NO_SYMBOLS)
        } else {
            let parts = self.parts.get_or_insert_with(|| crate::parts::footprints(folder.as_deref()));
            (parts, "MountingHole M3, TestPoint, Fiducial, Logo…", NO_LIBRARIES)
        };
        let (hits, none) = (library::search(parts, &self.part_query), parts.is_empty());
        // A part number's row (`None`): its part at Digi-Key, first when the user's key is set.
        #[cfg(not(target_arch = "wasm32"))]
        let ask = if sch_view { crate::lookup::row(&self.part_query, &self.lookup, crate::settings::digikey_key().is_ok()) } else { None };
        #[cfg(target_arch = "wasm32")]
        let ask = None::<(bool, String)>;
        let mut rows: Vec<Option<&LibPart>> = hits.iter().take(SHOWN).map(|&p| Some(p)).collect();
        match &ask {
            Some((true, _)) => rows.insert(0, None),
            Some(_) => rows.push(None),
            None => {}
        }
        // ↑↓ pick while the field is typed in.
        let typing = ui.memory(|m| m.has_focus(egui::Id::unique(ADD_FIELD)));
        let key = |ui: &mut egui::Ui, k| typing && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
        let step = key(ui, egui::Key::ArrowDown) as isize - key(ui, egui::Key::ArrowUp) as isize;
        self.part_pick = self.part_pick.saturating_add_signed(step).min(rows.len().saturating_sub(1));
        let mut add = None;
        #[cfg(not(target_arch = "wasm32"))]
        let (mut site_open, mut add_files, mut new_part) = (None::<String>, false, false);
        let back;
        {
            let field = egui::TextEdit::singleline(&mut self.part_query).id(egui::Id::unique(ADD_FIELD)).hint_text(hint);
            let field = ui.add(field.desired_width(f32::INFINITY));
            if field.changed() {
                self.part_pick = 0;
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                add = rows.get(self.part_pick).copied();
            }
            back = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
            ui.label(match (hits.len(), none) {
                (_, true) if cfg!(target_arch = "wasm32") => {
                    tr("Parts come from KiCad's libraries on your computer: add them in the desktop app").into()
                }
                (_, true) => tr(missing).into(),
                (n, _) if n > SHOWN => trf("{0} parts, the first {1}: type more to narrow", &[&n, &SHOWN]),
                (n, _) => crate::count(n, n!("part")),
            });
            // A part KiCad's libraries lack: its KiCad zip from a site that makes them, dropped on
            // the window (`parts.rs`).
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(mpn) = crate::lookup::part_number(&self.part_query) {
                ui.add_space(2.0);
                let note = tr("Not in KiCad's libraries? Download its KiCad zip from one of these, then add it:");
                ui.add(egui::Label::new(egui::RichText::new(note).small().color(theme::QUIET)).wrap());
                ui.horizontal_wrapped(|ui| {
                    for (site, search) in crate::parts::SITES {
                        if ui.link(egui::RichText::new(site).small()).on_hover_text(format!("{search}{mpn}")).clicked() {
                            site_open = Some(format!("{search}{}", crate::parts::query_value(mpn)));
                        }
                    }
                });
            }
            // A part from a vendor's files, or one made here: both into the project's own library.
            #[cfg(not(target_arch = "wasm32"))]
            ui.horizontal_wrapped(|ui| {
                for (cmd, clicked) in [(crate::cmd::Cmd::ImportPart, &mut add_files), (crate::cmd::Cmd::NewPart, &mut new_part)] {
                    let about = cmd.about();
                    *clicked = ui.button(format!("{}  {}", about.icon, about.name)).on_hover_text(about.hint).clicked();
                }
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for (k, row) in rows.iter().enumerate() {
                    // The part's name first, its library quiet after it.
                    let (name, lib, about, how) = match (row, &ask) {
                        (Some(f), _) => (f.name.clone(), f.lib.as_str(), f.about.as_str(), tr("Click or Enter adds it · ↑↓ pick")),
                        (None, ask) => {
                            let name = ask.as_ref().map_or(String::new(), |a| a.1.clone());
                            (name, "", tr(DIGIKEY), tr("Click or Enter asks Digi-Key · ↑↓ pick"))
                        }
                    };
                    let mut text = egui::text::LayoutJob::default();
                    let font = egui::FontId::proportional(13.0);
                    text.append(&name, 0.0, egui::TextFormat::simple(font.clone(), theme::TEXT));
                    text.append(lib, 8.0, egui::TextFormat::simple(font, theme::QUIET));
                    let hover = if about.is_empty() { format!("{lib}:{name}\n{how}") } else { format!("{lib}:{name}\n{about}\n{how}") };
                    let resp = ui.add(egui::Button::selectable(k == self.part_pick, text).truncate()).on_hover_text(hover);
                    if resp.clicked() {
                        add = Some(*row);
                    } else if step != 0 && k == self.part_pick {
                        resp.scroll_to_me(None);
                    }
                }
            });
        }
        if back {
            self.left.tab = Tab::Layers;
        }
        match add.map(|row| row.cloned()) {
            Some(Some(part)) if sch_view => self.pick_symbol(&part),
            Some(Some(part)) => self.add_part(&part),
            #[cfg(not(target_arch = "wasm32"))]
            Some(None) => self.look_up(),
            _ => {}
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(url) = site_open {
            self.open_url(ui.ctx(), &url);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if add_files {
            self.ask_parts();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if new_part {
            self.run(ui.ctx(), crate::cmd::Cmd::NewPart);
        }
    }

    /// DC IR-drop results (`I`): test current, totals, colour scale and every pad's drop.
    pub(crate) fn heat_tab(&mut self, ui: &mut egui::Ui) {
        {
            let (mut pick, mut new_src, mut cur_changed) = (None, None, false);
            if let (Some((ir, _)), Some(scene)) = (&self.ir, &self.scene) {
                {
                    ui.horizontal(|ui| {
                        ui.strong(trf("IR Drop: {0}", &[&self.board.nets[ir.net as usize]]));
                        cur_changed =
                            ui.add(egui::DragValue::new(&mut self.ir_current).speed(0.05).range(0.01..=100.0).suffix(" A")).changed();
                    });
                    ui.label(format!("{} -> {}  |  {:.2} mV ({:.2} mΩ)", ir.source, ir.sink, ir.max_drop_mv, ir.resistance_mohm));
                    let (power, peak) = (format!("{:.2}", ir.power_mw), format!("{:.1}", ir.max_j_a_mm2));
                    ui.label(trf("Power {0} mW  |  Peak {1} A/mm²", &[&power, &peak]));
                    let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(120.0), 10.0), egui::Sense::hover());
                    for k in 0..32 {
                        let (t0, t1) = (k as f32 / 32.0, (k + 1) as f32 / 32.0);
                        let r = egui::Rect::from_min_max(
                            egui::pos2(bar.left() + bar.width() * t0, bar.top()),
                            egui::pos2(bar.left() + bar.width() * t1, bar.bottom()),
                        );
                        ui.painter().rect_filled(r, 0.0, color32(heat_color((t0 + t1) * 0.5)));
                    }
                    ui.horizontal(|ui| {
                        ui.small(tr("0.0 mV (source)"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.small(trf("{0} mV (sink)", &[&format!("{:.2}", ir.max_drop_mv)]));
                        });
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                        for p in &ir.pads {
                            let t = if ir.max_drop_mv > 1e-9 { (p.drop_mv / ir.max_drop_mv) as f32 } else { 0.0 };
                            let label = format!("{}.{}: {:.2} mV ({:.2} mΩ)", p.reference, p.number, p.drop_mv, p.resistance_mohm);
                            let resp = ui
                                .selectable_label(false, egui::RichText::new(label).color(color32(heat_color(t))))
                                .on_hover_text(tr("Click to zoom · Right-click to set as source"));
                            if resp.clicked() {
                                pick = Some(scene.um(p.at));
                            }
                            if resp.secondary_clicked() {
                                new_src = Some(format!("{}.{}", p.reference, p.number));
                            }
                        }
                    });
                }
            }
            if let Some(s) = new_src {
                self.ir_source = Some(s);
                self.update_ir(true);
            } else if cur_changed {
                self.update_ir(false);
            }
            if let Some([x, y]) = pick {
                let b = [x - 500.0, y - 500.0, x + 500.0, y + 500.0];
                (self.found, self.fit) = (vec![b], Some(around(b, 2.0)));
            }
        }
    }

    /// Diff changes (`G`); a click zooms to one.
    pub(crate) fn changes_tab(&mut self, ui: &mut egui::Ui) {
        if let (Some((base, diff, _)), Some(scene)) = (&self.diff, &self.scene) {
            let mut pick = None;
            {
                ui.strong(trf("Since {0}: {1}", &[base, &crate::count(diff.changes.len(), n!("change"))]));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for c in &diff.changes {
                        let color = match c.kind {
                            ChangeKind::Added => theme::PASS,
                            ChangeKind::Removed => theme::FAIL,
                            ChangeKind::Changed => theme::WARN,
                        };
                        if ui.add(egui::Button::selectable(false, egui::RichText::new(&c.what).color(color)).wrap()).clicked() {
                            let ([x0, y0], [x1, y1]) = (scene.um(c.bounds.0), scene.um(c.bounds.1));
                            pick = Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]);
                        }
                    }
                });
            }
            if let Some(b) = pick {
                (self.found, self.fit) = (vec![b], Some(around(b, 2.0)));
            }
        }
    }

    /// The connections still to route, then the DRC violations (`D`); hover shows a violation's
    /// fix, a click zooms to one.
    pub(crate) fn drc_tab(&mut self, ui: &mut egui::Ui) {
        if let Some(scene) = &self.scene {
            let (mut pick, mut link) = (None, None);
            if !self.ratsnest.is_empty() {
                crate::dock::heading(ui, &trf("{0} to route", &[&self.ratsnest.len()]));
                egui::ScrollArea::vertical().id_salt("to route").max_height(160.0).show(ui, |ui| {
                    for l in &self.ratsnest {
                        let text = egui::RichText::new(format!("{} · {}", l.what(&self.board), crate::units::len(l.mm() * 1e6)))
                            .color(theme::WARN);
                        if ui.add(egui::Button::selectable(false, text).truncate()).on_hover_text(tr("Click to see it")).clicked() {
                            let ([ax, ay], [bx, by]) = (scene.um(l.a), scene.um(l.b));
                            link = Some([ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)]);
                        }
                    }
                });
                ui.add_space(8.0);
                crate::dock::heading(ui, tr("Design rules"));
            }
            if let Some(b) = link {
                (self.found, self.fit) = (vec![b], Some(around(b, 1.5)));
            }
            if self.violations.is_empty() {
                let text = format!("{}  {}", crate::icon::CHECKS, tr("Every design rule passes."));
                ui.label(egui::RichText::new(text).color(theme::PASS));
            }
            let color = |v: &cuprita_core::drc::Violation| if v.warning { theme::WARN } else { theme::TEXT_2 };
            let rows: Vec<Check> = self.violations.iter().map(|v| (v.kind_name(), v.what.clone(), &v.fix[..], color(v))).collect();
            if let Some(k) = check_rows(ui, &rows) {
                pick = Some(scene.um(self.violations[k].at));
            }
            if let Some([x, y]) = pick {
                let b = [x - 200.0, y - 200.0, x + 200.0, y + 200.0];
                (self.found, self.fit) = (vec![b], Some(around(b, 1.0)));
            }
        }
    }

    /// Bench BOM checklist (`Shift+B`): a line marks its parts; Space ticks it and moves on, ↑↓ select.
    pub(crate) fn bom_tab(&mut self, ui: &mut egui::Ui) {
        let before = self.ticked.clone();
        if let Some(scene) = &self.scene {
            let (n, cur, mut pick) = (self.bom.len(), self.bom_line, None);
            if n > 0 && !ui.ctx().egui_wants_keyboard_input() {
                let key = |k| ui.input(|i| i.key_pressed(k));
                // Space ticks the selected line and moves on to the next one left to do.
                if key(egui::Key::Space) {
                    if let Some(k) = cur {
                        self.ticked[k] = !self.ticked[k];
                    }
                    if cur.is_none_or(|k| self.ticked[k]) {
                        let from = cur.map_or(0, |k| k + 1);
                        pick = (from..n).chain(0..from).find(|&i| !self.ticked[i]);
                    }
                }
                if key(egui::Key::ArrowDown) {
                    pick = Some(cur.map_or(0, |k| (k + 1).min(n - 1)));
                }
                if key(egui::Key::ArrowUp) {
                    pick = Some(cur.map_or(0, |k| k.saturating_sub(1)));
                }
            }
            let scroll = pick.is_some();
            {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    let fp = &self.board.footprints;
                    for (k, line) in self.bom.iter().enumerate() {
                        let f = &fp[line[0]];
                        let refs: Vec<&str> = line.iter().map(|&i| fp[i].reference.as_str()).collect();
                        let name = f.lib_id.rsplit(':').next().unwrap_or_default();
                        let text = egui::RichText::new(format!("{}× {}  {name}\n{}", line.len(), f.value, refs.join(", ")));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.ticked[k], "");
                            let text = if self.ticked[k] { text.weak() } else { text };
                            let resp = ui
                                .add(egui::Button::selectable(cur == Some(k), text).wrap())
                                .on_hover_text(tr("Click to mark its parts · Space ticks the selected line and moves on · ↑↓ select"));
                            if resp.clicked() {
                                pick = Some(k);
                            } else if scroll && pick == Some(k) {
                                resp.scroll_to_me(Some(egui::Align::Center));
                            }
                        });
                    }
                });
            }
            // The whole board stays in view so every marked part can be found on the real one.
            if let Some(k) = pick {
                self.found = self.bom[k].iter().map(|&i| um_box(scene, self.board.footprints[i].bounds)).collect();
                (self.bom_line, self.fit) = (Some(k), Some(scene.bounds));
                ui.ctx().request_repaint(); // the list was drawn with the old selection
            }
        }
        if self.ticked != before {
            self.keep_ticks();
        }
    }
}
