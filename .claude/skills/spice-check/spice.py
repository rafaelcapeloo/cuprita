"""KiCad 10's SPICE netlists beside Cuprita's (skill spice-check). Run with `python3 -I`.

  spice.py sheet <dir> <spec.py>          write the test sheet a spec describes into <dir> (in the
                                          session scratch dir), export KiCad's netlist, print it
  spice.py diff <kicad.cir> <cuprita.cir>  parts, models, includes and directives that differ: numbers
                                          as ngspice reads them, KiCad's copies of library models
                                          (`Q7.Q2SC2240`) as the model, spacing and case ignored

A spec is Python: PARTS, [(lib, ref, value, {field: value}[, flags])]: lib R, C, L, V (two pins), Q
(three), M (four), U (five), U2 (two units of three), each pin on its own label `<ref>.<pin>`, the
fields on the first unit ("allunits" in flags: on every unit), flags "exclude", "dnp"; TEXTS, notes;
FILES, {name: text} beside the sheet (model libraries); VERSION, the sheet's (20250114).
"""
import itertools, os, re, subprocess, sys

# --- sheet ---
ids = itertools.count(1)
def uid(): return "00000000-0000-0000-0000-%012d" % next(ids)
FONT = '(effects (font (size 1.27 1.27)))'
PINS = {"R": 2, "C": 2, "L": 2, "V": 2, "Q": 3, "M": 4, "U": 5, "U2": 6}

def lib(name, units=1):
    """A library symbol of PINS[name] passive pins in `units` units, odd pins left, even right."""
    per = PINS[name] // units
    body = ""
    for u in range(units):
        pins = " ".join(f'(pin passive line (at {-2.54 if i % 2 == 0 else 2.54} {-2.54 * (i // 2)} {0 if i % 2 == 0 else 180}) (length 1.27)'
                        f' (name "~" {FONT}) (number "{i + 1}" {FONT}))' for i in range(u * per, (u + 1) * per))
        body += f' (symbol "{name}_{u + 1}_1" {pins})'
    return (f'(symbol "T:{name}" (exclude_from_sim no) (in_bom yes) (on_board yes) (property "Reference" "{name[0]}" (at 0 2.54 0) {FONT})'
            f' (property "Value" "{name}" (at 0 -2.54 0) {FONT}){body})')

