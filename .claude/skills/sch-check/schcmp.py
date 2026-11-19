"""Compares Cuprita's schematic drawing with KiCad's SVG plots.

schcmp.py <kicad svg dir> <cuprita texts dir> [--worst N]
    Pairs each page `<root>[-<sheet>...].svg` (kicad-cli sch export svg) with Cuprita's texts of
    the same name (texts.rs), and each text with KiCad's nearest of the same string within 1 mm.
    Prints, per ink, how many centres sit within 0.05 mm (centres compare: Hershey glyphs reach a
    stroke unit further each side than KiCad's), the worst, and texts only one side draws
    (KiCad's frame and title block aside; multi-line texts aren't compared, KiCad plots each
    line apart).
schcmp.py overlay <kicad.svg> <cuprita.png> x0,y0,x1,y1 <out.png>
    The same sheet box (mm) of KiCad's plot and of Cuprita's render (png example, 8192 px wide:
    the page and 2.5% of its width round it) as one image: red KiCad only, green Cuprita only.
"""
import ast, collections, os, re, subprocess, sys, tempfile


def kicad_texts(svg):
    """(text, pen, x0, y0, x1, y1, colour) of each stroked text, and the page size (mm); KiCad
    plots some texts twice in the same place, kept once."""
    s = open(svg, encoding='utf-8').read()
    size = tuple(float(v) for v in re.search(r'width="([\d.]+)mm" height="([\d.]+)mm"', s).groups())
    out, style = [], ('-', 0.0)
    for m in re.finditer(r'<g style="([^"]*)"|<g class="stroked-text"><desc>(.*?)</desc>(.*?)</g>', s, re.S):
        if m.group(1):
            st = re.search(r'stroke:(#[0-9A-Fa-f]{6}); stroke-width:([0-9.]+)', m.group(1))
            style = (st.group(1), float(st.group(2))) if st else ('-', 0.0)
            continue
        nums = [float(v) for v in re.findall(r'-?\d+\.\d+', m.group(3))]
        if nums:
            xs, ys = nums[0::2], nums[1::2]
            text = re.sub(r'&(lt|gt|amp|quot|apos);', lambda e: {'lt': '<', 'gt': '>', 'amp': '&', 'quot': '"', 'apos': "'"}[e.group(1)], m.group(2))
            out.append((text.strip(), style[1], min(xs), min(ys), max(xs), max(ys), style[0]))
    return list(dict.fromkeys(out)), size


def cuprita_texts(path):
    out = []
    for line in open(path, encoding='utf-8'):
        ink, pen, x0, y0, x1, y1, text = line.rstrip('\n').split(' ', 6)
        out.append((ast.literal_eval(text).strip(), float(pen), float(x0), float(y0), float(x1), float(y1), ink))
    return out


def compare(kdir, edir, worst_n):
    stats, worst, lonely = collections.defaultdict(lambda: [0, 0]), [], []
    for name in sorted(os.listdir(kdir)):
        page, txt = name[:-4], os.path.join(edir, name[:-4].replace('/', '_') + '.txt')
        if not name.endswith('.svg') or not os.path.exists(txt):
            print('no Cuprita page for', name)
            continue
        k, (w, h) = kicad_texts(os.path.join(kdir, name))
        ours = cuprita_texts(txt)
        # KiCad plots each line of a text apart: those of Cuprita's multi-line texts aren't compared.
        lines = {line.strip() for t, *_ in ours if '\n' in t for line in t.split('\n')}
        by, used = collections.defaultdict(list), set()
        for i, (t, _, x0, y0, x1, y1, _) in enumerate(k):
            by[t].append((i, (x0 + x1) / 2, (y0 + y1) / 2))
        for t, _, x0, y0, x1, y1, ink in ours:
            if '\n' in t:
                continue
            cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
            near = [(max(abs(cx - x), abs(cy - y)), i) for i, x, y in by[t] if abs(cx - x) < 1 and abs(cy - y) < 1]
            if not near:
                lonely.append((page, 'Cuprita only', ink, t))
                continue
            d, i = min(near)
            used.add(i)
            stats[ink][0] += 1
            stats[ink][1] += d <= 0.05
            worst.append((d, page, ink, t))
        # KiCad's frame and title block: within 12 mm of the page's edge, or the 108 x 32 mm box.
        frame = lambda x, y: x < 12 or y < 12 or x > w - 12 or y > h - 12 or (x > w - 120 and y > h - 44)
        alone = lambda i, t, x, y: i not in used and t not in lines and not frame(x, y)
        lonely += [(page, 'KiCad only', c, t) for i, (t, _, x0, y0, x1, y1, c) in enumerate(k) if alone(i, t, (x0 + x1) / 2, (y0 + y1) / 2)]
    for ink, (n, close) in sorted(stats.items()):
        print(f'{ink:14} {n:6} texts, {close:6} within 0.05 mm')
    for d, page, ink, t in sorted(worst, reverse=True)[:worst_n]:
        print(f'{d:7.3f} mm  {ink:12} {t!r} ({page})')
    for page, side, ink, t in lonely[:worst_n]:
        print(f'{side}: {ink} {t!r} ({page})')
    print(len(lonely), 'texts only one side draws')


def overlay(svg, png, box, out):
    x0, y0, x1, y1 = (float(v) for v in box.split(','))
    _, (w, h) = kicad_texts(svg)
    width = int(subprocess.run(['magick', 'identify', '-format', '%w', png], capture_output=True, text=True, check=True).stdout)
    ppm = width / (w * 1.05)
    height = int(subprocess.run(['magick', 'identify', '-format', '%h', png], capture_output=True, text=True, check=True).stdout)
    ox, oy = w * 0.025 * ppm, (height / ppm - h) / 2 * ppm
    geo = lambda dx, dy: f'{round((x1 - x0) * ppm)}x{round((y1 - y0) * ppm)}+{round(x0 * ppm + dx)}+{round(y0 * ppm + dy)}'
    with tempfile.TemporaryDirectory() as d:
        ink = ['-colorspace', 'gray', '-threshold', '85%', '-negate']
        subprocess.run(['magick', png, '-crop', geo(ox, oy), '+repage', *ink, f'{d}/e.png'], check=True)
        subprocess.run(['magick', '-density', str(ppm * 25.4), svg, '-background', 'white', '-flatten', '-crop', geo(0, 0), '+repage', *ink, f'{d}/k.png'], check=True)
        size = subprocess.run(['magick', 'identify', '-format', '%wx%h', f'{d}/k.png'], capture_output=True, text=True, check=True).stdout
        subprocess.run(['magick', f'{d}/k.png', f'{d}/e.png', '-size', size, 'xc:black', '-set', 'colorspace', 'sRGB', '-combine', '-type', 'TrueColor', out], check=True)


if __name__ == '__main__':
    a = sys.argv[1:]
    if a and a[0] == 'overlay':
        overlay(*a[1:5])
    else:
        compare(a[0], a[1], int(a[a.index('--worst') + 1]) if '--worst' in a else 15)
