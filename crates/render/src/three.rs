//! The 3D view: the board's body cut by its outline and holes, lit from near the camera, with its
//! outer copper, solder mask and silkscreen (the 2D scene's shapes) on its faces. It renders into
//! its own multisampled target, which `Renderer::draw3d` shows.

use crate::scene::{Inst, Scene, Vert};
use crate::{BACKGROUND, Renderer, srgb_to_linear};
use bytemuck::{Pod, Zeroable};
use cuprita_core::board::{Board, Footprint, Model};
use cuprita_core::models::Models;
use lyon_tessellation::math::point;
use lyon_tessellation::path::Polygon;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};
use std::ops::Range;
use wgpu::util::DeviceExt;

/// Vertical field of view, degrees.
const FOV: f32 = 30.0;
const SAMPLES: u32 = 4;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
/// How far above a face its layers' shapes lie, µm: about the copper's thickness, and far more
/// than the depth buffer can tell apart (`Orbit::camera` keeps it so).
const LIFT: f32 = 20.0;
/// A paint's place in its uniform buffer: dynamic offsets go in steps of 256 bytes.
const PAINT_STRIDE: u64 = 256;
/// Colours: bare laminate, the copper finish (ENIG gold), and the default solder mask and silkscreen.
const SUBSTRATE: [u8; 3] = [156, 140, 92];
const FINISH: [u8; 3] = [212, 175, 72];
const GREEN: [u8; 3] = [30, 110, 58];
const WHITE: [u8; 3] = [240, 240, 240];

/// Where the 3D view looks from: `dist` from `target` (scene µm, Z up out of the front), turned
/// `yaw` degrees about the board's normal (0 looks north, as the board is drawn) and tilted
/// `pitch` degrees up from the board (90 looks straight down at the front, -90 up at the back).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub target: [f32; 3],
    pub dist: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Orbit {
    /// The whole of `bounds` (scene µm) in a view of `size` pixels, from the south and above.
    pub fn fit(bounds: [f32; 4], size: [f32; 2]) -> Self {
        let [x0, y0, x1, y1] = bounds;
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let mut o = Orbit { target: [cx, cy, 0.0], dist: 100.0, yaw: 0.0, pitch: 50.0 };
        let ([r, u, b], ty) = (o.axes(), (FOV / 2.0).to_radians().tan());
        let tx = ty * size[0] / size[1].max(1.0);
        // As close as keeps each corner (from the target, Y north) inside both fields of view.
        for [x, y] in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
            let along = |a: [f32; 3]| a[0] * (x - cx) + a[1] * (cy - y);
            o.dist = o.dist.max(along(b) + 1.05 * (along(r).abs() / tx).max(along(u).abs() / ty));
        }
        o
    }

    /// A drag of `dx`, `dy` pixels turns the board as if grabbed.
    pub fn turn(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.4;
        self.pitch = (self.pitch + dy * 0.4).clamp(-90.0, 90.0);
    }

    /// A drag of `dx`, `dy` pixels in a view `height` pixels high slides the board along.
    pub fn pan(&mut self, dx: f32, dy: f32, height: f32) {
        let ([r, u, _], px) = (self.axes(), self.px(height));
        let w: [f32; 3] = std::array::from_fn(|k| (dy * u[k] - dx * r[k]) * px);
        self.target = [self.target[0] + w[0], self.target[1] - w[1], self.target[2] + w[2]];
    }

    /// Closer by a fifth per wheel notch.
    pub fn zoom(&mut self, notches: f32) {
        self.dist = (self.dist * 0.8f32.powf(notches)).clamp(100.0, 1e8);
    }

    /// Size of a pixel at the target, in a view `height` pixels high.
    fn px(&self, height: f32) -> f32 {
        2.0 * self.dist * (FOV / 2.0).to_radians().tan() / height.max(1.0)
    }

    /// Right, up and back (towards the eye) unit vectors, in a frame with Y north.
    fn axes(&self) -> [[f32; 3]; 3] {
        let ((sy, cy), (sp, cp)) = (self.yaw.to_radians().sin_cos(), self.pitch.to_radians().sin_cos());
        let (right, back) = ([cy, sy, 0.0], [cp * sy, -cp * cy, sp]);
        [right, cross(back, right), back]
    }

    /// The camera for a view of `size` pixels showing everything within `reach` µm of the
    /// target: the matrix from scene µm to clip space, the eye and the light (scene frame), and
    /// the size of a pixel at the target.
    fn camera(&self, size: [f32; 2], reach: f32) -> Cam {
        let [r, u, b] = self.axes();
        // The scene's Y points south: mirror it into the camera's frame, and back.
        let t = [self.target[0], -self.target[1], self.target[2]];
        let eye: [f32; 3] = std::array::from_fn(|k| t[k] + b[k] * self.dist);
        let row = |a: [f32; 3]| [a[0], -a[1], a[2], -(a[0] * eye[0] + a[1] * eye[1] + a[2] * eye[2])];
        let view = [row(r), row(u), row(b), [0.0, 0.0, 0.0, 1.0]];
        // The nearest plane as far out as keeps LIFT four steps of the depth buffer at `reach`.
        let far = self.dist + reach;
        let near = (self.dist / 100.0).max(far * far * 4.0 / (LIFT * (1 << 24) as f32)).min(self.dist / 2.0);
        let f = 1.0 / (FOV / 2.0).to_radians().tan();
        let (a, z) = (size[0] / size[1].max(1.0), far / (near - far));
        let proj = [[f / a, 0.0, 0.0, 0.0], [0.0, f, 0.0, 0.0], [0.0, 0.0, z, near * z], [0.0, 0.0, -1.0, 0.0]];
        // Column-major, as WGSL reads it: column i, row j of proj × view.
        let mvp: [[f32; 4]; 4] = std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| proj[j][k] * view[k][i]).sum()));
        let l = normalize(std::array::from_fn(|k| b[k] + 0.3 * u[k] - 0.2 * r[k]));
        let px = self.px(size[1]);
        Cam { mvp, eye: [eye[0], -eye[1], eye[2], 1.0], light: [l[0], -l[1], l[2], 0.0], px, _pad: [0.0; 3] }
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.map(|x| x / n)
}

