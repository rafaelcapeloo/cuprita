//! The board's own texts and drawings (`Board::own`): the words and lines of its silkscreen, the
//! notes on its user layers. Added and set here, each edit returning the patch that undoes it;
//! picked, they move, turn and go as parts do (`picked.rs`), and save as KiCad writes them
//! (`save.rs`, `crates/core/drawing.md`).

use crate::board::{Board, Geom, LayerId, Nm, Pt, Shape, Text, upright};
use crate::history::{Patch, Pick};

/// What a new line or text gets on a layer: the line's width, the text's glyph width and
/// height, and its pen.
pub type Ink = (Nm, (Nm, Nm), Nm);

impl Board {
    /// The layers a text or drawing may go on: all but the board's edge (the Outline tool's), its
    /// margin, the parts' courtyards and copper.
    // shortcut: no text or line on copper, which DRC and zone fills don't take them on; let
    // copper in once they do.
    pub fn drawing_layers(&self) -> Vec<LayerId> {
        let free = |name: &str| !["Edge.Cuts", "Margin"].contains(&name) && !name.ends_with(".CrtYd");
        (0..self.layers.len()).filter(|&l| self.copper >> l & 1 == 0 && free(&self.layers[l])).map(|l| l as LayerId).collect()
    }

    /// What the project draws new lines and texts with on `layer`: the silkscreen's, else the rest's.
    pub fn ink(&self, layer: LayerId) -> Ink {
        let silk = self.layers.get(layer as usize).is_some_and(|l| l.ends_with(".SilkS"));
        if silk { self.rules.silk_ink } else { self.rules.other_ink }
    }

    /// A text reading `words` at `at` on `layer`, as large as the project's new texts there,
    /// mirrored on a back layer so it reads from that side, as KiCad puts it.
    pub fn new_text(&self, words: &str, at: Pt, layer: LayerId) -> Text {
        let ((_, size, thickness), back) = (self.ink(layer), self.layers.get(layer as usize).is_some_and(|l| l.starts_with("B.")));
        Text { text: words.into(), at, size, thickness, layer, mirror: back, typed: true, ..Text::default() }
    }

    /// Adds `t` as the board's own text, after its last; returns its pick and the patch that
    /// undoes it.
    pub fn add_text(&mut self, t: Text) -> (Pick, Patch) {
        let undo = Patch::added(self);
        self.texts.push(Text { src: None, upright: false, typed: true, ..t });
        (Pick::Text(self.texts.len() - 1), undo)
    }

    /// Puts `to` in place of text `i` (the board's own, or a part's: its place, angle, size and
    /// pen), its words typed when they changed, a part's locked one kept upright; returns the
    /// patch that undoes it.
    pub fn set_text(&mut self, i: usize, to: Text) -> Patch {
        let (undo, was) = (Patch::item(self, Pick::Text(i)), &self.texts[i]);
        let angle = if was.upright { upright(to.angle) } else { to.angle };
        self.texts[i] = Text { src: was.src, typed: was.typed || was.text != to.text, angle, ..to };
        undo
    }

    /// Adds lines `width` wide on `layer` from each of `corners` to the next, and back to the
    /// first when `closed`, as the board's own drawings after its last; returns their picks and
    /// the patch that undoes it.
    pub fn add_lines(&mut self, corners: &[Pt], closed: bool, layer: LayerId, width: Nm) -> (Vec<Pick>, Patch) {
        let (undo, from) = (Patch::added(self), self.shapes.len());
        let n = corners.len().saturating_sub(usize::from(!closed || corners.len() < 3));
        let ends = corners.iter().zip(corners.iter().cycle().skip(1)).take(n);
        self.shapes.extend(ends.filter(|(a, b)| a != b).map(|(&a, &b)| Shape {
            geom: Geom::Segment { a, b },
            width,
            filled: false,
            layer,
        }));
        ((from..self.shapes.len()).map(Pick::Shape).collect(), undo)
    }

