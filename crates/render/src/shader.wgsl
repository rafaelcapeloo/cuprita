// Board renderer. World units are micrometres relative to the scene origin; Y points down.

struct View {
    center: vec2<f32>,
    scale: vec2<f32>,
    // World size of one physical pixel.
    px: f32,
    // Highlighted net, 0 for none.
    net: u32,
    // The active copper layer's slot + 1, 0 for none.
    active_slot: u32,
    // WebGL binds uniforms only in whole 16-byte blocks.
    _pad: f32,
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> colors: array<vec4<f32>, 256>;
// Each slot's copper, four to an element: 0 none, 1 an outer layer, 2 an inner one.
@group(0) @binding(6) var<uniform> copper: array<vec4<u32>, 64>;

// How bright copper other than the active layer draws: an outer layer, an inner one (mostly
// planes); and a copper zone's fill, so the tracks and pads on it show.
const OUTER: f32 = 0.55;
const INNER: f32 = 0.42;
const FILL: f32 = 0.75;

// Pipelines built with TOP redraw only the highlighted net, over everything else.
override TOP: bool = false;

// What TOP pipelines skip collapses to one point, which draws nothing.
fn clip(world: vec2<f32>, net: u32) -> vec4<f32> {
    let skip = select(view.net != 0u && net == view.net, net != view.net, TOP);
    return select(vec4<f32>((world - view.center) * view.scale, 0.0, 1.0), vec4<f32>(0.0, 0.0, 0.0, 1.0), skip);
}

// The colour of slot `tag` (+ 256 on a zone's fill): quieter on copper other than the active
// layer, but for the highlighted net, and on copper fills; darkened when another net is highlighted.
fn shade(tag: u32, net: u32) -> vec4<f32> {
    let slot = tag & 255u;
    let c = colors[slot];
    let kind = copper[slot / 4u][slot % 4u];
    let lit = view.net != 0u && net == view.net;
    let quiet = kind != 0u && view.active_slot != 0u && slot + 1u != view.active_slot && !lit;
    let k = select(1.0, select(OUTER, INNER, kind == 2u), quiet) * select(1.0, FILL, kind != 0u && tag > 255u);
    return vec4<f32>(c.rgb * k * select(0.3, 1.0, view.net == 0u || lit), c.a);
}

struct Sdf {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half: vec2<f32>,
    @location(2) @interpolate(flat) radius: f32,
    @location(3) @interpolate(flat) color: vec4<f32>,
};

// Corner `vi` of the quad around a rounded box, a pixel (`px`) wider to anti-alias, in the box's
// own frame; `pos` and `color` are the caller's.
fn sdf_corner(vi: u32, half_in: vec2<f32>, radius_in: f32, px: f32) -> Sdf {
    var out: Sdf;
    // Nothing thinner than a pixel, so hairlines stay visible when zoomed out.
    out.half = max(half_in, vec2<f32>(0.5 * px));
    out.radius = min(select(radius_in, max(radius_in, 0.5 * px), radius_in > 0.0), min(out.half.x, out.half.y));
    out.local = (vec2<f32>(f32(vi & 1u), f32(vi >> 1u)) * 2.0 - 1.0) * (out.half + px);
    return out;
}

// `v` in the frame whose x axis is `rot`.
fn turn(v: vec2<f32>, rot: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(v.x * rot.x - v.y * rot.y, v.x * rot.y + v.y * rot.x);
}

// How much of a pixel `px` wide the rounded box covers, from its signed distance.
fn coverage(in: Sdf, px: f32) -> f32 {
    let q = abs(in.local) - in.half + in.radius;
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - in.radius;
    return clamp(0.5 - d / px, 0.0, 1.0);
}

@vertex
fn vs_sdf(
    @builtin(vertex_index) vi: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_in: vec2<f32>,
    @location(2) rot: vec2<f32>,
    @location(3) radius_in: f32,
    @location(4) slot: u32,
    @location(5) net: u32,
) -> Sdf {
    var out = sdf_corner(vi, half_in, radius_in, view.px);
    out.pos = clip(center + turn(out.local, rot), net);
    out.color = shade(slot, net);
    return out;
}

@fragment
fn fs_sdf(in: Sdf) -> @location(0) vec4<f32> {
    let a = coverage(in, view.px);
    if a <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * a);
}

