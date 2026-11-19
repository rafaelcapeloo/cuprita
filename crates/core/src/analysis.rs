//! Net analysis on a parsed board: track length, IPC-2152 current, impedance and DC IR drop.

use crate::board::{Board, DEFAULT_BOARD, DEFAULT_CU, LayerId, LayerMask, NetId, Nm, Pt, reach, union};
use std::f64::consts::PI;

/// Electrical and physical summary of a net.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetInfo {
    pub tracks: usize,
    pub vias: usize,
    pub pads: usize,
    pub zones: usize,
    /// Total routed centerline length in millimetres (straight + arc tracks).
    pub length_mm: f64,
    /// Narrowest and widest track widths on the net (`(0, 0)` when it has no tracks).
    pub width: (Nm, Nm),
    /// Maximum continuous current (A) of the bottleneck track for a 10 °C IPC-2152 rise.
    pub current_a: f64,
    /// Characteristic impedance range (min, max) in Ω across the net's tracks.
    pub impedance_ohm: (f64, f64),
}

/// Voltage drop and path resistance from the current source to one pad on a net.
#[derive(Clone, Debug, PartialEq)]
pub struct PadDrop {
    pub pad: usize,
    pub reference: String,
    pub number: String,
    pub at: Pt,
    pub drop_mv: f64,
    pub resistance_mohm: f64,
}

/// DC IR-drop analysis result for a net at `current_a` Amps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrDrop {
    pub net: NetId,
    pub current_a: f64,
    pub source: String,
    pub sink: String,
    pub max_drop_mv: f64,
    pub resistance_mohm: f64,
    pub power_mw: f64,
    pub max_j_a_mm2: f64,
    pub track_drops: Vec<(f64, f64)>,
    pub via_drops: Vec<f64>,
    pub pad_drops: Vec<f64>,
    pub nodes: Vec<(LayerId, Pt, f64)>,
    pub pads: Vec<PadDrop>,
}

impl Board {
    /// Copper thickness on `layer` from the stackup, or 35 µm (1 oz) when unspecified.
    pub fn cu_thickness(&self, layer: LayerId) -> Nm {
        self.stackup.iter().find(|c| c.layer == layer).map_or(DEFAULT_CU, |c| c.thickness)
    }

    /// Single-ended characteristic impedance (Ω) of a trace of `width` on `layer` from the board
    /// stackup (Hammerstad-Jensen microstrip on outer layers, Wheeler stripline on inner layers).
    pub fn layer_impedance(&self, layer: LayerId, width: Nm) -> Option<f64> {
        if width <= 0 || self.stackup.len() < 2 {
            return None;
        }
        let i = self.stackup.iter().position(|c| c.layer == layer)?;
        let (w, t) = (width as f64, self.stackup[i].thickness.max(1) as f64);
        let z0 = if i == 0 || i + 1 == self.stackup.len() {
            let d = &self.stackup[if i == 0 { 0 } else { i - 1 }];
            let (h, er) = (d.dielectric as f64, d.epsilon_r);
            if h <= 0.0 || er < 1.0 {
                return None;
            }
            let c = if w / h <= 0.5 / PI { (4.0 * PI * w / t).ln() } else { (2.0 * h / t).ln() };
            let u = (w + (1.25 * t / PI) * (1.0 + c.max(0.0))) / h;
            let ee = 0.5 * (er + 1.0) + 0.5 * (er - 1.0) * (1.0 + 12.0 / u).powf(-0.5);
            if u <= 1.0 {
                60.0 / ee.sqrt() * (8.0 / u + 0.25 * u).ln()
            } else {
                120.0 * PI / (ee.sqrt() * (u + 1.393 + 0.667 * (u + 1.444).ln()))
            }
        } else {
            let (above, below) = (&self.stackup[i - 1], &self.stackup[i]);
            let (h1, h2) = (above.dielectric as f64, below.dielectric as f64);
            if h1 <= 0.0 || h2 <= 0.0 {
                return None;
            }
            let h = 2.0 * h1 * h2 / (h1 + h2);
            let er = (h2 * above.epsilon_r + h1 * below.epsilon_r) / (h1 + h2);
            if er < 1.0 {
                return None;
            }
            let x = 1.9 * (2.0 * h + t) / (0.8 * w + t);
            30.0 / er.sqrt() * (1.0 + 0.5 * x * (x + (x * x + 6.27).sqrt())).ln()
        };
        (z0 > 0.0 && z0.is_finite()).then_some(z0)
    }

