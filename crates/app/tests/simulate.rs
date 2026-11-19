//! Simulate by the program itself, as a user's run goes: where ngspice loads only inside KiCad's
//! Flatpak (its runtime's C library newer than the system's), `cuprita --ngspice` runs there.

use std::process::Command;

#[test]
fn the_program_simulates_a_corpus_demo() {
    let demo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/boards/sim/rectifier/rectifier.kicad_sch");
    if !demo.is_file() || cuprita_core::ngspice::found().is_err() {
        return eprintln!("no corpus (corpus/fetch.sh) or no ngspice");
    }
    let ask = format!(r#"{{"cmd":"simulate","path":"{}","signals":["/rect_out"]}}"#, demo.display());
    let out = Command::new(env!("CARGO_BIN_EXE_cuprita")).args(["--json", &ask]).output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    // A transient of 10 ms; the 1 V sine less a diode's drop on the output.
    assert!(said.contains(r#""analysis":"tran1""#) && said.contains(r#""to":0.01"#), "{said}");
    let peak =
        said.split(r#""name":"V(/rect_out)""#).nth(1).and_then(|s| s.split(r#""max":"#).nth(1)?.split(',').next()?.parse::<f64>().ok());
    assert!(peak.is_some_and(|v| (0.35..0.45).contains(&v)), "{peak:?} in {said}");
}
