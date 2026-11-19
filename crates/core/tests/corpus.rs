//! Tests on the boards `corpus/manifest.tsv` lists (fetch with `corpus/fetch.sh`; skipped when absent).

use cuprita_core::board::{Board, Connect, NetId, Pt, Src, Via, Zone};
use cuprita_core::drc::{self, Item};
use cuprita_core::history::{Patch, Pick};
use cuprita_core::route::Router;
use cuprita_core::schematic::{Ink, LabelKind, Project, Schematic};
use cuprita_core::sexpr::{Lexer, Tok};
use i_overlay::core::{fill_rule::FillRule, overlay_rule::OverlayRule, point_location::IntPointContainment, single::SingleIntOverlay};
use i_overlay::i_float::int::point::IntPoint;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The corpus boards fetched: the ones the manifest lists, not other files beside them (the app's
/// `_autosave-` of a board edited in it).
fn boards() -> Vec<PathBuf> {
    fetched().into_iter().filter(|p| p.extension().is_some_and(|e| e == "kicad_pcb")).collect()
}

/// The manifest's files fetched: a board `name` is `boards/name.kicad_pcb`, a project's other
/// files `boards/project/file`.
fn fetched() -> Vec<PathBuf> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let manifest = std::fs::read_to_string(corpus.join("manifest.tsv")).unwrap();
    let names = manifest.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split('\t').next());
    let file = |name: &str| corpus.join("boards").join(if name.contains('/') { name.to_string() } else { format!("{name}.kicad_pcb") });
    names.map(file).filter(|p| p.exists()).collect()
}

#[test]
fn corpus_schematics_parse() {
    for path in fetched().into_iter().filter(|p| p.extension().is_some_and(|e| e == "kicad_sch")) {
        let (name, src) = (path.display(), std::fs::read_to_string(&path).unwrap());
        let pro =
            std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().find(|e| e.path().extension().is_some_and(|e| e == "kicad_pro"));
        let pro = pro.map(|e| std::fs::read_to_string(e.path()).unwrap()).unwrap_or_default();
        let s = Schematic::parse(&src, &pro).unwrap_or_else(|e| panic!("{name}: {e} at {:?}", e.line_col(&src)));
        // Independent oracle: the item keywords in the raw text.
        let shapes = |ink: Ink| s.shapes.iter().filter(move |sh| sh.layer == ink as u8);
        assert_eq!(s.symbols.len(), count(&src, "(lib_id"), "{name}");
        assert_eq!(s.sheets.len(), count(&src, "(sheet"), "{name}");
        assert_eq!(shapes(Ink::Wire).filter(|sh| sh.filled).count(), count(&src, "(junction"), "{name}");
        assert_eq!(shapes(Ink::NoConnect).count(), 2 * count(&src, "(no_connect"), "{name}");
        assert_eq!(shapes(Ink::Bus).count(), count(&src, "(bus"), "{name}");
        let wires = count(&src, "(wire") + count(&src, "(bus_entry");
        assert_eq!(shapes(Ink::Wire).filter(|sh| !sh.filled).count(), wires, "{name}");
    }
}

/// The corpus projects' schematics: each read from its root sheet with its `.kicad_pro`; none
/// when the corpus isn't fetched.
fn projects() -> Vec<(&'static str, Project)> {
    let roots = [
        ("hackrf-one", "hackrf-one"),
        ("glasgow-revD1", "glasgow"),
        ("jetson-orin", "jetson-orin-baseboard"),
        ("kria-k26", "kria-k26-devboard"),
    ];
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/boards");
    (roots.into_iter())
        .filter(|(name, _)| dir.join(name).exists())
        .map(|(name, root)| {
            let dir = dir.join(name);
            let pro = std::fs::read_to_string(dir.join(format!("{root}.kicad_pro"))).unwrap();
            let read = |f: &str| std::fs::read_to_string(dir.join(f)).map_err(|e| e.to_string());
            (name, Project::load(&format!("{root}.kicad_sch"), &pro, read).unwrap())
        })
        .collect()
}

#[test]
fn corpus_projects_take_symbols_wires_and_labels_and_give_them_back() {
    // A resistor added to every page of each project (KiCad 6, 7, 8, 10; sheets used more than
    // once) shows there with a reference not used before, and a wire round it joins its pins in
    // the netlist, in a net a label on the wire names (global on every other page); undone, every
    // file is as read.
    let r = r#"(symbol "R" (in_bom yes) (property "Reference" "R" (at 2.032 0 90)) (property "Value" "R" (at 0 0 90))
        (symbol "R_0_1" (rectangle (start -1.016 -2.54) (end 1.016 2.54)))
        (symbol "R_1_1" (pin passive line (at 0 3.81 270) (length 1.27) (name "") (number "1"))
          (pin passive line (at 0 -3.81 90) (length 1.27) (name "") (number "2"))))"#;
    for (name, mut p) in projects() {
        let files: Vec<String> = p.pages.iter().map(|g| g.file.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let before: Vec<String> = files.iter().map(|f| p.text(f).unwrap().to_string()).collect();
        let used: Vec<String> = p.pages.iter().flat_map(|g| g.sheet.symbols.iter().map(|s| s.reference.clone())).collect();
        let t = std::time::Instant::now();
        let mut undo = vec![];
        for page in 0..p.pages.len() {
            let (u, r) = p.add_symbol(page, "Device", r, &[], Pt { x: 2_540_000, y: 5_080_000 }, 90.0).unwrap();
            assert!(!used.contains(&r) && p.pages[page].sheet.symbols.iter().any(|s| s.reference == r), "{name}: {r} on page {page}");
            undo.push((u, r));
            // Its pins sit at x -1.27 and 6.35 mm.
            let at = |x: f64, y: f64| Pt { x: (x * 1e6) as i32, y: (y * 1e6) as i32 };
            let w = p.add_wire(page, &[at(-1.27, 5.08), at(-1.27, 1.27), at(6.35, 1.27), at(6.35, 5.08)]).unwrap();
            undo.push((w, String::new()));
            undo.push((
                p.add_label(page, "CUPRITA", [LabelKind::Local, LabelKind::Global][page % 2], "input", at(2.54, 1.27), 0.0).unwrap(),
                String::new(),
            ));
        }
        let nets = p.netlist();
        let named = |r: &String| {
            let both = [(r.clone(), "1".to_string()), (r.clone(), "2".to_string())];
            nets.iter().any(|(net, pins)| (net == "CUPRITA" || net.ends_with("/CUPRITA")) && both.iter().all(|b| pins.contains(b)))
        };
        let mut parts = undo.iter().filter(|(_, r)| !r.is_empty());
        assert!(parts.all(|(_, r)| named(r)), "{name}: the new parts aren't wired and named in the netlist");
        eprintln!("{name}: {} symbols, wires and labels added in {:.1?}", undo.len(), t.elapsed());
        for (u, _) in undo.into_iter().rev() {
            p.apply(u);
        }
        for (f, text) in files.iter().zip(&before) {
            assert!(p.text(f) == Some(text.as_str()), "{name}: {f} differs after undoing");
        }
    }
}