/// The camera uniform: `mvp` is column-major, as WGSL reads matrices.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct Cam {
    mvp: [[f32; 4]; 4],
    eye: [f32; 4],
    light: [f32; 4],
    px: f32,
    _pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct Paint {
    color: [f32; 4],
    z: f32,
    _pad: [f32; 3],
}

/// A mesh vertex in its own frame, with its sRGB colour.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct Vert3 {
    pos: [f32; 3],
    normal: [f32; 3],
    color: [u8; 4],
}

/// Where a mesh goes: rows of a 3 × 4 matrix from its frame to scene µm.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct Place([[f32; 4]; 3]);

const IDENTITY: Place = Place([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]]);

/// Stencil use of a pass over a face: test the reference (`KEEP`), mark solder mask openings
/// (`OPEN`: 0 → 1), or mark holes (`HOLE`: → 2, no colour). It starts at `OFF`, which meshes
/// (the board's body) overwrite with 0, so nothing lies on a face beyond the board's edge.
const KEEP: usize = 0;
const OPEN: usize = 1;
const HOLE: usize = 2;
const OFF: u32 = 3;

/// One pass of the 2D scene's slots over a face, with a stencil use, its reference and a paint.
#[derive(Debug, PartialEq)]
struct Step {
    slots: Vec<usize>,
    stencil: usize,
    reference: u32,
    paint: usize,
}

/// What the 3D view draws: its pipelines, the board's body and how the faces are painted.
pub(crate) struct Three {
    mesh: wgpu::RenderPipeline,
    sdf: [wgpu::RenderPipeline; 3],
    tri: [wgpu::RenderPipeline; 2],
    blit: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    cam: wgpu::Buffer,
    paints: wgpu::Buffer,
    bind: wgpu::BindGroup,
    verts: wgpu::Buffer,
    indices: wgpu::Buffer,
    places: wgpu::Buffer,
    /// Index and place ranges of each mesh.
    meshes: Vec<(Range<u32>, Range<u32>)>,
    steps: Vec<Step>,
    /// How far from the camera's target things can be, µm: the board's size, half its thickness
    /// and its tallest part, which the depth buffer must tell apart.
    reach: f32,
    target: Option<Target>,
}

