---
name: icons
description: Add an icon to Cuprita's app (a Phosphor glyph in the subset font crates/app/assets/Phosphor.ttf), or re-subset the UI fonts. Use when a button, tab or panel needs an icon that crates/app/src/icon.rs lacks.
---
# Add an icon

Icons are glyphs of Phosphor (MIT, regular weight) in a font subset to the ones `icon.rs` lists;
`theme.rs` puts it after the text faces. Pick one at https://phosphoricons.com.

1. Get the font and its names, outside the repo, in an empty folder:
   `curl -sSLO https://unpkg.com/@phosphor-icons/web@2.1.1/src/regular/Phosphor.ttf` and
   `curl -sSLO https://unpkg.com/@phosphor-icons/web@2.1.1/src/regular/style.css`.
2. Its codepoint: `grep -A1 '\.ph-<name>:before' style.css` (e.g. `\e760` for `factory`).
3. Add `pub(crate) const WHAT_IT_DOES: &str = "\u{e760}"; // factory` to `crates/app/src/icon.rs`
   (named for what it does; the comment is its Phosphor name). Only icons something uses.
4. Re-subset to every codepoint `icon.rs` lists (pyftsubset is fontTools'):
   ```
   P=$(sed -nE 's/.*\\u\{([0-9a-f]+)\}.*/U+\1/p' crates/app/src/icon.rs | paste -sd,)
   pyftsubset <dir>/Phosphor.ttf --unicodes="$P" --no-hinting --layout-features='' \
     --output-file=crates/app/assets/Phosphor.ttf
   ```
5. `cargo test -p cuprita theme` checks every icon has its glyph.

# Text faces
`Inter-Medium`, `Inter-SemiBold` (github.com/rsms/inter, v4.1 `extras/ttf`) and
`JetBrainsMono-Regular` (v2.304) are subset to the characters the app shows:
`pyftsubset <font>.ttf --unicodes="U+0020-007E,U+00A0-017F,U+0391-03A9,U+03B1-03C9,U+2000-206F,U+2070-209F,U+20AC,U+2100-214F,U+2190-21FF,U+2200-22FF,U+2300-23FF,U+25A0-25FF,U+2713-2717" --layout-features='kern,ccmp,locl,mark,mkmk' --no-hinting --output-file=crates/app/assets/<font>.ttf`
(egui autohints them). A new sign in a text: add its range, then run the theme test.