#[test]
fn corpus_netlists_match_kicad() {
    // KiCad 10's netlist export of each project (`kicad-cli sch export netlist`): its nets and
    // pins, and an FNV-1a digest of its nets (`name\tref-pin ref-pin…` lines, pins and lines
    // sorted; skill `erc-check`, `kicad.py digest`).
    let kicad = [
        ("hackrf-one", 379, 1358, 0x4839_5d71_4e44_31a2),
        ("glasgow-revD1", 598, 2532, 0x6d42_8c62_b8bd_b580),
        ("jetson-orin", 791, 2670, 0xed3d_e63f_ae80_762b),
        ("kria-k26", 804, 2659, 0x64c8_800a_0695_849c),
    ];
    let kicad: HashMap<&str, (usize, usize, u64)> =
        kicad.into_iter().map(|(name, nets, pins, digest)| (name, (nets, pins, digest))).collect();
    for (name, p) in projects() {
        let t = std::time::Instant::now();
        let nets = p.netlist();
        eprintln!("{name}: {} nets in {:.1?}", nets.len(), t.elapsed());
        let mut lines: Vec<String> = (nets.iter())
            .map(|(net, pins)| {
                let mut pins: Vec<String> = pins.iter().map(|(r, n)| format!("{r}-{n}")).collect();
                pins.sort();
                format!("{net}\t{}", pins.join(" "))
            })
            .collect();
        lines.sort();
        let digest = lines.join("\n").bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
        let pins = nets.iter().map(|(_, p)| p.len()).sum();
        assert_eq!((nets.len(), pins, digest), kicad[name], "{name}: nets, pins and digest differ from KiCad's netlist export");
    }
}

#[test]
fn corpus_boards_take_their_schematics_parts_and_nets() {
    // Each corpus board updated from its project's schematic, as KiCad 10 reads it: its pads stay
    // grouped as they were, only names change (KiCad 6 and 7 named some nets otherwise), and
    // glasgow-revD1 and jetson-orin, as KiCad 10 names them, don't change; glasgow-revD1's U9
    // takes the part number its symbol has since. Then a resistor wired to itself under a label
    // comes in below the board on that net. Saved, the text reads back to the nets, values and
    // fields updated; undone, the board is as read.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/boards");
    let r = r#"(symbol "R" (in_bom yes) (property "Reference" "R" (at 2.032 0 90)) (property "Value" "R" (at 0 0 90))
        (property "Footprint" "Cuprita:R_Test" (at 0 0 0))
        (symbol "R_1_1" (pin passive line (at 0 3.81 270) (length 1.27) (name "") (number "1"))
          (pin passive line (at 0 -3.81 90) (length 1.27) (name "") (number "2"))))"#;
    let fp = r#"(footprint "R_Test" (layer "F.Cu") (property "Reference" "REF**" (at 0 -1.5 0) (layer "F.SilkS"))
        (property "Value" "R_Test" (at 0 1.5 0) (layer "F.Fab"))
        (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu" "F.Mask")) (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu" "F.Mask")))"#;
    let library = |id: &str| if id == "Cuprita:R_Test" { Ok(fp.to_string()) } else { Err(format!("{id} isn't in this test")) };
    let nets = |b: &Board| {
        let name = |id: NetId| b.nets[id as usize].clone();
        let pads = b.pads.iter().map(|p| name(p.net));
        let copper = b.tracks.iter().map(|t| name(t.net)).chain(b.vias.iter().map(|v| name(v.net)));
        pads.chain(copper).chain(b.zones.iter().map(|z| name(z.net))).collect::<Vec<_>>()
    };
    for (name, mut p) in projects() {
        let src = std::fs::read_to_string(dir.join(format!("{name}.kicad_pcb"))).unwrap();
        let mut b = Board::parse(&src).unwrap();
        let before = (b.nets.clone(), b.pads.clone(), b.footprints.len());
        let t = std::time::Instant::now();
        let (undo, done) = b.update(&p, library);
        eprintln!("{name}: {done:?} in {:.1?}", t.elapsed());
        let (mut to, mut from) = (HashMap::new(), HashMap::new());
        for (old, new) in before.1.iter().zip(&b.pads) {
            assert_eq!(*to.entry(old.net).or_insert(new.net), new.net, "{name}: a net's pads went apart");
            assert_eq!(*from.entry(new.net).or_insert(old.net), old.net, "{name}: two nets' pads joined");
        }
        assert_eq!((done.parts, done.moved, done.added, done.missing.is_empty()), (0, 0, 0, true), "{name}: {done:?}");
        assert_eq!((done.values, done.fields, undo.is_none()), (0, usize::from(name == "glasgow-revD1"), name == "jetson-orin"), "{name}");
        // A resistor wired pin to pin under the label CUPRITA, on the root sheet.
        let (_, reference) = p.add_symbol(0, "Device", r, &[], Pt { x: 2_540_000, y: 5_080_000 }, 90.0).unwrap();
        let at = |x: f64, y: f64| Pt { x: (x * 1e6) as i32, y: (y * 1e6) as i32 };
        p.add_wire(0, &[at(-1.27, 5.08), at(-1.27, 1.27), at(6.35, 1.27), at(6.35, 5.08)]).unwrap();
        p.add_label(0, "CUPRITA", LabelKind::Local, "", at(2.54, 1.27), 0.0).unwrap();
        let low = b.footprints.iter().map(|f| f.bounds.1.y).max().unwrap();
        let (again, done) = b.update(&p, library);
        assert_eq!((done.parts, done.moved, done.added), (1, 0, 1), "{name}: {done:?}");
        let f = b.footprints.last().unwrap();
        assert_eq!((f.reference.as_str(), f.lib_id.as_str()), (reference.as_str(), "Cuprita:R_Test"), "{name}");
        assert!(b.pads[f.pads.clone()].iter().all(|q| b.nets[q.net as usize] == "/CUPRITA") && f.bounds.0.y > low, "{name}: {f:?}");
        let back = Board::parse(&b.write(&src).unwrap()).unwrap();
        let g = back.footprints.last().unwrap();
        assert_eq!((&g.reference, &g.value, &g.lib_id), (&f.reference, &f.value, &f.lib_id), "{name}");
        assert!(nets(&back) == nets(&b), "{name}: the saved text reads back to other nets");
        let fields = |b: &Board| b.footprints.iter().map(|f| (f.value.clone(), f.fields.clone())).collect::<Vec<_>>();
        assert!(fields(&back) == fields(&b), "{name}: the saved text reads back to other values or fields");
        b.apply(again.unwrap());
        undo.map(|undo| b.apply(undo));
        assert!((b.nets, b.pads, b.footprints.len()) == before, "{name}: undone, the board isn't as read");
    }
}

