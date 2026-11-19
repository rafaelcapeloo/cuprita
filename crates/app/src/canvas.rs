//! The board canvas: pan (drag), zoom at the cursor (wheel), click to pick (Shift+click or a
//! Shift+drag box several), drag what is picked; the board is a wgpu paint callback (`Paint`),
//! tool marks are egui shapes over it.

use crate::tr::tr;
use crate::{App, grid, ruler_text, theme, um_box, zone};
use cuprita_core::board::{Nm, Pt};
use cuprita_core::drc::Item;
use cuprita_core::history::Pick;
use cuprita_render::{Renderer, View};
use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};

impl App {
    pub(crate) fn canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.settle_view(ui.ctx(), rect, resp.dragged() || wheel_notches(ui) != 0.0 || ui.input(|i| i.any_touches()));
        let Some(scene) = &self.scene else { return };
        let x_sign = if self.flipped { -1.0 } else { 1.0 };
        let to_um = |pos: egui::Pos2, center: [f32; 2], scale: f32| {
            let off = pos - rect.center();
            [center[0] + off.x * scale * x_sign, center[1] + off.y * scale]
        };
        let visible: Vec<bool> = scene.layers.iter().map(|l| l.visible).collect();
        // Picking reach: 3 px in scene µm, and in board nm.
        let (tol, tol_nm) = (3.0 * self.um_per_pt, (3000.0 * self.um_per_pt) as Nm);
        let (mut ir_changed, mut route_at, mut zone_at, mut pair_at) = (false, None, None, None);
        let pointer = resp.hover_pos().map(|p| scene.nm(to_um(p, self.center, self.um_per_pt)));
        self.pointer = pointer;
        // The route and zone tools' own keys.
        let typing = ui.ctx().egui_wants_keyboard_input();
        let key = |k| !typing && ui.input(|i| i.key_pressed(k));
        let (via, enter, back) = (key(egui::Key::V), key(egui::Key::Enter), key(egui::Key::Backspace));
        // Shift frees a corner's angle, Ctrl (KiCad's key) points and drags from the grid.
        let (shift, free) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        if let (true, Some(pos)) = (resp.clicked(), resp.interact_pointer_pos()) {
            let at = to_um(pos, self.center, self.um_per_pt);
            if self.measuring {
                let at = scene.um(self.aim(scene.nm(at), tol_nm, true, free).0);
                self.ruler = match self.ruler {
                    Some((a, _, false)) => Some((a, at, true)),
                    _ => Some((at, at, false)),
                };
            } else if self.routing && !(self.show_diff || self.show_ir) {
                route_at = Some(scene.nm(at));
            } else if self.pairing && !(self.show_diff || self.show_ir) {
                pair_at = Some(scene.nm(at));
            } else if self.zoning && !(self.show_diff || self.show_ir) {
                zone_at = Some(self.aim(scene.nm(at), tol_nm, shift, free).0);
            } else {
                let picked = scene.net_at(at, tol, &visible, self.flipped, Some(self.active as usize));
                if self.show_ir {
                    if picked != 0 && picked != self.net {
                        (self.net, self.ir_source, ir_changed) = (picked, None, true);
                    }
                } else if self.show_diff {
                    self.net = picked;
                } else if shift {
                    // Shift+click adds what `pick_at` picks to what is picked, or takes it away.
                    if let Some(p) = self.pick_at(scene.nm(at), tol_nm) {
                        let mut all = self.picks().to_vec();
                        match all.iter().position(|&q| q == p) {
                            Some(k) => drop(all.remove(k)),
                            None => all.push(p),
                        }
                        (self.selected, self.picked) = one_or_more(all);
                        self.status.clear();
                    }
                } else {
                    // What `pick_at` picks is selected and lights its net (the pad's, for a part
                    // clicked on one); a part clicked off its pads lights none; elsewhere a click
                    // where there is no net clears the highlight.
                    let (at, b) = (scene.nm(at), &self.board);
                    let hit = b.copper_at(at, tol_nm, &self.shown_copper());
                    (self.selected, self.picked) = (self.pick_at(at, tol_nm), Vec::new());
                    self.net = match (hit, self.selected) {
                        (Some(Item::Pad(i)), _) => b.pads[i].net,
                        (Some(Item::Track(i)), _) => b.tracks[i].net,
                        (Some(Item::Via(i)), _) => b.vias[i].net,
                        (None, Some(Pick::Zone(i))) => b.zones[i].net,
                        (None, Some(_)) => 0,
                        (None, None) => picked,
                    };
                }
            }
        }
        // With the outline tool, a drag before any corner draws a rectangle, from where it was
        // pressed to where it is let go.
        let board_at = |pos| self.aim(scene.nm(to_um(pos, self.center, self.um_per_pt)), tol_nm, true, free).0;
        let boxes = self.zoning && (self.outlining || self.lining);
        let rect_from = (boxes && self.zone.is_none() && resp.drag_started_by(egui::PointerButton::Primary))
            .then(|| ui.input(|i| i.pointer.press_origin()))
            .flatten()
            .map(board_at);
        let dragging = rect_from.is_some() || self.zone.as_ref().is_some_and(|z| z.rect);
        let rect_to = (dragging && resp.drag_stopped()).then(|| ui.input(|i| i.pointer.latest_pos())).flatten().map(board_at);
        // A Shift+drag draws a box, anywhere (zone fills cover most boards); a drag that starts on
        // an item picked moves what is picked, on a handle of the zone picked alone its corner
        // (`corners.rs`) (board view only); other drags pan.
        let press = (resp.drag_started_by(egui::PointerButton::Primary)
            && !(self.measuring || self.routing || self.pairing || self.zoning || self.show_diff || self.show_ir))
            .then(|| ui.input(|i| i.pointer.press_origin()))
            .flatten()
            .map(|pos| to_um(pos, self.center, self.um_per_pt));
        if let (Some(at), true) = (press, shift) {
            self.boxing = Some(scene.nm(at));
        } else if let Some(at) = press.filter(|_| !self.viewer)
            && let Some(held) = match self.selected {
                Some(z @ Pick::Zone(_)) if self.handle(scene.nm(at)).is_some() => Some(z),
                _ => self.picks().iter().find(|&&p| self.board.hits(p, scene.nm(at), tol_nm)).copied(),
            }
        {
            self.grab = Some((at, held));
        }
        // A grab on a handle of the zone picked alone moves that corner, any other what is picked.
        let on_handle =
            self.grab.is_some_and(|(g, p)| matches!(p, Pick::Zone(_)) && self.selected == Some(p) && self.handle(scene.nm(g)).is_some());
        // How far what is picked moves: the item grabbed onto the grid (`grid::drag_by`).
        let moved = match (self.grab.filter(|_| !on_handle), ui.input(|i| i.pointer.latest_pos())) {
            (Some((g, held)), Some(pos)) => {
                let to = scene.nm(to_um(pos, self.center, self.um_per_pt));
                Some(grid::drag_by(&self.board, self.board_grid(), held, (scene.nm(g), to), tol_nm, free))
            }
            _ => None,
        };
        let offset = moved.map(|d| [d.x as f32 / 1000.0, d.y as f32 / 1000.0]);
        // Where the selected zone's handle dragged lands, snapped as a zone corner.
        let handle_drag = match (self.grab.filter(|_| on_handle), ui.input(|i| i.pointer.latest_pos())) {
            (Some((g, _)), Some(pos)) => {
                Some((scene.nm(g), self.aim(scene.nm(to_um(pos, self.center, self.um_per_pt)), tol_nm, true, free).0))
            }
            _ => None,
        };
        let box_to = (self.boxing.is_some() && resp.drag_stopped())
            .then(|| ui.input(|i| i.pointer.latest_pos()))
            .flatten()
            .map(|pos| scene.nm(to_um(pos, self.center, self.um_per_pt)));
        // One finger or the mouse pans; two fingers are `zoom_at`'s.
        if self.grab.is_none() && self.boxing.is_none() && !dragging && ui.input(|i| i.multi_touch().is_none()) {
            let d = resp.drag_delta();
            self.center = [self.center[0] - d.x * self.um_per_pt * x_sign, self.center[1] - d.y * self.um_per_pt];
        }
        if ir_changed {
            self.update_ir(true);
        }
        if self.route.is_some() && via {
            self.route_via();
        }
        if self.route.is_some() && enter {
            self.route_fix(true);
        }
        if self.zone.is_some() && enter {
            self.zone_close();
        }
        if self.zone.is_some() && back {
            self.zone_back();
        }
        if self.pair.is_some() && enter {
            self.pair_finish();
        }
        if self.pair.is_some() && back {
            self.pair_back();
        }
        match pair_at {
            Some(_) if resp.double_clicked() => self.pair_finish(),
            Some(at) => self.pair_click(at, tol_nm),
            None => {}
        }
        if let Some(at) = pointer.filter(|_| self.pair.is_some()) {
            self.pair_to(at, tol_nm);
        }
        match route_at {
            Some(_) if resp.double_clicked() => self.route_fix(true),
            Some(at) => self.route_click(at, tol_nm),
            None => {}
        }
        if let Some(at) = pointer.filter(|_| self.route.is_some()) {
            self.route_to(at, tol_nm, free);
        }
        match zone_at {
            Some(_) if resp.double_clicked() => self.zone_close(),
            Some(at) => self.zone_click(at, tol_nm),
            None => {}
        }
        if let Some(at) = rect_from {
            self.zone = self.outline_draft(at, true);
        }
        if let (Some(at), Some(z)) = (rect_to, self.zone.as_mut().filter(|z| z.rect)) {
            let from = z.corners[0];
            (z.corners, z.rect) = (zone::rectangle(from, at), false);
            // Lines close back on the corner they start at.
            if self.lining {
                z.corners.push(from);
            }
            if at.x == from.x || at.y == from.y {
                (self.zone, self.status) = (None, tr("Drag across: a rectangle needs a width and a height").into());
            } else {
                self.zone_close();
            }
        }
        // A box let go picks what it takes too: drawn left to right on screen what it holds, right
        // to left what it touches (KiCad's rule), on the copper shown.
        if let (Some(from), Some(to)) = (self.boxing, box_to) {
            let mut all = self.picks().to_vec();
            all.extend(self.board.items_in(from, to, (to.x < from.x) != self.flipped, &self.shown_copper(), self.shown_layers()));
            all.sort_unstable();
            all.dedup();
            (self.selected, self.picked) = one_or_more(all);
            (self.boxing, self.status) = (None, String::new());
        }
        if let (true, Some(d)) = (resp.drag_stopped(), moved) {
            self.grab = None;
            match self.selected {
                _ if d == Pt::default() => {}
                // A track alone drags with its neighbours following, as KiCad's drag; else all move.
                Some(Pick::Track(i)) => self.drag_track(i, d),
                _ => self.move_picks(d),
            }
        }
        if let (true, Some((from, to))) = (resp.drag_stopped(), handle_drag) {
            self.grab = None;
            self.drop_handle(from, to);
        }
        let Some(scene) = &self.scene else { return };
        if let (Some((a, _, false)), Some(pos)) = (self.ruler, resp.hover_pos()) {
            let b = scene.um(self.aim(scene.nm(to_um(pos, self.center, self.um_per_pt)), tol_nm, true, free).0);
            self.ruler = Some((a, b, false));
        }
        // Esc drops the track being routed, the zone being drawn or a drag first; then it clears
        // the selection, highlight, markers, ruler and tools.
        if ui.input(|i| i.key_pressed(egui::Key::Escape))
            && self.route.take().is_none()
            && self.pair.take().is_none()
            && self.zone.take().is_none()
            && self.grab.take().is_none()
            && self.boxing.take().is_none()
        {
            if !self.show_ir {
                self.net = 0;
            }
            (self.found, self.bom_line, self.measuring, self.ruler, self.selected, self.routing, self.zoning, self.writing) =
                (Vec::new(), None, false, None, None, false, false, None);
            self.pairing = false;
            self.picked.clear();
        }
        // Under the pointer, or between two fingers on the canvas.
        let touched = ui.input(|i| i.multi_touch()).map(|t| t.center_pos).filter(|p| rect.contains(*p));
        if let Some(pos) = resp.hover_pos().or(touched) {
            zoom_at(ui, pos, (&mut self.center, &mut self.um_per_pt), to_um);
        }
        let ppp = ui.ctx().pixels_per_point();
        let size = [rect.width() * ppp, rect.height() * ppp];
        let mut view = View::new(self.center, self.um_per_pt / ppp, size).flip(self.flipped).active(Some(self.active as usize));
        view.net = if self.show_diff || self.show_ir { 1 } else { self.net };
        let screen = |[x, y]: [f32; 2]| rect.center() + egui::vec2((x - self.center[0]) * x_sign, y - self.center[1]) / self.um_per_pt;
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, Paint(view, visible)));
        // The grid's dots, faint over the board.
        let corners = [rect.min, rect.max].map(|q| scene.nm(to_um(q, self.center, self.um_per_pt)));
        grid::dots(ui.painter(), corners, self.board_grid(), grid::ON_BOARD, |p| screen(scene.um(p)));
        // The ratsnest: thin lines, the lit net's brighter.
        if !(self.show_diff || self.show_ir) {
            for l in &self.ratsnest {
                let (lit, p) = (l.net == self.net, ui.painter());
                let stroke = if lit { (1.5, theme::SIGNAL) } else { (1.0, theme::TEXT.gamma_multiply(0.55)) };
                p.line_segment([screen(scene.um(l.a)), screen(scene.um(l.b))], stroke);
            }
        }
        if self.right.shows(crate::dock::Tab::Checks) {
            for v in &self.violations {
                let color = if v.warning { theme::WARN } else { theme::FAIL };
                ui.painter().circle_stroke(screen(scene.um(v.at)), 6.0, (2.0, color));
            }
        }
        let cyan = theme::SIGNAL;
        let ping = crate::motion::eased(ui.ctx(), self.pinged.1, crate::motion::ARRIVE);
        for &[x0, y0, x1, y1] in &self.found {
            let marker = egui::Rect::from_two_pos(screen([x0, y0]), screen([x1, y1])).expand(3.0);
            ui.painter().rect_stroke(marker, 2.0, (2.0, cyan), egui::StrokeKind::Outside);
            crate::helpers::ping(ui.painter(), marker, ping);
        }
        self.draw_route(ui, |p| screen(scene.um(p)));
        self.draw_pair(ui, |p| screen(scene.um(p)));
        let aim = pointer.filter(|_| self.zone.is_some()).map(|p| self.aim(p, tol_nm, shift, free).0);
        self.draw_zone(ui, aim, |p| screen(scene.um(p)));
        // Each part picked: its box; while dragged, the box and its pads where it would land.
        let (dragged, [dx, dy]) = (self.grab.is_some(), self.grab.and(offset).unwrap_or_default());
        for (i, f) in
            self.picks().iter().filter_map(|&p| if let Pick::Footprint(i) = p { Some((i, &self.board.footprints[i])) } else { None })
        {
            let [x0, y0, x1, y1] = um_box(scene, f.bounds);
            let marker = egui::Rect::from_two_pos(screen([x0 + dx, y0 + dy]), screen([x1 + dx, y1 + dy])).expand(3.0);
            ui.painter().rect_stroke(marker, 2.0, (2.0, cyan), egui::StrokeKind::Outside);
            // Guides, for a part dragged alone: the nearest parts whose anchors line up with where it
            // lands, each way, and how far.
            if let (true, Some(d), [_]) = (dragged, moved, self.picks()) {
                let at = Pt { x: f.at.x + d.x, y: f.at.y + d.y };
                let others = self.board.footprints.iter().enumerate().filter(|&(k, _)| k != i).map(|(_, o)| o.at);
                for o in grid::lined_up(at, others).into_iter().flatten() {
                    let (a, b) = (screen(scene.um(o)), screen(scene.um(at)));
                    ui.painter().extend(egui::Shape::dashed_line(&[a, b], (1.0, cyan.gamma_multiply(0.7)), 5.0, 4.0));
                    let label = format!("Δ {}", crate::units::len(((o.x - at.x) as f64).hypot((o.y - at.y) as f64)));
                    ui.painter().text(
                        a.lerp(b, 0.5) + egui::vec2(6.0, -4.0),
                        egui::Align2::LEFT_BOTTOM,
                        label,
                        egui::FontId::monospace(11.0),
                        cyan,
                    );
                }
            }
            for p in self.board.pads[f.pads.clone()].iter().filter(|_| dragged) {
                if !p.custom.is_empty() {
                    for ring in p.copper() {
                        let pts = ring.into_iter().map(|q| scene.um(q)).map(|[x, y]| screen([x + dx, y + dy])).collect();
                        ui.painter().add(egui::Shape::closed_line(pts, (1.5, cyan)));
                    }
                    continue;
                }
                let ([x, y], (s, c)) = (scene.um(p.at), p.angle.to_radians().sin_cos());
                let (s, c, hw, hh) = (s as f32, c as f32, p.size.0 as f32 / 2000.0, p.size.1 as f32 / 2000.0);
                // KiCad angles turn counter-clockwise on the Y-down board: local x runs along (cos, -sin).
                let corner = |u: f32, v: f32| screen([x + dx + u * hw * c + v * hh * s, y + dy - u * hw * s + v * hh * c]);
                let outline = vec![corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
                ui.painter().add(egui::Shape::closed_line(outline, (1.5, cyan)));
            }
        }
        // A track picked glows along its length; a via picked gets a ring; a zone picked shows its
        // outline, its corners marked: while dragged, where they would land; the zone picked alone
        // shows its handles.
        let shifted = |p: Pt| {
            let [x, y] = scene.um(p);
            screen([x + dx, y + dy])
        };
        // A track alone, dragged: it and its neighbours as they would land (`Board::dragged`).
        let plan = match (self.selected, moved.filter(|_| dragged)) {
            (Some(Pick::Track(i)), Some(d)) => self.board.dragged(i, d).ok(),
            _ => None,
        };
        if let Some((changed, added)) = &plan {
            for t in changed.iter().map(|(_, t)| t).chain(added) {
                let width = t.width as f32 / 1000.0 / self.um_per_pt + 6.0;
                ui.painter().line_segment([screen(scene.um(t.a)), screen(scene.um(t.b))], (width, cyan.gamma_multiply(0.4)));
            }
        }
        for &pick in self.picks().iter().filter(|_| plan.is_none()) {
            match pick {
                Pick::Zone(i) if self.selected == Some(pick) => self.draw_handles(ui, i, handle_drag, pointer, |p| screen(scene.um(p))),
                Pick::Zone(i) => {
                    for ring in &self.board.zones[i].outline {
                        let pts: Vec<_> = ring.iter().map(|&p| shifted(p)).collect();
                        for &q in &pts {
                            ui.painter().circle_filled(q, 3.0, cyan);
                        }
                        ui.painter().add(egui::Shape::closed_line(pts, (2.0, cyan)));
                    }
                }
                Pick::Track(i) => {
                    let t = &self.board.tracks[i];
                    let pts = [t.a].into_iter().chain(t.mid).chain([t.b]).map(shifted).collect();
                    let width = t.width as f32 / 1000.0 / self.um_per_pt + 6.0;
                    ui.painter().add(egui::Shape::line(pts, (width, cyan.gamma_multiply(0.4))));
                }
                Pick::Via(i) => {
                    let v = &self.board.vias[i];
                    ui.painter().circle_stroke(shifted(v.at), v.size as f32 / 2000.0 / self.um_per_pt + 3.0, (2.0, cyan));
                }
                // A text picked gets a box, a drawing glows along its line.
                Pick::Text(_) | Pick::Shape(_) => {
                    ui.painter().add(crate::helpers::drawing_mark(&self.board, pick, self.um_per_pt, cyan, shifted));
                }
                Pick::Footprint(_) => {}
            }
        }
        // The box being drawn (Shift+drag).
        if let (Some(from), Some(to)) = (self.boxing, pointer) {
            crate::helpers::pick_box(ui.painter(), screen(scene.um(from)), screen(scene.um(to)));
        }
        if let Some((a, b, _)) = self.ruler {
            let (sa, sb) = (screen(a), screen(b));
            let color = theme::SIGNAL;
            let p = ui.painter();
            p.line_segment([sa, egui::pos2(sb.x, sa.y)], (1.0, color.gamma_multiply(0.5)));
            p.line_segment([egui::pos2(sb.x, sa.y), sb], (1.0, color.gamma_multiply(0.5)));
            p.line_segment([sa, sb], (2.0, color));
            for pt in [sa, sb] {
                p.circle_filled(pt, 3.5, color);
            }
            let galley = p.layout_no_wrap(ruler_text(a, b), egui::FontId::proportional(12.0), color);
            let box_rect = egui::Align2::CENTER_BOTTOM.anchor_size(sa.lerp(sb, 0.5) - egui::vec2(0.0, 6.0), galley.size());
            p.rect_filled(box_rect.expand(4.0), 6.0, theme::GROUND.gamma_multiply(0.92));
            p.galley(box_rect.min, galley, color);
        }
        let snapped = self.board_helpers(ui, rect, pointer, tol_nm, |p| screen(scene.um(p)));
        self.snap_to(ui.ctx(), snapped);
        self.fade_in(ui, rect);
    }

    /// Takes the canvas `rect`'s view to the box asked for (`App::fit`): at once when the canvas
    /// shows something new, else gliding there, so the eye keeps its place (`motion.rs`); the
    /// hand moving the view (`moved`) stops a glide. A view still as fitted follows its canvas: a
    /// window resized, a dock opened, a phone turned (a page may open before its window has its
    /// size). What is marked as found is rung anew.
    pub(crate) fn settle_view(&mut self, ctx: &egui::Context, rect: egui::Rect, moved: bool) {
        use crate::motion::{GLIDE, eased, lerp};
        let size = [rect.width(), rect.height()];
        let again = self.fitted.filter(|f| f.1 != size && !moved && self.fit.is_none()).map(|f| f.0);
        if let Some(b) = self.fit.take().or(again) {
            let (to, at_once) = (cuprita_render::fit(b, size), self.fresh || again.is_some());
            self.glide = (!at_once).then(|| ((self.center, self.um_per_pt), to, web_time::Instant::now()));
            if at_once {
                (self.center, self.um_per_pt) = to;
            }
            self.fitted = Some((b, size));
        }
        self.fresh = false;
        if moved {
            (self.glide, self.fitted) = (None, None);
        }
        if let Some((from, to, start)) = self.glide {
            let t = eased(ctx, start, GLIDE);
            self.center = [lerp(from.0[0], to.0[0], t), lerp(from.0[1], to.0[1], t)];
            // The scale by ratio: each moment zooms as much as the last.
            self.um_per_pt = from.1 * (to.1 / from.1).powf(t);
            if t >= 1.0 {
                self.glide = None;
            }
        }
        let found = if self.sch_view() { self.sch.as_ref().map_or(&[][..], |s| &s.found) } else { &self.found };
        if self.pinged.0 != found {
            self.pinged = (found.to_vec(), web_time::Instant::now());
        }
    }

    /// What is picked: the item selected, else the items picked together.
    pub(crate) fn picks(&self) -> &[Pick] {
        self.selected.as_ref().map_or(&self.picked, std::slice::from_ref)
    }

    /// What a click at `at` (within `tol`) picks on the layers shown: the pad (its part), via or
    /// track on top (none a plane above hides: what shows is what a click takes), else the
    /// board's own text or drawing there (small in a part's box), else a part's text there (its
    /// reference, its value: picked alone), else the part whose box holds it, else the zone there
    /// (by fill or outline).
    pub(crate) fn pick_at(&self, at: Pt, tol: Nm) -> Option<Pick> {
        let (b, layers) = (&self.board, self.shown_copper());
        match b.copper_shown_at(at, tol, &layers) {
            Some(Item::Pad(i)) => Some(Pick::Footprint(b.pads[i].footprint as usize)),
            Some(Item::Track(i)) => Some(Pick::Track(i)),
            Some(Item::Via(i)) => Some(Pick::Via(i)),
            None => (b.drawing_at(at, tol, self.shown_layers()))
                .or_else(|| b.part_text_at(at, tol, self.shown_layers()))
                .or_else(|| b.footprint_at(at, tol, self.flipped).map(Pick::Footprint))
                .or_else(|| b.zone_at(at, tol, &layers).map(Pick::Zone)),
        }
    }
}