    /// Puts `to` in place of the board's own drawing `i`; returns the patch that undoes it.
    pub fn set_shape(&mut self, i: usize, to: Shape) -> Patch {
        let undo = Patch::item(self, Pick::Shape(i));
        self.shapes[i] = to;
        undo
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::MM;
    use crate::history::History;

    /// Two parts with a note between them in the file, the board's edge and silkscreen after.
    const BOARD: &str = r#"(kicad_pcb
	(version 20240108)
	(generator "pcbnew")
	(layers
		(0 "F.Cu" signal)
		(31 "B.Cu" signal)
		(36 "B.SilkS" user "B.Silkscreen")
		(37 "F.SilkS" user "F.Silkscreen")
		(40 "Dwgs.User" user "User.Drawings")
		(44 "Edge.Cuts" user)
		(46 "B.CrtYd" user "B.Courtyard")
	)
	(net 0 "")
	(footprint "R"
		(layer "F.Cu")
		(at 10 10)
		(property "Reference" "R1"
			(at 0 -1 0)
			(layer "F.SilkS")
		)
		(fp_line
			(start -1 -1)
			(end 1 -1)
			(stroke
				(width 0.1)
				(type solid)
			)
			(layer "F.SilkS")
		)
	)
	(gr_text "rev ${REVISION}"
		(at 5 5 0)
		(layer "F.SilkS")
		(uuid "11111111-1111-4111-8111-111111111111")
		(effects
			(font
				(face "Arial")
				(size 1.5 1.2)
				(thickness 0.2)
			)
			(justify left bottom)
		)
	)
	(footprint "C"
		(layer "F.Cu")
		(at 20 10)
		(property "Reference" "C1"
			(at 0 -1 0)
			(layer "F.SilkS")
			(hide yes)
		)
		(fp_line
			(start -1 1)
			(end 1 1)
			(stroke
				(width 0.1)
				(type solid)
			)
			(layer "F.SilkS")
		)
	)
	(gr_text "hidden"
		(at 1 1 0)
		(layer "F.SilkS")
		(hide yes)
	)
	(gr_text "Made by hand"
		(at 15 20 90)
		(layer "F.SilkS")
		(uuid "22222222-2222-4222-8222-222222222222")
		(effects
			(font
				(size 1 1)
				(thickness 0.15)
			)
		)
	)
	(gr_circle
		(center 30 30)
		(end 32 30)
		(stroke
			(width 0.1)
			(type solid)
		)
		(fill none)
		(layer "F.SilkS")
		(uuid "33333333-3333-4333-8333-333333333333")
	)
	(gr_arc
		(start 40 30)
		(mid 41.414214 33.414214)
		(end 38 32)
		(stroke
			(width 0.1)
			(type solid)
		)
		(layer "Dwgs.User")
		(uuid "44444444-4444-4444-8444-444444444444")
	)
)
"#;

    fn pt(x: f64, y: f64) -> Pt {
        Pt { x: (x * 1e6).round() as Nm, y: (y * 1e6).round() as Nm }
    }

    fn layer(b: &Board, name: &str) -> LayerId {
        b.layers.iter().position(|l| l == name).unwrap() as LayerId
    }

    /// The parts' items, by their references.
    fn parts(b: &Board) -> Vec<(String, Vec<Shape>, Vec<Text>)> {
        b.footprints.iter().map(|f| (f.reference.clone(), b.shapes[f.shapes.clone()].to_vec(), b.texts[f.texts.clone()].to_vec())).collect()
    }