#[test]
fn corpus_parts_take_the_footprint_their_symbol_names() {
    // A resistor on each side of each corpus board whose symbol now names another footprint
    // comes in as that one: in its place, on its side, its texts and fields as they were, its
    // pads on the nets the old ones had. Saved, the text reads back as updated; undone, as read.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/boards");
    let fp = r#"(footprint "R_Test" (layer "F.Cu") (property "Reference" "REF**" (at 0 -1.5 0) (layer "F.SilkS"))
        (fp_line (start -2 -1) (end 2 -1) (stroke (width 0.1)) (layer "F.SilkS"))
        (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu" "F.Mask")) (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu" "F.Mask")))"#;
    let library = |id: &str| if id == "Cuprita:R_Test" { Ok(fp.to_string()) } else { Err(format!("{id} isn't in this test")) };
    for (name, mut p) in projects() {
        let src = std::fs::read_to_string(dir.join(format!("{name}.kicad_pcb"))).unwrap();
        let mut b = Board::parse(&src).unwrap();
        b.update(&p, library);
        let read = b.clone();
        let resistor = |back: bool| {
            let mut fps = b.footprints.iter().enumerate().filter(|(_, f)| f.reference.starts_with('R') && f.pads.len() == 2);
            fps.find(|(_, f)| f.back == back && p.fields(&f.reference).is_some()).map(|(i, _)| i)
        };
        let parts: Vec<usize> = [false, true].into_iter().filter_map(resistor).collect();
        for &i in &parts {
            p.set_field(&b.footprints[i].reference, "Footprint", "Cuprita:R_Test").unwrap();
        }
        let (undo, done) = b.update(&p, library);
        assert_eq!((done.footprints, done.parts, done.missing.len()), (parts.len(), 0, 0), "{name}: {done:?}");
        for &i in &parts {
            let (f, g) = (&b.footprints[i], &read.footprints[i]);
            // KiCad 8 keeps the footprint's name as a field too: it names the new one.
            let fields =
                |f: &cuprita_core::board::Footprint| f.fields.iter().filter(|(k, _)| k != "Footprint").cloned().collect::<Vec<_>>();
            assert_eq!(
                (f.lib_id.as_str(), f.at, f.angle, f.back, fields(f)),
                ("Cuprita:R_Test", g.at, g.angle, g.back, fields(g)),
                "{name}"
            );
            assert!(b.texts[f.texts.clone()] == read.texts[g.texts.clone()], "{name}: {}'s texts moved", f.reference);
            let nets = |b: &Board, f: &cuprita_core::board::Footprint| {
                b.pads[f.pads.clone()].iter().map(|q| b.nets[q.net as usize].clone()).collect::<Vec<_>>()
            };
            assert_eq!(nets(&b, f), nets(&read, g), "{name}: {}", f.reference);
        }
        let back = Board::parse(&b.write(&src).unwrap()).unwrap();
        let (theirs, ours) = (as_written(&back), as_written(&b));
        assert!(theirs.pads.iter().map(|q| q.at).eq(ours.pads.iter().map(|q| q.at)), "{name}: pads");
        assert!(theirs.shapes == ours.shapes && theirs.texts == ours.texts, "{name}: graphics and texts");
        assert!(back.footprints.iter().map(|f| &f.lib_id).eq(b.footprints.iter().map(|f| &f.lib_id)), "{name}: footprints");
        b.apply(undo.unwrap());
        assert!(
            b.footprints == read.footprints && b.pads == read.pads && b.shapes == read.shapes,
            "{name}: undone, the board isn't as read"
        );
    }
}

#[test]
fn corpus_erc_matches_kicad_cli() {
    // `kicad-cli sch erc --severity-all` (KiCad 10) on each project, but for the checks needing
    // libraries or not made (off-grid, ground pins) and where KiCad tells a problem inside a
    // sheet once per use (glasgow-revD1's level shifters, used four times: 23 power inputs and 4
    // dangling bus entries, 8 entry ends besides); we tell one once, and no end of a dangling wire.
    let kicad = [
        ("hackrf-one", &[("same_local_global_label", 1)][..]),
        (
            "glasgow-revD1",
            &[
                ("hier_label_mismatch", 1),
                ("multiple_net_names", 4),
                ("net_not_bus_member", 16),
                ("pin_to_pin", 4),
                ("power_pin_not_driven", 20),
                ("wire_dangling", 1),
            ],
        ),
        ("jetson-orin", &[("same_local_global_label", 6)]),
        ("kria-k26", &[("hier_label_mismatch", 4), ("isolated_pin_label", 185), ("power_pin_not_driven", 24)]),
    ];
    let kicad: HashMap<&str, &[(&str, usize)]> = kicad.into_iter().collect();
    for (name, p) in projects() {
        let t = std::time::Instant::now();
        let vs = p.erc();
        eprintln!("{name}: ERC {} violations in {:.1?}", vs.len(), t.elapsed());
        let mut got: HashMap<&str, usize> = HashMap::new();
        vs.iter().for_each(|v| *got.entry(v.kind).or_default() += 1);
        let want: HashMap<&str, usize> = kicad[name].iter().copied().collect();
        assert_eq!(got, want, "{name}: ERC violations by kind");
    }
}

