#!/usr/bin/env bash
# packaging/libraries.sh: KiCad's symbol and footprint libraries and its 3D models (STEP) into
# target/libraries, which every installer carries (and `install.sh` installs), so Parts and the
# 3D view have them where no KiCad is installed: `library::kicad_dirs` looks beside the app,
# after an installed KiCad's own. A build run from the source finds them in target/libraries
# itself. They are KiCad's (CC-BY-SA 4.0, with its exception for the designs made with them):
# their LICENSE.md files go along. 3.6 GB there, 0.7 GB to download.
# Each release archive must match its SHA-256 here: for another version, set it and the three
# sums (`sha256sum` of the archives this downloads).
set -euo pipefail
cd "$(dirname "$0")/.."
version=10.0.8
symbols=f7fc4be38bd21ff97fa7532dd61b98600981d66986a7958d5a7c274bc435250c
footprints=94d712756d3129df9886fd005a69669973a31865827eadf762722eeefcdb1763
packages3D=efb6b74aa8c343aac882c1c225e0a214df15cfe7a45694af8df1b44273c0b858
out=target/libraries
stamp="$version symbols footprints 3dmodels"
[ "$(cat $out/version 2>/dev/null)" != "$stamp" ] || exit 0
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sum() { { sha256sum "$1" 2>/dev/null || shasum -a 256 "$1"; } | cut -d' ' -f1; }
for kind in symbols footprints packages3D; do
  url=https://gitlab.com/kicad/libraries/kicad-$kind/-/archive/$version/kicad-$kind-$version.tar.gz
  curl -fsSL --retry 3 -o "$work/$kind.tar.gz" "$url"
  [ "$(sum "$work/$kind.tar.gz")" = "${!kind}" ] || { echo "libraries.sh: $url is not the archive expected (SHA-256 $(sum "$work/$kind.tar.gz")): check it, then set the sum in packaging/libraries.sh"; exit 1; }
  tar -xzf "$work/$kind.tar.gz" -C "$work"
  rm "$work/$kind.tar.gz"
done
rm -rf $out
mkdir -p $out/symbols $out/footprints $out/3dmodels
# KiCad keeps a symbol library as a folder, a file per symbol, and installs it as one file
# (its tools/kicad_lib_pack.py): the first file's header, every file's symbols, the last line.
for dir in "$work/kicad-symbols-$version"/*.kicad_symdir; do
  awk 'FNR == 1 { body = 0 } /^\t\(symbol / { body = 1 } !body { if (NR == FNR) print; next } $0 != ")" { print } END { print ")" }' \
    "$dir"/*.kicad_sym > "$out/symbols/$(basename "$dir" .kicad_symdir).kicad_sym"
done
mv "$work/kicad-footprints-$version"/*.pretty $out/footprints/
mv "$work/kicad-packages3D-$version"/*.3dshapes $out/3dmodels/
cp "$work/kicad-packages3D-$version/LICENSE.md" "$work/kicad-packages3D-$version/CREDITS.md" $out/3dmodels/
cp "$work/kicad-symbols-$version/LICENSE.md" $out/LICENSE.md
echo "$stamp" > $out/version
echo "KiCad $version's libraries are in $out: $(ls $out/symbols | wc -l) of symbols, $(ls $out/footprints | wc -l) of footprints, $(ls $out/3dmodels | wc -l) of 3D models"
