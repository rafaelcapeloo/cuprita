#!/usr/bin/env python3
"""packaging/notices.py: writes packaging/CRATES.txt, the licence texts of every Rust crate
Cuprita is built from (Cargo.lock, every system's), which the installers carry: their licences
(MIT, Apache-2.0, BSD…) ask that the text goes with the program. Each text once, with the crates
under it. Run it after Cargo.lock changes: `check.sh` fails when the file is older than the lock
(its first line holds the lock's SHA-256)."""
import hashlib
import json
import pathlib
import subprocess

root = pathlib.Path(__file__).resolve().parent.parent
meta = json.loads(subprocess.run(['cargo', 'metadata', '--format-version', '1', '--locked'], cwd=root, capture_output=True, check=True).stdout)
ours = set(meta['workspace_members'])
texts, bare = {}, []
for p in sorted((p for p in meta['packages'] if p['id'] not in ours), key=lambda p: (p['name'].lower(), p['version'])):
    name = f"{p['name']} {p['version']} ({p.get('license') or 'see its source'})"
    files = sorted(f for f in pathlib.Path(p['manifest_path']).parent.iterdir()
                   if f.is_file() and f.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'UNLICENSE', 'COPYRIGHT')))
    if not files:
        bare.append(name)
    for f in files:
        text = '\n'.join(line.rstrip() for line in f.read_text(errors='replace').strip().splitlines())
        texts.setdefault(text, []).append(name)
lock = hashlib.sha256((root / 'Cargo.lock').read_bytes()).hexdigest()
out = [f'Cargo.lock {lock}', '', 'The Rust crates Cuprita is built from, and the licence texts they come with.',
       'Made by packaging/notices.py; not every crate goes into every system\'s program.', '']
if bare:
    out += ['Crates that carry no licence file of their own (their licence, as each declares it):', *('  ' + n for n in bare), '']
for text, crates in sorted(texts.items(), key=lambda t: (-len(t[1]), t[1][0])):
    out += ['=' * 78, *sorted(set(crates)), '-' * 78, text, '']
(root / 'packaging/CRATES.txt').write_text('\n'.join(out) + '\n')
print(f'packaging/CRATES.txt: {len(meta["packages"]) - len(ours)} crates, {len(texts)} texts, {len(bare)} without a file')
