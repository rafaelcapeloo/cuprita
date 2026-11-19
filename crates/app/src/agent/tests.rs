use super::*;

pub(crate) fn test_session() -> Session {
    let board = Board::parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (net 0 "") (net 1 "/Bus/SCL") (net 2 "GND")
          (footprint "R" (layer "F.Cu") (at 10 20)
            (property "Reference" "R1" (at 0 0) (layer "F.SilkS")) (property "Value" "10k") (property "Datasheet" "r.pdf")
            (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 1 "/Bus/SCL"))
            (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 2 "GND")))
          (segment (start 9 20) (end 11 20) (width 0.5) (layer "B.Cu") (net 2))
          (gr_rect (start 0 0) (end 20 30) (width 0.1) (layer "Edge.Cuts")))"#,
    )
    .unwrap();
    let scene = build(&board);
    Session { name: "t.kicad_pcb".into(), board, scene: Some(scene), ..Session::default() }
}

#[test]
fn cli_queries_finds_and_picks_nets() {
    let mut s = test_session();
    let sum = Val::parse(&s.cli_line(r#"{"cmd":"query"}"#)).unwrap();
    assert_eq!((sum.str("board"), sum.num("footprints"), sum.num("nets")), (Some("t.kicad_pcb"), Some(1.0), Some(2.0)));
    let r1 = Val::parse(&s.cli_line(r#"{"cmd":"find","query":"r1"}"#)).unwrap();
    assert_eq!((r1.str("kind"), r1.str("reference")), (Some("footprint"), Some("R1")));
    assert_eq!(
        (r1.str("value"), r1.str("footprint"), r1.at(&["fields", "Datasheet"])),
        (Some("10k"), Some("R"), Some(&Val::Str("r.pdf".into())))
    );
    let pads = r1
        .arr("pads")
        .unwrap()
        .iter()
        .map(|p| (p.str("number").unwrap(), p.str("net"), p.get("at_mm").unwrap().to_json()))
        .collect::<Vec<_>>();
    assert_eq!(pads, [("1", Some("/Bus/SCL"), "[9,20]".to_string()), ("2", Some("GND"), "[11,20]".to_string())]);
    assert_eq!(r1.arr("pads").unwrap()[0].get("layers").map(Val::to_json).as_deref(), Some(r#"["F.Cu"]"#));
    let gnd = Val::parse(&s.cli_line(r#"{"cmd":"find","query":"GND"}"#)).unwrap();
    assert_eq!(gnd.get("nodes").map(Val::to_json).as_deref(), Some(r#"["R1 pin 2"]"#));
    // A part's pin finds its net.
    for (pin, net) in [("R1 pin 2", "GND"), ("r1.1", "/Bus/SCL"), ("R1-2", "GND")] {
        let found = Val::parse(&s.cli_line(&format!(r#"{{"cmd":"find","query":"{pin}"}}"#))).unwrap();
        assert_eq!(found.str("name"), Some(net), "{pin}");
    }
    let mut bom = |f: &str| s.cli_line(&format!(r#"{{"cmd":"query","what":"bom","filter":"{f}"}}"#));
    assert_eq!(
        (bom("10K").as_str(), bom("C1").as_str()),
        (r#"{"bom":[{"value":"10k","footprint":"R","references":["R1"]}]}"#, r#"{"bom":[]}"#)
    );
    let scl = Val::parse(&s.cli_line(r#"{"cmd":"find","query":"scl"}"#)).unwrap();
    assert_eq!((scl.str("kind"), scl.str("name"), scl.num("id")), (Some("net"), Some("/Bus/SCL"), Some(1.0)));
    // At (9, 20), F.Cu has pad 1 (/Bus/SCL) and B.Cu has the GND track: flipping picks B.Cu.
    let front = Val::parse(&s.cli_line(r#"{"cmd":"query","at":[9.0,20.0]}"#)).unwrap();
    let back = Val::parse(&s.cli_line(r#"{"cmd":"query","at":[9.0,20.0],"flip":true}"#)).unwrap();
    assert_eq!((front.str("name"), back.str("name")), (Some("/Bus/SCL"), Some("GND")));
    assert_eq!((back.num("length_mm"), back.num("tracks"), back.num("pads")), (Some(2.0), Some(1.0), Some(1.0)));
    assert!(back.num("current_a").unwrap() > 1.0 && back.arr("impedance_ohm").unwrap()[0] != Val::Num(0.0));
    // No rule broken; the GND track on the back joins nothing yet, which is a warning as KiCad's.
    let drc = Val::parse(&s.cli_line(r#"{"cmd":"drc"}"#)).unwrap();
    let said = &drc.arr("violations").unwrap()[0];
    assert_eq!(
        (drc.num("count"), drc.num("errors"), said.str("kind"), said.str("severity")),
        (Some(1.0), Some(0.0), Some("track_dangling"), Some("warning"))
    );
    // GND's pad is on F.Cu, its track on B.Cu: a via still has to join them.
    assert_eq!(drc.arr("unconnected").unwrap()[0].str("what"), Some("GND: R1 pad 2 – Track on B.Cu"));
    let dir = std::env::temp_dir().join(format!("cuprita-diff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pcb = dir.join("b.kicad_pcb");
    std::fs::write(&pcb, r#"(kicad_pcb (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "GND"))"#).unwrap();
    let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
    git(&["init", "-q"]);
    git(&["add", "b.kicad_pcb"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
    std::fs::write(
        &pcb,
        r#"(kicad_pcb (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "GND")
          (segment (start 0 0) (end 5 0) (width 0.2) (layer "F.Cu") (net 1)))"#,
    )
    .unwrap();
    s.open(&pcb).unwrap();
    let diff = Val::parse(&s.cli_line(r#"{"cmd":"diff"}"#)).unwrap();
    assert_eq!((diff.str("base"), diff.num("count")), (Some("HEAD"), Some(1.0)));
    let ir = Val::parse(&s.cli_line(r#"{"cmd":"ir_drop","net":"GND","current_a":1.0}"#)).unwrap();
    assert!(ir.num("drop_mv").unwrap() > 10.0 && ir.num("resistance_mohm").unwrap() > 10.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_places_footprints() {
    let mut s = test_session();
    let r1 = s.cli_line(r#"{"cmd":"place","reference":"r1","at":[12.5,20],"angle":90}"#);
    assert_eq!(r1, r#"{"reference":"R1","at_mm":[12.5,20],"angle":90,"back":false,"bounds_mm":[11.793,18.293,13.207,21.707]}"#);
    // Pad 1 turned from (9, 20) to (12.5, 21), and picking sees it there.
    let pick = Val::parse(&s.cli_line(r#"{"cmd":"query","at":[12.5,21.0]}"#)).unwrap();
    assert_eq!(pick.str("name"), Some("/Bus/SCL"));
    let listed = Val::parse(&s.cli_line(r#"{"cmd":"query","what":"footprints"}"#)).unwrap();
    assert_eq!(listed.arr("footprints").map(|f| f[0].to_json()), Some(r1));
    // On the back, mirrored top to bottom: its angle negated, pad 1 from (12.5, 21) to
    // (12.5, 19) on B.Cu, which the track's GND hides from the front at (12.5, 20).
    let r1 = s.cli_line(r#"{"cmd":"place","reference":"R1","side":"back"}"#);
    assert_eq!(r1, r#"{"reference":"R1","at_mm":[12.5,20],"angle":270,"back":true,"bounds_mm":[11.793,18.293,13.207,21.707]}"#);
    let pick = Val::parse(&s.cli_line(r#"{"cmd":"query","at":[12.5,19.0],"flip":true}"#)).unwrap();
    assert_eq!(pick.str("name"), Some("/Bus/SCL"));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"R1 flipped to the bottom"}"#);
    let errors = [
        r#"{"cmd":"place","reference":"R1","at":[1]}"#,
        r#"{"cmd":"place","reference":"R1","angle":"90"}"#,
        r#"{"cmd":"place","reference":"Q9"}"#,
        r#"{"cmd":"place","reference":"R1","side":"left"}"#,
    ]
    .map(|c| Val::parse(&s.cli_line(c)).unwrap().str("error").map(String::from));
    let why = ["at must be [x_mm, y_mm]", "angle must be a number of degrees", "not found: Q9", "side left: give front or back"];
    assert_eq!(errors.map(Option::unwrap), why);
}

#[test]
fn cli_ends_a_route_with_a_via_that_stops_inside_the_board() {
    let board = Board::parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (net 0 "") (net 1 "A")
          (footprint "R" (layer "F.Cu") (at 10 20) (property "Reference" "R1" (at 0 0) (layer "F.SilkS"))
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "A")))
          (gr_rect (start 0 0) (end 40 40) (width 0.1) (layer "Edge.Cuts")))"#,
    )
    .unwrap();
    let scene = build(&board);
    let mut s = Session { name: "t.kicad_pcb".into(), board, scene: Some(scene), ..Session::default() };
    // From R1's pad on the top to a via down to the first inner layer only: blind.
    let r = Val::parse(&s.cli_line(r#"{"cmd":"route","from":[10,20],"to":[15,20],"via_to":"In1.Cu"}"#)).unwrap();
    assert_eq!((r.get("via"), r.get("reached")), (Some(&Val::Bool(true)), Some(&Val::Bool(true))));
    let v = s.board.vias[0];
    assert_eq!((v.at, v.layers.count_ones(), s.board.span(&v)), (mm_pt(15.0, 20.0), 2, cuprita_core::board::Span::Blind));
    // With `via` alone it goes through the board; a layer without copper is no place to stop.
    s.cli_line(r#"{"cmd":"route","from":[15,20],"to":[20,20],"via":true}"#);
    assert_eq!(s.board.vias[1].layers, s.board.copper);
    let error = s.cli_line(r#"{"cmd":"route","from":[10,20],"to":[12,20],"via_to":"Edge.Cuts"}"#);
    assert_eq!(error, r#"{"error":"no copper layer Edge.Cuts for via_to; see query layers"}"#);
}

#[test]
fn cli_routes_a_pair_and_evens_its_sides() {
    // A connector's D+ and D- pads 1 mm apart, a chip's 0.5 mm apart 20 mm on and 6 mm down.
    let board = Board::parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (net 0 "") (net 1 "D+") (net 2 "D-") (net 3 "GND")
          (footprint "J" (layer "F.Cu") (at 10 10)
            (pad "1" smd rect (at 0 -0.5) (size 1.2 0.6) (layers "F.Cu") (net 1 "D+"))
            (pad "2" smd rect (at 0 0.5) (size 1.2 0.6) (layers "F.Cu") (net 2 "D-")))
          (footprint "U" (layer "F.Cu") (at 30 16)
            (pad "1" smd rect (at 0 -0.25) (size 0.8 0.3) (layers "F.Cu") (net 1 "D+"))
            (pad "2" smd rect (at 0 0.25) (size 0.8 0.3) (layers "F.Cu") (net 2 "D-")))
          (footprint "TP" (layer "F.Cu") (at 20 10) (pad "1" smd rect (at 0 0) (size 2 2) (layers "F.Cu") (net 3 "GND")))
          (gr_rect (start 0 0) (end 40 30) (width 0.1) (layer "Edge.Cuts")))"#,
    )
    .unwrap();
    let scene = build(&board);
    let mut s = Session { name: "t.kicad_pcb".into(), board, scene: Some(scene), ..Session::default() };
    let r = Val::parse(&s.cli_line(r#"{"cmd":"pair","from":[10,9.5],"to":[30,16.2],"through":[[15,14]]}"#)).unwrap();
    assert_eq!(r.get("pair").map(Val::to_json).as_deref(), Some(r#"["D+","D-"]"#));
    assert_eq!(
        (r.num("width_mm"), r.num("gap_mm"), r.get("reached"), r.str("layer")),
        (Some(0.2), Some(0.25), Some(&Val::Bool(true)), Some("F.Cu"))
    );
    let drc = Val::parse(&s.cli_line(r#"{"cmd":"drc"}"#)).unwrap();
    assert_eq!((drc.num("errors"), drc.arr("unconnected").map(<[Val]>::len)), (Some(0.0), Some(0)));
    // The longer side stays; the shorter gets a meander when it is short by enough for one.
    let skew = r.num("skew_mm").unwrap();
    assert!(skew.abs() < 1.5, "{skew}");
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"Pair D+ and D- routed"}"#);
    let errors = [
        r#"{"cmd":"pair","from":[20,10],"to":[30,16]}"#,
        r#"{"cmd":"pair","from":[10,9.5]}"#,
        r#"{"cmd":"pair","from":[10,9.5],"to":[30,16.2],"through":[1]}"#,
    ]
    .map(|c| Val::parse(&s.cli_line(c)).unwrap().str("error").map(String::from).unwrap());
    assert_eq!(
        errors,
        [
            "GND is no side of a differential pair: their nets are named as D+ and D-, or CLK_P and CLK_N",
            "to must be [x_mm, y_mm]",
            "through must be [[x_mm, y_mm], …], the corners to lead the pair by"
        ]
    );
}

#[test]
fn cli_tunes_a_net_to_a_length_or_to_its_pairs_other_side() {
    // D+ runs 20 mm, D- 24 mm beside it, 5 mm apart.
    let board = Board::parse(
        r#"(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (net 0 "") (net 1 "D+") (net 2 "D-") (net 3 "CLK")
          (segment (start 10 10) (end 30 10) (width 0.2) (layer "F.Cu") (net 1))
          (segment (start 10 15) (end 34 15) (width 0.2) (layer "F.Cu") (net 2))
          (segment (start 10 25) (end 30 25) (width 0.2) (layer "F.Cu") (net 3))
          (gr_rect (start 0 0) (end 40 40) (width 0.1) (layer "Edge.Cuts")))"#,
    )
    .unwrap();
    let scene = build(&board);
    let mut s = Session { name: "t.kicad_pcb".into(), board, scene: Some(scene), ..Session::default() };
    let r = Val::parse(&s.cli_line(r#"{"cmd":"tune","at":[20,10],"match":true}"#)).unwrap();
    assert_eq!((r.str("net"), r.num("length_mm"), r.num("turns")), (Some("D+"), Some(24.0), Some(3.0)));
    assert!((s.board.net_length(1) - s.board.net_length(2)).abs() < 20.0);
    let r = Val::parse(&s.cli_line(r#"{"cmd":"tune","at":[20,25],"length":21.5}"#)).unwrap();
    assert_eq!((r.str("net"), r.num("length_mm"), r.num("turns")), (Some("CLK"), Some(21.5), Some(1.0)));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"CLK tuned to 21.500 mm"}"#);
    let errors =
        [r#"{"cmd":"tune","at":[20,25],"match":true}"#, r#"{"cmd":"tune","at":[20,25]}"#, r#"{"cmd":"tune","at":[5,5],"length":30}"#]
            .map(|c| Val::parse(&s.cli_line(c)).unwrap().str("error").map(String::from).unwrap());
    assert_eq!(
        errors,
        [
            "CLK is no side of a differential pair: give length",
            "give length, the net's in mm, or match: true for its differential pair's other side",
            "no track at [5, 5]; query at says what is there"
        ]
    );
}

#[test]
fn cli_writes_and_draws_on_the_silkscreen() {
    let mut s = test_session();
    let (texts, shapes) = (s.board.texts.len(), s.board.shapes.len());
    let added = Val::parse(&s.cli_line(r#"{"cmd":"text","text":"REV A\n2026","at":[15,10],"height":1.5,"angle":90}"#)).unwrap();
    assert_eq!(added.str("added"), Some("Text \"REV A\" on F.SilkS"));
    let t = s.board.texts.last().unwrap();
    assert_eq!(
        (t.text.as_str(), t.at, t.size, t.thickness, t.angle),
        ("REV A\n2026", mm_pt(15.0, 10.0), (1_500_000, 1_500_000), 225_000, 90.0)
    );
    // A box, closed back on its first corner, as wide as the project draws silkscreen.
    let drawn = s.cli_line(r#"{"cmd":"line","points":[[2,2],[7,2],[7,5],[2,5]],"closed":true}"#);
    assert_eq!(drawn, r#"{"lines":4,"layer":"F.SilkS","width_mm":0.15}"#);
    assert_eq!((s.board.texts.len(), s.board.shapes.len()), (texts + 1, shapes + 4));
    // Deleted by a point on them, as a click picks them; undone, back.
    assert_eq!(s.cli_line(r#"{"cmd":"delete","at":[2,3.5]}"#), r#"{"deleted":"Line on F.SilkS"}"#);
    assert_eq!(s.cli_line(r#"{"cmd":"delete","at":[15,10]}"#), r#"{"deleted":"Text \"REV A\" on F.SilkS"}"#);
    assert_eq!((s.board.texts.len(), s.board.shapes.len()), (texts, shapes + 3));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"Text \"REV A\" on F.SilkS deleted"}"#);
    let errors = [
        r#"{"cmd":"text","text":" ","at":[1,1]}"#,
        r#"{"cmd":"text","text":"A","at":[1]}"#,
        r#"{"cmd":"text","text":"A","at":[1,1],"layer":"F.Cu"}"#,
        r#"{"cmd":"text","text":"A","at":[1,1],"height":0}"#,
        r#"{"cmd":"line","points":[[0,0]]}"#,
        r#"{"cmd":"line","points":[[0,0],[1,1]],"width":-1}"#,
    ]
    .map(|c| Val::parse(&s.cli_line(c)).unwrap().str("error").map(|e| e.split(':').next().unwrap_or_default().to_string()));
    let why = [
        "give text, the words to write",
        "at must be [x_mm, y_mm]",
        "no layer F.Cu to draw on",
        "height must be more than 0 mm",
        "points must be [[x_mm, y_mm], …], two or more",
        "width must be more than 0 mm",
    ];
    assert_eq!(errors.map(Option::unwrap), why);
}

#[test]
fn cli_draws_the_boards_outline() {
    let mut s = test_session();
    let before = s.board.outline_size();
    let tab = r#"{"cmd":"outline","corners":[[0,1],[3,1],[3,0],[9,0],[9,1],[12,1],[12,20],[0,20]]}"#;
    assert_eq!(s.cli_line(tab), r#"{"outline_mm":[0,0,12,20]}"#);
    assert_eq!(s.board.outline_size(), Some((12_000_000, 20_000_000)));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"Outline drawn"}"#);
    assert_eq!(s.board.outline_size(), before);
    let error = r#"{"error":"corners must be [[x_mm, y_mm], …], three or more"}"#;
    assert_eq!(s.cli_line(r#"{"cmd":"outline","corners":[[0,0],[1,1]]}"#), error);
}

#[test]
fn cli_adds_library_parts() {
    let mut s = test_session();
    let dir = std::env::temp_dir().join(format!("cuprita-add-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let part = |name: &str| {
        let path = dir.join(format!("{name}.kicad_mod"));
        let pad = r#"(pad "1" thru_hole circle (at 0 0) (size 6 6) (drill 3) (layers "*.Cu"))"#;
        std::fs::write(&path, format!(r#"(footprint "{name}" (layer "F.Cu") (property "Reference" "REF**") {pad})"#)).unwrap();
        LibPart { lib: "MountingHole".into(), name: name.into(), path, about: String::new() }
    };
    s.parts = Some(vec![part("MountingHole_3.2mm_M3"), part("MountingHole_3.2mm_M3_Pad")]);
    let h1 = s.cli_line(r#"{"cmd":"add","footprint":"MountingHole_3.2mm_M3","at":[5,5],"angle":90}"#);
    assert_eq!(h1, r#"{"reference":"H1","at_mm":[5,5],"angle":90,"back":false,"bounds_mm":[0.757,0.757,9.243,9.243]}"#);
    let errors = [
        r#"{"cmd":"add","footprint":"m3","at":[5,5]}"#,
        r#"{"cmd":"add","footprint":"m4","at":[5,5]}"#,
        r#"{"cmd":"add","footprint":"pad","at":[5]}"#,
    ]
    .map(|c| Val::parse(&s.cli_line(c)).unwrap().str("error").map(String::from).unwrap());
    assert_eq!(
        errors,
        [
            "2 parts match m3: MountingHole:MountingHole_3.2mm_M3, MountingHole:MountingHole_3.2mm_M3_Pad…; give one as Library:Name",
            "no KiCad library part matches m4; give words of Library:Name (MountingHole M3)",
            "at must be [x_mm, y_mm]"
        ]
    );
    // The only match is enough; undo takes it back.
    let h2 = Val::parse(&s.cli_line(r#"{"cmd":"add","footprint":"pad","at":[15,5]}"#)).unwrap();
    assert_eq!((h2.str("reference"), s.board.footprints.len()), (Some("H2"), 3));
    s.cli_line(r#"{"cmd":"undo"}"#);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(s.board.footprints.len(), 2);
}

#[test]
fn cli_routes_tracks_with_vias() {
    let mut s = test_session();
    let r = Val::parse(&s.cli_line(r#"{"cmd":"route","from":[11.1,20],"to":[15,25],"layer":"F.Cu","via":true}"#)).unwrap();
    assert_eq!((r.str("net"), r.str("layer"), r.num("width_mm")), (Some("GND"), Some("F.Cu"), Some(0.25)));
    let pts = r.arr("points_mm").unwrap();
    assert_eq!((pts.len(), pts[0].to_json(), pts[2].to_json()), (3, "[11,20]".into(), "[15,25]".into()));
    assert_eq!(
        (r.bool("reached"), r.bool("connected"), r.bool("via"), r.num("violations")),
        (Some(true), Some(false), Some(true), Some(0.0))
    );
    assert_eq!((s.board.tracks.len(), s.board.vias.len()), (3, 1));
    // From the via on B.Cu, back to the GND track's end there: connected.
    let back = Val::parse(&s.cli_line(r#"{"cmd":"route","from":[15,25],"to":[11,20],"layer":"B.Cu"}"#)).unwrap();
    assert_eq!((back.bool("reached"), back.bool("connected")), (Some(true), Some(true)));
    let errors = [
        r#"{"cmd":"route","from":[5,5],"to":[6,6]}"#,
        r#"{"cmd":"route","from":[11,20],"to":[6]}"#,
        r#"{"cmd":"route","from":[11,20],"to":[6,6],"layer":"Edge.Cuts"}"#,
        r#"{"cmd":"route","from":[11,20],"to":[6,6],"width_mm":0}"#,
    ]
    .map(|c| Val::parse(&s.cli_line(c)).unwrap().str("error").map(String::from).unwrap());
    assert_eq!(
        errors,
        [
            "from must be on a pad, via or track",
            "to must be [x_mm, y_mm]",
            "no copper layer Edge.Cuts; see query layers",
            "width_mm must be a positive number"
        ]
    );
}

#[test]
fn cli_routes_shoving_other_nets_aside() {
    let mut s = test_session();
    // A /Bus/SCL track across the way, 0.3 mm under where GND's route ends.
    s.board.tracks.extend(tracks(&[mm_pt(2.0, 25.0), mm_pt(18.0, 25.0)], 250_000, 0, 1));
    let route = r#"{"cmd":"route","from":[11,20],"to":[11,24.7],"layer":"F.Cu","shove":true}"#;
    let r = Val::parse(&s.cli_line(route)).unwrap();
    assert_eq!((r.bool("reached"), r.num("moved"), r.num("violations"), r.get("walked_round")), (Some(true), Some(1.0), Some(0.0), None));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"1 track added, 1 moved aside"}"#);
    // Locked, it stays: the route stops short of it.
    s.board.tracks[1].locked = true;
    let r = Val::parse(&s.cli_line(route)).unwrap();
    assert_eq!((r.bool("reached"), r.num("moved")), (Some(false), Some(0.0)));
    // Along the board's edge it has no room to make way: the route walks round, and says why.
    let mut s = test_session();
    s.board.tracks.extend(tracks(&[mm_pt(2.0, 29.3), mm_pt(18.0, 29.3)], 250_000, 0, 1));
    let r = Val::parse(&s.cli_line(&route.replace("24.7", "29"))).unwrap();
    assert_eq!((r.bool("reached"), r.num("moved"), r.str("walked_round")), (Some(false), Some(0.0), Some("The board edge is in the way")));
}

#[test]
fn cli_undoes_and_redoes_edits() {
    let mut s = test_session();
    let cmd = |s: &mut Session, c: &str| s.cli_line(&format!(r#"{{"cmd":"{c}"}}"#));
    s.cli_line(r#"{"cmd":"place","reference":"R1","at":[12.5,20]}"#);
    s.cli_line(r#"{"cmd":"route","from":[11.1,20],"to":[15,25],"layer":"F.Cu","via":true}"#);
    assert_eq!((s.board.tracks.len(), s.board.vias.len()), (3, 1));
    assert_eq!(cmd(&mut s, "undo"), r#"{"undone":"Via added"}"#);
    assert_eq!((s.board.tracks.len(), s.board.vias.len()), (1, 0));
    assert_eq!(cmd(&mut s, "undo"), r#"{"undone":"R1 placed"}"#);
    assert_eq!((s.board.footprints[0].at, cmd(&mut s, "undo")), (mm_pt(10.0, 20.0), r#"{"error":"nothing to undo"}"#.into()));
    assert_eq!((cmd(&mut s, "redo"), s.board.footprints[0].at), (r#"{"redone":"R1 placed"}"#.into(), mm_pt(12.5, 20.0)));
    // The scene follows: the part's pad is picked where it went.
    let pick = Val::parse(&s.cli_line(r#"{"cmd":"query","at":[11.5,20.0]}"#)).unwrap();
    assert_eq!(pick.str("name"), Some("/Bus/SCL"));
}

#[test]
fn cli_fills_zones_and_undoes_it() {
    let board = Board::parse(
        r#"(kicad_pcb (version 20260206) (layers (0 "F.Cu" signal)) (net 0 "") (net 1 "GND")
          (zone (net 1) (layer "F.Cu") (polygon (pts (xy 0 0) (xy 10 0) (xy 10 10) (xy 0 10)))))"#,
    )
    .unwrap();
    let mut s = Session { scene: Some(build(&board)), board, ..test_session() };
    let mut cmd = |c: &str| s.cli_line(&format!(r#"{{"cmd":"{c}"}}"#));
    assert_eq!((cmd("fill"), cmd("fill")), (r#"{"zones_changed":1}"#.into(), r#"{"zones_changed":0}"#.into()));
    // The scene follows: the fill is picked.
    let pick = Val::parse(&s.cli_line(r#"{"cmd":"query","at":[5,5]}"#)).unwrap();
    assert_eq!((pick.str("name"), s.board.fills.len()), (Some("GND"), 1));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"1 zone filled"}"#);
    assert!(s.board.fills.is_empty());
}

#[test]
fn cli_adds_zones_and_undoes_them() {
    let mut s = test_session();
    let zone = |s: &mut Session, args: &str| s.cli_line(&format!(r#"{{"cmd":"zone",{args}}}"#));
    let square = r#""corners":[[0,0],[20,0],[20,30],[0,30]]"#;
    for (args, error) in [
        (format!(r#""net":"gnd",{square}"#), "give the zone's net and layer by name (query nets, query layers)"),
        (format!(r#""net":"VCC","layer":"F.Cu",{square}"#), "no net VCC; see query nets"),
        (format!(r#""net":"gnd","layer":"Edge.Cuts",{square}"#), "no copper layer Edge.Cuts; see query layers"),
        (r#""net":"gnd","layer":"F.Cu","corners":[[0,0],[20,0],[20]]"#.into(), "corners must be [[x_mm, y_mm], …], three or more"),
    ] {
        assert_eq!(zone(&mut s, &args), format!(r#"{{"error":{}}}"#, json_str(error)));
    }
    assert_eq!(zone(&mut s, &format!(r#""net":"gnd","layer":"f.cu",{square}"#)), r#"{"added":"Zone (GND) on F.Cu","filled_areas":1}"#);
    // The scene follows: the fill is picked.
    let pick = Val::parse(&s.cli_line(r#"{"cmd":"query","at":[5,5]}"#)).unwrap();
    assert_eq!(pick.str("name"), Some("GND"));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"Zone (GND) on F.Cu added"}"#);
    assert!(s.board.zones.is_empty() && s.board.fills.is_empty());
    // Pads joined solid, as asked; taken back in one step, settings and all.
    zone(&mut s, &format!(r#""net":"gnd","layer":"F.Cu","pads":"solid",{square}"#));
    assert_eq!(s.board.zones[0].connect, Connect::Solid);
    s.cli_line(r#"{"cmd":"undo"}"#);
    assert!(s.board.zones.is_empty() && s.board.fills.is_empty());
    let error = r#"{"error":"pads ok: give thermal, solid, thru_hole_thermal or none"}"#;
    assert_eq!(zone(&mut s, &format!(r#""net":"gnd","layer":"F.Cu","pads":"ok",{square}"#)), error);
}

#[test]
fn cli_tidies_texts_as_one_undo_step() {
    let mut s = test_session();
    let done = Val::parse(&s.cli_line(r#"{"cmd":"tidy","references":["r1"]}"#)).unwrap();
    assert_eq!((done.num("placed"), done.num("hidden"), done.num("parts_changed")), (Some(1.0), Some(0.0), Some(1.0)));
    // R1's reference, on its pads, reads above them at KiCad's size; again nothing moves.
    let r1 = &s.board.texts[0];
    assert_eq!((r1.size, r1.angle, r1.at.y < 19_000_000), ((1_000_000, 1_000_000), 0.0, true));
    assert_eq!(Val::parse(&s.cli_line(r#"{"cmd":"tidy"}"#)).unwrap().num("parts_changed"), Some(0.0));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"Texts of 1 tidied"}"#);
    assert_eq!(s.board.texts[0].at, Pt { x: 10_000_000, y: 20_000_000 });
    assert_eq!(s.cli_line(r#"{"cmd":"tidy","references":["Q9"]}"#), r#"{"error":"not found: Q9"}"#);
}

#[test]
fn cli_moves_sizes_hides_and_shows_a_parts_text() {
    let mut s = test_session();
    let run = |s: &mut Session, args: &str| s.cli_line(&format!(r#"{{"cmd":"part_text","reference":"r1"{args}}}"#));
    // Said where it is; moved, turned and sized as one step, the part staying.
    let is = Val::parse(&run(&mut s, "")).unwrap();
    assert_eq!(
        (is.str("text"), is.str("layer"), is.arr("at_mm").map(<[Val]>::len), is.num("height_mm")),
        (Some("R1"), Some("F.SilkS"), Some(2), Some(1.0))
    );
    let set = Val::parse(&run(&mut s, r#","at":[10,17.5],"angle":90,"height":0.8"#)).unwrap();
    assert_eq!((set.num("angle"), set.num("height_mm"), s.board.texts[0].thickness), (Some(90.0), Some(0.8), 120_000));
    assert_eq!((s.board.texts[0].at, s.board.footprints[0].at), (Pt { x: 10_000_000, y: 17_500_000 }, Pt { x: 10_000_000, y: 20_000_000 }));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"Text \"R1\" of R1 set"}"#);
    // Hidden, its part keeps it: it must be shown before it is changed.
    assert_eq!(run(&mut s, r#","show":false"#), r#"{"text":"R1","of":"R1","shown":false}"#);
    assert_eq!((s.board.texts.len(), s.board.footprints[0].hidden.len()), (0, 1));
    assert_eq!(run(&mut s, r#","at":[1,1]"#), r#"{"error":"R1's text R1 is hidden: show it first (show: true)"}"#);
    assert_eq!(Val::parse(&run(&mut s, r#","show":true"#)).unwrap().str("layer"), Some("F.SilkS"));
    assert_eq!((s.board.texts.len(), s.board.texts[0].at), (1, Pt { x: 10_000_000, y: 20_000_000 }));
    assert_eq!(run(&mut s, r#","text":"MPN""#), r#"{"error":"R1 has no text MPN; it has: R1"}"#);
    assert_eq!(s.cli_line(r#"{"cmd":"part_text","reference":"Q9"}"#), r#"{"error":"not found: Q9"}"#);
}

#[test]
fn cli_deletes_parts_tracks_vias_and_zones() {
    let mut s = test_session();
    let del = |s: &mut Session, args: &str| s.cli_line(&format!(r#"{{"cmd":"delete",{args}}}"#));
    s.cli_line(r#"{"cmd":"route","from":[11.1,20],"to":[15,25],"layer":"F.Cu","via":true}"#);
    let via = s.board.vias[0].at;
    let at_via = format!(r#""at":[{},{}]"#, via.x as f64 / 1e6, via.y as f64 / 1e6);
    assert_eq!((s.board.tracks.len(), s.board.vias.len(), s.board.footprints.len()), (3, 1, 1));
    // Between R1's pads only the B.Cu track lies; on a pad, the part must be named.
    for (args, error) in [
        (r#""at":[10,20],"layer":"F.Cu""#, "no track, via, text, drawing or zone at [10, 20]; query at says what is there"),
        (r#""at":[9,20]"#, "[9, 20] is on a pad of R1: to delete the part, give its reference"),
        (r#""reference":"Q9""#, "not found: Q9"),
        (r#""at":[1]"#, "give reference (a part) or at [x_mm, y_mm] (a track, via, text, drawing or zone)"),
    ] {
        assert_eq!(del(&mut s, args), format!(r#"{{"error":{}}}"#, json_str(error)));
    }
    assert_eq!(del(&mut s, r#""at":[10,20]"#), r#"{"deleted":"Track (GND) on B.Cu"}"#);
    assert_eq!(del(&mut s, &at_via), r#"{"deleted":"Via (GND)"}"#);
    assert_eq!(del(&mut s, r#""reference":"r1""#), r#"{"deleted":"R1"}"#);
    assert_eq!((s.board.tracks.len(), s.board.vias.len(), s.board.footprints.len(), s.board.pads.len()), (2, 0, 0, 0));
    // Undone, all three come back.
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"R1 deleted"}"#);
    s.cli_line(r#"{"cmd":"undo"}"#);
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"Track (GND) on B.Cu deleted"}"#);
    assert_eq!((s.board.tracks.len(), s.board.vias.len(), s.board.footprints.len(), s.board.pads.len()), (3, 1, 1, 2));
    // A zone by its fill, or its outline; undone, it comes back.
    s.cli_line(r#"{"cmd":"zone","net":"GND","layer":"B.Cu","corners":[[2,2],[6,2],[6,6],[2,6]]}"#);
    assert_eq!(del(&mut s, r#""at":[4,4]"#), r#"{"deleted":"Zone (GND) on B.Cu"}"#);
    assert_eq!((s.board.zones.len(), s.board.fills.len()), (0, 0));
    assert_eq!(s.cli_line(r#"{"cmd":"undo"}"#), r#"{"undone":"Zone (GND) on B.Cu deleted"}"#);
    assert_eq!(del(&mut s, r#""at":[2,4.03]"#), r#"{"deleted":"Zone (GND) on B.Cu"}"#);
}

#[test]
fn cli_saves_edits_keeping_the_previous_version() {
    let dir = std::env::temp_dir().join(format!("cuprita-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (pcb, bak) = (dir.join("b.kicad_pcb"), dir.join("b.kicad_pcb-bak"));
    let src = r#"(kicad_pcb (version 20240108) (footprint "R" (at 10 20) (property "Reference" "R1")))"#;
    std::fs::write(&pcb, src).unwrap();
    let mut s = Session::default();
    s.open(&pcb).unwrap();
    s.cli_line(r#"{"cmd":"place","reference":"R1","at":[12.5,20]}"#);
    let saved = Val::parse(&s.cli_line(r#"{"cmd":"save"}"#)).unwrap();
    assert_eq!((saved.str("saved"), saved.str("backup")), (pcb.to_str(), bak.to_str()));
    let moved = src.replace("(at 10 20)", "(at 12.5 20)");
    assert_eq!((std::fs::read_to_string(&pcb).unwrap(), std::fs::read_to_string(&bak).unwrap()), (moved, src.into()));
    // The next save writes every edit into the text read; a file gone since leaves nothing to keep.
    std::fs::remove_file(&pcb).unwrap();
    s.cli_line(r#"{"cmd":"place","reference":"R1","at":[12.5,21]}"#);
    assert_eq!(Val::parse(&s.cli_line(r#"{"cmd":"save"}"#)).unwrap().get("backup"), Some(&Val::Null));
    assert_eq!(std::fs::read_to_string(&pcb).unwrap(), src.replace("(at 10 20)", "(at 12.5 21)"));
    // Both moves undone and saved: the first text again.
    for _ in 0..2 {
        s.cli_line(r#"{"cmd":"undo"}"#);
    }
    s.cli_line(r#"{"cmd":"save"}"#);
    assert_eq!(std::fs::read_to_string(&pcb).unwrap(), src);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_writes_fab_files_next_to_the_board() {
    let dir = std::env::temp_dir().join(format!("cuprita-fab-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pcb = dir.join("b.kicad_pcb");
    let src = r#"(kicad_pcb (version 20240108) (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
      (footprint "R:R_0603" (layer "F.Cu") (at 10 20) (property "Reference" "R1") (property "Value" "10k") (attr smd)
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
      (gr_rect (start 0 0) (end 20 30) (stroke (width 0.1)) (layer "Edge.Cuts")))"#;
    std::fs::write(&pcb, src).unwrap();
    let mut s = Session::default();
    s.open(&pcb).unwrap();
    let paths = |v: &Val, k: &str| v.arr(k).unwrap().iter().map(Val::to_json).collect::<Vec<_>>();
    let names = ["b-gerbers.zip", "b-bom.csv", "b-positions.csv"].map(|n| json_str(&dir.join(n).display().to_string()));
    let out = Val::parse(&s.cli_line(r#"{"cmd":"fab"}"#)).unwrap();
    assert_eq!((paths(&out, "files"), paths(&out, "backups"), out.num("zones_changed")), (names.to_vec(), vec![], Some(0.0)));
    let read = |n: &str| std::fs::read(dir.join(n)).unwrap();
    assert_eq!(&read("b-gerbers.zip")[..4], b"PK\x03\x04");
    assert_eq!(read("b-positions.csv"), b"Designator,Mid X,Mid Y,Layer,Rotation\nR1,10,-20,Top,0\n");
    assert!(String::from_utf8(read("b-bom.csv")).unwrap().ends_with("\n10k,R1,R_0603,1,,,\n"));
    // Written again, the files there are kept.
    let out = Val::parse(&s.cli_line(r#"{"cmd":"fab"}"#)).unwrap();
    assert_eq!(paths(&out, "backups").len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}
