//! Footprints' 3D models: their files found where KiCad finds them, with nothing to set up, and
//! read in parallel.

use crate::board::{Board, Model, Src};
use crate::history::Patch;
use crate::library::kicad_dirs;
use crate::step::{self, Mesh};
use crate::threads::par_map;
use crate::tr::trf;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 3D models by their file as written: the mesh, or what is wrong with it and how to fix it.
pub type Models = HashMap<String, Result<Mesh, String>>;

impl Board {
    /// Gives footprint `i` the shown 3D models `models` in place of its own, as saving writes
    /// them (a footprint kept as library text has it rewritten now); returns the patch that
    /// undoes it.
    pub fn set_models(&mut self, i: usize, models: Vec<Model>) -> Patch {
        let undo = Patch::footprint(self, i);
        let f = &mut self.footprints[i];
        if let Src::Library(text) | Src::Swapped(_, text) = &mut f.src {
            *text = crate::save::models::rewritten(text, &models);
        }
        f.models = models;
        undo
    }
}

/// The model files the board's footprints show, each once, as written.
pub fn files(b: &Board) -> Vec<String> {
    let mut files: Vec<String> = b.footprints.iter().flat_map(|f| &f.models).map(|m| m.file.clone()).collect();
    files.sort_unstable();
    files.dedup();
    files
}

/// Reads model `files` (as written) of a board in folder `dir`, on all cores.
pub fn load(files: &[String], dir: Option<&Path>) -> Models {
    let kicad = kicad_dirs("3dmodels", "_3DMODEL_DIR");
    let read = |file: &String| {
        let path = find(file, dir, &kicad)?;
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        step::read(&data).map_err(|e| format!("{}: {e}", path.display()))
    };
    files.iter().cloned().zip(par_map(files, read)).collect()
}

/// Where model `file` (as a board writes it) of a board in folder `dir` is, as `load` finds it.
pub fn path(file: &str, dir: Option<&Path>) -> Result<PathBuf, String> {
    find(file, dir, &kicad_dirs("3dmodels", "_3DMODEL_DIR"))
}

