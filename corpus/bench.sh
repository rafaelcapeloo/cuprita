#!/usr/bin/env bash
# corpus/bench.sh: times Cuprita beside KiCad on two corpus projects (./corpus/fetch.sh brings
# them), each doing the same job on the same files, and prints the README's table. Median of
# $RUNS runs (default 5), in milliseconds.
# - Commands (DRC, ERC, fab files, PDF) are timed as one waits for them: from the program's
#   start to its exit, reading the files and writing the result included.
# - Opening a board and refilling its zones are timed inside each program (KiCad's Python,
#   Cuprita's examples), without its start.
# - KiCad is the Flatpak org.kicad.kicad, timed inside its sandbox: the sandbox's own start
#   (0.2 s) isn't counted. Its sandbox sees the home folder, so the copies go in ~/.cache.
# - DRC and ERC run on a copy of each project where the kinds only KiCad checks (footprints and
#   symbols against their libraries, text sizes, starved thermals, off-grid ends…) are set to
#   ignore, so both check the same kinds; KiCad as it ships takes 5 to 60% longer.
set -euo pipefail
cd "$(dirname "$0")/.."
runs=${RUNS:-5}
work=$HOME/.cache/cuprita-bench
cargo build --release --quiet -p cuprita -p cuprita-core --examples
bin=$PWD/target/release
rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT
cat > "$work/kicad.py" <<'EOF'
import sys, time, pcbnew
t0 = time.perf_counter(); board = pcbnew.LoadBoard(sys.argv[1]); t1 = time.perf_counter()
pcbnew.ZONE_FILLER(board).Fill(board.Zones()); t2 = time.perf_counter()
print("TIMES %.1f %.1f" % ((t1 - t0) * 1e3, (t2 - t1) * 1e3))
EOF
cat > "$work/same_kinds.py" <<'EOF'
import json, sys
pro = json.load(open(sys.argv[1]))
drc = pro.setdefault("board", {}).setdefault("design_settings", {}).setdefault("rule_severities", {})
for kind in ["lib_footprint_issues", "lib_footprint_mismatch", "starved_thermal", "holes_co_located", "footprint_type_mismatch",
             "text_height", "text_thickness", "padstack", "nonmirrored_text_on_back_layer"]:
    drc[kind] = "ignore"
erc = pro.setdefault("erc", {}).setdefault("rule_severities", {})
for kind in ["lib_symbol_issues", "lib_symbol_mismatch", "footprint_link_issues", "endpoint_off_grid", "unconnected_wire_endpoint"]:
    erc[kind] = "ignore"
json.dump(pro, open(sys.argv[1], "w"), indent=2)
EOF
median() { sort -n | sed -n "$(( (runs + 1) / 2 ))p"; }
ms() { local s e; s=$(date +%s%N); "$@" > /dev/null 2>&1 || true; e=$(date +%s%N); echo $(( (e - s) / 1000000 )); }
# A command of Cuprita's, then one of KiCad's (timed by the sandbox's own shell), each $runs times.
ours() { for _ in $(seq "$runs"); do ms "$@"; done | median; }
kicad() { for _ in $(seq "$runs"); do flatpak run --command=bash org.kicad.kicad -c "s=\$(date +%s%N); $1 >/dev/null 2>&1; e=\$(date +%s%N); echo \$(( (e - s) / 1000000 ))" 2>/dev/null | tail -1; done | median; }
inside() { for _ in $(seq "$runs"); do flatpak run --command=python3 org.kicad.kicad "$work/kicad.py" "$1" 2>/dev/null | sed -n "s/^TIMES //p"; done; }
row() { printf '| %s | %s | %s | %s | %s× |\n' "$1" "$2" "$3" "$4" "$(awk -v k="$3" -v c="$4" 'BEGIN { printf (k / c >= 10 ? "%.0f" : "%.1f"), k / c }')"; }
echo "| Job | Project | KiCad $(flatpak run --command=kicad-cli org.kicad.kicad version 2>/dev/null | tail -1) | Cuprita $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1) | |"
echo "|---|---|--:|--:|--:|"
for project in glasgow-revD1:glasgow kria-k26:kria-k26-devboard; do
  name=${project%%:*} root=${project##*:} dir=$work/${project%%:*}
  mkdir -p "$dir"
  cp corpus/boards/"$name"/* "$dir/"
  cp "corpus/boards/$name.kicad_pcb" "$dir/$root.kicad_pcb"
  python3 "$work/same_kinds.py" "$dir/$root.kicad_pro"
  board=$dir/$root.kicad_pcb sheet=$dir/$root.kicad_sch
  inside "$board" > "$work/inside"
  theirs_open=$(cut -d' ' -f1 "$work/inside" | median) theirs_fill=$(cut -d' ' -f2 "$work/inside" | median)
  ours_open=$(for _ in $(seq "$runs"); do "$bin/examples/load" "$board" | sed -n 's/.* in \([0-9.]*\) ms .*/\1/p'; done | median)
  ours_fill=$(for _ in $(seq "$runs"); do "$bin/examples/fill" "$board" | sed -n 's/.* zones filled in \([0-9.]*\) ms.*/\1/p'; done | median)
  row "Open the board" "$name" "$theirs_open" "$ours_open"
  row "Refill every zone" "$name" "$theirs_fill" "$ours_fill"
  row "DRC" "$name" "$(kicad "kicad-cli pcb drc -o $dir/k.rpt $board")" "$(ours "$bin/cuprita" --json "$board" '{"cmd":"drc"}')"
  row "ERC" "$name" "$(kicad "kicad-cli sch erc -o $dir/k.erc $sheet")" "$(ours "$bin/cuprita" --json "{\"cmd\":\"erc\",\"path\":\"$sheet\"}")"
  row "Schematic as a PDF" "$name" "$(kicad "kicad-cli sch export pdf -o $dir/k.pdf $sheet")" "$(ours "$bin/cuprita" --json "{\"cmd\":\"pdf\",\"path\":\"$sheet\"}")"
  # KiCad: Gerbers, then drill files. Cuprita's one command also refills the zones and writes the BOM and positions.
  row "Fab files" "$name" "$(kicad "kicad-cli pcb export gerbers -o $dir/k/ $board; kicad-cli pcb export drill -o $dir/k/ $board")" "$(ours "$bin/cuprita" --json "$board" '{"cmd":"fab"}')"
done