    /// Computes total track length, track width range, IPC-2152 current (10 °C rise at the
    /// bottleneck track) and characteristic impedance range for `net`.
    pub fn inspect_net(&self, net: NetId) -> NetInfo {
        if net == 0 {
            return NetInfo::default();
        }
        let vias = self.vias.iter().filter(|v| v.net == net).count();
        let pads = self.pads.iter().filter(|p| p.net == net).count();
        let zones = self.fills.iter().filter(|z| z.net == net).count();
        let (mut tracks, mut len_nm, mut w_min, mut w_max) = (0, 0.0, Nm::MAX, 0);
        let (mut bottleneck, mut z_min, mut z_max) = (None::<(Nm, Nm)>, f64::INFINITY, 0.0f64);
        for t in self.tracks.iter().filter(|t| t.net == net) {
            tracks += 1;
            len_nm += t.len();
            if t.width > 0 {
                (w_min, w_max) = (w_min.min(t.width), w_max.max(t.width));
                let cu = self.cu_thickness(t.layer);
                if bottleneck.is_none_or(|(bw, bt)| (t.width as i64) * (cu as i64) < (bw as i64) * (bt as i64)) {
                    bottleneck = Some((t.width, cu));
                }
                if let Some(z) = self.layer_impedance(t.layer, t.width) {
                    (z_min, z_max) = (z_min.min(z), z_max.max(z));
                }
            }
        }
        NetInfo {
            tracks,
            vias,
            pads,
            zones,
            length_mm: len_nm / 1e6,
            width: if w_max > 0 { (w_min, w_max) } else { (0, 0) },
            current_a: bottleneck.map_or(0.0, |(w, cu)| ipc2152_current(w, cu, 10.0)),
            impedance_ohm: if z_max > 0.0 { (z_min, z_max) } else { (0.0, 0.0) },
        }
    }

    /// Best default net for DC IR-drop analysis (prefers routed power rails with pads).
    pub fn default_ir_net(&self) -> NetId {
        (1..self.nets.len() as NetId)
            .max_by_key(|&n| {
                let (t, p) = (self.tracks.iter().filter(|t| t.net == n).count(), self.pads.iter().filter(|p| p.net == n).count());
                let u = self.nets[n as usize].to_ascii_uppercase();
                let pwr = u.starts_with('+') || u.starts_with('V') || u.contains("VCC") || u.contains("VDD");
                (t > 0 && p >= 2, pwr && t > 1, t + p)
            })
            .unwrap_or(0)
    }