/// Where the 3D view renders: multisampled colour and depth, resolved into `shot`, which the
/// `blit` binding shows.
struct Target {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    shot: wgpu::Texture,
    shot_view: wgpu::TextureView,
    blit: wgpu::BindGroup,
}

impl Renderer {
    /// Uploads what the 3D view draws of `board`, whose 2D scene `scene` is uploaded: its body,
    /// and the footprints' 3D models `models` has read.
    pub fn upload3d(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, board: &Board, scene: &Scene, models: &Models) {
        let three = self.three.get_or_insert_with(|| Three::new(device, &self.shader, self.format));
        let colors = Colors::of(board);
        let h = board.thickness as f32 / 2000.0;
        let (mut verts, mut indices) = body(board, scene, colors.mask);
        let (mut places, mut meshes) = (vec![IDENTITY], vec![(0..indices.len() as u32, 0..1)]);
        // Each model once, placed on every footprint that shows it.
        let mut users: Vec<(&String, Place)> = board
            .footprints
            .iter()
            .flat_map(|f| {
                f.models.iter().filter(|m| matches!(models.get(&m.file), Some(Ok(_)))).map(move |m| (&m.file, place(f, m, scene, h)))
            })
            .collect();
        users.sort_by(|a, b| a.0.cmp(b.0));
        let mut tallest = 0.0f32;
        for group in users.chunk_by(|a, b| a.0 == b.0) {
            let (Some(Ok(mesh)), base, first) = (models.get(group[0].0), verts.len() as u32, indices.len() as u32) else { continue };
            verts.extend((0..mesh.pos.len()).map(|i| Vert3 { pos: mesh.pos[i], normal: mesh.normal[i], color: mesh.color[i] }));
            indices.extend(mesh.indices.iter().map(|i| i + base));
            tallest = mesh.pos.iter().fold(tallest, |t, p| t.max(p[2].abs() * 1000.0));
            meshes.push((first..indices.len() as u32, places.len() as u32..(places.len() + group.len()) as u32));
            places.extend(group.iter().map(|u| u.1));
        }
        let buf = |data: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: data, usage });
        three.verts = buf(bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
        three.indices = buf(bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
        three.places = buf(bytemuck::cast_slice(&places), wgpu::BufferUsages::VERTEX);
        three.meshes = meshes;
        three.reach = scene.bounds[2] - scene.bounds[0] + scene.bounds[3] - scene.bounds[1] + h + tallest;
        let (steps, paints) = steps(board, &colors, h + LIFT);
        for (i, (rgb, z)) in paints.into_iter().enumerate() {
            let c = rgb.map(|v| if self.srgb { srgb_to_linear(v as f32 / 255.0) } else { v as f32 / 255.0 });
            let paint = Paint { color: [c[0], c[1], c[2], 1.0], z, _pad: [0.0; 3] };
            queue.write_buffer(&three.paints, i as u64 * PAINT_STRIDE, bytemuck::bytes_of(&paint));
        }
        three.steps = steps;
    }

    /// Renders the 3D view, `size` pixels, from `orbit` into its own target (`draw3d` shows it).
    pub fn render3d(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        orbit: &Orbit,
    ) {
        let (Some(g), Some(t)) = (&self.gpu, &mut self.three) else { return };
        let size = size.map(|s| s.clamp(1, device.limits().max_texture_dimension_2d));
        if t.target.as_ref().is_none_or(|x| x.size != size) {
            t.target = Some(Target::new(device, self.format, size, &t.blit_layout, &t.sampler));
        }
        queue.write_buffer(&t.cam, 0, bytemuck::bytes_of(&orbit.camera(size.map(|s| s as f32), t.reach)));
        let target = t.target.as_ref().expect("made above");
        let [r, gr, b, _] = BACKGROUND.map(|c| f64::from(if self.srgb { srgb_to_linear(c) } else { c }));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.msaa,
                depth_slice: None,
                resolve_target: Some(&target.shot_view),
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r, g: gr, b, a: 1.0 }), store: wgpu::StoreOp::Discard },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(OFF), store: wgpu::StoreOp::Discard }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &t.bind, &[0]);
        pass.set_stencil_reference(0);
        pass.set_pipeline(&t.mesh);
        pass.set_vertex_buffer(0, t.verts.slice(..));
        pass.set_index_buffer(t.indices.slice(..), wgpu::IndexFormat::Uint32);
        let place = size_of::<Place>() as u64;
        for (indices, places) in &t.meshes {
            // Offset the buffer instead of using first_instance, which WebGL lacks.
            pass.set_vertex_buffer(1, t.places.slice(places.start as u64 * place..places.end as u64 * place));
            pass.draw_indexed(indices.clone(), 0, 0..places.len() as u32);
        }
        let stride = size_of::<Inst>() as u64;
        for s in &t.steps {
            pass.set_bind_group(0, &t.bind, &[(s.paint as u64 * PAINT_STRIDE) as u32]);
            pass.set_stencil_reference(s.reference);
            for d in s.slots.iter().filter_map(|&slot| g.draws.iter().find(|d| d.slot == slot)) {
                for tris in [&d.fills, &d.indices].into_iter().filter(|r| !r.is_empty() && s.stencil != HOLE) {
                    pass.set_pipeline(&t.tri[s.stencil]);
                    pass.set_vertex_buffer(0, g.verts.slice(..));
                    pass.set_index_buffer(g.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(tris.clone(), 0, 0..1);
                }
                if !d.insts.is_empty() {
                    pass.set_pipeline(&t.sdf[s.stencil]);
                    pass.set_vertex_buffer(0, g.insts.slice(d.insts.start as u64 * stride..d.insts.end as u64 * stride));
                    pass.draw(0..4, 0..d.insts.len() as u32);
                }
            }
        }
    }

    /// Shows the last 3D view rendered, filling the pass's viewport.
    pub fn draw3d(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(Three { blit, target: Some(target), .. }) = &self.three {
            pass.set_pipeline(blit);
            pass.set_bind_group(0, &target.blit, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// The last 3D view rendered (`render3d`), for reading back.
    pub(crate) fn shot3d(&self) -> Option<&wgpu::Texture> {
        Some(&self.three.as_ref()?.target.as_ref()?.shot)
    }
}

impl Three {
    fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule, format: wgpu::TextureFormat) -> Self {
        let uniform = |binding, dynamic, size: usize| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: wgpu::BufferSize::new(size as u64),
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[uniform(2, false, size_of::<Cam>()), uniform(3, true, size_of::<Paint>())],
        });
        let usage = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let cam =
            device.create_buffer(&wgpu::BufferDescriptor { label: None, size: size_of::<Cam>() as u64, usage, mapped_at_creation: false });
        let paints =
            device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16 * PAINT_STRIDE, usage, mapped_at_creation: false });
        let paint = wgpu::BufferBinding { buffer: &paints, offset: 0, size: wgpu::BufferSize::new(size_of::<Paint>() as u64) };
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 2, resource: cam.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Buffer(paint) },
            ],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let srgb = [("SRGB", if format.is_srgb() { 1.0 } else { 0.0 })];
        let pipeline = |vs: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout>], depth: wgpu::DepthStencilState, writes| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                primitive: wgpu::PrimitiveState {
                    topology: if buffers.len() == 1 && buffers[0].as_ref().is_some_and(|b| b.step_mode == wgpu::VertexStepMode::Instance) {
                        wgpu::PrimitiveTopology::TriangleStrip
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    ..Default::default()
                },
                depth_stencil: Some(depth),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &srgb, ..Default::default() },
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: writes })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let inst = wgpu::VertexBufferLayout {
            array_stride: size_of::<Inst>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32, 4 => Uint32, 5 => Uint32],
        };
        let vert = wgpu::VertexBufferLayout {
            array_stride: size_of::<Vert>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Uint32, 2 => Uint32],
        };
        let mesh = [
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vert3>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Place>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4],
            }),
        ];
        // Meshes write depth; shapes on a face test it against them but leave it as it is.
        let depth = |write, compare, pass_op| {
            let side = wgpu::StencilFaceState {
                compare,
                fail_op: wgpu::StencilOperation::Keep,
                depth_fail_op: wgpu::StencilOperation::Keep,
                pass_op,
            };
            wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(write),
                depth_compare: Some(if write { wgpu::CompareFunction::Less } else { wgpu::CompareFunction::LessEqual }),
                stencil: wgpu::StencilState { front: side, back: side, read_mask: 0xFF, write_mask: 0xFF },
                bias: Default::default(),
            }
        };
        let (equal, always) = (wgpu::CompareFunction::Equal, wgpu::CompareFunction::Always);
        let (keep, incr, replace) = (wgpu::StencilOperation::Keep, wgpu::StencilOperation::IncrementClamp, wgpu::StencilOperation::Replace);
        let all = wgpu::ColorWrites::ALL;
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&blit_layout)],
                immediate_size: 0,
            })),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_blit"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_blit"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: all })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let empty = |usage| device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage, mapped_at_creation: false });
        Self {
            mesh: pipeline("vs_mesh", "fs_mesh", &mesh, depth(true, always, replace), all),
            sdf: [(equal, keep, all), (equal, incr, all), (always, replace, wgpu::ColorWrites::empty())]
                .map(|(c, op, w)| pipeline("vs_sdf3", "fs_sdf3", &[Some(inst.clone())], depth(false, c, op), w)),
            tri: [keep, incr].map(|op| pipeline("vs_tri3", "fs_tri", &[Some(vert.clone())], depth(false, equal, op), all)),
            blit,
            blit_layout,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor::default()),
            cam,
            paints,
            bind,
            verts: empty(wgpu::BufferUsages::VERTEX),
            indices: empty(wgpu::BufferUsages::INDEX),
            places: empty(wgpu::BufferUsages::VERTEX),
            meshes: vec![],
            steps: vec![],
            reach: 0.0,
            target: None,
        }
    }
}

