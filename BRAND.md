# Cuprita brand

Named for cuprite (Portuguese: cuprita), Cu₂O, the red ore copper comes from and the first skin on a board's copper.
Tagline: *Circuit boards at the speed of thought.* Brand page (private): https://claude.ai/artifact/DFoqisnBV1XDeDmkw5qudx.
Tokens: `crates/app/src/theme.rs`.

## Promise
- Instant: nothing waits on a spinner; heavy work runs beside the user, never in front.
- Opens what you have: KiCad files, saved byte for byte; no import, account or lock-in.
- One window, one flow: Schematic → Board → 3D → Make, one selection shared.
- Tells you what's left: unrouted, rule errors, missing footprints and part numbers.
- Helps the hand: crosshair, snap rings, 45° locks, a readout by the cursor.
- Private AI: a local model, through the same commands the user has, each one shown.

## Mark
A routed trace draws a C: 45° corners as copper turns, a via at each end
(`crates/app/assets/cuprita.svg`, 64-unit grid). App icon: the mark in #14100C on a copper
tile; in the app: copper on graphite. Below 24 px drop the vias. Wordmark: `cuprita`,
lower case, Inter SemiBold, −2 % tracking.

## Colour (dark only: CAD work happens on dark canvases)
| Token | Hex | Use |
|---|---|---|
| CANVAS | #0B0E12 | behind boards and sheets (render `BACKGROUND`) |
| GROUND | #101318 | top bar, status bar, pages |
| PANEL / RAISED | #13161B / #1B2027 | docks / fields, cards, popups |
| LINE | #1F242C | hairlines between areas |
| TEXT / TEXT_2 / QUIET | #E8EBEF / #C9CFD8 / #8C95A2 | titles / body / captions (≥ 4.5:1) |
| COPPER | #E8864F | the one accent: active tool, main action, focus; text on it #14100C |
| SIGNAL | #3FD0F0 | what the canvas selects, snaps to, measures |
| PASS / WARN / FAIL | #3DBE8B / #F0B44C / #F25F5C | checks; never the only cue (icon or word too) |

Layer colours stay KiCad-like (front red, back blue) so engineers read boards at a glance; the
active copper layer bright on top, the other copper dimmer, inner planes quiet.

## Type
Inter Medium for words (light on dark needs the weight), Inter SemiBold for titles,
JetBrains Mono for numbers, references and nets. Sizes: 11.5 caption, 13 body, 17 title,
26+ page title. Fonts are subset (`crates/app/assets/`, OFL): Latin, Greek, symbols.

## Shape
Radius 7 controls, 10–12 cards and popups, 14 dialogs. Hairlines, no heavy borders. Floating
things (tool dock, readouts) get a shadow; docked panels don't. Spacing on a 4 px grid.

## Motion (`crates/app/src/motion.rs`)
A change is a move the eye follows, then over: 0.12 s what answers the hand (a tool's wash, a
workbench fading in), 0.22 s the view gliding to what Find, Fit or a check shows, 0.45 s what
arrives (3D tilting up or turning over, a ring out from what was found). Ease out, no bounce, no
loop, nothing while the hand draws, no frame for motion once it ends; Settings can reduce it.

## Voice
Verbs first, units always, the next step in the same sentence; never blame the user.
- Say "3 nets left to route. Press X on a pad to start." Not "Ratsnest incomplete."
- Say "C4 is 0.08 mm from GND; it needs 0.2 mm. Move it right." Not "Clearance violation."
- Keys are named as KiCad names them; Cuprita keeps KiCad's keys where they exist.

## Don't
Decorative gradients and glows (a halo that shows clearance is a tool), motion that says
nothing, emoji, a second accent colour, all-caps buttons, modal dialogs in the middle of a
task, a spinner where a number would do.