    /// Solves the DC resistor network of `net` at `current_a` Amps from `source` to `sink` (auto-picked when `None`).
    pub fn ir_drop(&self, net: NetId, current_a: f64, source: Option<&str>, sink: Option<&str>) -> IrDrop {
        const CU_RHO: f64 = 17.24; // Ω·nm at 20 °C
        const VIA_PLATE: f64 = 25_000.0; // nm
        let mut out = IrDrop {
            net,
            current_a,
            track_drops: vec![(0.0, 0.0); self.tracks.len()],
            via_drops: vec![0.0; self.vias.len()],
            pad_drops: vec![0.0; self.pads.len()],
            ..IrDrop::default()
        };
        if net == 0 || net as usize >= self.nets.len() || current_a <= 0.0 {
            return out;
        }
        let mut nodes: Vec<(LayerMask, Pt, Nm)> = Vec::new();
        let pad_nodes: Vec<_> = self
            .pads
            .iter()
            .enumerate()
            .filter(|(_, p)| p.net == net && (p.layers & self.copper) != 0)
            .map(|(i, p)| {
                (i, {
                    nodes.push((p.layers & self.copper, p.at, reach(p)));
                    nodes.len() - 1
                })
            })
            .collect();
        let via_nodes: Vec<_> = self
            .vias
            .iter()
            .enumerate()
            .filter(|(_, v)| v.net == net && (v.layers & self.copper) != 0)
            .map(|(i, v)| {
                (i, {
                    nodes.push((v.layers & self.copper, v.at, v.size / 2));
                    nodes.len() - 1
                })
            })
            .collect();
        let junc0 = nodes.len();
        let dist = |a: Pt, b: Pt| ((b.x - a.x) as f64).hypot((b.y - a.y) as f64);
        let pt_node = |nodes: &mut Vec<(LayerMask, Pt, Nm)>, layer: LayerId, pt: Pt, r: Nm| -> usize {
            let m = 1u128 << layer;
            if let Some((&(_, u), _)) = pad_nodes
                .iter()
                .filter(|&&(_, u)| (nodes[u].0 & m) != 0)
                .map(|x| (x, dist(pt, nodes[x.1].1) - nodes[x.1].2 as f64))
                .filter(|&(_, d)| d <= r as f64)
                .min_by(|a, b| a.1.total_cmp(&b.1))
            {
                return u;
            }
            for (u, n) in nodes.iter_mut().enumerate().skip(junc0) {
                if n.0 == m && dist(pt, n.1) <= n.2.max(r).max(10_000) as f64 {
                    n.2 = n.2.max(r);
                    return u;
                }
            }
            nodes.push((m, pt, r));
            nodes.len() - 1
        };
        let stack_h = self.stackup.iter().map(|c| (c.thickness + c.dielectric) as f64).sum::<f64>().max(DEFAULT_BOARD as f64);
        let mut edges: Vec<(usize, usize, f64)> = Vec::new();
        let mut add_r = |u: usize, v: usize, r: f64| {
            if u != v {
                edges.push((u, v, 1.0 / r.max(1e-6)));
            }
        };
        for &(vi, uv) in &via_nodes {
            let v = &self.vias[vi];
            let r_via = CU_RHO * stack_h / (PI * v.drill.max(v.size / 2).max(100_000) as f64 * VIA_PLATE);
            let mut m = v.layers & self.copper;
            while m != 0 {
                let ul = pt_node(&mut nodes, m.trailing_zeros() as LayerId, v.at, v.size / 2);
                add_r(uv, ul, r_via * 0.5);
                m &= m - 1;
            }
        }
        for (i, &(_, u)) in pad_nodes.iter().enumerate() {
            for &(_, v) in &pad_nodes[i + 1..] {
                if (nodes[u].0 & nodes[v].0) != 0 && dist(nodes[u].1, nodes[v].1) <= (nodes[u].2 + nodes[v].2) as f64 {
                    add_r(u, v, CU_RHO / DEFAULT_CU as f64);
                }
            }
        }
        let mut track_ends: Vec<(usize, usize, usize)> = Vec::new();
        for (ti, t) in self.tracks.iter().enumerate().filter(|(_, t)| t.net == net) {
            track_ends.push((ti, pt_node(&mut nodes, t.layer, t.a, t.width / 2), pt_node(&mut nodes, t.layer, t.b, t.width / 2)));
        }
        let via_set = pad_nodes.len()..junc0;
        let mut track_segs: Vec<(usize, usize, usize, f64)> = Vec::new();
        for &(ti, ua, ub) in &track_ends {
            let t = &self.tracks[ti];
            let (w, cu, len) = (t.width.max(1) as f64, self.cu_thickness(t.layer).max(1) as f64, t.len().max(1.0));
            let (r_track, m) = (CU_RHO * len / (w * cu), 1u128 << t.layer);
            let mut stops = vec![(0.0, ua), (1.0, ub)];
            if t.mid.is_none() && len > 1.0 {
                let (dx, dy) = ((t.b.x - t.a.x) as f64, (t.b.y - t.a.y) as f64);
                let l2 = dx * dx + dy * dy;
                for (u, n) in nodes.iter().enumerate() {
                    if !via_set.contains(&u) && u != ua && u != ub && (n.0 & m) != 0 {
                        let s = ((n.1.x - t.a.x) as f64 * dx + (n.1.y - t.a.y) as f64 * dy) / l2;
                        if (0.0..=1.0).contains(&s)
                            && ((n.1.x - t.a.x) as f64 - s * dx).hypot((n.1.y - t.a.y) as f64 - s * dy) <= (n.2 + t.width / 2) as f64
                        {
                            stops.push((s, u));
                        }
                    }
                }
                stops.sort_by(|a, b| a.0.total_cmp(&b.0));
            }
            for w in stops.windows(2) {
                let r_seg = (r_track * (w[1].0 - w[0].0).max(1e-3)).max(1e-6);
                add_r(w[0].1, w[1].1, r_seg);
                if w[0].1 != w[1].1 {
                    track_segs.push((ti, w[0].1, w[1].1, r_seg));
                }
            }
        }
        for z in self.fills.iter().filter(|z| z.net == net) {
            let Some((lo, hi)) = z.pts.iter().map(|&p| (p, p)).reduce(union) else { continue };
            let (m, rs) = (1u128 << z.layer, CU_RHO / self.cu_thickness(z.layer).max(1) as f64);
            let mut zn: Vec<usize> = (0..nodes.len())
                .filter(|&u| {
                    let (nl, p, r) = nodes[u];
                    !via_set.contains(&u)
                        && (nl & m) != 0
                        && p.x + r >= lo.x
                        && p.x - r <= hi.x
                        && p.y + r >= lo.y
                        && p.y - r <= hi.y
                        && in_poly(p, r, &z.pts)
                })
                .collect();
            for axis in [|p: Pt| (p.x, p.y), |p: Pt| (p.y, p.x)] {
                zn.sort_unstable_by_key(|&u| axis(nodes[u].1));
                zn.windows(2).for_each(|w| add_r(w[0], w[1], rs * 1.5));
            }
        }
        if nodes.is_empty() {
            return out;
        }
        let n = nodes.len();
        let mut uf: Vec<usize> = (0..n).collect();
        let find = |uf: &mut [usize], mut x: usize| {
            while uf[x] != x {
                uf[x] = uf[uf[x]];
                x = uf[x];
            }
            x
        };
        for &(u, v, _) in &edges {
            let (ru, rv) = (find(&mut uf, u), find(&mut uf, v));
            uf[ru] = rv;
        }
        let comp: Vec<usize> = (0..n).map(|u| find(&mut uf, u)).collect();
        let (mut comp_pads, mut comp_edges) = (vec![0usize; n], vec![0usize; n]);
        edges.iter().for_each(|&(u, _, _)| comp_edges[comp[u]] += 1);
        pad_nodes.iter().for_each(|&(_, u)| comp_pads[comp[u]] += 1);
        let pad_ref = |pi: usize| self.footprints.get(self.pads[pi].footprint as usize).map_or("", |f| f.reference.as_str());
        let pad_name = |pi: usize| match (pad_ref(pi), self.pads[pi].number.as_str()) {
            ("", n) => n.to_string(),
            (r, "") => r.to_string(),
            (r, n) => format!("{r}.{n}"),
        };
        let match_pad = |q: &str, pi: usize| {
            let (q, r, num) = (q.trim(), pad_ref(pi), self.pads[pi].number.as_str());
            r.eq_ignore_ascii_case(q)
                || pad_name(pi).eq_ignore_ascii_case(q)
                || q.split_once(['.', ':', ' ']).is_some_and(|(qr, qn)| r.eq_ignore_ascii_case(qr) && num.eq_ignore_ascii_case(qn))
        };
        let src_pick = source.and_then(|q| pad_nodes.iter().copied().find(|&(pi, _)| match_pad(q, pi))).or_else(|| {
            pad_nodes.iter().copied().max_by_key(|&(pi, u)| {
                let pwr = matches!(
                    pad_ref(pi).as_bytes().first().map(u8::to_ascii_uppercase),
                    Some(b'J' | b'P' | b'B' | b'V' | b'U' | b'Q' | b'D' | b'L' | b'F')
                );
                let (c, p) = (comp[u], &self.pads[pi]);
                (comp_edges[c] > 0, comp_pads[c], pwr, p.size.0 as i64 * p.size.1 as i64, usize::MAX - pi)
            })
        });
        let (src_node, active_comp) = match src_pick {
            Some((pi, u)) => (out.source = pad_name(pi), (u, comp[u])).1,
            None => {
                out.source = "source".into();
                let u = (0..n).max_by_key(|&u| (comp_edges[comp[u]], usize::MAX - u)).unwrap_or(0);
                (u, comp[u])
            }
        };
        let mut b = vec![0.0; n];
        if let Some((pi, u)) =
            sink.and_then(|q| pad_nodes.iter().copied().find(|&(pi, u)| comp[u] == active_comp && u != src_node && match_pad(q, pi)))
        {
            (b[u], out.sink) = (current_a, pad_name(pi));
        } else {
            let loads: Vec<usize> = pad_nodes.iter().filter_map(|&(_, u)| (comp[u] == active_comp && u != src_node).then_some(u)).collect();
            if !loads.is_empty() {
                loads.iter().for_each(|&u| b[u] += current_a / loads.len() as f64);
            } else if let Some(far) = (0..n)
                .filter(|&u| comp[u] == active_comp && u != src_node)
                .max_by(|&a, &c| dist(nodes[src_node].1, nodes[a].1).total_cmp(&dist(nodes[src_node].1, nodes[c].1)))
            {
                (b[far], out.sink) = (current_a, "sink".into());
            }
        }
        let mut diag = vec![0.0; n];
        let active_edges: Vec<_> = edges.into_iter().filter(|&(u, v, _)| comp[u] == active_comp && comp[v] == active_comp).collect();
        active_edges.iter().for_each(|&(u, v, g)| (diag[u] += g, diag[v] += g).0);
        b[src_node] = 0.0;
        let dot = |a: &[f64], b: &[f64]| -> f64 { a.iter().zip(b).map(|(x, y)| x * y).sum() };
        let pre = |r: &[f64]| -> Vec<f64> { (0..n).map(|i| if i != src_node && diag[i] > 0.0 { r[i] / diag[i] } else { 0.0 }).collect() };
        let (mut v, mut r, mut ap) = (vec![0.0; n], b, vec![0.0; n]);
        let mut z = pre(&r);
        let (mut p, mut rz) = (z.clone(), dot(&r, &z));
        for _ in 0..(n * 2).clamp(64, 2000) {
            if rz <= 1e-24 {
                break;
            }
            ap.fill(0.0);
            for &(u, w, g) in &active_edges {
                let d = (p[u] - p[w]) * g;
                ap[u] += d;
                ap[w] -= d;
            }
            ap[src_node] = 0.0;
            let pap = dot(&p, &ap);
            if pap <= 1e-30 {
                break;
            }
            let alpha = rz / pap;
            v.iter_mut().zip(&p).for_each(|(vi, pi)| *vi += alpha * pi);
            r.iter_mut().zip(&ap).for_each(|(ri, api)| *ri -= alpha * api);
            r[src_node] = 0.0;
            z = pre(&r);
            let rz_new = dot(&r, &z);
            let beta = rz_new / rz;
            rz = rz_new;
            p.iter_mut().zip(&z).for_each(|(pi, zi)| *pi = zi + beta * *pi);
        }
        v.iter_mut().for_each(|x| *x = x.max(0.0));
        out.max_drop_mv = v.iter().copied().fold(0.0, f64::max) * 1e3;
        out.resistance_mohm = out.max_drop_mv / current_a;
        out.power_mw = active_edges.iter().map(|&(u, w, g)| g * (v[u] - v[w]).powi(2)).sum::<f64>() * 1e3;
        track_ends.iter().for_each(|&(ti, ua, ub)| out.track_drops[ti] = (v[ua] * 1e3, v[ub] * 1e3));
        for &(ti, u0, u1, r_seg) in &track_segs {
            let t = &self.tracks[ti];
            let area_mm2 = (t.width.max(1) as f64 / 1e6) * (self.cu_thickness(t.layer).max(1) as f64 / 1e6);
            out.max_j_a_mm2 = out.max_j_a_mm2.max(((v[u0] - v[u1]).abs() / r_seg) / area_mm2);
        }
        via_nodes.iter().for_each(|&(vi, uv)| out.via_drops[vi] = v[uv] * 1e3);
        for &(pi, u) in &pad_nodes {
            let drop_mv = v[u] * 1e3;
            out.pad_drops[pi] = drop_mv;
            out.pads.push(PadDrop {
                pad: pi,
                reference: pad_ref(pi).into(),
                number: self.pads[pi].number.clone(),
                at: self.pads[pi].at,
                drop_mv,
                resistance_mohm: drop_mv / current_a,
            });
        }
        out.pads.sort_by(|a, b| b.drop_mv.total_cmp(&a.drop_mv).then(a.pad.cmp(&b.pad)));
        if out.sink.is_empty()
            && let Some(first) = out.pads.first()
        {
            out.sink = pad_name(first.pad);
        }
        for (u, &(mut m, at, _)) in nodes.iter().enumerate() {
            if comp[u] == active_comp && !via_set.contains(&u) {
                while m != 0 {
                    out.nodes.push((m.trailing_zeros() as LayerId, at, v[u] * 1e3));
                    m &= m - 1;
                }
            }
        }
        out
    }
}

