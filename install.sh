#!/usr/bin/env bash
# Installs Cuprita for this user on Linux: the app as ~/.local/bin/cuprita, its launcher (Cuprita in
# the desktop's app grid), its icon, which Wayland shows on its window only through the
# launcher, the page `cuprita --serve` shows (`web.sh` builds it), in Cuprita's data folder, and
# KiCad's symbol and footprint libraries and 3D models (`packaging/libraries.sh` fetches them) beside the app,
# in ~/.local/share/cuprita/kicad. `./install.sh remove` takes them away again. It claims no file types, so KiCad files
# still open where they did; Cuprita opens them with Open… or a drop.
set -euo pipefail
cd "$(dirname "$0")"
data=${XDG_DATA_HOME:-$HOME/.local/share}
app=$HOME/.local/bin/cuprita
launcher=$data/applications/cuprita.desktop
icons=("$data/icons/hicolor/scalable/apps/cuprita.svg" "$data/icons/hicolor/256x256/apps/cuprita.png")
web=$data/cuprita/web
libraries=$HOME/.local/share/cuprita/kicad
if [ "${1:-}" = remove ]; then
  rm -f "$app" "$launcher" "${icons[@]}"
  rm -rf "$web" "$libraries"
  echo "Removed $app, its launcher, icon, page and libraries"
else
  cargo build --release -p cuprita
  ./web.sh
  install -Dm755 target/release/cuprita "$app"
  rm -rf "$web" && mkdir -p "$web" && cp target/web/index.html target/web/*.js target/web/*.wasm target/web/*.svg "$web/"
  packaging/libraries.sh
  # 3.6 GB: copied again only when their version changes.
  cmp -s target/libraries/version "$libraries/version" || { rm -rf "$libraries" && mkdir -p "$(dirname "$libraries")" && cp -R target/libraries "$libraries"; }
  install -Dm644 crates/app/assets/cuprita.svg "${icons[0]}"
  install -Dm644 crates/app/assets/cuprita.png "${icons[1]}"
  mkdir -p "$(dirname "$launcher")"
  # The file is named for the window's app id (`cuprita`), which is how the desktop matches them;
  # its app is this one, by its whole path (the desktop's PATH may lack ~/.local/bin).
  sed "s|^Exec=.*|Exec=\"$app\" %f|" crates/app/assets/cuprita.desktop > "$launcher"
  echo "Installed $app: Cuprita is in the app grid"
fi
# The desktop reads its launchers and icons again (either tool may be missing: then at next login).
update-desktop-database -q "$data/applications" 2>/dev/null || true
gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
