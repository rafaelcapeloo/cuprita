//! The `render` command: a PNG of the board, a found part or net, a box, a visual diff, a DC
//! IR-drop heatmap, or the board in 3D, drawn headless.

use super::{Session, mm_box, mm_pt};
use crate::{Found, find_target, load_base, save_png};
use cuprita_core::diff::Diff;
use cuprita_core::json::{Val, json_str};
use cuprita_core::models;
use cuprita_render::{Orbit, build_diff, build_ir, render_rgba, render_rgba3d};
use std::path::Path;

impl Session {
    pub(super) fn render(&mut self, args: &Val) -> Result<String, String> {
        let scene = self.scene.as_ref().ok_or("no board open")?;
        let out = args.str("out").ok_or("missing out")?;
        let width = args.num("width").unwrap_or(1600.0) as u32;
        let flip = args.bool("flip").unwrap_or(false);
        let mut net = match args.str("net") {
            Some(q) => self.board.find_net(q).ok_or_else(|| format!("not found: {q}"))?,
            None => 0,
        };
        let alt_scene = match (args.get("diff"), args.get("ir")) {
            (Some(Val::Bool(true)), _) => {
                let (_, old) = load_base(self.path.as_deref(), None)?;
                Some(build_diff(&old, &self.board, &Diff::between(&old, &self.board)))
            }
            (Some(Val::Str(b)), _) => {
                let (_, old) = load_base(self.path.as_deref(), Some(b))?;
                Some(build_diff(&old, &self.board, &Diff::between(&old, &self.board)))
            }
            (_, Some(v)) => {
                let ir_net = match v {
                    Val::Str(q) => self.board.find_net(q).ok_or_else(|| format!("not found: {q}"))?,
                    Val::Bool(true) if net != 0 => net,
                    Val::Bool(true) => self.board.default_ir_net(),
                    _ => 0,
                };
                (ir_net != 0).then(|| {
                    let ir = self.board.ir_drop(ir_net, args.num("current_a").unwrap_or(1.0), args.str("source"), args.str("sink"));
                    build_ir(&self.board, &ir)
                })
            }
            _ => None,
        };
        if alt_scene.is_some() {
            net = 1;
        }
        let bounds = if let Some([Val::Num(x0), Val::Num(y0), Val::Num(x1), Val::Num(y1)]) = args.arr("bounds_mm") {
            let ([ax, ay], [bx, by]) = (scene.um(mm_pt(*x0, *y0)), scene.um(mm_pt(*x1, *y1)));
            [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)]
        } else if let Some(q) = args.str("find") {
            match find_target(&self.board, scene, q).ok_or_else(|| format!("not found: {q}"))? {
                Found::Footprint { view, .. } => view,
                Found::Net { net: n, view, .. } => {
                    if net == 0 {
                        net = n;
                    }
                    view.unwrap_or(scene.bounds)
                }
            }
        } else if args.bool("3d").unwrap_or(false) {
            scene.bounds
        } else {
            // As the window fits it: the parts beside the board too, where an update leaves them.
            crate::whole(&self.board, scene)
        };
        let visible: Vec<bool> = match args.arr("layers") {
            Some(names) => {
                scene.layers.iter().map(|l| names.iter().any(|v| matches!(v, Val::Str(s) if s.eq_ignore_ascii_case(&l.name)))).collect()
            }
            None => scene.layers.iter().map(|l| l.visible).collect(),
        };
        let three = args.bool("3d").unwrap_or(false);
        let (w, h, rgba) = if three {
            let missing: Vec<String> = models::files(&self.board).into_iter().filter(|f| !self.models.contains_key(f)).collect();
            self.models.extend(models::load(&missing, self.path.as_deref().and_then(Path::parent)));
            let size = [width.clamp(1, 8192), (width * 3 / 4).clamp(1, 8192)];
            let mut o = Orbit::fit(bounds, size.map(|s| s as f32));
            if flip {
                (o.yaw, o.pitch) = (180.0, -o.pitch);
            }
            (size[0], size[1], render_rgba3d(scene, &self.board, &self.models, &o, size))
        } else {
            render_rgba(alt_scene.as_ref().unwrap_or(scene), bounds, width, &visible, net, flip)
        };
        save_png(Path::new(out), w, h, &rgba).map_err(|e| e.to_string())?;
        // A 3D render says which models it couldn't show, and why.
        let missing = if three {
            let files = models::files(&self.board);
            let errors: Vec<String> = files.iter().filter_map(|f| self.models.get(f)?.as_ref().err()).map(|e| json_str(e)).collect();
            format!(r#","models_missing":[{}]"#, errors.join(","))
        } else {
            String::new()
        };
        Ok(format!(r#"{{"out":{},"width":{w},"height":{h},"bounds_mm":{}{missing}}}"#, json_str(out), mm_box(scene, bounds)))
    }
}