struct Tri {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_tri(@location(0) p: vec2<f32>, @location(1) slot: u32, @location(2) net: u32) -> Tri {
    var out: Tri;
    out.pos = clip(p, net);
    out.color = shade(slot, net);
    return out;
}

@fragment
fn fs_tri(in: Tri) -> @location(0) vec4<f32> {
    return in.color;
}

// 3D view: scene micrometres with Z up out of the board's front. Meshes (the board's body, parts'
// models) are lit from near the camera; the outer layers' shapes lie on the body's faces.

struct Cam {
    mvp: mat4x4<f32>,
    eye: vec4<f32>,
    // Unit vector towards the light.
    light: vec4<f32>,
    // Size of a pixel at the camera's target.
    px: f32,
    // A vec3 would align to 16 bytes, past the Rust struct's end.
    _pad: f32,
    _pad2: vec2<f32>,
};

// Per pass over a face: the colour of its shapes and the height they lie at.
struct Paint {
    color: vec4<f32>,
    z: f32,
    _pad: f32,
    _pad2: vec2<f32>,
};

@group(0) @binding(2) var<uniform> cam: Cam;
@group(0) @binding(3) var<uniform> paint: Paint;

// Meshes' colours are sRGB; pipelines built with SRGB light them in linear light.
override SRGB: bool = false;

@vertex
fn vs_sdf3(
    @builtin(vertex_index) vi: u32,
    @location(0) center: vec2<f32>,
    @location(1) half_in: vec2<f32>,
    @location(2) rot: vec2<f32>,
    @location(3) radius_in: f32,
    @location(4) slot: u32,
    @location(5) net: u32,
) -> Sdf {
    var out = sdf_corner(vi, half_in, radius_in, cam.px);
    out.pos = cam.mvp * vec4<f32>(center + turn(out.local, rot), paint.z, 1.0);
    out.color = paint.color;
    return out;
}

@fragment
fn fs_sdf3(in: Sdf) -> @location(0) vec4<f32> {
    // A pixel's size where this one lies, as perspective makes it.
    let a = coverage(in, max(length(dpdx(in.local)), length(dpdy(in.local))));
    if a <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * a);
}

@vertex
fn vs_tri3(@location(0) p: vec2<f32>, @location(1) slot: u32, @location(2) net: u32) -> Tri {
    var out: Tri;
    out.pos = cam.mvp * vec4<f32>(p, paint.z, 1.0);
    out.color = paint.color;
    return out;
}

struct Lit {
    @builtin(position) pos: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

// A mesh vertex, placed by its instance's 3 × 4 matrix (rows `r0`..`r2`).
@vertex
fn vs_mesh(
    @location(0) p: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) r0: vec4<f32>,
    @location(4) r1: vec4<f32>,
    @location(5) r2: vec4<f32>,
) -> Lit {
    let q = vec4<f32>(p, 1.0);
    var out: Lit;
    out.world = vec3<f32>(dot(r0, q), dot(r1, q), dot(r2, q));
    out.normal = vec3<f32>(dot(r0.xyz, n), dot(r1.xyz, n), dot(r2.xyz, n));
    out.pos = cam.mvp * vec4<f32>(out.world, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_mesh(in: Lit) -> @location(0) vec4<f32> {
    let v = normalize(cam.eye.xyz - in.world);
    // Lit on whichever side faces the camera: models' normals point either way.
    var n = normalize(in.normal);
    if dot(n, v) < 0.0 {
        n = -n;
    }
    let l = cam.light.xyz;
    let spec = pow(max(dot(n, normalize(l + v)), 0.0), 48.0);
    let base = select(in.color.rgb, pow(in.color.rgb, vec3<f32>(2.2)), SRGB);
    return vec4<f32>(base * (0.35 + 0.65 * max(dot(n, l), 0.0)) + 0.2 * spec, 1.0);
}

@group(0) @binding(4) var shot: texture_2d<f32>;
@group(0) @binding(5) var shot_sampler: sampler;

struct Blit {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle over the viewport, showing the 3D view's last frame.
@vertex
fn vs_blit(@builtin(vertex_index) vi: u32) -> Blit {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: Blit;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_blit(in: Blit) -> @location(0) vec4<f32> {
    return textureSample(shot, shot_sampler, in.uv);
}