#[test]
fn corpus_simulations_write_kicads_parts() {
    // KiCad 10's simulation demos (`boards/sim`): each SPICE netlist has as many parts as KiCad's
    // export (`kicad-cli sch export netlist --format spice`; skill `spice-check` compares them
    // line by line), its libraries included where they are.
    let kicad = [
        ("rectifier", "rectifier", 5),
        ("sallen_key", "sallen_key", 8),
        ("laser_driver", "laser_driver", 13),
        ("v_i_sources", "v_i_sources", 36),
        ("amplifier-ac", "amplifier-ac", 23),
        ("subsheets", "mainsheet", 13),
        ("class-d", "Class-D", 39),
        ("up-down-counter", "up-down-c", 3),
        ("buck_conv", "buck_conv", 8),
        ("q17", "Q17ng", 80),
    ];
    let sim = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/boards/sim");
    for (name, root, parts) in kicad.into_iter().filter(|(name, ..)| sim.join(name).exists()) {
        let dir = sim.join(name);
        let pro = std::fs::read_to_string(dir.join(format!("{root}.kicad_pro"))).unwrap();
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).map_err(|e| e.to_string());
        let p = Project::load(&format!("{root}.kicad_sch"), &pro, read).unwrap();
        let t = std::time::Instant::now();
        let deck = p.spice(&dir).unwrap_or_else(|e| panic!("{name}: {e}"));
        eprintln!("{name}: netlist written in {:.1?}", t.elapsed());
        // Its part lines: not the title, dot lines, comments, `+` lines, couplings or `.control`.
        let mut control = false;
        let written = deck.text.lines().skip(1).filter(|l| {
            let l = l.trim().to_ascii_lowercase();
            control |= l.starts_with(".control");
            let part = !control && !l.is_empty() && !l.starts_with(['.', '*', '+', 'k']);
            control &= !l.starts_with(".endc");
            part
        });
        assert_eq!(written.count(), parts, "{name}: parts");
        for lib in deck.text.lines().filter_map(|l| l.strip_prefix(".include \"")) {
            assert!(Path::new(lib.trim_end_matches('"')).is_file(), "{name}: {lib}");
        }
    }
}

#[test]
fn corpus_simulations_run_in_ngspice() {
    // The demos that simulate in under a second (class-d and q17 take 13 and 25 s), as their
    // circuits work.
    let sim = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/boards/sim");
    let demos = [
        ("rectifier", "rectifier"),
        ("sallen_key", "sallen_key"),
        ("laser_driver", "laser_driver"),
        ("v_i_sources", "v_i_sources"),
        ("amplifier-ac", "amplifier-ac"),
        ("subsheets", "mainsheet"),
        ("up-down-counter", "up-down-c"),
        ("buck_conv", "buck_conv"),
    ];
    for (name, root) in demos.into_iter().filter(|(name, _)| sim.join(name).exists()) {
        let dir = sim.join(name);
        let pro = std::fs::read_to_string(dir.join(format!("{root}.kicad_pro"))).unwrap();
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).map_err(|e| e.to_string());
        let deck = Project::load(&format!("{root}.kicad_sch"), &pro, read).unwrap().spice(&dir).unwrap();
        let t = std::time::Instant::now();
        let s = match cuprita_core::ngspice::simulate(&deck, &std::sync::atomic::AtomicBool::new(false)) {
            Err(e) if !cuprita_core::ngspice::loads() => return eprintln!("{e}"),
            s => s.unwrap_or_else(|e| panic!("{name}: {e}")),
        };
        eprintln!("{name}: simulated in {:.1?}", t.elapsed());
        let v = |net: &str| -> Vec<f64> {
            let p = &s.plots[0];
            let v = p.vectors.iter().find(|v| v.name == net).unwrap_or_else(|| panic!("{name}: no {net} in {}", p.name));
            if v.im.is_empty() { v.re.clone() } else { v.re.iter().zip(&v.im).map(|(re, im)| re.hypot(*im)).collect() }
        };
        let (min, max) = (|net: &str| v(net).into_iter().fold(f64::MAX, f64::min), |net: &str| v(net).into_iter().fold(f64::MIN, f64::max));
        match name {
            // A 1 V sine less a diode's drop.
            "rectifier" => assert!((0.35..0.45).contains(&max("/rect_out")), "{name}: {}", max("/rect_out")),
            // A low-pass filter: all of 1 Hz, next to nothing of 1 MHz.
            "sallen_key" => {
                let gain = v("/lowpass");
                assert!((gain[0] - 1.0).abs() < 1e-3 && gain[gain.len() - 1] < 1e-3, "{name}: {gain:?}");
            }
            "v_i_sources" => {
                assert!(v("/vdc").iter().all(|&dc| dc == 2.345), "{name}: /vdc");
                assert_eq!((min("/vpulse"), max("/vpulse")), (-1.0, 1.0), "{name}: /vpulse");
            }
            // Its XSPICE state machine counts, its outputs going to 5 V.
            "up-down-counter" => assert_eq!(max("/q1"), 5.0, "{name}: /q1"),
            _ => {}
        }
    }
}

#[test]
fn corpus_boards_parse_losslessly() {
    let boards = boards();
    if boards.is_empty() {
        eprintln!("corpus not fetched; run corpus/fetch.sh");
    }
    for path in boards {
        let name = path.display();
        let src = std::fs::read_to_string(&path).unwrap();
        assert_lossless(&src, &name.to_string());
        let b = Board::parse(&src).unwrap_or_else(|e| panic!("{name}: {e} at {:?}", e.line_col(&src)));
        // Independent oracle: count the item keywords in the raw text.
        assert_eq!(b.tracks.iter().filter(|t| t.mid.is_none()).count(), count(&src, "(segment"), "{name}");
        assert_eq!(b.vias.len(), count(&src, "(via"), "{name}");
        assert_eq!(b.pads.len(), count(&src, "(pad"), "{name}");
        assert_eq!(b.stackup.len(), b.copper.count_ones() as usize, "{name}");
        assert!(
            b.stackup[..b.stackup.len().saturating_sub(1)].iter().all(|c| c.thickness > 0 && c.dielectric > 0 && c.epsilon_r > 1.0),
            "{name}"
        );
    }
}

