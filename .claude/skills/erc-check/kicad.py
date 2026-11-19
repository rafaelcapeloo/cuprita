"""KiCad 10's nets and ERC, beside Cuprita's (skill erc-check). Run with `python3 -I`.

  kicad.py sheet <dir> <spec.py>   write the project a spec describes into <dir> (in the session
                                   scratch dir), export KiCad's netlist and ERC, print both
  kicad.py nets <kicad.net> <nets.tsv>  KiCad's nets beside Cuprita's (`check` output)
  kicad.py erc <erc.json> <erc.tsv>     ERC counts by kind, KiCad's beside Cuprita's
  kicad.py digest <kicad.net>...        nets, pins and digest, as `tests/corpus.rs` pins them

A spec is Python: LIBS, the library symbols ("R": pins 1 and 2 at x -2.54 and +2.54; "PWR": a
power symbol, hidden input VCC; "FLAG": PWR_FLAG; or `part(name, [(type, name, hidden)…])`, pins
down its left side 2.54 apart, ending 5.08 left of the anchor; or raw `(symbol …)` text), SHEETS,
{"root": [items], "<file>.kicad_sch": [items]}, and PRO, the `.kicad_pro` text (optional).
Items: ("sym", lib, ref, x, y[, angle, value]), ("wire", (x, y), (x, y)…), ("bus", …),
("entry", x, y, dx, dy), ("junction", x, y), ("nc", x, y), (kind, name, x, y[, angle, shape])
for kind label / global_label / hierarchical_label, ("sheet", name, file, x, y, [(pin, shape,
x, y)…][, (w, h)]). Coordinates in mm; a child sheet's symbols get each use's references (+100).
"""
import itertools, json, os, re, subprocess, sys
from collections import Counter

ids = itertools.count(1)
def uid(): return "00000000-0000-0000-0000-%012d" % next(ids)
FONT = '(effects (font (size 1.27 1.27)))'
def pin(kind, name, number, x, y, angle, length, hidden=False):
    return f'(pin {kind} line (at {x} {y} {angle}) (length {length}){" (hide yes)" if hidden else ""} (name "{name}" {FONT}) (number "{number}" {FONT}))'
def symbol(name, body, power=False, ref="U", value=None):
    return (f'(symbol "T:{name}"{" (power)" if power else ""} (exclude_from_sim no) (in_bom yes) (on_board yes)'
            f' (property "Reference" "{ref}" (at 0 2.54 0) {FONT}) (property "Value" "{value or name}" (at 0 -2.54 0) {FONT})'
            f' (symbol "{name}_1_1" {body}))')
LIB = {
    "R": symbol("R", pin("passive", "~", 1, -2.54, 0, 0, 1.27) + pin("passive", "~", 2, 2.54, 0, 180, 1.27), ref="R"),
    "PWR": symbol("PWR", pin("power_in", "VCC", 1, 0, 0, 90, 0, True), True, "#PWR", "VCC"),
    "FLAG": symbol("FLAG", pin("power_out", "pwr", 1, 0, 0, 90, 0), True, "#FLG", "PWR_FLAG"),
}
def part(name, pins):
    return symbol(name, " ".join(pin(t, n, i + 1, -5.08, -2.54 * i, 0, 2.54, h) for i, (t, n, h) in enumerate(pins)))

def bump(ref, k):
    m = re.match(r"(#?[A-Za-z_]+)(\d+)$", ref)
    return ref if k == 0 or not m else f"{m.group(1)}{int(m.group(2)) + 100 * k}"

def items(spec, paths, sheet_ids):
    out = []
    for it in spec:
        k = it[0]
        if k == "sym":
            lib, ref, x, y = it[1:5]
            angle, value = (it[5] if len(it) > 5 else 0), (it[6] if len(it) > 6 else lib)
            inst = " ".join(f'(path "{p}" (reference "{bump(ref, n)}") (unit 1))' for n, p in enumerate(paths))
            out.append(f'(symbol (lib_id "T:{lib}") (at {x} {y} {angle}) (unit 1) (uuid "{uid()}") (property "Reference" "{ref}" (at {x} {y - 3} 0) {FONT})'
                       f' (property "Value" "{value}" (at {x} {y + 3} 0) {FONT}) (instances (project "t" {inst})))')
        elif k in ("wire", "bus"):
            out += [f'({k} (pts (xy {a[0]} {a[1]}) (xy {b[0]} {b[1]})) (uuid "{uid()}"))' for a, b in zip(it[1:], it[2:])]
        elif k == "entry":
            out.append(f'(bus_entry (at {it[1]} {it[2]}) (size {it[3]} {it[4]}) (uuid "{uid()}"))')
        elif k in ("junction", "nc"):
            out.append(f'({"no_connect" if k == "nc" else "junction"} (at {it[1]} {it[2]}) (uuid "{uid()}"))')
        elif k in ("label", "global_label", "hierarchical_label"):
            angle = it[4] if len(it) > 4 else 0
            shape = f'(shape {it[5] if len(it) > 5 else "input"}) ' if k != "label" else ""
            out.append(f'({k} "{it[1]}" {shape}(at {it[2]} {it[3]} {angle}) {FONT} (uuid "{uid()}"))')
        elif k == "sheet":
            name, file, x, y, pins = it[1:6]
            w, h = it[6] if len(it) > 6 else (20, 20)
            sheet_ids[name] = uid()
            ps = " ".join(f'(pin "{n}" {t} (at {px} {py} 180) (uuid "{uid()}") {FONT})' for n, t, px, py in pins)
            out.append(f'(sheet (at {x} {y}) (size {w} {h}) (uuid "{sheet_ids[name]}") (property "Sheetname" "{name}" (at {x} {y - 1} 0) {FONT})'
                       f' (property "Sheetfile" "{file}" (at {x} {y + h + 1} 0) {FONT}) {ps} (instances (project "t" (path "{paths[0]}" (page "{len(sheet_ids) + 1}")))))')
        else:
            out.append(it[1])
    return out

