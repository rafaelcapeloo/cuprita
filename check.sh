#!/usr/bin/env bash
# Run before every commit. Fails on format, lint, tests, leftover temp files or oversized docs.
set -euo pipefail
cd "$(dirname "$0")"
cargo fmt --all --check
cargo clippy --workspace --all-targets --quiet -- -D warnings
cargo clippy -p cuprita --target wasm32-unknown-unknown --quiet -- -D warnings
cargo test --workspace --quiet
fail=0
while read -r f; do
  n=$(wc -l < "$f")
  [ "$n" -le 60 ] || { echo "doc too long ($n > 60 lines): $f"; fail=1; }
done < <(git ls-files --cached --others --exclude-standard '*.md')
# The crates' licence texts the installers carry are those of this Cargo.lock.
[ "$(head -1 packaging/CRATES.txt)" = "Cargo.lock $(sha256sum < Cargo.lock | cut -d' ' -f1)" ] || { echo "Cargo.lock changed: run python3 packaging/notices.py"; fail=1; }
junk=$(git ls-files --others --exclude-standard | grep -E '(\.(tmp|bak|orig|rej|log)|~)$|(^|/)(tmp|scratch)[^/]*$' || true)
[ -z "$junk" ] || { printf 'remove temp files:\n%s\n' "$junk"; fail=1; }
exit $fail