#[test]
fn corpus_boards_save_losslessly() {
    for path in boards() {
        let (name, src) = (path.display(), std::fs::read_to_string(&path).unwrap());
        let mut b = Board::parse(&src).unwrap();
        assert!(b.write(&src).unwrap() == src, "{name}: an unedited board must save byte for byte");
        // Remove the file's first track, last via and first zone, add a corner on the last zone's
        // first edge and change its settings (refilled alone); move the part with the most pads
        // and turn it a quarter (from a right angle, so the rotations are exact), then add two
        // tracks from its anchor, a via and a zone of its net around it, and delete the part in
        // the middle of the file.
        if !b.tracks.is_empty() {
            b.tracks.remove(0);
        }
        b.vias.pop();
        if !b.zones.is_empty() {
            b.apply(Patch::delete(&b, Pick::Zone(0)));
        }
        let last = b.zones.len().checked_sub(1);
        if let Some(k) = last {
            let mut z = b.zones[k].clone();
            let (p, q) = (z.outline[0][0], z.outline[0][1]);
            z.add_corner(0, 0, Pt { x: p.x + (q.x - p.x) / 2, y: p.y + (q.y - p.y) / 2 }).unwrap();
            (z.priority, z.clearance, z.connect, z.thermal_gap) = (z.priority + 1, z.clearance + 10_000, Connect::None, z.thermal_gap / 2);
            // Zones have names from KiCad 6.
            if b.version >= 20211014 {
                z.name = format!("{}2", z.name);
            }
            b.set_zone(k, z);
        }
        let i =
            (0..b.footprints.len()).filter(|&i| b.footprints[i].angle % 90.0 == 0.0).max_by_key(|&i| b.footprints[i].pads.len()).unwrap();
        let (f, mm) = (&b.footprints[i], 1_000_000);
        let (at, angle, net) = (Pt { x: f.at.x + mm / 2, y: f.at.y - mm / 4 }, f.angle + 90.0, b.pads[f.pads.start].net);
        b.place_footprint(i, at, angle);
        let (corner, end) = (Pt { x: at.x + mm, y: at.y }, Pt { x: at.x + mm, y: at.y + mm });
        b.add_tracks(&[at, corner, end], b.rules.class_track_width, b.stackup[0].layer, net);
        b.vias.push(Via {
            at: end,
            size: b.rules.class_via_diameter,
            drill: b.rules.class_via_drill,
            layers: b.copper,
            net,
            locked: false,
        });
        let around = [(-5, -5), (5, -5), (5, 5), (-5, 5)].map(|(dx, dy)| Pt { x: end.x + dx * mm, y: end.y + dy * mm });
        let zone = b.add_zone(around.to_vec(), b.stackup[0].layer, net);
        let middle = b.footprints.len() / 2;
        b.apply(Patch::delete(&b, Pick::Footprint(middle + usize::from(middle == i))));
        let t = std::time::Instant::now();
        let out = b.write(&src).unwrap();
        eprintln!("{name}: written in {:.1?}", t.elapsed());
        // Read back, the new text numbers the footprints anew, and KiCad 10 text the nets where
        // first used, which the deleted part can change.
        let back = Board::parse(&out).unwrap();
        let ids: HashMap<&str, usize> = back.nets.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let net: Vec<NetId> = b.nets.iter().map(|n| ids.get(n.as_str()).map_or(0, |&i| i as NetId)).collect();
        b.footprints.iter_mut().enumerate().for_each(|(k, f)| f.src = Src::File(k));
        b.pads.iter_mut().for_each(|p| p.net = net[p.net as usize]);
        b.tracks.iter_mut().for_each(|t| t.net = net[t.net as usize]);
        b.vias.iter_mut().for_each(|v| v.net = net[v.net as usize]);
        b.fills.iter_mut().for_each(|z| z.net = net[z.net as usize]);
        b.zones.iter_mut().enumerate().for_each(|(k, z)| (z.net, z.src) = (net[z.net as usize], Some(k)));
        assert!(
            back.footprints == b.footprints && back.pads == b.pads && back.shapes == b.shapes && back.texts == b.texts,
            "{name}: parts"
        );
        // KiCad 5 draws fills with an outline: those refilled here are saved shrunk, read back
        // grown, in as many pieces or not.
        let kicad5 = b.version < 20211014;
        let fills = if kicad5 { b.zones[last.unwrap_or(zone)].fills.start } else { b.fills.len() };
        assert!(back.tracks == b.tracks && back.vias == b.vias && back.fills[..fills] == b.fills[..fills], "{name}: copper");
        let zones = |b: &Board| {
            b.zones.iter().map(|z| Zone { fills: if kicad5 { 0..0 } else { z.fills.clone() }, ..z.clone() }).collect::<Vec<_>>()
        };
        assert!(zones(&back) == zones(&b) && !b.zones[zone].fills.is_empty(), "{name}: zones");
    }
}

