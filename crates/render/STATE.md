# crates/render: state

wgpu renderer for a `Board`. The CPU side (`scene.rs`) is testable without a GPU. The 3D view:
`three.md`.

## Entry points
- `build(&Board) -> Scene` — primitives in back-to-front draw slots (one per layer + vias + holes);
  `Scene::layers` gives each slot's name, colour and default visibility.
- `rebuild(&Board, Scene) -> Scene` — after an edit: keeps the old scene's layer visibility and,
  while the origin stays, the triangles of each zone fill the board still has (`Zones::tris`, by
  fill): a refill draws only the fills it changed (kria-k26 6 ms, `build` 44).
- `build_diff(old, new, &Diff) -> Scene` — visual diff scene (unchanged dimmed, removed red, added green).
- `build_ir(board, &IrDrop) -> Scene`, `heat_color(t)` — DC IR-drop thermal heatmap scene and colour ramp.
- `build_sch(&Project, page) -> Scene` — a schematic page: its paper, then a slot per `Ink` in
  KiCad's default colours, its drawing sheet (`Project::frame`) first (all above the stackup, so
  flipping keeps them).
- `Scene::net_at(p, tol, visible, flip, active)` — net of the topmost visible item at a point, as
  `draw_order` stacks them (linear scan, ≤ 2 ms on kria-k26). `Scene::um` / `Scene::nm` — nm ↔ µm.
- `Renderer::{new, upload, set_view, draw}` — one colour target, no depth, 1× MSAA;
  `draw(pass, visible, &view)` skips hidden slots, reverses the stackup when the view is flipped
  and puts its active copper just above the other copper (`scene::draw_order`).
- `View::new(center_um, um_per_px, size_px)`, `View::{flip, active}`, `View::net`, `fit(bounds, size_px)`.
- `Renderer::shot(device, queue, bounds, width, visible, net, flip)` — the scene uploaded drawn into a
  texture of its own and read back, the side in view's outer copper active (a window's renderer
  makes Home's pictures with it); `render_rgba(scene, bounds, width, visible, net, flip)` — the
  same headless; used by `examples/png.rs` (skill `look`) and the app's agent `render` tool.

## How it draws
- `Inst` = rotated rounded box, drawn by an SDF shader: capsules for tracks/lines, boxes and
  circles for pads, vias, holes. Nothing renders thinner than 1 px.
- Zone fills and filled polygons → lyon triangles (even-odd), indexed. Zone fills' come first in
  `verts`/`indices` (`Draw::fills`, drawn before `Draw::indices`), so `rebuild` keeps them as they are.
- Text → `cuprita_core::font::strokes` polylines → capsules on the text's layer.
- Scene units: µm (f32) relative to `Scene::origin` (board outline centre).
- Colours per slot in a uniform array (≤ 256 slots). Every layer is built; Paste, Mask, Adhes, Fab
  (values, bodies: an assembly drawing's) and `*.Cu` layers the board's table lacks (KiCad hides
  them) start hidden. Masks are 45% opaque.
- Net highlight: instances and vertices carry their net (0 = none). With `View::net` set, the
  shader darkens other nets to 30%, then `draw` repeats with `TOP` pipelines that keep only
  that net, so it shows above the planes covering it.
- Active copper (`View::active`, the app's active layer) draws at full brightness; the shader,
  from each slot's `scene::copper` (uniform `copper`), draws other outer copper at 55 %, inner
  layers (mostly planes) at 42 %, the lit net's excepted, and copper zone fills (`Vert::slot |
  ZONE`) at 75 %, so the tracks and pads on a plane show; all opaque. Tuned on hackrf-one,
  glasgow-revD1 and kria-k26 (skill `look`); kria-k26 600–1100 fps uncapped (XWayland).

## Missing
Translucency (masks only), zone outlines, exact trapezoids (custom pads draw their whole
`Pad::copper()`, as triangles), level of detail for zoomed-out huge boards.

## Gotchas
- Pad rotation: KiCad degrees θ → local x axis (cos θ, −sin θ), because Y points down.
- Instance ranges are drawn by offsetting the vertex buffer (WebGL has no first_instance).
- WebGL binds uniforms only in whole 16-byte blocks: pad WGSL uniform structs to match the Rust
  ones (test `uniforms_match_rust_and_fit_webgl`).