fn in_poly(p: Pt, r: Nm, pts: &[Pt]) -> bool {
    let (px, py, rf, mut inside) = (p.x as f64, p.y as f64, r as f64, false);
    for (i, &a) in pts.iter().enumerate() {
        let b = pts[(i + 1) % pts.len()];
        let (ax, ay, bx, by) = (a.x as f64, a.y as f64, b.x as f64, b.y as f64);
        inside ^= ((ay > py) != (by > py)) && (px < (bx - ax) * (py - ay) / (by - ay) + ax);
        let (dx, dy) = (bx - ax, by - ay);
        let t = if dx == 0.0 && dy == 0.0 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0) };
        if rf > 0.0 && (px - (ax + t * dx)).hypot(py - (ay + t * dy)) <= rf {
            return true;
        }
    }
    pts.len() >= 3 && inside
}

/// Maximum continuous current (A) for a copper trace of `width` and `thickness` at temperature
/// rise `delta_t` (°C), from the IPC-2152 universal baseline chart (Figure 5-1).
pub fn ipc2152_current(width: Nm, thickness: Nm, delta_t: f64) -> f64 {
    if width <= 0 || thickness <= 0 || delta_t <= 0.0 {
        return 0.0;
    }
    let area_sq_mil = (width as f64 / 25_400.0) * (thickness as f64 / 25_400.0);
    let k1 = 117.555 * delta_t.powf(-0.913) + 1.15;
    let k2 = 0.84 * delta_t.powf(-0.108) + 1.159;
    (area_sq_mil / k1).powf(1.0 / k2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::CuLayer;

    #[test]
    fn inspects_net_length_current_and_impedance() {
        let b = Board::parse(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal))
              (setup (stackup
                (layer "F.Cu" (type "copper") (thickness 0.035))
                (layer "dielectric 1" (type "prepreg") (thickness 0.2104) (epsilon_r 4.6))
                (layer "In1.Cu" (type "copper") (thickness 0.018))
                (layer "dielectric 2" (type "core") (thickness 0.1) (epsilon_r 4.1)
                  addsublayer (thickness 0.1) (epsilon_r 4.1))
                (layer "In2.Cu" (type "copper") (thickness 0.018))
                (layer "dielectric 3" (type "prepreg") (thickness 0.2104) (epsilon_r 4.6))
                (layer "B.Cu" (type "copper") (thickness 0.035))))
              (net 0 "") (net 1 "RF") (net 2 "BUS")
              (footprint "U" (layer "F.Cu") (at 0 0)
                (property "Reference" "U1" (at 0 0) (layer "F.SilkS"))
                (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "RF"))
                (pad "2" smd rect (at 10 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "RF")))
              (segment (start 0 0) (end 10 0) (width 0.29337) (layer "F.Cu") (net 1))
              (arc (start -1 0) (mid 0 1) (end 1 0) (width 0.29337) (layer "F.Cu") (net 1))
              (segment (start 0 5) (end 4 5) (width 0.12) (layer "In1.Cu") (net 2))
              (via (at 4 5) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 2)))"#,
        )
        .unwrap();
        assert_eq!(b.stackup[0], CuLayer { layer: 0, thickness: 35_000, dielectric: 210_400, epsilon_r: 4.6 });
        assert_eq!(b.stackup[1], CuLayer { layer: 1, thickness: 18_000, dielectric: 200_000, epsilon_r: 4.1 });
        let rf = b.inspect_net(1);
        assert_eq!((rf.tracks, rf.vias, rf.pads, rf.width), (2, 0, 2, (293_370, 293_370)));
        // 10 mm straight + π mm semicircle = 13.14159 mm.
        assert!((rf.length_mm - (10.0 + PI)).abs() < 1e-4);
        assert!(rf.current_a > 0.9 && rf.current_a < 1.2);
        assert!((rf.impedance_ohm.0 - 54.1).abs() < 1.0 && rf.impedance_ohm.0 == rf.impedance_ohm.1);
        let bus = b.inspect_net(2);
        assert_eq!((bus.tracks, bus.vias, bus.length_mm), (1, 1, 4.0));
        assert!((bus.impedance_ohm.0 - 57.2).abs() < 1.0);
        // Narrower 0.12 mm inner trace on 18 µm copper carries less current than the 0.293 mm outer trace on 35 µm copper.
        assert!(bus.current_a > 0.2 && bus.current_a < rf.current_a);
        assert_eq!(b.default_ir_net(), 1);
        let ir = b.ir_drop(1, 2.0, None, None);
        assert_eq!((ir.source.as_str(), ir.sink.as_str(), ir.pads.len()), ("U1.1", "U1.2", 2));
        // 10 mm of 0.29337 mm × 35 µm copper is 16.79 mΩ → 33.58 mV at 2 A, 67.16 mW, 194.8 A/mm².
        assert!((ir.resistance_mohm - 16.79).abs() < 0.1 && (ir.max_drop_mv - 33.58).abs() < 0.2);
        assert!((ir.power_mw - 67.16).abs() < 0.4 && (ir.max_j_a_mm2 - 194.8).abs() < 1.0);
    }
}