#[test]
fn corpus_parts_flip_and_back() {
    // Every part of each corpus board flipped (as KiCad 10's Flip: skill kicad-check): the text
    // saved reads back to the parts flipped; flipped back, the board saves byte for byte, and so
    // does that text read and flipped back.
    for path in boards() {
        let (name, src) = (path.display(), std::fs::read_to_string(&path).unwrap());
        let mut b = Board::parse(&src).unwrap();
        let t = std::time::Instant::now();
        (0..b.footprints.len()).for_each(|i| b.flip_footprint(i));
        let flipped = t.elapsed();
        let out = b.write(&src).unwrap();
        eprintln!("{name}: {} parts flipped in {flipped:.1?}, written in {:.1?}", b.footprints.len(), t.elapsed() - flipped);
        let mut back = Board::parse(&out).unwrap();
        // Layers met first on a flipped part (B.Fab of a KiCad 4 board) come in another order.
        let ours: HashMap<&str, u8> = b.layers.iter().enumerate().map(|(i, l)| (l.as_str(), i as u8)).collect();
        let to: Vec<u8> = back.layers.iter().map(|l| ours[l.as_str()]).collect();
        let mask = |m: u128| (0..to.len()).filter(|&l| m >> l & 1 != 0).fold(0u128, |o, l| o | 1 << to[l]);
        let mut theirs = back.clone();
        theirs.pads.iter_mut().for_each(|p| p.layers = mask(p.layers));
        theirs.shapes.iter_mut().for_each(|s| s.layer = to[s.layer as usize]);
        theirs.texts.iter_mut().for_each(|x| x.layer = to[x.layer as usize]);
        theirs.footprints.iter_mut().flat_map(|f| &mut f.hidden).for_each(|x| x.layer = to[x.layer as usize]);
        let area = |b: &Board| b.pads.iter().map(|p| custom_area(&p.custom)).collect::<Vec<_>>();
        assert!(area(&b).iter().zip(area(&back)).all(|(a, c)| (a - c).abs() <= 1e-4 * a), "{name}: custom pads' copper");
        let near = |p: Pt, q: Pt| (p.x - q.x).abs() <= 1000 && (p.y - q.y).abs() <= 1000;
        assert!(b.footprints.iter().zip(&back.footprints).all(|(f, g)| near(f.bounds.0, g.bounds.0) && near(f.bounds.1, g.bounds.1)));
        let (ours, theirs) = (as_written(&b), as_written(&theirs));
        assert!(theirs.footprints == ours.footprints && theirs.pads == ours.pads, "{name}: parts and pads");
        assert!(theirs.shapes == ours.shapes && theirs.texts == ours.texts, "{name}: graphics and texts");
        for i in 0..b.footprints.len() {
            b.flip_footprint(i);
            back.flip_footprint(i);
        }
        assert!(b.write(&src).unwrap() == src, "{name}: flipped back, the board saves as read");
        assert!(back.write(&out).unwrap() == src, "{name}: flipped back after saving, the board saves as read");
    }
}

#[test]
fn corpus_texts_tidy_save_and_flip() {
    // Every part's texts tidied (references placed or hidden, values in their bodies): the text
    // saved reads back to the texts tidied, and so it does with every part flipped then; all
    // undone, the board saves as read.
    for path in boards() {
        let (name, src) = (path.display(), std::fs::read_to_string(&path).unwrap());
        let mut b = Board::parse(&src).unwrap();
        let t = std::time::Instant::now();
        let (undo, done) = b.tidy_texts(&[]);
        let tidied = t.elapsed();
        let out = b.write(&src).unwrap();
        eprintln!("{name}: {done:?} in {tidied:.1?}, written in {:.1?}", t.elapsed() - tidied);
        let near = |b: &Board, back: &Board| {
            let name = |b: &Board, t: &cuprita_core::board::Text| b.layers[t.layer as usize].clone();
            b.texts.len() == back.texts.len()
                && b.texts.iter().zip(&back.texts).all(|(t, u)| {
                    let off = (t.angle - u.angle).rem_euclid(360.0);
                    (t.at.x - u.at.x).abs() <= 2
                        && (t.at.y - u.at.y).abs() <= 2
                        && off.min(360.0 - off) < 1e-6
                        && name(b, t) == name(back, u)
                        && (t.size, t.thickness, t.mirror, &t.text) == (u.size, u.thickness, u.mirror, &u.text)
                })
        };
        assert!(near(&b, &Board::parse(&out).unwrap()), "{name}: tidied, read back");
        (0..b.footprints.len()).for_each(|i| b.flip_footprint(i));
        assert!(near(&b, &Board::parse(&b.write(&src).unwrap()).unwrap()), "{name}: tidied and flipped, read back");
        (0..b.footprints.len()).for_each(|i| b.flip_footprint(i));
        b.apply(undo);
        assert!(b.write(&src).unwrap() == src, "{name}: undone, the board saves as read");
    }
}

#[test]
fn corpus_items_move_turn_and_go_together() {
    // About 200 items spread over each corpus board (parts, tracks, vias, zones) moved, turned a
    // quarter about a millimetre point and deleted, each one edit with the zones it changes
    // refilled: its re-checks find what full checks find; saved once turned and once gone, the
    // text reads back to the board. All undone, it saves as read.
    let sorted = |mut v: Vec<drc::Violation>| {
        v.sort_unstable_by(|a, b| (a.kind, a.items, &a.what, a.at.x, a.at.y).cmp(&(b.kind, b.items, &b.what, b.at.x, b.at.y)));
        v
    };
    for path in boards() {
        let (name, src) = (path.display(), std::fs::read_to_string(&path).unwrap());
        let mut b = Board::parse(&src).unwrap();
        let every = |n: usize, k: usize| (0..n).step_by((n / k).max(1));
        let picks: Vec<Pick> = (every(b.footprints.len(), 20).map(Pick::Footprint))
            .chain(every(b.tracks.len(), 150).map(Pick::Track))
            .chain(every(b.vias.len(), 25).map(Pick::Via))
            .chain(every(b.zones.len(), 5).map(Pick::Zone))
            .collect();
        let (lo, hi) = b.extent(&picks).unwrap();
        let about = Pt { x: (lo.x / 2 + hi.x / 2) / 1_000_000 * 1_000_000, y: (lo.y / 2 + hi.y / 2) / 1_000_000 * 1_000_000 };
        let (mut vs, mut rs, mut undone) = (drc::check(&b), b.ratsnest(), vec![]);
        for what in ["moved", "turned", "deleted"] {
            let t = std::time::Instant::now();
            let undo = match what {
                "moved" => b.move_items(&picks, Pt { x: 1_000_000, y: -500_000 }),
                "turned" => b.turn_items(&picks, about, 90.0),
                _ => b.delete_items(&picks),
            };
            let (undo, refilled) = b.refill(undo);
            let edited = t.elapsed();
            vs = drc::recheck(&b, vs, &undo);
            rs = cuprita_core::ratsnest::recheck(&b, rs, &undo);
            eprintln!(
                "{name}: {} items {what}, {refilled} zones refilled in {edited:.1?}, re-checked in {:.1?}",
                picks.len(),
                t.elapsed() - edited
            );
            assert!(sorted(vs.clone()) == sorted(drc::check(&b)) && rs == b.ratsnest(), "{name}: {what}, re-checked");
            undone.push(undo);
            if what == "moved" {
                continue;
            }
            let back = Board::parse(&b.write(&src).unwrap()).unwrap();
            let parts = |b: &Board| b.footprints.iter().map(|f| (f.reference.clone(), f.at, f.angle, f.back)).collect::<Vec<_>>();
            let pads = |b: &Board| b.pads.iter().map(|p| (p.at, p.angle)).collect::<Vec<_>>();
            let zones = |b: &Board| b.zones.iter().map(|z| z.outline.clone()).collect::<Vec<_>>();
            assert!(parts(&back) == parts(&b) && pads(&back) == pads(&b), "{name}: {what}, parts");
            assert!(named_copper(&back) == named_copper(&b) && zones(&back) == zones(&b), "{name}: {what}, copper");
        }
        undone.into_iter().rev().for_each(|undo| _ = b.apply(undo));
        assert!(b.write(&src).unwrap() == src, "{name}: every edit undone, the board saves as read");
    }
}

