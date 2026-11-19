//! Cuprita's look (`BRAND.md`): graphite surfaces, one copper accent, signal cyan for what the
//! canvas points at; Inter for words, JetBrains Mono for numbers, Phosphor icons (`icon.rs`).

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, vec2};
use std::sync::Arc;

const fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Behind the board and the sheets.
pub(crate) const CANVAS: Color32 = rgb(0x0B0E12);
/// The top bar, the status bar and pages such as Home.
pub(crate) const GROUND: Color32 = rgb(0x101318);
/// Docks beside the canvas.
pub(crate) const PANEL: Color32 = rgb(0x13161B);
/// Fields, cards, popups.
pub(crate) const RAISED: Color32 = rgb(0x1B2027);
pub(crate) const HOVER: Color32 = rgb(0x262C35);
/// Hairlines between areas.
pub(crate) const LINE: Color32 = rgb(0x1F242C);
pub(crate) const TEXT: Color32 = rgb(0xE8EBEF);
/// Labels and most of what panels say.
pub(crate) const TEXT_2: Color32 = rgb(0xC9CFD8);
/// Captions, hints, what is off.
pub(crate) const QUIET: Color32 = rgb(0x8C95A2);
/// The accent: the active tool, the main action, focus.
pub(crate) const COPPER: Color32 = rgb(0xE8864F);
/// Text on copper.
pub(crate) const ON_COPPER: Color32 = rgb(0x14100C);
/// What the canvas selects, snaps to or measures.
pub(crate) const SIGNAL: Color32 = rgb(0x3FD0F0);
pub(crate) const PASS: Color32 = rgb(0x3DBE8B);
pub(crate) const WARN: Color32 = rgb(0xF0B44C);
pub(crate) const FAIL: Color32 = rgb(0xF25F5C);

/// Whether the schematic shows on a dark page (Settings; `sheet.rs` `page_scene`), as the rest of
/// the window, else on KiCad's light paper.
static DARK_SHEET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub(crate) fn dark_sheet() -> bool {
    DARK_SHEET.load(std::sync::atomic::Ordering::Relaxed)
}

/// Shows sheets on a dark page or not, kept for the next run (`sheet`).
pub(crate) fn set_dark_sheet(dark: bool) {
    DARK_SHEET.store(dark, std::sync::atomic::Ordering::Relaxed);
    crate::recent::keep("sheet", if dark { "dark" } else { "light" });
}

/// SemiBold Inter, for titles and what must stand out.
pub(crate) fn strong() -> FontFamily {
    FontFamily::Name("strong".into())
}

/// `c` laid at `alpha` over `under`: an opaque tint (a wash of copper on a panel).
pub(crate) fn tint(under: Color32, c: Color32, alpha: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * alpha).round() as u8;
    Color32::from_rgb(mix(under.r(), c.r()), mix(under.g(), c.g()), mix(under.b(), c.b()))
}

fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::empty();
    let font = |bytes: &'static [u8]| Arc::new(FontData::from_static(bytes));
    for (name, bytes) in [
        ("Inter", &include_bytes!("../assets/Inter-Medium.ttf")[..]),
        ("Inter SemiBold", include_bytes!("../assets/Inter-SemiBold.ttf")),
        ("JetBrains Mono", include_bytes!("../assets/JetBrainsMono-Regular.ttf")),
        ("Phosphor", include_bytes!("../assets/Phosphor.ttf")),
    ] {
        f.font_data.insert(name.into(), font(bytes));
    }
    // The icons come last: a text face has every glyph a text needs before them.
    let family = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
    f.families.insert(FontFamily::Proportional, family(&["Inter", "Phosphor"]));
    f.families.insert(strong(), family(&["Inter SemiBold", "Phosphor"]));
    f.families.insert(FontFamily::Monospace, family(&["JetBrains Mono", "Inter", "Phosphor"]));
    f
}

