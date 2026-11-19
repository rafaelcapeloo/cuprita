# crates/core: electrical rules (`schematic/erc.rs`)

On the nets of `nets.md`.

`Project::erc()` — `Violation { kind, error, page, at, what, fix }` in plain words (the language
shown, `tools.md`), page by page, errors first. Kinds (English keys) and severities are KiCad's
(`.kicad_pro` `erc.rule_severities`, `ignore` drops a check; pin conflicts by its `erc.pin_map`,
else KiCad's map). Checks:
- Pins: not connected (free ones and flagged ones aside; a hidden power input when its net has
  other pins); an input or power input that nothing drives (KiCad's drivers: output,
  bidirectional, tri-state, passive and power pins; of power inputs, power outputs only); pin
  type conflicts (each pin against the earlier one worst for it); no-connect pins wired.
- No-connect flags on no pin, or on a net with pins at two places.
- Labels on no pin or one; a local and a global label of one name on a sheet; names alike but
  in case; two or more names on one net or bus (once each).
- Wires and bus entries on nothing; wire ends on nothing; a sheet-named net entering a bus that
  hasn't it as a member of the bus's name (its first, as KiCad ranks names; once per net);
  sheet pins and hierarchical labels without each other.
- References used twice, or not numbered.

## Where we differ from KiCad (`kicad-cli sch erc`), on purpose
- A problem inside a sheet used more than once is told once, on its first page (KiCad tells
  some once per use); labels are checked there, as KiCad does.
- Places are the problem's own (KiCad marks a wire's far end at its start).
- A wire on nothing isn't told again for its ends; names alike but in case once per two names.
- KiCad's CLI tells no references used twice or unnumbered; we do.

## Checked
- `erc.rs` tests: each check, its place, the project's severities, messages.
- `tests/corpus.rs`: ERC counts by kind on the four corpus projects equal `kicad-cli sch erc`'s
  (skill `erc-check`) but for the differences above.

## Speed
≤ 2.0 ms a corpus project, its nets included (`kicad-cli sch erc` 1.3–2.5 s); the `load`
example times it on a root `.kicad_sch`.

## Missing (`// shortcut:` in the code)
Checks needing the symbol and footprint libraries; off-grid points, bus syntax and conflicts,
net classes, units (missing, extra, unlike), ground pins, simulation, four-way junctions,
labels on two wires.
To look into: on glasgow-revD1 with its `.kicad_pro`, 20 of KiCad's 23 undriven power pins and 1 of
its 4 dangling wires are found (`corpus/bench.md`, 2026-11-18); kria-k26 agrees kind for kind.