/// The board's tracks and vias in words, their layers and nets by name, sorted: a board saved
/// writes the moved ones after the file's last, and KiCad 10 numbers nets where first used.
fn named_copper(b: &Board) -> Vec<String> {
    let (layer, net) = (|l: u8| &b.layers[l as usize], |n: NetId| &b.nets[n as usize]);
    let tracks = b.tracks.iter().map(|t| format!("{:?} {:?} {:?} {} {} {}", t.a, t.b, t.mid, t.width, layer(t.layer), net(t.net)));
    let vias = b.vias.iter().map(|v| format!("{:?} {} {} {:x} {}", v.at, v.size, v.drill, v.layers, net(v.net)));
    let mut all: Vec<String> = tracks.chain(vias).collect();
    all.sort_unstable();
    all
}

/// Board `b` with its angles as saving writes them (to the micro-degree), without its custom
/// pads' copper and its parts' boxes: flipped, the strokes of their lines and arcs lie within nm
/// of how reading the flipped text draws them (`custom_area`).
fn as_written(b: &Board) -> Board {
    let deg = |a: f64| (a * 1e6).round() / 1e6;
    let mut b = b.clone();
    b.footprints.iter_mut().for_each(|f| (f.angle, f.bounds) = (deg(f.angle), Default::default()));
    b.texts.iter_mut().chain(b.footprints.iter_mut().flat_map(|f| &mut f.hidden)).for_each(|t| t.angle = deg(t.angle));
    b.pads.iter_mut().for_each(|p| (p.angle, p.custom) = (deg(p.angle), vec![]));
    b
}

/// The area of a custom pad's copper (`Pad::custom`), nm².
fn custom_area(contours: &[Vec<Pt>]) -> f64 {
    let cross = |c: &Vec<Pt>| {
        (0..c.len()).map(|i| (c[i], c[(i + 1) % c.len()])).map(|(p, q)| p.x as f64 * q.y as f64 - q.x as f64 * p.y as f64).sum::<f64>()
    };
    contours.iter().map(cross).sum::<f64>().abs() / 2.0
}

#[test]
fn corpus_drc_matches_kicad_cli() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/boards");
    if !dir.exists() {
        return;
    }
    for (board, expected) in [
        (
            "framework-card",
            &[("clearance", 19usize), ("copper_edge_clearance", 16), ("track_width", 0), ("drill_out_of_range", 0), ("hole_to_hole", 0)][..],
        ),
        (
            "framework-mcu",
            &[("clearance", 19), ("copper_edge_clearance", 44), ("track_width", 0), ("drill_out_of_range", 0), ("hole_to_hole", 0)],
        ),
        (
            "hackrf-operacake",
            &[("clearance", 0), ("copper_edge_clearance", 58), ("track_width", 0), ("drill_out_of_range", 12), ("hole_to_hole", 4)],
        ),
        ("hackrf-one", &[("copper_edge_clearance", 23), ("track_width", 6), ("drill_out_of_range", 0), ("hole_to_hole", 0)]),
    ] {
        let src = std::fs::read_to_string(dir.join(format!("{board}.kicad_pcb"))).unwrap();
        let b = Board::parse(&src).unwrap();
        let vs = drc::check(&b);
        for &(kind, want) in expected.iter().chain(&[("via_diameter", 0), ("annular_width", 0)]) {
            let got = vs.iter().filter(|v| v.kind == kind).count();
            assert_eq!(got, want, "{board} {kind}: got {got}, want {want}");
        }
    }
    let mut kria = Board::parse(&std::fs::read_to_string(dir.join("kria-k26.kicad_pcb")).unwrap()).unwrap();
    let t = std::time::Instant::now();
    let vs = drc::check(&kria);
    let dt = t.elapsed();
    eprintln!("kria-k26 DRC: {} violations in {dt:.1?}", vs.len());
    assert!(dt.as_secs_f64() < 2.0, "kria-k26 DRC took {dt:?}");
    // Turning the part with the most pads, a re-check of its pads finds what a full check finds.
    let big = (0..kria.footprints.len()).max_by_key(|&i| kria.footprints[i].pads.len()).unwrap();
    let f = &kria.footprints[big];
    let (at, angle, undo) = (Pt { x: f.at.x + 500_000, y: f.at.y }, f.angle + 90.0, Patch::footprint(&kria, big));
    kria.place_footprint(big, at, angle);
    let t = std::time::Instant::now();
    let new = drc::recheck(&kria, vs, &undo);
    eprintln!("kria-k26 re-check of {} pads: {:.1?}", kria.footprints[big].pads.len(), t.elapsed());
    let sorted = |mut v: Vec<drc::Violation>| {
        v.sort_unstable_by(|a, b| (a.kind, a.items, &a.what, a.at.x, a.at.y).cmp(&(b.kind, b.items, &b.what, b.at.x, b.at.y)));
        v
    };
    assert_eq!(sorted(new.clone()), sorted(drc::check(&kria)));
    // Deleting a part moves the pads after it up: their violations follow them.
    let undo = kria.apply(Patch::delete(&kria, Pick::Footprint(kria.footprints.len() / 2)));
    let t = std::time::Instant::now();
    let new = drc::recheck(&kria, new, &undo);
    eprintln!("kria-k26 re-check after a delete: {:.1?}", t.elapsed());
    assert_eq!(sorted(new), sorted(drc::check(&kria)));
}

