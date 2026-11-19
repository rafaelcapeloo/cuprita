#!/usr/bin/env bash
# Browser build into target/web, static files to host anywhere. `./web.sh serve` also serves it
# on http://127.0.0.1:8080. Installs the wasm-bindgen CLI matching Cargo.lock when missing.
set -euo pipefail
cd "$(dirname "$0")"
v=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p')
[ "$(wasm-bindgen --version 2>/dev/null)" = "wasm-bindgen $v" ] || cargo install --locked wasm-bindgen-cli --version "$v"
cargo build --profile web --target wasm32-unknown-unknown -p cuprita
wasm=target/wasm32-unknown-unknown/web/cuprita.wasm
# File names change with every build, so a browser never mixes cached old files with new ones.
name=cuprita-$(cksum < "$wasm" | cut -d' ' -f1)
rm -rf target/web
# Without the functions' names, 6% less to download: a panic still says what and where.
wasm-bindgen --target web --no-typescript --remove-name-section --out-name "$name" --out-dir target/web "$wasm"
sed "s/cuprita\.js/$name.js/" crates/app/index.html > target/web/index.html
cp crates/app/assets/cuprita.svg target/web/
[ "${1:-}" != serve ] || python3 -m http.server -b 127.0.0.1 -d target/web 8080
