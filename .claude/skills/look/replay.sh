#!/usr/bin/env bash
# replay.sh on|off: puts replay.rs into the app's main.rs, so the real window replays the steps
# in $CUPRITA_REPLAY, or puts main.rs back as it was before `on`. Build after either; never
# commit main.rs while it is on.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
cd "$(git rev-parse --show-toplevel)"
main=crates/app/src/main.rs
# One copy kept per checkout, so worktrees don't share it.
kept=${TMPDIR:-/tmp}/cuprita-main-$(pwd | cksum | cut -d' ' -f1).rs
case ${1:-} in
  on)
    [ ! -e "$kept" ] || { echo "replay is on already: replay.sh off first"; exit 1; }
    grep -q '^impl eframe::App for App {$' "$main" || { echo "no 'impl eframe::App for App {' in $main"; exit 1; }
    cp "$main" "$kept"
    sed -i "/^impl eframe::App for App {\$/r $here/replay.rs" "$main"
    echo "replay on: build, then CUPRITA_REPLAY=<steps> target/release/cuprita …" ;;
  off)
    [ -e "$kept" ] || { echo "replay is off"; exit 1; }
    # Touched, or cargo (which goes by mtime) keeps the build with the replay in it.
    mv "$kept" "$main" && touch "$main"
    echo "replay off: main.rs as it was" ;;
  *) echo "usage: replay.sh on|off"; exit 2 ;;
esac