#[test]
fn corpus_routes_keep_the_clearances() {
    let Ok(src) = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/boards/hackrf-one.kicad_pcb")) else {
        return;
    };
    let mut b = Board::parse(&src).unwrap();
    // Pads of one net on the top layer, 3 to 40 mm apart, routed one after the other: between
    // its ends, where it may meet crowded pads, each path keeps clear of everything.
    let (top, width) = (b.stackup[0].layer, b.rules.class_track_width);
    let pairs: Vec<_> = (0..b.pads.len())
        .step_by(7)
        .filter_map(|i| {
            let (p, on) = (&b.pads[i], |l: u128| l & 1 << top != 0);
            let d = |q: Pt| ((q.x - p.at.x) as f64).hypot((q.y - p.at.y) as f64);
            let q = b.pads.iter().find(|q| q.net == p.net && on(q.layers) && (3e6..40e6).contains(&d(q.at)))?;
            (p.net != 0 && on(p.layers)).then_some((p.net, p.at, q.at))
        })
        .take(12)
        .collect();
    let mut middles = 0;
    for (net, from, to) in pairs {
        let mut r = Router::new(&b, net, top, width, from);
        let path = r.path(from, r.snap(to, 0));
        let new = b.add_tracks(&path, width, top, net);
        let inner: Vec<_> = new.clone().skip(1).take(new.len().saturating_sub(2)).map(Item::Track).collect();
        middles += inner.len();
        assert_eq!(drc::check_items(&b, &inner), vec![], "{} from {from:?}: {path:?}", b.nets[net as usize]);
    }
    assert!(middles > 10, "too few paths with middle segments: {middles}");
}

#[test]
fn corpus_fills_keep_other_nets_out() {
    for path in boards() {
        let (name, src) = (path.display(), std::fs::read_to_string(&path).unwrap());
        let mut b = Board::parse(&src).unwrap();
        let t = std::time::Instant::now();
        b.fill_zones();
        eprintln!("{name}: {} zones filled in {:.1?}", b.zones.len(), t.elapsed());
        // Each zone's fills follow the last one's, on its layers and net.
        let mut next = 0;
        for z in &b.zones {
            assert_eq!(z.fills.start, next, "{name}");
            assert!(b.fills[z.fills.clone()].iter().all(|f| f.net == z.net && z.layers & 1 << f.layer != 0), "{name}");
            next = z.fills.end;
        }
        assert_eq!(next, b.fills.len(), "{name}");
        // No pad, via or track of another net has its centre, nor custom pad a corner, in a fill on its layer.
        let pt = |p: Pt| IntPoint::new(p.x, p.y);
        let mut copper: Vec<(u128, NetId, IntPoint)> = b.pads.iter().map(|p| (p.layers, p.net, pt(p.at))).collect();
        copper.extend(b.pads.iter().flat_map(|p| p.copper().into_iter().flatten().map(|q| (p.layers, p.net, pt(q)))));
        copper.extend(b.vias.iter().map(|v| (v.layers, v.net, pt(v.at))));
        copper.extend(
            b.tracks.iter().map(|t| (1 << t.layer, t.net, pt(t.mid.unwrap_or(Pt { x: t.a.x / 2 + t.b.x / 2, y: t.a.y / 2 + t.b.y / 2 })))),
        );
        for f in &b.fills {
            let ring: Vec<IntPoint> = f.pts.iter().map(|&p| pt(p)).collect();
            let (lo, hi) = ring.iter().fold((ring[0], ring[0]), |(l, h), p| {
                (IntPoint::new(l.x.min(p.x), l.y.min(p.y)), IntPoint::new(h.x.max(p.x), h.y.max(p.y)))
            });
            let near: Vec<IntPoint> = copper
                .iter()
                .filter(|c| c.0 & 1 << f.layer != 0 && (f.net == 0 || c.1 != f.net))
                .map(|c| c.2)
                .filter(|p| (lo.x..=hi.x).contains(&p.x) && (lo.y..=hi.y).contains(&p.y))
                .collect();
            // Its cuts resolved into holes first: point location needs clean outlines.
            let shapes = ring.as_slice().overlay(&[] as &[IntPoint], OverlayRule::Subject, FillRule::EvenOdd);
            let inside: Vec<_> = near.iter().zip(shapes.contains_points(&near)).filter(|c| c.1).map(|c| c.0).collect();
            assert!(
                inside.is_empty(),
                "{name}: fill of {} on {} covers other copper at {:?}",
                b.nets[f.net as usize],
                b.layers[f.layer as usize],
                &inside[..inside.len().min(3)]
            );
        }
        // Saved, the fills read back as refilled (KiCad 5's shrunk: it draws them with an outline).
        let back = Board::parse(&b.write(&src).unwrap()).unwrap();
        assert!(b.version < 20211014 || back.fills == b.fills, "{name}: saved fills");
    }
}

/// Tokens plus the whitespace between them must cover every byte, with balanced parens.
fn assert_lossless(src: &str, name: &str) {
    let (mut lex, mut end, mut depth) = (Lexer::new(src), 0, 0i64);
    while let Some((tok, start, stop)) = lex.next_tok().unwrap() {
        assert!(src[end..start].bytes().all(|b| b.is_ascii_whitespace()), "{name}: lost bytes at {end}");
        depth += match tok {
            Tok::Open => 1,
            Tok::Close => -1,
            _ => 0,
        };
        assert!(depth >= 0, "{name}: unbalanced ')' at {start}");
        end = stop;
    }
    assert!(src[end..].bytes().all(|b| b.is_ascii_whitespace()) && depth == 0, "{name}: bad ending");
}

/// Counts `needle` followed by whitespace: `(via` but not `(via_size`.
fn count(src: &str, needle: &str) -> usize {
    let next_is_space = |i: usize| src.as_bytes().get(i + needle.len()).is_some_and(|b| b.is_ascii_whitespace());
    src.match_indices(needle).filter(|&(i, _)| next_is_space(i)).count()
}