def write(d, spec):
    g = {"part": part}
    exec(open(spec).read(), g)
    libs = " ".join(LIB.get(l, l) for l in g["LIBS"])
    root, sheet_ids, texts = uid(), {}, {}
    texts["t.kicad_sch"] = items(g["SHEETS"]["root"], [f"/{root}"], sheet_ids)
    for file, spec_items in g["SHEETS"].items():
        if file != "root":
            uses = [it[1] for it in g["SHEETS"]["root"] if it[0] == "sheet" and it[2] == file]
            texts[file] = items(spec_items, [f"/{root}/{sheet_ids[u]}" for u in uses], sheet_ids)
    os.makedirs(d, exist_ok=True)
    for file, body in texts.items():
        tail = '(sheet_instances (path "/" (page "1")))' if file == "t.kicad_sch" else ""
        u = root if file == "t.kicad_sch" else uid()
        open(os.path.join(d, file), "w").write(f'(kicad_sch (version 20260306) (generator "eeschema") (uuid "{u}") (paper "A4") (lib_symbols {libs})\n'
                                               + "\n".join(body) + tail + ")\n")
    open(os.path.join(d, "t.kicad_pro"), "w").write(g.get("PRO", '{"meta": {"filename": "t.kicad_pro", "version": 3}}'))

def kicad_cli(d, *args):
    s = os.path.dirname(os.path.abspath(d))
    r = subprocess.run(["flatpak", "run", f"--filesystem={s}", "--command=kicad-cli", "org.kicad.kicad", "sch", *args], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout + r.stderr)

def sexpr(text):
    stack = [[]]
    for t in re.findall(r'"(?:[^"\\]|\\.)*"|[()]|[^\s()"]+', text):
        if t == "(": stack.append([])
        elif t == ")": x = stack.pop(); stack[-1].append(x)
        else: stack[-1].append(t[1:-1] if t.startswith('"') else t)
    return stack[0][0]

def netlist(path):
    """KiCad's netlist: {net name: sorted ref-pin}."""
    nets = {}
    for sec in (s for s in sexpr(open(path).read()) if isinstance(s, list) and s[0] == "nets"):
        for net in sec[1:]:
            name = next(x[1] for x in net if isinstance(x, list) and x[0] == "name")
            nodes = [dict((y[0], y[1]) for y in x[1:]) for x in net if isinstance(x, list) and x[0] == "node"]
            nets[name] = sorted(f"{n['ref']}-{n['pin']}" for n in nodes)
    return nets

def violations(path, skip=("footprint_link_issues", "lib_symbol_issues", "lib_symbol_mismatch", "endpoint_off_grid")):
    for sheet in json.load(open(path))["sheets"]:
        for v in sheet["violations"]:
            if v["type"] not in skip:
                # kicad-cli writes positions in units of 100 mm.
                yield sheet["path"], v, "; ".join(f"{i['description']} @({i['pos']['x'] * 100:.2f},{i['pos']['y'] * 100:.2f})" for i in v["items"])

def compare_nets(knet, ours):
    k = {frozenset(v): n for n, v in netlist(knet).items()}
    e = {}
    for line in open(ours):
        name, _, pins = line.rstrip("\n").partition("\t")
        e[frozenset(pins.split())] = name
    print(f"KiCad {len(k)} nets, Cuprita {len(e)}; the same: {len(set(k) & set(e))}")
    at = {p: s for s in e for p in s}
    for s in sorted(set(k) - set(e), key=len, reverse=True)[:8]:
        print(f"  KiCad's {k[s]} ({len(s)} pins) is Cuprita's", sorted({e.get(at.get(p), "?") or "unnamed" for p in s})[:6])
    named = [(k[s], e[s]) for s in set(k) & set(e) if k[s] != e[s]]
    print(f"  names unlike KiCad's: {len(named)}", named[:8])

def compare_erc(kjson, ours):
    k = Counter(v["type"] for _, v, _ in violations(kjson))
    e = Counter(line.split("\t")[0] for line in open(ours) if line.strip())
    for kind in sorted(set(k) | set(e)):
        print(f"  {kind:28} KiCad {k[kind]:4}  Cuprita {e[kind]:4}{'' if k[kind] == e[kind] else '  <--'}")

if __name__ == "__main__":
    cmd, *args = sys.argv[1:]
    if cmd == "sheet":
        d = args[0]
        write(d, args[1])
        kicad_cli(d, "export", "netlist", "--format", "kicadsexpr", "-o", f"{d}/t.net", f"{d}/t.kicad_sch")
        kicad_cli(d, "erc", "--format", "json", "--severity-all", "-o", f"{d}/erc.json", f"{d}/t.kicad_sch")
        for name, pins in sorted(netlist(f"{d}/t.net").items()):
            print(f"  NET {name!r}: {' '.join(pins)}")
        for path, v, where in violations(f"{d}/erc.json"):
            print(f"  ERC {path} {v['severity']} {v['type']}: {v['description']} | {where}")
    elif cmd == "nets":
        compare_nets(*args)
    elif cmd == "erc":
        compare_erc(*args)
    elif cmd == "digest":
        for path in args:
            nets = netlist(path)
            lines = sorted(n + "\t" + " ".join(v) for n, v in nets.items())
            h = 0xcbf29ce484222325
            for b in "\n".join(lines).encode():
                h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
            print(os.path.basename(path), len(nets), sum(map(len, nets.values())), hex(h))