    #[test]
    fn the_boards_own_texts_and_drawings_are_those_outside_its_parts() {
        let mut b = Board::parse(BOARD).unwrap();
        // R1's line and reference, the note, C1's line (its reference hidden), then the rest.
        assert_eq!(b.own(), (vec![2, 3], vec![1, 2]));
        assert_eq!(
            (b.texts[1].text.as_str(), b.texts[1].src, b.texts[2].src, b.texts[1].rough),
            ("rev ${REVISION}", Some(0), Some(1), true)
        );
        let names = |b: &Board| b.drawing_layers().into_iter().map(|l| b.layers[l as usize].clone()).collect::<Vec<_>>();
        assert_eq!(names(&b), ["B.SilkS", "F.SilkS", "Dwgs.User"]);
        // A text is found by its box, a drawing by its line, on the layers shown.
        let (silk, all) = (1u128 << layer(&b, "F.SilkS"), u128::MAX);
        assert_eq!(b.drawing_at(pt(15.0, 18.0), 50_000, all), Some(Pick::Text(2)));
        assert_eq!(b.drawing_at(pt(32.0, 30.02), 50_000, all), Some(Pick::Shape(2)));
        assert_eq!((b.drawing_at(pt(30.0, 30.0), 50_000, all), b.drawing_at(pt(10.0, 9.0), 50_000, all)), (None, None));
        // A part's text is found apart, on the layers shown, and knows its part.
        let part_text = |x, shown| b.part_text_at(pt(x, 9.0), 50_000, shown);
        assert_eq!([part_text(10.0, all), part_text(10.0, 1), part_text(15.0, all)], [Some(Pick::Text(0)), None, None]);
        assert_eq!([0, 1, 2, 3].map(|i| b.text_part(i)), [Some(0), None, None, None]);
        assert_eq!((b.drawing_at(pt(40.0, 30.0), 50_000, silk), b.drawing_at(pt(40.0, 30.0), 50_000, all)), (None, Some(Pick::Shape(3))));
        // A box takes them with the rest; the project's defaults size what is new.
        let taken = b.items_in(pt(0.0, 0.0), pt(50.0, 50.0), false, &[], all);
        assert_eq!(taken, [Pick::Footprint(0), Pick::Footprint(1), Pick::Text(1), Pick::Text(2), Pick::Shape(2), Pick::Shape(3)]);
        assert_eq!(b.items_in(pt(0.0, 0.0), pt(50.0, 50.0), false, &[], silk).len(), 5);
        b.apply_pro(r#"{"board": {"design_settings": {"defaults": {"silk_line_width": 0.12, "silk_text_size_h": 0.8, "silk_text_size_v": 0.9, "silk_text_thickness": 0.1}}}}"#);
        let (front, back, notes) = (layer(&b, "F.SilkS"), layer(&b, "B.SilkS"), layer(&b, "Dwgs.User"));
        assert_eq!((b.ink(front), b.ink(notes)), ((120_000, (800_000, 900_000), 100_000), (100_000, (MM, MM), 150_000)));
        let t = b.new_text("J1", pt(1.0, 2.0), back);
        assert_eq!(
            (t.size, t.thickness, t.mirror, b.new_text("J1", pt(1.0, 2.0), front).mirror),
            ((800_000, 900_000), 100_000, true, false)
        );
    }

    #[test]
    fn texts_and_lines_come_and_go_among_the_parts_items_and_undo() {
        let (orig, mut b, mut h) = (Board::parse(BOARD).unwrap(), Board::parse(BOARD).unwrap(), History::default());
        let silk = layer(&b, "F.SilkS");
        let (text, undo) = b.add_text(b.new_text("v1.0", pt(3.0, 3.0), silk));
        h.push(undo, "text".into());
        let (lines, undo) = b.add_lines(&[pt(0.0, 0.0), pt(5.0, 0.0), pt(5.0, 5.0), pt(5.0, 5.0)], true, silk, 150_000);
        h.push(undo, "lines".into());
        // A corner clicked twice draws no line; closed, the last corner joins the first.
        assert_eq!((text, lines.as_slice()), (Pick::Text(3), &[Pick::Shape(4), Pick::Shape(5), Pick::Shape(6)][..]));
        assert_eq!(
            (
                b.clone().add_lines(&[pt(0.0, 0.0), pt(1.0, 0.0)], true, silk, 150_000).0.len(),
                b.clone().add_lines(&[], false, silk, 1).0.len()
            ),
            (1, 0)
        );
        // The note between the parts goes with R1 and the first line: C1 keeps its own line.
        let before = parts(&b);
        h.push(b.delete_items(&[Pick::Text(1), Pick::Footprint(0), Pick::Shape(4)]), "deleted".into());
        assert_eq!((parts(&b), b.own()), (before[1..].to_vec(), (vec![1, 2, 3, 4], vec![0, 1])));
        assert_eq!((b.texts[0].text.as_str(), b.texts[1].text.as_str()), ("Made by hand", "v1.0"));
        // Retyped, moved and turned, a text keeps its place in the file; only words typed count.
        let moved = b.set_text(0, Text { text: "Made by hand".into(), at: pt(16.0, 21.0), ..b.texts[0].clone() });
        assert_eq!((b.texts[0].typed, b.texts[0].src), (false, Some(1)));
        h.push(moved, "moved".into());
        h.push(b.set_text(0, Text { text: "Made\nby hand".into(), ..b.texts[0].clone() }), "typed".into());
        assert_eq!((b.texts[0].typed, b.texts[0].src), (true, Some(1)));
        h.push(b.turn_items(&[Pick::Text(0), Pick::Shape(1), Pick::Shape(2)], pt(30.0, 30.0), 90.0), "turned".into());
        assert_eq!((b.texts[0].at, b.texts[0].angle), (pt(21.0, 44.0), 180.0));
        let Geom::Arc { start, end, .. } = b.shapes[2].geom else { panic!() };
        assert_eq!(
            (b.shapes[1].geom.clone(), start, end),
            (Geom::Circle { center: pt(30.0, 30.0), radius: 2 * MM }, pt(30.0, 20.0), pt(32.0, 22.0))
        );
        for _ in 0..6 {
            h.undo(&mut b);
        }
        assert_eq!((parts(&b), &b.shapes, &b.texts), (parts(&orig), &orig.shapes, &orig.texts));
    }

    #[test]
    fn a_parts_text_moves_turns_and_grows_alone_and_delete_hides_it() {
        let (orig, mut b, mut h) = (Board::parse(BOARD).unwrap(), Board::parse(BOARD).unwrap(), History::default());
        // Picked with its part it goes as the part does, once; alone, the part stays.
        h.push(b.move_items(&[Pick::Footprint(0), Pick::Text(0)], pt(1.0, 0.0)), "both".into());
        assert_eq!((b.texts[0].at, b.footprints[0].at), (pt(11.0, 9.0), pt(11.0, 10.0)));
        h.push(b.move_items(&[Pick::Text(0)], pt(0.5, -1.0)), "moved".into());
        assert_eq!((b.texts[0].at, b.footprints[0].at), (pt(11.5, 8.0), pt(11.0, 10.0)));
        // Locked as KiCad's are, it stays upright: a quarter turn, then level again.
        h.push(b.turn_items(&[Pick::Text(0)], pt(11.5, 8.0), 90.0), "turned".into());
        let mut level = b.clone();
        level.turn_items(&[Pick::Text(0)], pt(11.5, 8.0), 90.0);
        assert_eq!((b.texts[0].upright, b.texts[0].angle, level.texts[0].angle), (true, 90.0, 0.0));
        h.push(b.set_text(0, Text { size: (800_000, 900_000), thickness: 120_000, ..b.texts[0].clone() }), "sized".into());
        // Saved in the part, where the part now is; read back the same.
        let out = b.write(BOARD).unwrap();
        let saved = "\t\t(property \"Reference\" \"R1\"\n\t\t\t(at 0.5 -2 90)\n\t\t\t(layer \"F.SilkS\") (effects (font (size 0.9 0.8) (thickness 0.12)))\n\t\t)\n";
        assert!(out.contains(saved) && out.contains("(at 11 10)"), "{out}");
        assert_eq!((parts(&Board::parse(&out).unwrap()), out.matches("(hide yes)").count()), (parts(&b), 2));
        // Deleted with the note and the other part: the part keeps it, hidden, and saves it so.
        h.push(b.delete_items(&[Pick::Text(0), Pick::Text(1), Pick::Footprint(1)]), "hidden".into());
        assert_eq!(
            (b.footprints.len(), b.footprints[0].texts.clone(), b.footprints[0].hidden.iter().map(|t| t.src).collect::<Vec<_>>(), b.own()),
            (1, 0..0, vec![Some(0)], (vec![1, 2], vec![0]))
        );
        let out = b.write(BOARD).unwrap();
        assert!(out.contains("(property \"Reference\" \"R1\"\n\t\t\t(at 0 -1 0)\n\t\t\t(layer \"F.SilkS\")\n\t\t\t(hide yes)\n"), "{out}");
        assert_eq!((parts(&Board::parse(&out).unwrap()), out.matches("(gr_text").count()), (parts(&b), 2));
        for _ in 0..5 {
            h.undo(&mut b);
        }
        assert_eq!((parts(&b), &b.shapes, &b.texts, b.write(BOARD).unwrap().as_str()), (parts(&orig), &orig.shapes, &orig.texts, BOARD));
    }

    #[test]
    fn a_parts_hidden_text_is_kept_and_shown_again_saved_so() {
        // C1's reference, hidden in the file, is kept where it would be, and follows its part.
        let (mut b, mut h) = (Board::parse(BOARD).unwrap(), History::default());
        assert_eq!(
            (b.footprints[1].hidden.len(), b.footprints[1].hidden[0].text.as_str(), b.footprints[1].hidden[0].src),
            (1, "C1", Some(0))
        );
        h.push(b.move_items(&[Pick::Footprint(1)], pt(1.0, 0.0)), "moved".into());
        assert_eq!((b.footprints[1].hidden[0].at, b.write(BOARD).unwrap().matches("(hide yes)").count()), (pt(21.0, 9.0), 2));
        // Shown again, it is the part's last text, picked; its `(hide yes)` leaves the file.
        let (pick, undo) = b.show_text(1, 0);
        h.push(undo, "shown".into());
        assert_eq!((pick, b.footprints[1].texts.clone(), b.text_part(2), b.footprints[1].hidden.len()), (Pick::Text(2), 2..3, Some(1), 0));
        let out = b.write(BOARD).unwrap();
        assert!(out.contains("(property \"Reference\" \"C1\"\n\t\t\t(at 0 -1 0)\n\t\t\t(layer \"F.SilkS\")\n\t\t)"), "{out}");
        assert_eq!((parts(&Board::parse(&out).unwrap()), out.matches("(hide yes)").count()), (parts(&b), 1));
        // Hidden again by Delete, it saves as the file had it; undone, byte for byte.
        h.push(b.delete_items(&[pick]), "hidden".into());
        assert_eq!(b.write(BOARD).unwrap().matches("(hide yes)").count(), 2);
        for _ in 0..3 {
            h.undo(&mut b);
        }
        assert_eq!(b.write(BOARD).unwrap(), BOARD);
        // KiCad 7 and before hide with a bare word, in the text or in its effects.
        let six = "(kicad_pcb (version 20211014) (generator pcbnew)\n  (layers (0 \"F.Cu\" signal) (37 \"F.SilkS\" user))\n  (net 0 \"\")\n  (footprint \"R\" (layer \"F.Cu\") (at 10 10)\n    (fp_text reference \"R1\" (at 0 -1) (layer \"F.SilkS\") hide\n      (effects (font (size 1 1) (thickness 0.15)))\n    )\n    (fp_text value \"1k\" (at 0 1) (layer \"F.SilkS\")\n      (effects (font (size 1 1) (thickness 0.15)) hide)\n    )\n  )\n)\n";
        let mut b = Board::parse(six).unwrap();
        assert_eq!((b.write(six).unwrap().as_str(), b.footprints[0].hidden.len()), (six, 2));
        b.show_text(0, 1);
        b.show_text(0, 0);
        let out = b.write(six).unwrap();
        assert_eq!(out, six.replace(" hide", ""));
        assert_eq!((Board::parse(&out).unwrap().texts.len(), b.texts.len()), (2, 2));
    }

    /// The board's own texts and drawings as a file gives them back: without where they came from.
    fn own(b: &Board) -> (Vec<Text>, Vec<Shape>) {
        let (shapes, texts) = b.own();
        let mut texts: Vec<Text> = texts.into_iter().map(|i| Text { src: None, typed: false, ..b.texts[i].clone() }).collect();
        texts.sort_by_key(|t| (t.at.x, t.at.y));
        let mut shapes: Vec<Shape> = shapes.into_iter().map(|i| b.shapes[i].clone()).collect();
        shapes.sort_by_key(|s| format!("{s:?}"));
        (texts, shapes)
    }

    #[test]
    fn saves_texts_where_they_are_and_what_the_board_gained_as_kicad_writes_it() {
        let mut b = Board::parse(BOARD).unwrap();
        assert_eq!(b.write(BOARD).unwrap(), BOARD);
        let (front, back) = (layer(&b, "F.SilkS"), layer(&b, "B.SilkS"));
        // The note retyped and put on the back, mirrored; the other text moved, turned and made
        // larger; the circle moved; a text and two lines new.
        b.set_text(1, Text { text: "rev 2".into(), layer: back, mirror: true, ..b.texts[1].clone() });
        b.set_text(2, Text { size: (2 * MM, 3 * MM), thickness: 300_000, ..b.texts[2].clone() });
        b.move_items(&[Pick::Text(2), Pick::Shape(2)], pt(1.0, 1.5));
        b.turn_items(&[Pick::Text(2)], pt(16.0, 21.5), -90.0);
        b.add_text(b.new_text("J1 \"in\"\n3V3", pt(7.0, 8.0), back));
        b.add_lines(&[pt(0.0, 0.0), pt(5.0, 0.0), pt(5.0, 2.5)], false, front, 150_000);
        let out = b.write(BOARD).unwrap();
        // Each text edited where it is: its id and face stay, the file's variable gives way to
        // the words typed.
        let note = "\t(gr_text \"rev 2\"\n\t\t(at 5 5 0)\n\t\t(layer \"B.SilkS\")\n\t\t(uuid \"11111111-1111-4111-8111-111111111111\")\n\t\t(effects\n\t\t\t(font\n\t\t\t\t(face \"Arial\")\n\t\t\t\t(size 1.5 1.2)\n\t\t\t\t(thickness 0.2)\n\t\t\t)\n\t\t\t(justify left bottom mirror)\n\t\t)\n\t)\n";
        let made = "\t(gr_text \"Made by hand\"\n\t\t(at 16 21.5 0)\n\t\t(layer \"F.SilkS\")\n\t\t(uuid \"22222222-2222-4222-8222-222222222222\")\n\t\t(effects\n\t\t\t(font\n\t\t\t\t(size 3 2)\n\t\t\t\t(thickness 0.3)\n\t\t\t)\n\t\t)\n\t)\n";
        assert!(out.contains(note) && out.contains(made), "{out}");
        // What is new, after the file's last drawing, as KiCad 8 writes it; the circle moved anew.
        let new = "\t(gr_text \"J1 \\\"in\\\"\\n3V3\"\n\t\t(at 7 8 0)\n\t\t(layer \"B.SilkS\")\n\t\t(effects\n\t\t\t(font\n\t\t\t\t(size 1 1)\n\t\t\t\t(thickness 0.15)\n\t\t\t)\n\t\t\t(justify mirror)\n\t\t)\n\t\t(uuid \"";
        let circle = "\t(gr_circle\n\t\t(center 31 31.5)\n\t\t(end 33 31.5)\n\t\t(stroke\n\t\t\t(width 0.1)\n\t\t\t(type default)\n\t\t)\n\t\t(fill none)\n\t\t(layer \"F.SilkS\")\n\t\t(uuid \"";
        assert!(out.contains(new) && out.contains(circle) && !out.contains("33333333"), "{out}");
        assert!(out.find("(gr_arc").unwrap() < out.find("(gr_circle").unwrap() && out.matches("(gr_line").count() == 2, "{out}");
        // Read back, the board has what was written; the hidden text stays in the file.
        let read = Board::parse(&out).unwrap();
        assert_eq!((own(&read), parts(&read)), (own(&b), parts(&b)));
        assert!(out.contains("(gr_text \"hidden\""));
        // Deleted, they leave the file; the parts' own stay.
        b.delete_items(&[Pick::Text(1), Pick::Text(2), Pick::Text(3), Pick::Shape(2), Pick::Shape(3)]);
        let out = b.write(BOARD).unwrap();
        assert_eq!((out.matches("(gr_text").count(), out.matches("(gr_arc").count(), out.matches("(fp_line").count()), (1, 0, 2), "{out}");
    }

    #[test]
    fn older_files_get_texts_and_drawings_as_their_kicad_wrote_them() {
        // KiCad 6: a line each, a locked text's flag before its words, arcs by three points.
        let six = r#"(kicad_pcb (version 20211014) (generator pcbnew)
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (37 "F.SilkS" user "F.Silkscreen"))
  (net 0 "")
  (gr_text locked "KEEP" (at 1 2) (layer "F.SilkS") (tstamp 0531e45d-5da3-40cb-95c7-f1053d99016a)
    (effects (font (size 1 1) (thickness 0.15)))
  )
  (gr_poly (pts (xy 0 0) (xy 4 0) (xy 4 3)) (layer "F.SilkS") (width 0.1) (fill solid) (tstamp 29d7b6ac-0000-4000-8000-000000000001))
)
"#;
        let mut b = Board::parse(six).unwrap();
        assert_eq!((b.texts[0].text.as_str(), b.write(six).unwrap().as_str()), ("KEEP", six));
        b.set_text(0, Text { text: "OUT".into(), angle: 90.0, mirror: true, ..b.texts[0].clone() });
        b.add_text(b.new_text("A", pt(3.0, 4.0), 2));
        b.move_items(&[Pick::Shape(0)], pt(1.0, 0.0));
        let out = b.write(six).unwrap();
        let kept = "  (gr_text locked \"OUT\" (at 1 2 90) (layer \"F.SilkS\") (tstamp 0531e45d-5da3-40cb-95c7-f1053d99016a)\n    (effects (justify mirror) (font (size 1 1) (thickness 0.15)))\n  )\n";
        let new = "  (gr_text \"A\" (at 3 4) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))) (tstamp ";
        let poly = "  (gr_poly (pts (xy 1 0) (xy 5 0) (xy 5 3)) (layer \"F.SilkS\") (width 0.1) (fill solid) (tstamp ";
        assert!(out.contains(kept) && out.contains(new) && out.contains(poly) && !out.contains("29d7b6ac"), "{out}");
        assert_eq!(own(&Board::parse(&out).unwrap()), own(&b));
        // KiCad 5: bare words, no ids, an arc by its centre, where it starts and its angle.
        let five = "(kicad_pcb (version 20171130) (host pcbnew 5.1.9)\n  (layers (0 F.Cu signal) (31 B.Cu signal) (37 F.SilkS user))\n  (net 0 \"\")\n  (gr_arc (start 10 10) (end 12 10) (angle 90) (layer F.SilkS) (width 0.12))\n)\n";
        let mut b = Board::parse(five).unwrap();
        b.move_items(&[Pick::Shape(0)], pt(0.5, -1.0));
        b.add_text(b.new_text("TP 1", pt(3.0, 4.0), 2));
        let out = b.write(five).unwrap();
        let arc = "  (gr_arc (start 10.5 9) (end 12.5 9) (angle 90) (layer F.SilkS) (width 0.12))\n";
        let text = "  (gr_text \"TP 1\" (at 3 4) (layer F.SilkS) (effects (font (size 1 1) (thickness 0.15))))\n";
        assert!(out.contains(arc) && out.contains(text), "{out}");
        let (read, Geom::Arc { start, mid, end }) = (Board::parse(&out).unwrap(), b.shapes[0].geom.clone()) else { panic!() };
        let Geom::Arc { start: s, mid: m, end: e } = read.shapes[0].geom else { panic!() };
        let near = |a: Pt, b: Pt| (a.x - b.x).abs() <= 2 && (a.y - b.y).abs() <= 2;
        assert!(near(s, start) && near(m, mid) && near(e, end), "{:?} {:?}", read.shapes[0].geom, b.shapes[0].geom);
    }
}