/// Sets Cuprita's fonts and style on `ctx`.
pub(crate) fn apply(ctx: &egui::Context) {
    DARK_SHEET.store(crate::recent::kept("sheet").as_deref() != Some("light"), std::sync::atomic::Ordering::Relaxed);
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
            (TextStyle::Heading, FontId::new(17.0, strong())),
        ]
        .into();
        let sp = &mut s.spacing;
        (sp.item_spacing, sp.button_padding, sp.interact_size) = (vec2(8.0, 6.0), vec2(10.0, 5.0), vec2(24.0, 26.0));
        (sp.window_margin, sp.menu_margin, sp.indent) = (Margin::same(14), Margin::same(6), 16.0);
        sp.scroll = egui::style::ScrollStyle::floating();
        let v = &mut s.visuals;
        (v.panel_fill, v.window_fill, v.extreme_bg_color, v.faint_bg_color) = (PANEL, RAISED, rgb(0x0F1216), rgb(0x171B21));
        (v.code_bg_color, v.hyperlink_color, v.warn_fg_color, v.error_fg_color) = (RAISED, rgb(0xF0A070), WARN, FAIL);
        (v.window_stroke, v.window_corner_radius, v.menu_corner_radius) =
            (Stroke::new(1.0, rgb(0x2C333D)), CornerRadius::same(12), CornerRadius::same(10));
        v.window_shadow = Shadow { offset: [0, 14], blur: 36, spread: 0, color: Color32::from_black_alpha(130) };
        v.popup_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(110) };
        v.weak_text_color = Some(QUIET);
        // A selected row or tool: a wash of copper, its text in light copper.
        v.selection.bg_fill = tint(PANEL, COPPER, 0.2);
        v.selection.stroke = Stroke::new(1.0, rgb(0xF6C29E));
        let w = &mut v.widgets;
        for (state, fill, line, text) in [
            (&mut w.noninteractive, PANEL, LINE, TEXT_2),
            (&mut w.inactive, rgb(0x1F242C), rgb(0x2E353F), TEXT),
            (&mut w.hovered, HOVER, rgb(0x3A424E), TEXT),
            (&mut w.active, rgb(0x2C333D), rgb(0x4A525E), TEXT),
            (&mut w.open, rgb(0x2C333D), rgb(0x3A424E), TEXT),
        ] {
            (state.bg_fill, state.weak_bg_fill, state.bg_stroke) = (fill, fill, Stroke::new(1.0, line));
            (state.fg_stroke, state.corner_radius, state.expansion) = (Stroke::new(1.0, text), CornerRadius::same(7), 0.0);
        }
    });
}

/// Cuprita's mark, a routed trace drawing a C that ends in two vias, in the square `rect`:
/// `drawn` of it, 0 to 1, as a route lays it (from its top via round to its bottom one), which
/// is how it shows when the app starts.
pub(crate) fn mark(painter: &egui::Painter, rect: egui::Rect, color: Color32, drawn: f32) {
    let at = |x: f32, y: f32| rect.min + vec2(x, y) * (rect.width() / 64.0);
    let stroke = Stroke::new(rect.width() * 6.5 / 64.0, color);
    let trace = [at(45.0, 15.0), at(27.5, 15.0), at(19.0, 23.5), at(19.0, 40.5), at(27.5, 49.0), at(45.0, 49.0)];
    let long = |pts: &[egui::Pos2]| pts.windows(2).map(|ab| ab[0].distance(ab[1])).sum::<f32>();
    // The start of `pts`, `reach` long.
    let laid = |pts: &[egui::Pos2], mut reach: f32| {
        let mut out = vec![pts[0]];
        for ab in pts.windows(2) {
            let step = ab[0].distance(ab[1]);
            out.push(ab[0].lerp(ab[1], (reach / step).clamp(0.0, 1.0)));
            reach -= step;
            if reach <= 0.0 {
                break;
            }
        }
        out
    };
    let reach = drawn.clamp(0.0, 1.0) * long(&trace);
    painter.add(egui::Shape::line(laid(&trace, reach), stroke));
    // Each via once the copper reaches it.
    for ((x, y), from) in [((46.0, 15.0), 0.0), ((46.0, 49.0), long(&trace))] {
        if reach >= from - 0.01 {
            painter.circle_filled(at(x, y), rect.width() * 6.5 / 64.0, color);
        }
    }
}

/// A workbench tab, `width` wide: its icon and name, raised when it is the one shown (the header
/// draws the copper line under it, which glides from tab to tab).
pub(crate) fn tab(ui: &mut egui::Ui, shown: bool, icon: &str, name: &str, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), egui::Sense::click());
    let p = ui.painter();
    if shown || resp.hovered() {
        p.rect_filled(rect, 7.0, if shown { HOVER } else { RAISED });
    }
    let color = if shown || resp.hovered() { TEXT } else { QUIET };
    let icon = p.layout_no_wrap(icon.into(), FontId::proportional(15.0), if shown { COPPER } else { color });
    let name = p.layout_no_wrap(name.into(), FontId::new(13.0, strong()), color);
    // An icon alone (a narrow window's tab) sits in the middle.
    let gap = if name.is_empty() { 0.0 } else { 7.0 };
    let x = rect.center().x - (icon.size().x + gap + name.size().x) / 2.0;
    let left = |w: f32, h: f32| egui::pos2(x + w, rect.center().y - h / 2.0);
    p.galley(left(icon.size().x + gap, name.size().y), name, color);
    p.galley(left(0.0, icon.size().y), icon, color);
    resp
}