/// Picks as `App::selected` and `App::picked` hold them: one item selected, else several picked.
pub(crate) fn one_or_more(picks: Vec<Pick>) -> (Option<Pick>, Vec<Pick>) {
    if let [one] = picks[..] { (Some(one), Vec::new()) } else { (None, picks) }
}

/// Zooms a view's centre and scale by the wheel's turn this frame around the pointer at `pos`:
/// the point under it stays put (`to_um` maps a screen point to the scene with a centre and scale).
pub(crate) fn zoom_at(
    ui: &egui::Ui,
    pos: egui::Pos2,
    (center, scale): (&mut [f32; 2], &mut f32),
    to_um: impl Fn(egui::Pos2, [f32; 2], f32) -> [f32; 2],
) {
    // Two fingers: a pinch zooms about the point between them, which the view follows as they slide.
    let touch = ui.input(|i| i.multi_touch());
    let (pos, by) = touch.map_or((pos, 0.8f32.powf(wheel_notches(ui))), |t| (t.center_pos, 1.0 / t.zoom_delta));
    let slid = touch.map_or(egui::Vec2::ZERO, |t| t.translation_delta);
    if by != 1.0 || slid != egui::Vec2::ZERO {
        let before = to_um(pos - slid, *center, *scale);
        *scale = (*scale * by).clamp(0.01, 1e5);
        let after = to_um(pos, *center, *scale);
        *center = [center[0] + before[0] - after[0], center[1] + before[1] - after[1]];
    }
}

/// How far the wheel turned this frame, in notches (touchpads scroll in points).
pub(crate) fn wheel_notches(ui: &egui::Ui) -> f32 {
    ui.input(|i| {
        i.events
            .iter()
            .map(|e| match e {
                egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, .. } => delta.y / 50.0,
                egui::Event::MouseWheel { delta, .. } => delta.y,
                _ => 0.0,
            })
            .sum()
    })
}

/// Paints the visible slots of the scene uploaded into egui's render pass.
pub(crate) struct Paint(pub(crate) View, pub(crate) Vec<bool>);

impl egui_wgpu::CallbackTrait for Paint {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(r) = resources.get::<Renderer>() {
            r.set_view(queue, &self.0);
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        if let Some(r) = resources.get::<Renderer>() {
            r.draw(pass, &self.1, &self.0);
        }
    }
}
