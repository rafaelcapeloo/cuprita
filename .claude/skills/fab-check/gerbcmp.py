#!/usr/bin/env python3
"""Rasterize Gerber X2 / Excellon files and compare them (skill fab-check).

gerbcmp.py A B [--um 10] [--tol 1] [--box x0,y0,x1,y1] [--png out.png]
  Two files: the area only one covers (beyond --tol pixels of the other); --png draws A-only red,
  B-only green, both grey; --box (Gerber mm, Y up) limits it to a window.
gerbcmp.py DIR_A DIR_B [--um 10]: every file of A against the file of B of the same kind (Protel
  extension, -PTH.drl, -NPTH.drl, other .drl by name).
gerbcmp.py A --png out.png: draws one file.
Reads what KiCad and Cuprita write: FS/MO, AD (C, R, O, P, macros), AM (primitives 1, 4, 5, 20,
21), D01-D03, G01-G03, G75, G36/G37, dark polarity only; Excellon tools, hits and G85 slots.
Needs pycairo and numpy.
"""
import math, os, re, sys
import cairo
import numpy as np


def ev(e, v):
    e = re.sub(r"\$(\d+)", lambda m: repr(v.get(int(m.group(1)), 0.0)), e).replace("x", "*").replace("X", "*")
    return float(eval(e, {"__builtins__": {}}))


def parse_gerber(text):
    ops, aps, macros = [], {}, {}
    dec, ap, x, y, interp, region, path, rpath = 6, None, 0.0, 0.0, 1, None, [], []

    def flush():
        nonlocal path
        if path:
            ops.append(("stroke", aps[ap], path))
            path = []

    for b in re.findall(r"%[^%]*%|[^%*]*\*", text.replace("\r", "").replace("\n", "")):
        if b.startswith("%"):
            body = b[1:-1]
            if body.startswith("FS"):
                dec = int(re.search(r"X(\d)(\d)", body).group(2))
            elif body.startswith("MO") and not body.startswith("MOMM"):
                raise SystemExit("only mm files: " + body)
            elif body.startswith("AM"):
                parts = body[:-1].split("*")
                macros[parts[0][2:]] = [p for p in parts[1:] if p and not p.startswith("0")]
            elif body.startswith("AD"):
                m = re.match(r"ADD(\d+)([A-Za-z_$][\w.$]*),?(.*)\*", body)
                aps[int(m.group(1))] = (m.group(2), [float(v) for v in m.group(3).split("X")] if m.group(3) else [])
            elif body.startswith(("LPC", "LR", "LM", "LS", "SR", "IP")):
                raise SystemExit("unsupported: " + body)
            continue
        b = b[:-1]
        if not b or b.startswith("G04"):
            continue
        if b in ("M02", "M00"):
            break
        g = re.match(r"G0?(\d+)", b)
        if g:
            n = int(g.group(1))
            if n in (1, 2, 3):
                interp = n
            elif n == 36:
                flush()
                region, rpath = [], []
            elif n == 37:
                ops.append(("region", region + [rpath] * bool(rpath)))
                region = None
            elif n == 74:
                raise SystemExit("G74 single quadrant mode unsupported")
            b = b[g.end():]
            if not b:
                continue
        d = re.fullmatch(r"D(\d+)", b)
        if d and int(d.group(1)) >= 10:
            flush()
            ap = int(d.group(1))
            continue
        m = re.fullmatch(r"(?:X(-?\d+))?(?:Y(-?\d+))?(?:I(-?\d+))?(?:J(-?\d+))?D0?([123])", b)
        if not m:
            raise SystemExit("cannot read block: " + b)
        nx, ny, i, j = (int(g) / 10 ** dec if g else None for g in m.groups()[:4])
        nx, ny, i, j = x if nx is None else nx, y if ny is None else ny, i or 0.0, j or 0.0
        if m.group(5) == "1":
            seg = ("L", x, y, nx, ny) if interp == 1 else ("A", x, y, nx, ny, x + i, y + j, interp == 2)
            (rpath if region is not None else path).append(seg)
        elif m.group(5) == "2":
            if region is not None:
                if rpath:
                    region.append(rpath)
                rpath = []
            else:
                flush()
        else:
            flush()
            ops.append(("flash", aps[ap], nx, ny, macros))
        x, y = nx, ny
    flush()
    return ops


def parse_drill(text):
    ops, tools, k, cur = [], {}, 1.0, None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("INCH"):
            k = 25.4
        if m := re.fullmatch(r"T(\d+)C([\d.]+)", line):
            tools[int(m.group(1))] = float(m.group(2)) * k
        elif m := re.fullmatch(r"T(\d+)", line):
            cur = int(m.group(1))
        elif m := re.fullmatch(r"X(-?[\d.]+)Y(-?[\d.]+)(?:G85X(-?[\d.]+)Y(-?[\d.]+))?", line):
            x, y = float(m.group(1)) * k, float(m.group(2)) * k
            if m.group(3):
                ops.append(("stroke", ("C", [tools[cur]]), [("L", x, y, float(m.group(3)) * k, float(m.group(4)) * k)]))
            else:
                ops.append(("flash", ("C", [tools[cur]]), x, y, {}))
    return ops