impl Target {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
    ) -> Self {
        let texture = |format, sample_count, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let draw = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let shot = texture(format, 1, draw | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let shot_view = shot.create_view(&Default::default());
        let blit = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&shot_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        let msaa = texture(format, SAMPLES, draw).create_view(&Default::default());
        let depth = texture(DEPTH, SAMPLES, draw).create_view(&Default::default());
        Self { size, msaa, depth, shot, shot_view, blit }
    }
}

/// The board's colours in 3D, sRGB: solder mask (front, back), silkscreen, and copper under mask.
struct Colors {
    mask: [[u8; 3]; 2],
    silk: [[u8; 3]; 2],
    under: [[u8; 3]; 2],
}

impl Colors {
    fn of(b: &Board) -> Self {
        let mask = [0, 1].map(|i| named(b.mask_colors[i].as_deref(), GREEN));
        // Copper shows through the mask as a lighter, warmer shade of it.
        let under = mask.map(|m| std::array::from_fn(|k| (m[k] as f32 + (FINISH[k] as f32 - m[k] as f32) * 0.15) as u8));
        Self { mask, silk: [0, 1].map(|i| named(b.silk_colors[i].as_deref(), WHITE)), under }
    }
}

/// A stackup colour (`"Black"`, `"Matte Blue"`, `"#1A2B3CFF"`) as sRGB, else `default`.
fn named(name: Option<&str>, default: [u8; 3]) -> [u8; 3] {
    const NAMES: [(&str, [u8; 3]); 8] = [
        ("black", [22, 22, 24]),
        ("white", [235, 235, 235]),
        ("red", [150, 28, 28]),
        ("blue", [24, 60, 140]),
        ("purple", [80, 38, 120]),
        ("yellow", [214, 190, 40]),
        ("green", GREEN),
        ("gr", [110, 110, 110]),
    ];
    let Some(name) = name else { return default };
    if let Some(v) = name.strip_prefix('#').and_then(|h| u32::from_str_radix(h.get(..6)?, 16).ok()) {
        return [(v >> 16) as u8, (v >> 8) as u8, v as u8];
    }
    let name = name.to_ascii_lowercase();
    NAMES.iter().find(|(k, _)| name.contains(k)).map_or(default, |&(_, c)| c)
}

