//! The window's own title bar and edges on Wayland, where GNOME draws neither: dragging the
//! header moves the window and a double click on it maximizes it, the window's buttons end it
//! on the right, its edges resize it, and a hairline outlines it.

use crate::theme;
use crate::tr::tr;
use eframe::egui::{self, CursorIcon as C, Painter, Pos2, ResizeDirection as D, Stroke, ViewportCommand, vec2};

/// How far in from the window's side a press resizes it.
const EDGE: f32 = 5.0;
/// How far from a corner a press on a side takes that corner.
const CORNER: f32 = 16.0;

/// Whether the window draws its own title bar: on Wayland, where GNOME draws none (KDE would:
/// one look on both beats two).
pub(crate) fn own() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// The header behind its buttons, `bar` (sensing clicks and drags): a drag moves the window, a
/// double click maximizes or restores it.
pub(crate) fn title_bar(ctx: &egui::Context, bar: &egui::Response) {
    if bar.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized(ctx)));
    } else if bar.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// The window's buttons, laid right to left: close, maximize (restore), minimize, drawn as
/// GNOME draws them.
pub(crate) fn buttons(ui: &mut egui::Ui) {
    let max = maximized(ui.ctx());
    let button = |ui: &mut egui::Ui, tip: &str, glyph: &dyn Fn(&Painter, Pos2, Stroke)| {
        let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), egui::Sense::click());
        ui.painter().circle_filled(rect.center(), 11.0, if resp.hovered() { theme::HOVER } else { theme::RAISED });
        glyph(ui.painter(), rect.center(), Stroke::new(1.5, theme::TEXT_2));
        resp.on_hover_text(tip).clicked()
    };
    let d = 3.5;
    let square = |p: &Painter, c: Pos2, s| {
        p.rect_stroke(egui::Rect::from_center_size(c, vec2(2.0 * d, 2.0 * d)), 1.0, s, egui::StrokeKind::Middle);
    };
    if button(ui, tr("Close"), &|p, c, s| {
        p.line_segment([c + vec2(-d, -d), c + vec2(d, d)], s);
        p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], s);
    }) {
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
    let restore = |p: &Painter, c: Pos2, s| {
        square(p, c + vec2(-1.0, 1.0), s);
        p.line_segment([c + vec2(-d + 1.0, -d - 1.0), c + vec2(d + 1.0, -d - 1.0)], s);
        p.line_segment([c + vec2(d + 1.0, -d - 1.0), c + vec2(d + 1.0, d - 1.0)], s);
    };
    if button(ui, if max { tr("Restore") } else { tr("Maximize") }, if max { &restore } else { &square }) {
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
    }
    if button(ui, tr("Minimize"), &|p, c, s| {
        p.line_segment([c + vec2(-d, d), c + vec2(d, d)], s);
    }) {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

/// The window's edges, unless it is maximized: a hairline outlines it, the pointer near one
/// shows the way it resizes the window and a press there starts that. Called last in a frame,
/// so its pointer wins over what lies under it.
pub(crate) fn edges(ctx: &egui::Context) {
    if maximized(ctx) {
        return;
    }
    let r = ctx.content_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("window edge"));
    ctx.layer_painter(layer).rect_stroke(r, 0.0, (1.0, theme::LINE), egui::StrokeKind::Inside);
    let Some((way, icon)) = ctx.input(|i| i.pointer.hover_pos()).and_then(|p| way(r, p)) else { return };
    ctx.set_cursor_icon(icon);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(way));
    }
}

/// Which way a press at `p` resizes the window `r`, with the pointer that shows it; `None`
/// away from its edges.
fn way(r: egui::Rect, p: Pos2) -> Option<(D, C)> {
    let (west, east, north, south) = (p.x - r.left(), r.right() - p.x, p.y - r.top(), r.bottom() - p.y);
    // -1 within `near` of the first side, 1 of the second, else 0.
    let side = |a: f32, b: f32, near: f32| {
        if a < near {
            -1
        } else if b < near {
            1
        } else {
            0
        }
    };
    let (h, v) = if north.min(south) < EDGE {
        (side(west, east, CORNER), side(north, south, EDGE))
    } else if west.min(east) < EDGE {
        (side(west, east, EDGE), side(north, south, CORNER))
    } else {
        return None;
    };
    Some(match (h, v) {
        (-1, -1) => (D::NorthWest, C::ResizeNorthWest),
        (1, -1) => (D::NorthEast, C::ResizeNorthEast),
        (-1, 1) => (D::SouthWest, C::ResizeSouthWest),
        (1, 1) => (D::SouthEast, C::ResizeSouthEast),
        (0, -1) => (D::North, C::ResizeNorth),
        (0, 1) => (D::South, C::ResizeSouth),
        (-1, 0) => (D::West, C::ResizeWest),
        _ => (D::East, C::ResizeEast),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizes_by_the_side_or_the_corner_near_the_pointer() {
        let r = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let at = |x, y| way(r, egui::pos2(x, y)).map(|w| w.0);
        assert_eq!((at(400.0, 2.0), at(2.0, 300.0), at(400.0, 598.0)), (Some(D::North), Some(D::West), Some(D::South)));
        // Along a side, near the corner, the corner.
        assert_eq!((at(10.0, 2.0), at(797.0, 590.0)), (Some(D::NorthWest), Some(D::SouthEast)));
        assert_eq!(at(400.0, 300.0), None);
    }
}