def load(path):
    text = open(path, encoding="utf-8", errors="replace").read()
    return parse_drill(text) if text.lstrip().startswith("M48") else parse_gerber(text)


def extent(ops):
    xs, ys = [], []
    for o in ops:
        if o[0] == "flash":
            xs.append(o[2]), ys.append(o[3])
        else:
            for c in o[1] if o[0] == "region" else [o[2]]:
                for s in c:
                    xs += [s[1], s[3]]
                    ys += [s[2], s[4]]
    return (min(xs), min(ys), max(xs), max(ys)) if xs else None


def trace(ctx, segs):
    ctx.move_to(segs[0][1], segs[0][2])
    for s in segs:
        if s[0] == "L":
            ctx.line_to(s[3], s[4])
            continue
        _, x0, y0, x1, y1, cx, cy, cw = s
        r, a0, a1 = math.hypot(x0 - cx, y0 - cy), math.atan2(y0 - cy, x0 - cx), math.atan2(y1 - cy, x1 - cx)
        if abs(x0 - x1) < 1e-9 and abs(y0 - y1) < 1e-9:
            a1 = a0 - 2 * math.pi if cw else a0 + 2 * math.pi
        (ctx.arc_negative if cw else ctx.arc)(cx, cy, r, a0, a1)


def primitives(ctx, prims, params):
    """Draws a macro's primitives at the origin; exposure off clears (inside a group)."""
    v = {i + 1: p for i, p in enumerate(params)}
    for p in prims:
        if p.startswith("$"):
            k, e = p.split("=", 1)
            v[int(k[1:])] = ev(e, v)
            continue
        f = [ev(t, v) for t in p.split(",")]
        code = int(f[0])
        ctx.set_operator(cairo.OPERATOR_OVER if f[1] > 0 else cairo.OPERATOR_CLEAR)
        ctx.save()
        if code == 1:
            ctx.rotate(math.radians(f[5] if len(f) > 5 else 0.0))
            ctx.arc(f[3], f[4], f[2] / 2, 0, 2 * math.pi)
        elif code == 4:
            n = int(f[2])
            ctx.rotate(math.radians(f[-1]))
            for k in range(n + 1):
                ctx.line_to(f[3 + 2 * k], f[4 + 2 * k])
            ctx.close_path()
        elif code == 5:
            ctx.rotate(math.radians(f[6] if len(f) > 6 else 0.0))
            for k in range(int(f[2])):
                a = 2 * math.pi * k / int(f[2])
                ctx.line_to(f[3] + f[5] / 2 * math.cos(a), f[4] + f[5] / 2 * math.sin(a))
            ctx.close_path()
        elif code == 20:
            w, x0, y0, x1, y1, rot = f[2:8]
            ctx.rotate(math.radians(rot))
            l = math.hypot(x1 - x0, y1 - y0) or 1.0
            nx, ny = -(y1 - y0) / l * w / 2, (x1 - x0) / l * w / 2
            for px, py in ((x0 + nx, y0 + ny), (x1 + nx, y1 + ny), (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)):
                ctx.line_to(px, py)
            ctx.close_path()
        elif code == 21:
            ctx.rotate(math.radians(f[6]))
            ctx.rectangle(f[4] - f[2] / 2, f[5] - f[3] / 2, f[2], f[3])
        else:
            raise SystemExit("macro primitive %d unsupported" % code)
        ctx.fill()
        ctx.restore()
    ctx.set_operator(cairo.OPERATOR_OVER)


def flash(ctx, ap, x, y, macros):
    kind, v = ap
    ctx.save()
    ctx.translate(x, y)
    if kind == "C":
        ctx.arc(0, 0, v[0] / 2, 0, 2 * math.pi)
    elif kind == "R":
        ctx.rectangle(-v[0] / 2, -v[1] / 2, v[0], v[1])
    elif kind == "O":
        w, h = v[0], v[1]
        r = min(w, h) / 2
        if w >= h:
            ctx.arc(w / 2 - r, 0, r, -math.pi / 2, math.pi / 2)
            ctx.arc(r - w / 2, 0, r, math.pi / 2, 3 * math.pi / 2)
        else:
            ctx.arc(0, h / 2 - r, r, 0, math.pi)
            ctx.arc(0, r - h / 2, r, math.pi, 2 * math.pi)
        ctx.close_path()
    elif kind == "P":
        rot = math.radians(v[2]) if len(v) > 2 else 0.0
        for k in range(int(v[1])):
            a = rot + 2 * math.pi * k / int(v[1])
            ctx.line_to(v[0] / 2 * math.cos(a), v[0] / 2 * math.sin(a))
        ctx.close_path()
    elif all(not p.startswith("$") and ev(p.split(",")[1], dict(enumerate(v, 1))) > 0 for p in macros[kind]):
        primitives(ctx, macros[kind], v)  # all exposure on: draw straight onto the image
        ctx.restore()
        return
    else:
        ctx.rectangle(-50, -50, 100, 100)  # a group only as big as an aperture can be
        ctx.clip()
        ctx.push_group()
        primitives(ctx, macros[kind], v)
        ctx.pop_group_to_source()
        ctx.paint()
        ctx.restore()
        return
    ctx.fill()
    ctx.restore()