/// The passes over the front and back faces, and their paints (sRGB, height): holes first, then
/// copper under the mask, the mask's openings (bare laminate), the copper in them, and the
/// silkscreen around them.
fn steps(b: &Board, c: &Colors, z: f32) -> (Vec<Step>, Vec<([u8; 3], f32)>) {
    let layer = |name: &str| b.layers.iter().position(|l| l == name);
    let cu = [b.stackup.first(), b.stackup.last().filter(|_| b.stackup.len() > 1)].map(|c| c.map(|c| c.layer as usize));
    let (vias, holes) = (b.layers.len(), b.layers.len() + 1);
    let (mut steps, mut paints) = (vec![], vec![]);
    for (side, (mask, silk)) in [("F.Mask", "F.SilkS"), ("B.Mask", "B.SilkS")].into_iter().enumerate() {
        let z = if side == 0 { z } else { -z };
        let copper: Vec<usize> = cu[side].into_iter().chain([vias]).collect();
        let open: Vec<usize> =
            layer(mask).into_iter().chain([vias].into_iter().filter(|_| ![b.rules.tented.0, b.rules.tented.1][side])).collect();
        for (slots, stencil, reference, rgb) in [
            (vec![holes], HOLE, 2, SUBSTRATE),
            (copper.clone(), KEEP, 0, c.under[side]),
            (open, OPEN, 0, SUBSTRATE),
            (copper, KEEP, 1, FINISH),
            (layer(silk).into_iter().collect(), KEEP, 0, c.silk[side]),
        ] {
            steps.push(Step { slots, stencil, reference, paint: paints.len() });
            paints.push((rgb, z));
        }
    }
    (steps, paints)
}