/// Where model `file` is: its `${VAR}`s from the environment, else the board's folder
/// (`KIPRJMOD`) or KiCad's 3D model folders (`KISYS3DMOD`, `KICAD*_3DMODEL_DIR`, any version); a
/// relative file is in the board's folder. A `.wrl` that isn't there is read as the `.step`
/// beside it: KiCad 10 ships only those.
// shortcut: variables only KiCad's settings define (Preferences > Configure Paths) must be set in
// the environment; read `kicad_common.json` when someone keeps models that way.
fn find(file: &str, dir: Option<&Path>, kicad: &[PathBuf]) -> Result<PathBuf, String> {
    let (var, rest) = match file.strip_prefix("${").or_else(|| file.strip_prefix("$(")) {
        Some(s) => s.split_once(['}', ')']).map_or((None, file), |(v, r)| (Some(v), r.trim_start_matches(['/', '\\']))),
        None => (None, file),
    };
    let env = var.and_then(std::env::var_os).map(PathBuf::from);
    let (roots, installed): (Vec<PathBuf>, _) = match var {
        None => (dir.into_iter().map(Path::to_path_buf).collect(), false),
        Some(_) if env.is_some() => (env.into_iter().collect(), false),
        Some("KIPRJMOD") => (dir.into_iter().map(Path::to_path_buf).collect(), false),
        Some(v) if v == "KISYS3DMOD" || v.starts_with("KICAD") && v.ends_with("_3DMODEL_DIR") => (kicad.to_vec(), true),
        Some(v) => return Err(trf("{0}: {1} is not set; set it to the folder of its 3D models", &[&file, &v])),
    };
    let rest = rest.replace('\\', "/");
    for path in roots.iter().map(|r| r.join(&rest)) {
        let wrl = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wrl") || e.eq_ignore_ascii_case("wrz"));
        let step = [path.with_extension("step"), path.with_extension("stp")].into_iter().find(|p| p.is_file());
        match (wrl, step) {
            (true, Some(step)) => return Ok(step),
            (true, None) if path.is_file() => {
                return Err(trf("{0}: WRL models aren't read; give the part a STEP model", &[&path.display()]));
            }
            (false, _) if path.is_file() => return Ok(path),
            _ => {}
        }
    }
    let looked: Vec<String> = roots.iter().filter(|r| r.is_dir()).map(|r| r.display().to_string()).collect();
    Err(match (looked.is_empty(), var) {
        (true, Some(v)) if installed => {
            trf("{0}: KiCad's 3D models aren't installed; install them, or set {1} to their folder", &[&file, &v])
        }
        (true, _) => {
            let roots = roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", ");
            trf("{0}: not found: no folder {1}", &[&file, &roots])
        }
        _ => trf("{0}: not found in {1}", &[&file, &looked.join(", ")]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_models_where_kicad_does_and_steps_for_wrls() {
        let tmp = std::env::temp_dir().join(format!("cuprita-models-{}", std::process::id()));
        let (kicad, board) = (tmp.join("3dmodels"), tmp.join("board"));
        std::fs::create_dir_all(kicad.join("R.3dshapes")).unwrap();
        std::fs::create_dir_all(board.join("lib")).unwrap();
        std::fs::write(kicad.join("R.3dshapes/R.step"), "").unwrap();
        std::fs::write(board.join("lib/X.STEP"), "").unwrap();
        std::fs::write(board.join("W.wrl"), "").unwrap();
        let find = |f: &str| find(f, Some(&board), std::slice::from_ref(&kicad));
        assert_eq!(find("${KICAD10_3DMODEL_DIR}/R.3dshapes/R.step"), Ok(kicad.join("R.3dshapes/R.step")));
        // KiCad 5 linked WRL files, which KiCad 10 ships as STEP.
        assert_eq!(find("${KISYS3DMOD}/R.3dshapes/R.wrl"), Ok(kicad.join("R.3dshapes/R.step")));
        assert_eq!(find("${KIPRJMOD}/lib/X.STEP"), Ok(board.join("lib/X.STEP")));
        assert_eq!(find("lib/X.STEP"), Ok(board.join("lib/X.STEP")));
        assert!(find("W.wrl").unwrap_err().contains("WRL models aren't read"));
        assert!(find("${MY_LIB_3D}/A.step").unwrap_err().contains("MY_LIB_3D is not set"));
        assert_eq!(find("${KIPRJMOD}/none.step"), Err(format!("${{KIPRJMOD}}/none.step: not found in {}", board.display())));
        let bare = super::find("${KICAD10_3DMODEL_DIR}/R.step", None, &[]);
        assert!(bare.unwrap_err().contains("aren't installed; install them, or set KICAD10_3DMODEL_DIR"));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn gives_a_footprint_new_models_that_save_and_read_back() {
        let new = Model {
            file: "${KIPRJMOD}/libraries/p.3dshapes/LED.step".into(),
            offset: [0.0, -1.5, 0.0],
            scale: [1.0; 3],
            rotate: [0.0, 0.0, 90.0],
        };
        let kicad8 = "(kicad_pcb (version 20240108)\n\t(footprint \"LED_SMD:LED_0603\"\n\t\t(layer \"F.Cu\")\n\t\t(at 10 10)\n\t\t(model \"${KICAD8_3DMODEL_DIR}/Old.wrl\"\n\t\t\t(offset\n\t\t\t\t(xyz 0 0 0)\n\t\t\t)\n\t\t)\n\t\t(model \"Hidden.step\" (hide yes))\n\t)\n)\n";
        // KiCad 6 on one line a list, in spaces; a footprint without a model.
        let kicad6 = "(kicad_pcb (version 20211014)\n  (footprint \"LED_SMD:LED_0603\" (layer \"F.Cu\")\n    (at 10 10)\n  )\n)\n";
        for (src, wants) in [
            (kicad8, "\t\t(model \"${KIPRJMOD}/libraries/p.3dshapes/LED.step\"\n\t\t\t(offset\n\t\t\t\t(xyz 0 -1.5 0)\n\t\t\t)"),
            (
                kicad6,
                "    (model \"${KIPRJMOD}/libraries/p.3dshapes/LED.step\"\n      (offset (xyz 0 -1.5 0))\n      (scale (xyz 1 1 1))\n      (rotate (xyz 0 0 90))\n    )",
            ),
        ] {
            let mut b = Board::parse(src).unwrap();
            let undo = b.set_models(0, vec![new.clone()]);
            let saved = b.write(src).unwrap();
            assert!(saved.contains(wants), "{saved}");
            assert!(!saved.contains("Old.wrl") && saved.contains("Hidden.step") == src.contains("Hidden.step"), "{saved}");
            assert_eq!(Board::parse(&saved).unwrap().footprints[0].models, std::slice::from_ref(&new));
            // Undone, the file is as it was.
            b.apply(undo);
            assert_eq!(b.write(src).unwrap(), src);
        }
        // A footprint added from a library has its own text rewritten.
        let mut b = Board::parse(kicad6).unwrap();
        let lib = "(footprint \"LED_0805\" (version 20240108) (layer \"F.Cu\")\n\t(pad \"1\" smd rect (at -1 0) (size 1 1) (layers \"F.Cu\"))\n)\n";
        let i = b.add_footprint(lib, "LED_SMD", crate::board::Pt { x: 0, y: 0 }, 0.0).unwrap();
        b.set_models(i, vec![new.clone()]);
        let saved = b.write(kicad6).unwrap();
        assert_eq!(Board::parse(&saved).unwrap().footprints[i].models, [new]);
    }
}