def render(ops, box, um):
    s = um / 1000.0
    w, h = int(math.ceil((box[2] - box[0]) / s)) + 1, int(math.ceil((box[3] - box[1]) / s)) + 1
    surf = cairo.ImageSurface(cairo.FORMAT_A8, w, h)
    ctx = cairo.Context(surf)
    ctx.set_matrix(cairo.Matrix(1 / s, 0, 0, -1 / s, -box[0] / s, box[3] / s))
    ctx.set_line_cap(cairo.LINE_CAP_ROUND)
    ctx.set_line_join(cairo.LINE_JOIN_ROUND)
    for o in ops:
        if o[0] == "flash":
            flash(ctx, o[1], o[2], o[3], o[4])
        elif o[0] == "region":
            for c in o[1]:
                trace(ctx, c)
                ctx.close_path()
            ctx.fill()
        elif o[1][0] == "C" and o[1][1][0] > 0:
            ctx.set_line_width(o[1][1][0])
            trace(ctx, o[2])
            ctx.stroke()
    surf.flush()
    return np.frombuffer(surf.get_data(), np.uint8).reshape(h, surf.get_stride())[:, :w] >= 128


def grow(m, k):
    for _ in range(k):
        g = m.copy()
        g[1:] |= m[:-1]
        g[:-1] |= m[1:]
        g[:, 1:] |= m[:, :-1]
        g[:, :-1] |= m[:, 1:]
        m = g
    return m


def png(path, rgb):
    h, w, _ = rgb.shape
    buf = np.zeros((h, cairo.ImageSurface.format_stride_for_width(cairo.FORMAT_RGB24, w) // 4, 4), np.uint8)
    buf[:, :w, :3] = rgb[..., ::-1]
    cairo.ImageSurface.create_for_data(memoryview(buf), cairo.FORMAT_RGB24, w, h, buf.shape[1] * 4).write_to_png(path)


def compare(a_path, b_path, um, tol, box=None, out=None):
    sets = [load(p) for p in (a_path, b_path)]
    exts = [e for e in map(extent, sets) if e]
    if not exts:
        return "both empty"
    box = box or (min(e[0] for e in exts) - 1, min(e[1] for e in exts) - 1, max(e[2] for e in exts) + 1, max(e[3] for e in exts) + 1)
    a, b = (render(o, box, um) for o in sets)
    only_a, only_b, px = a & ~grow(b, tol), b & ~grow(a, tol), (um / 1000.0) ** 2
    text = "A %.3f mm², B %.3f mm²; only A %.4f mm², only B %.4f mm²" % (a.sum() * px, b.sum() * px, only_a.sum() * px, only_b.sum() * px)
    for name, m in (("only A", only_a), ("only B", only_b)):
        ys, xs = np.nonzero(m)
        if len(xs):
            step = max(1, len(xs) // 6)
            spots = sorted({(round(float(box[0] + x * um / 1000), 1), round(float(y * um / 1000 - box[3]), 1)) for x, y in zip(xs[::step], ys[::step])})
            text += "\n  %s near (board mm, Y down) %s" % (name, spots[:6])
    if out:
        rgb = np.zeros(a.shape + (3,), np.uint8)
        rgb[a & b] = (90, 90, 90)
        rgb[a & ~b], rgb[b & ~a] = (120, 40, 40), (40, 120, 40)
        rgb[only_a], rgb[only_b] = (255, 0, 0), (0, 255, 0)
        png(out, rgb)
    return text


def kind(name):
    for k in ("-PTH.drl", "-NPTH.drl"):
        if name.endswith(k):
            return k
    return name.rsplit(".", 1)[-1].lower() if not name.endswith(".drl") else name.rsplit("-", 1)[-1]


def main():
    args, o = [], {"--um": "10", "--tol": "1", "--box": None, "--png": None}
    it = iter(sys.argv[1:])
    for a in it:
        if a in o:
            o[a] = next(it)
        else:
            args.append(a)
    um, tol = float(o["--um"]), int(o["--tol"])
    box = tuple(map(float, o["--box"].split(","))) if o["--box"] else None
    if len(args) == 1:
        ops = load(args[0])
        img = render(ops, box or extent(ops), um)
        png(o["--png"], np.stack([np.where(img, 255, 0).astype(np.uint8)] * 3, -1))
    elif os.path.isdir(args[0]):
        theirs = {kind(n): n for n in os.listdir(args[1])}
        for n in sorted(os.listdir(args[0])):
            if n.endswith((".gbrjob", ".csv", ".zip")):
                continue
            other = theirs.get(kind(n))
            res = compare(os.path.join(args[0], n), os.path.join(args[1], other), um, tol) if other else "no match in " + args[1]
            print("%-32s %s" % (n, res))
    else:
        print(compare(args[0], args[1], um, tol, box, o["--png"]))


if __name__ == "__main__":
    main()
