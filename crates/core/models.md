# crates/core: 3D models (`models.rs`, `step.rs`, `step/face.rs`, `body.rs`)

## Finding them (`models::{files, load}`)
- A footprint's `(model "file" …)` (`Model`: offset in mm, scale, turns in degrees; hidden ones
  dropped; KiCad 5's `at` offset is in inches) names its file with `${VAR}`s: the environment's,
  else the board's folder (`KIPRJMOD`) or KiCad's 3D model folders (`KISYS3DMOD`,
  `KICAD*_3DMODEL_DIR` of any version), found as footprints are (`library::kicad_dirs`). A file
  without a variable is beside the board.
- Decision: STEP only. KiCad 10's library ships 7,251 STEP files and no WRL, so a `.wrl` link reads
  the `.step` beside it; a model only in WRL says so.
- `load` reads the files on all cores (`threads::par_map`); an error says what and how to fix it
  (`ANT3DMDL is not set; set it to the folder of its 3D models`, `not found in <folders>`,
  `KiCad's 3D models aren't installed`). KiCad 10 renamed some (`SOP-8` → `SOIC-8`): not found.

## Reading STEP (`step::read` → `Mesh`: mm, Z up, a normal and an sRGB colour per vertex)
- Part 21 text (bytes, `''` in strings, complex entities); lengths in mm, m or inches, angles in
  radians or degrees.
- Shapes: `MANIFOLD_SOLID_BREP`, `BREP_WITH_VOIDS`, `FACETED_BREP`, `SHELL_BASED_SURFACE_MODEL` of
  the representations its assembly tree places (`REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION`:
  from item 1 to item 2 of its `ITEM_DEFINED_TRANSFORMATION`; `MAPPED_ITEM`), each read once.
- Colours: `STYLED_ITEM`s' `COLOUR_RGB` or pre-defined colour, a face's before its solid's; else grey.
- Edges: lines, circles, ellipses, B-splines (rational too), within 10 µm (`TOL`), ≥ 12 a turn.
- Faces (`step/face.rs`): laid out in their surface's own coordinates (plane; cylinder and cone:
  angle, height; sphere; torus), unwrapped round the axis, with seams and poles; two loops round
  the axis bound a band. Lyon triangulates, Delaunay flips even out its slivers, spheres and tori
  split inner edges until within `TOL`. Boundary points are the edges' own, so faces meet without
  cracks (tested). B-spline and swept faces lie flat on the plane they fit best.

## The board (`Board::body`)
Its area inside the Edge.Cuts outlines (else the box around its copper) less cutouts and every
drilled hole, as i_overlay contours: the 3D view's faces and sides (5 ms on kria-k26).

## Checked
- All 7,251 KiCad 10 library models: no error, 4 of 2.6 M faces left out, 38 M triangles, 38 s
  on one core (89 MB/s); glasgow-revD1's 41 models in 65 ms on 12 threads.
- Against `kicad-cli pcb render` (skill `look`): placement (turns about X, Y then Z, offset, a
  back part turned over about X) and the look of caps, LEDs, crystals, DIP, QFP, TO, connectors.

## Missing
WRL files; transparency; B-spline faces curve only as far as their edges do; variables only
KiCad's path settings define (set them in the environment); a STEP of the whole board.
