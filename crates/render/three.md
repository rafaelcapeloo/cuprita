# crates/render: the 3D view (`three.rs`)

The board in 3D for the app's 3D workbench and the agent's `render` with `3d`. Parent: `STATE.md`.

## Entry points
- `Orbit::{fit, turn, pan, zoom}` — the camera (yaw, pitch, distance round a target).
- `Renderer::{upload3d, render3d, draw3d}` — upload the board's body and its models
  (`cuprita_core::models::Models`), render into the renderer's own 4× MSAA target, show it in
  another pass; `render_rgba3d` (`png --3d`, agent `render` with `3d`).

## How it draws
- Meshes (the body from `Board::body`, each model once with an instance per footprint) write
  depth; lit from near the camera on both sides, sRGB vertex colours (`SRGB` constant). The
  body's sides are laminate but a plated hole's wall, its barrel, in the finish: each hole is a
  ring of the body round its centre, found on a 10 µm grid of vias' and copper pads' centres.
- Front and back faces: the 2D scene's slots drawn again at ±(thickness/2 + `LIFT`) through
  `cam.mvp`: holes mark the stencil, copper in a lighter shade of the mask, openings (`*.Mask`)
  in bare laminate, copper in them gold, silkscreen around them (`steps`). Meshes clear the
  stencil's "off board" mark, so nothing lies past the board's edge. Mask and silk colours come
  from the stackup (`named`: `Black`, `#RRGGBB`); green and white without.
- Models sit as KiCad places them (`place`), checked against `kicad-cli pcb render`.

## Missing
Paste, picking, other finishes than gold.
