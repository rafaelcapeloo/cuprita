# Parts

A part is a symbol on the schematic and a footprint on the board. The parts a project brings in
or makes go into its own library (`libraries/` beside it), which KiCad reads too.

- A adds a symbol from KiCad's libraries, found by name, by what it is, or by part number
  (`R`, `usb c`, `LM358DR`); R turns it before the click that puts it down. Cuprita comes with
  KiCad's symbols, footprints and 3D models; with KiCad installed, its own are used.
- A part number can also come from Digi-Key: KiCad's symbol for it (or a resistor's, capacitor's
  or inductor's with its value), its footprint, datasheet, maker and number. It needs your own
  free key: make an app with Product Information V4 at developer.digikey.com and paste its
  Client ID and Secret in Settings (Ctrl+,).
- A part KiCad's libraries lack: Parts links to SnapMagic, Ultra Librarian and SamacSys for its
  number; download its KiCad zip there, then Add part files… puts it into the project's own
  library, which KiCad reads too, and Parts shows it: Enter adds it, and F8 brings its footprint
  and 3D model to the board.
- Or make it: New part… (in Parts, or Ctrl+K) takes its name, its pins (type how many, Fill, then
  name them and pick each one's type and side) and its footprint, picked from the libraries or
  made from its package (SOIC, QFN, a pin header…, with the datasheet's sizes); both show as you
  type, and Create part puts them into the project's library, ready to add.
- Click a part to see its fields in Inspect and type in them (Value, Datasheet, MPN, LCSC…);
  Footprint › Choose… lists the footprints that fit it.