/// Where model `m` of footprint `f` goes: from the model's millimetres (Y north, Z up out of its
/// side) to scene µm, on its side's face `h` µm from the board's middle. KiCad turns models
/// clockwise about X, then Y, then Z, offsets them, then turns them with their footprint; a part on
/// the back is turned over about X first (checked against `kicad-cli pcb render`).
fn place(f: &Footprint, m: &Model, scene: &Scene, h: f32) -> Place {
    type M3 = [[f64; 3]; 3];
    let mul = |a: M3, b: M3| -> M3 { std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum())) };
    let turn = |axis: usize, deg: f64| -> M3 {
        let ((s, c), (i, j)) = (deg.to_radians().sin_cos(), ((axis + 1) % 3, (axis + 2) % 3));
        let mut t: M3 = std::array::from_fn(|r| std::array::from_fn(|c| if r == c { 1.0 } else { 0.0 }));
        (t[i][i], t[i][j], t[j][i], t[j][j]) = (c, -s, s, c);
        t
    };
    let [rx, ry, rz] = m.rotate;
    let scale: M3 = std::array::from_fn(|r| std::array::from_fn(|c| if r == c { m.scale[r] } else { 0.0 }));
    let own = mul(turn(2, -rz), mul(turn(1, -ry), mul(turn(0, -rx), scale)));
    let fp = mul(turn(2, f.angle), turn(0, if f.back { 180.0 } else { 0.0 }));
    let (lin, at) = (mul(fp, own), scene.um(f.at));
    let off: [f64; 3] = std::array::from_fn(|r| (0..3).map(|k| fp[r][k] * m.offset[k]).sum());
    let origin = [at[0] as f64, at[1] as f64, if f.back { -h } else { h } as f64];
    // Millimetres to µm; the scene's Y points south.
    Place(std::array::from_fn(|r| {
        let k = if r == 1 { -1000.0 } else { 1000.0 };
        [lin[r][0] * k, lin[r][1] * k, lin[r][2] * k, off[r] * k + origin[r]].map(|v| v as f32)
    }))
}

