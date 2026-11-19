#!/usr/bin/env bash
# Downloads the files in manifest.tsv into corpus/boards/ (git-ignored) and verifies them: a
# board `name` is `name.kicad_pcb`; a project's other files are `project/file` as named.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p boards
while IFS=$'\t' read -r name sha license url; do
  [[ -z "$name" || "$name" == \#* ]] && continue
  if [[ "$name" == */* ]]; then f="boards/$name"; else f="boards/$name.kicad_pcb"; fi
  [[ -f "$f" ]] || curl -fsSL --retry 2 --create-dirs -o "$f" "$url"
  echo "$sha  $f" | sha256sum --check --quiet || { echo "checksum mismatch: $f ($license)" >&2; exit 1; }
done < manifest.tsv
