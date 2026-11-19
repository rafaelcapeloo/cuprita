# crates/core: a part number at Digi-Key (`digikey.rs`)

`digikey::lookup(mpn, key)` asks Digi-Key for the part of that number (any case): its maker, what it
is, datasheet, narrowest category and parameters (`Part`). `Part::kicad(symbols, footprints)`
gives KiCad's symbol for it, the symbol's text (`library::symbol`) and the fields it takes on a
sheet (`Fields`). Desktop only: the browser can't run curl.

## The user's own key, nothing else to set up
- Digi-Key's Product Information V4, free: an app made at developer.digikey.com; its Client ID
  and Secret (`Key`) given to `lookup`: the app's Settings keep it, `env_key()` reads it from
  `DIGIKEY_CLIENT_ID` and `DIGIKEY_CLIENT_SECRET` (KiCost's and Digi-Key's Python client's names,
  so their users have them set), which win.
- Two requests by curl: a token (OAuth client credentials, `/v1/oauth2/token`), then a keyword
  search (`/products/v4/search/keyword`); its exact matches first, then the products numbered so.
  curl reads the requests from its input: the key stays out of the list of processes.
- Errors in Digi-Key's own words (`ErrorMessage` from the token, RFC 7807 `title`/`detail` from
  the search); a refused key (401, 403) says to check its Client ID and Secret at
  developer.digikey.com; no such part names the numbers Digi-Key has instead.

## What goes on the sheet (`Part::kicad`)
- The symbol KiCad names for its number (`library::named`: LM358DR, LM358), else for a chip
  resistor, ceramic capacitor or fixed inductor (its category) `Device:R`, `C` or `L`, its value
  as KiCad writes them (`10k`, `100n` for "0.1 µF", `4.7u`); for an LED (indicator, white or
  colour lighting) `Device:LED`, a single diode `D` (`D_Schottky` by its technology, `D_Zener`),
  valued by their number; else no symbol: the error says what the part is and its category
  (Digi-Key's makers' symbols aren't in its API), and Parts links to sites that give it.
- Its footprint: the symbol's own (79% of KiCad 10's have one), else its package's that the
  symbol's footprint filters allow (`library::fits`), the plainest: "8-SOIC (0.154\", 3.90mm
  Width)" gives `SOIC-8_3.9x4.9mm_P1.27mm`, "8-TSSOP (… 4.40mm Width)" `TSSOP-8_4.4x3mm_P0.65mm`,
  "0603 (1608 Metric)" a resistor's `R_0603_1608Metric`. A package named without a digit
  (Radial, Nonstandard) gets none rather than a guess, as an LED's "1411 (3528 Metric)" does
  (the trade's 2835 and its 3528 share the size, not the pads).
- `Datasheet`, `Manufacturer`, `MPN`: the BOM's columns (`bom.rs`) and the chat's datasheets.

## Checked
`digikey.rs` tests: a reply shaped as Digi-Key's OpenAPI spec says (fields, categories, a
scheme-less link), its errors as its servers answer them (both forms, probed with a made-up key),
values, symbols and footprints. On KiCad 10's libraries, 15 parts as Digi-Key describes them
(passives, LM358 in SOIC, DIP and TSSOP, 74HC595D, STM32F103C8T6): each the right symbol and
footprint, 0.1–5 ms (the symbol's library read). Against Digi-Key itself (the owner's key, its
categories learned so): a Cree J 2835 LED, a 0603 indicator LED, BAT54, 1N4148W, BZT52C5V1.

## Missing
Other distributors (Mouser's API gives no package, so no footprint), transistors without a
KiCad symbol of their own (pins in each package's order), polarized capacitors, a token kept
between lookups (`// shortcut:`).