/// The main action's button: copper, its text dark.
pub(crate) fn primary(text: &str) -> egui::Button<'static> {
    let text = egui::RichText::new(text.to_string()).family(strong()).color(ON_COPPER);
    egui::Button::new(text).fill(COPPER).stroke(Stroke::NONE).corner_radius(8).min_size(vec2(0.0, 32.0))
}

/// A dock's tab: its icon and name, underlined in copper while shown.
pub(crate) fn dock_tab(ui: &mut egui::Ui, shown: bool, icon: &str, name: &str, count: Option<(usize, Color32)>) -> egui::Response {
    let color = if shown { TEXT } else { QUIET };
    let text = format!("{icon}  {name}");
    let galley = ui.painter().layout_no_wrap(text, FontId::new(12.5, strong()), color);
    // A count after the name, in its own colour (Checks: what is left to fix).
    let badge = count.map(|(n, c)| ui.painter().layout_no_wrap(n.to_string(), FontId::new(11.0, strong()), c));
    let width = galley.size().x + badge.as_ref().map_or(0.0, |b| b.size().x + 6.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 38.0), egui::Sense::click());
    let color = if resp.hovered() { TEXT } else { color };
    if let Some(b) = badge {
        let at = rect.left_center() + vec2(galley.size().x + 6.0, -b.size().y / 2.0);
        ui.painter().galley(at, b, Color32::PLACEHOLDER);
    }
    ui.painter().galley(rect.left_center() - vec2(0.0, galley.size().y / 2.0), galley, color);
    if shown {
        ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, COPPER));
    }
    resp
}

/// A square button holding an icon, `size` across: a copper wash while `on`, grey while not
/// `enabled`; `key` in a corner names its key.
pub(crate) fn tool(ui: &mut egui::Ui, icon: &str, size: f32, on: bool, enabled: bool, key: &str) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), sense);
    let p = ui.painter();
    // The copper wash of the tool on comes and goes, a quick move.
    let wash = crate::motion::on(ui.ctx(), resp.id.with("on"), on);
    if enabled && resp.hovered() && !on {
        p.rect_filled(rect, 8.0, HOVER);
    }
    if wash > 0.0 {
        p.rect_filled(rect, 8.0, tint(RAISED, COPPER, 0.18).gamma_multiply(wash));
    }
    let color = match (on, enabled) {
        (true, _) => COPPER,
        (_, true) => TEXT_2,
        _ => QUIET.gamma_multiply(0.6),
    };
    p.text(rect.center(), egui::Align2::CENTER_CENTER, icon, FontId::proportional(size * 0.48), color);
    if !key.is_empty() {
        let at = rect.right_bottom() - vec2(4.0, 2.0);
        p.text(at, egui::Align2::RIGHT_BOTTOM, key, FontId::monospace(9.0), if on { COPPER } else { QUIET });
    }
    resp
}

/// The window icon: the mark on copper (`assets/cuprita.svg`, as PNG).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn window_icon() -> egui::IconData {
    let mut png = png::Decoder::new(std::io::Cursor::new(&include_bytes!("../assets/cuprita.png")[..])).read_info().expect("icon PNG");
    let mut rgba = vec![0; png.output_buffer_size().expect("icon size")];
    let info = png.next_frame(&mut rgba).expect("icon PNG");
    egui::IconData { rgba, width: info.width, height: info.height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tints_toward_the_colour() {
        assert_eq!(tint(rgb(0x000000), rgb(0xFF8040), 0.5), rgb(0x804020));
        assert_eq!(tint(PANEL, COPPER, 0.0), PANEL);
    }

    #[test]
    fn has_a_glyph_for_every_icon_and_sign_shown() {
        let ctx = egui::Context::default();
        apply(&ctx);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear(); // no GPU to hand the font atlas to
        let icons: String = include_str!("icon.rs")
            .split("\\u{")
            .skip(1)
            .filter_map(|s| char::from_u32(u32::from_str_radix(&s[..s.find('}')?], 16).ok()?))
            .collect();
        // Every sign the app's and core's texts use.
        let signs = "…°·µΩ²é→—–×π↑↓±μü∞−";
        ctx.fonts_mut(|f| {
            for font in [FontId::proportional(13.0), FontId::new(13.0, strong()), FontId::monospace(12.5)] {
                assert!(f.has_glyphs(&font, signs) && f.has_glyphs(&font, &icons), "{font:?}");
            }
        });
        assert_eq!(icons.chars().count(), include_str!("icon.rs").matches("pub(crate) const").count());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn reads_the_window_icon() {
        assert_eq!(window_icon().rgba.len(), 256 * 256 * 4);
    }
}