def sheet(d, spec):
    g = {}
    exec(open(spec).read(), g)
    root, out = uid(), []
    for k, part in enumerate(g["PARTS"]):
        name, ref, value, fields, flags = (list(part) + [""])[:5]
        x, y = 50.8 + 25.4 * (k % 8), 50.8 + 25.4 * (k // 8)
        units = 2 if name == "U2" else 1
        per = PINS[name] // units
        for u in range(units):
            ux = x + 12.7 * u
            props = f'(property "Reference" "{ref}" (at {ux} {y - 5} 0) {FONT}) (property "Value" "{value}" (at {ux} {y + 5} 0) {FONT})'
            if u == 0 or "allunits" in flags:
                props += "".join(f' (property "{f}" "{v}" (at {ux} {y} 0) (hide yes) {FONT})' for f, v in fields.items())
            sim, dnp = ("yes" if "exclude" in flags else "no"), ("yes" if "dnp" in flags else "no")
            out.append(f'(symbol (lib_id "T:{name}") (at {ux} {y} 0) (unit {u + 1}) (exclude_from_sim {sim}) (in_bom yes) (on_board yes)'
                       f' (dnp {dnp}) (uuid "{uid()}") {props} (instances (project "t" (path "/{root}" (reference "{ref}") (unit {u + 1})))))')
            for i in range(u * per, (u + 1) * per):
                out.append(f'(label "{ref}.{i + 1}" (at {ux + (-2.54 if i % 2 == 0 else 2.54)} {y + 2.54 * (i // 2)} 0) {FONT} (uuid "{uid()}"))')
    for k, t in enumerate(g.get("TEXTS", [])):
        t = t.replace("\\", "\\\\").replace("\n", "\\n").replace('"', '\\"')
        out.append(f'(text "{t}" (exclude_from_sim no) (at 30 {200 + 10 * k} 0) {FONT} (uuid "{uid()}"))')
    libs = " ".join(lib(n, 2 if n == "U2" else 1) for n in PINS)
    os.makedirs(d, exist_ok=True)
    with open(f"{d}/t.kicad_sch", "w") as f:
        f.write(f'(kicad_sch (version {g.get("VERSION", 20250114)}) (generator "eeschema") (uuid "{root}") (paper "A3") (lib_symbols {libs})\n'
                + "\n".join(out) + '\n(sheet_instances (path "/" (page "1"))))\n')
    with open(f"{d}/t.kicad_pro", "w") as f:
        f.write('{"meta": {"filename": "t.kicad_pro", "version": 3}}')
    for name, text in g.get("FILES", {}).items():
        os.makedirs(os.path.dirname(f"{d}/{name}"), exist_ok=True)
        with open(f"{d}/{name}", "w") as f:
            f.write(text)
    cli = ["flatpak", "run", f"--filesystem={d}", "--command=kicad-cli", "org.kicad.kicad", "sch", "export", "netlist", "--format", "spice"]
    r = subprocess.run(cli + ["-o", f"{d}/k.cir", f"{d}/t.kicad_sch"], capture_output=True, text=True)
    print(r.stderr.strip() + "\n" + open(f"{d}/k.cir").read().replace(d, "DIR"))

# --- diff ---
SCALE = {"t": 1e12, "g": 1e9, "meg": 1e6, "k": 1e3, "mil": 25.4e-6, "m": 1e-3, "u": 1e-6, "n": 1e-9, "p": 1e-12, "f": 1e-15, "a": 1e-18}
ANALYSES = (".op", ".tran", ".ac", ".dc", ".noise", ".tf", ".pz", ".sens", ".disto", ".sp")

def norm(tok):
    """A token as ngspice reads a number (`4.7k`, `1Meg`, `10uF`, `1e3`), else in lower case."""
    m = re.fullmatch(r"([+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?)(meg|mil|[tgkmunpfa])?[a-zµΩ;%/]*", tok.lower())
    return f"{float(m.group(1)) * SCALE.get(m.group(2) or '', 1):.6g}" if m else tok.lower()

def tokens(text):
    t = [norm(t) for t in re.sub(r"[()=,]", " \\g<0> ", text).split()]
    # KiCad writes SIN's defaults (0 0 0) when the source has an AC part.
    while "sin" in t and t[t.index(")") - 1] == "0" and t.index(")") - t.index("(") > 4:
        del t[t.index(")") - 1]
    return t

def read(path):
    deck = {"include": set(), "model": {}, "directive": [], "part": {}}
    joined = []
    for line in open(path).read().splitlines():
        if line.startswith("+") and joined:
            joined[-1] += " " + line[1:]
        else:
            joined.append(line)
    control = False
    for s in (l.strip() for l in joined[1:]):
        low = s.lower()
        control |= low.startswith(".control")
        if not s or low == ".end":
            pass
        elif low.startswith(".include"):
            deck["include"].add(s.split(None, 1)[1].strip('"'))
        elif low.startswith(".model"):
            w = s.split(None, 2)
            deck["model"][w[1].lower()] = tokens(w[2] if len(w) > 2 else "")
        elif control or s[0] in ".*" or low.startswith("k"):
            deck["directive"].append(" ".join(low.split()))
        else:
            name, rest = (s.split(None, 1) + [""])[:2]
            t = tokens(rest)
            # `R1 a b r='…'` is `R1 a b '…'` to ngspice.
            if name[0].lower() in "rcl" and t[2:4] == [name[0].lower(), "="]:
                del t[2:4]
            deck["part"][name.lower()] = t
        control &= not low.startswith(".endc")
    return deck

def diff(kpath, epath):
    k, e = read(kpath), read(epath)
    same = 0
    for name in sorted(set(k["part"]) | set(e["part"])):
        kt, et = k["part"].get(name), e["part"].get(name)
        if kt and et and kt != et:
            ref = name[1:] if name[0] in "xmqbdjz" and name[1:2].isalpha() else name
            kt = [t.split(".", 1)[1] if t.startswith((name + ".", ref + ".")) else t for t in kt]
        if kt == et:
            same += 1
        else:
            print(f"part {name}:\n  kicad  {' '.join(kt or ['(none)'])}\n  cuprita {' '.join(et or ['(none)'])}")
    for name in sorted(set(k["model"]) | set(e["model"])):
        if k["model"].get(name) != e["model"].get(name):
            print(f"model {name}:\n  kicad  {k['model'].get(name)}\n  cuprita {e['model'].get(name)}")
    for x in sorted(k["include"] ^ e["include"]):
        print(f"include only in {'kicad' if x in k['include'] else 'cuprita'}: {x}")
    wbk = [d for d in e["directive"] if d.startswith(ANALYSES) and d not in k["directive"]]
    if wbk:
        print("analyses only in cuprita (from the workbook): " + "; ".join(wbk))
    kd, ed = sorted(k["directive"]), sorted(d for d in e["directive"] if d not in wbk)
    if kd != ed:
        print("directives:\n  kicad  " + "\n         ".join(kd) + "\n  cuprita " + "\n         ".join(ed))
    print(f"{same} of {len(set(k['part']) | set(e['part']))} parts the same")

cmd, *args = sys.argv[1:]
{"sheet": lambda d, spec: sheet(os.path.abspath(d), spec), "diff": diff}[cmd](*args)