/// The board's body in scene µm, Z up out of the front: its faces in the `mask` colours (front,
/// back) and its sides, cutouts' and unplated holes' too, in the laminate's; a plated hole's
/// wall is its barrel, in the copper's finish.
fn body(board: &Board, scene: &Scene, mask: [[u8; 3]; 2]) -> (Vec<Vert3>, Vec<u32>) {
    let h = board.thickness as f32 / 2000.0;
    let rings: Vec<Vec<[f32; 2]>> = board.body().iter().map(|c| c.iter().map(|&p| scene.um(p)).collect()).collect();
    let points: Vec<Vec<_>> = rings.iter().map(|r| r.iter().map(|&[x, y]| point(x, y)).collect()).collect();
    let polys: Vec<_> = points.iter().map(|r| Polygon { points: r, closed: true }).collect();
    let mut faces: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut sink = BuffersBuilder::new(&mut faces, |v: FillVertex| v.position().to_array());
    // A failure leaves the faces out; the sides still show the outline.
    let _ = FillTessellator::new().tessellate(polys.iter().flat_map(Polygon::path_events), &FillOptions::DEFAULT, &mut sink);
    let rgba = |[r, g, b]: [u8; 3]| [r, g, b, 255];
    let (mut verts, mut indices) = (vec![], vec![]);
    for (z, color) in [(h, mask[0]), (-h, mask[1])] {
        let base = verts.len() as u32;
        verts.extend(faces.vertices.iter().map(|&[x, y]| Vert3 { pos: [x, y, z], normal: [0.0, 0.0, z.signum()], color: rgba(color) }));
        indices.extend(faces.indices.iter().map(|i| i + base));
    }
    // Each hole is a ring of its own round its centre: plated ones are found by it, on a 10 µm
    // grid (vias are; pads with copper are, as fabs plate them).
    let cell = |[x, y]: [f32; 2]| ((x / 10.0).round() as i64, (y / 10.0).round() as i64);
    let pads = board.pads.iter().filter(|p| p.drill.is_some() && p.layers & board.copper != 0).map(|p| p.at);
    let plated: std::collections::HashSet<_> = board.vias.iter().map(|v| v.at).chain(pads).map(|p| cell(scene.um(p))).collect();
    for ring in &rings {
        let n = ring.len().max(1) as f32;
        let (x, y) = ring.iter().fold((0.0, 0.0), |(x, y), p| (x + p[0] / n, y + p[1] / n));
        let (cx, cy) = cell([x, y]);
        let barrel = (-1..=1).any(|dx| (-1..=1).any(|dy| plated.contains(&(cx + dx, cy + dy))));
        let color = rgba(if barrel { FINISH } else { SUBSTRATE });
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let (normal, base) = ([b[1] - a[1], a[0] - b[0], 0.0], verts.len() as u32);
            verts.extend([(a, -h), (b, -h), (b, h), (a, h)].map(|(p, z)| Vert3 { pos: [p[0], p[1], z], normal, color }));
            indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
        }
    }
    (verts, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;

    #[test]
    fn body_has_faces_and_sides_and_layers_paint_its_faces() {
        let b = Board::parse(
            r#"(kicad_pcb (general (thickness 1)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (38 "B.Mask" user) (39 "F.Mask" user) (44 "Edge.Cuts" user))
              (setup (stackup (layer "F.Mask" (type "Top Solder Mask") (color "Black")) (layer "F.Cu" (type "copper")) (layer "B.Cu" (type "copper"))) (tenting back))
              (gr_rect (start 0 0) (end 10 10) (layer "Edge.Cuts") (width 0.05)))"#,
        )
        .unwrap();
        let (verts, indices) = body(&b, &build(&b), Colors::of(&b).mask);
        // Two faces of two triangles, and four sides.
        assert_eq!((verts.len(), indices.len()), (2 * 4 + 4 * 4, 2 * 6 + 4 * 6));
        assert_eq!(
            (verts[0].pos[2], verts[0].color, verts[4].pos[2], verts[4].color),
            (500.0, [22, 22, 24, 255], -500.0, [30, 110, 58, 255])
        );
        assert!(verts[8..].iter().all(|v| v.pos[0].abs() == 5000.0 || v.pos[1].abs() == 5000.0));
        // Per face: holes, copper under mask, openings (with the vias on the untented front), copper in them, silkscreen.
        let (steps, paints) = steps(&b, &Colors::of(&b), 520.0);
        let front: Vec<_> = steps[..5].iter().map(|s| (s.slots.clone(), s.stencil, s.reference)).collect();
        assert_eq!(front, [(vec![6], HOLE, 2), (vec![0, 5], KEEP, 0), (vec![3, 5], OPEN, 0), (vec![0, 5], KEEP, 1), (vec![], KEEP, 0)]);
        assert_eq!((steps[7].slots.clone(), paints[0].1, paints[5].1), (vec![2], 520.0, -520.0));
        assert_eq!(
            [named(Some("Matte Blue"), WHITE), named(Some("#102030FF"), WHITE), named(None, WHITE)],
            [[24, 60, 140], [16, 32, 48], WHITE]
        );
    }

    #[test]
    fn plated_holes_have_barrels_of_the_finish() {
        let b = Board::parse(
            r#"(kicad_pcb (general (thickness 1)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 10) (layer "Edge.Cuts") (width 0.05))
              (via (at 5 5) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu"))
              (footprint "H" (layer "F.Cu") (at 15 5) (pad "" np_thru_hole circle (at 0 0) (size 2 2) (drill 2) (layers "*.Mask"))))"#,
        )
        .unwrap();
        let (verts, _) = body(&b, &build(&b), Colors::of(&b).mask);
        // The sides near each hole: the via's in the finish, the unplated hole's and the edge's bare.
        let near = |x: f32| verts.iter().filter(move |v| v.normal[2] == 0.0 && (v.pos[0] - x).abs() < 1500.0).map(|v| v.color);
        let (scene, finish, bare) = (build(&b), rgba(FINISH), rgba(SUBSTRATE));
        let [via, hole] = [5, 15].map(|mm| scene.um(cuprita_core::board::Pt { x: mm * 1_000_000, y: 5_000_000 })[0]);
        assert!(near(via).all(|c| c == finish) && near(via).count() > 0);
        assert!(near(hole).all(|c| c == bare) && near(hole).count() > 0);
    }

    fn rgba([r, g, b]: [u8; 3]) -> [u8; 4] {
        [r, g, b, 255]
    }

    #[test]
    fn models_sit_on_their_footprints_as_kicad_places_them() {
        let b = Board::parse(
            r#"(kicad_pcb (general (thickness 1)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
              (footprint "A" (layer "F.Cu") (at 0 0 90) (model "a.step" (offset (xyz 0 2 0)) (rotate (xyz 0 0 90))))
              (footprint "B" (layer "B.Cu") (at 10 0) (model "b.step")))"#,
        )
        .unwrap();
        let scene = build(&b);
        let at = |i: usize, p: [f32; 3]| {
            let Place(m) = place(&b.footprints[i], &b.footprints[i].models[0], &scene, 500.0);
            [0, 1, 2].map(|r| (m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3]).round())
        };
        // A turns its model a quarter clockwise, the part a quarter back: its X still runs east,
        // and the offset north turned with the part points west. On the front face, 500 µm up.
        assert_eq!(at(0, [1.0, 0.0, 0.0]), [-1000.0, 0.0, 500.0]);
        // On the back, turned over about X: its north runs south (scene Y down), its top down.
        assert_eq!(at(1, [0.0, 1.0, 1.0]), [10_000.0, 1000.0, -1500.0]);
    }

    #[test]
    fn orbit_sees_the_board_from_where_it_says() {
        let o = Orbit::fit([-10_000.0, -5_000.0, 10_000.0, 5_000.0], [800.0, 600.0]);
        let cam = o.camera([800.0, 600.0], 30_000.0);
        // Clip position of a scene point, divided by w.
        let clip = |p: [f32; 3]| {
            let v: [f32; 4] = std::array::from_fn(|i| cam.mvp[0][i] * p[0] + cam.mvp[1][i] * p[1] + cam.mvp[2][i] * p[2] + cam.mvp[3][i]);
            [v[0] / v[3], v[1] / v[3], v[2] / v[3]]
        };
        // The target is in the middle; from the south, the board's east is right and its north up.
        let [x, y, z] = clip([0.0, 0.0, 0.0]);
        assert!(x.abs() < 1e-4 && y.abs() < 1e-4 && (0.0..1.0).contains(&z));
        assert!(clip([5_000.0, 0.0, 0.0])[0] > 0.0 && clip([0.0, -5_000.0, 0.0])[1] > 0.0);
        // The eye is south of the board (scene Y down) and above it; the whole board is in view.
        assert!(cam.eye[1] > 0.0 && cam.eye[2] > 0.0);
        let corners = [[-10_000.0, -5_000.0, 0.0], [10_000.0, -5_000.0, 0.0], [10_000.0, 5_000.0, 0.0], [-10_000.0, 5_000.0, 0.0]];
        assert!(corners.iter().all(|&p| clip(p)[..2].iter().all(|v| v.abs() <= 1.0)));
        // Snugly: a corner nearly touches the view's edge.
        assert!(corners.iter().any(|&p| clip(p)[..2].iter().any(|v| v.abs() > 0.9)));
        // Panning right slides the board right: its target moves left (west).
        let mut panned = o;
        panned.pan(100.0, 0.0, 600.0);
        assert!(panned.target[0] < 0.0 && panned.target[1].abs() < 1e-3);
    }
}
