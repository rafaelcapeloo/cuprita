# crates/app: the panels over the window

The palette, Help and Settings: each an `Area` over the window, closed by Esc or a click outside.
Parent: `ui.md`.

- `palette.rs` — Ctrl+K (and Ctrl+F, `Cmd::Find`): `hits` are the commands that can run, then (once
  typed) the parts and nets of the view and the projects to open (`Home::recent`, `Home::examples`), by `score` (a start, a word's start, a part, the letters
  in order); Enter runs a command or `find`s a part or net. Its field asks for the focus before it
  shows each frame: egui drops focus given to a field in a new floating area.
- `help.rs` — Help (F1; the ? on Home): `GROUPS` lists every command once by where it works (a
  test keeps it whole), each with its keys and hint, `matches` filters by the words typed; First
  board and First steps at the top. `App::help` (`Help`) is its filter and the guide's page
  shown: its Guide tab draws `docs/*.md` in the language shown (`docs/<tag>/`; `docs.rs`: `GUIDES`, built in, `pages`; `show`
  draws the Markdown they use, a bullet opening with a link goes to that page; tests check every
  page is there and links to pages), opening on Home, the commands elsewhere.
- `settings.rs` — Settings (Ctrl+,, KiCad's Preferences key; the gear on Home): units, motion
  (`motion.rs`: `set`, kept as `motion`), the
  language (`tr.rs`; as the system's, or one chosen, kept as `language`), the Digi-Key key (`digikey_key`: the environment's, else the one kept in the data folder's
  `digikey`, mode 0600; the UI and agents look parts up with it), the data folder (Show).
  Help and Settings close each other.
- `share.rs` — Share (the header's QR button): `share_panel`, under its button (its place kept in
  egui's memory as `BUTTON`), the served page's address as a code and in letters; `web.md`.
