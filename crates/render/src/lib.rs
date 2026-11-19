//! wgpu board renderer: SDF instances for tracks, pads and vias; triangles for zones and polygons.

mod scene;
mod three;
pub use scene::{
    ADDED_COLOR, ADDED_SLOT, BACKGROUND, Draw, HEAT_SLOT, HEAT_STEPS, Inst, Layer, REMOVED_COLOR, REMOVED_SLOT, Scene, Vert, build,
    build_diff, build_ir, build_sch, heat_color, rebuild,
};
pub use three::Orbit;

use cuprita_core::board::Board;
use cuprita_core::models::Models;
use wgpu::util::DeviceExt;

/// Camera uniform. Build it with `View::new`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct View {
    center: [f32; 2],
    scale: [f32; 2],
    px: f32,
    /// Highlighted net, 0 for none: everything else is dimmed and the net is drawn on top.
    pub net: u32,
    /// The active copper layer's slot + 1, 0 for none (`active`).
    active: u32,
    _pad: f32,
}

impl View {
    /// `center` in scene micrometres; `um_per_px` is the world size of one physical pixel.
    pub fn new(center: [f32; 2], um_per_px: f32, size_px: [f32; 2]) -> Self {
        let scale = [2.0 / (size_px[0] * um_per_px), -2.0 / (size_px[1] * um_per_px)];
        Self { center, scale, px: um_per_px, net: 0, active: 0, _pad: 0.0 }
    }

    /// Draws copper slot `slot` above the other copper at full brightness, the other copper
    /// quieter and inner layers (mostly planes) quieter still, but for the highlighted net.
    pub fn active(mut self, slot: Option<usize>) -> Self {
        self.active = slot.map_or(0, |s| s as u32 + 1);
        self
    }

    /// Mirrors the board horizontally so it is seen from the back.
    pub fn flip(mut self, flip: bool) -> Self {
        if flip {
            self.scale[0] = -self.scale[0];
        }
        self
    }

    pub fn flipped(&self) -> bool {
        self.scale[0] < 0.0
    }
}

/// Centre and micrometres-per-pixel that fit `bounds` in a viewport, with a small margin.
pub fn fit(bounds: [f32; 4], size_px: [f32; 2]) -> ([f32; 2], f32) {
    let [x0, y0, x1, y1] = bounds;
    let um_per_px = ((x1 - x0) / size_px[0]).max((y1 - y0) / size_px[1]) * 1.05;
    ([(x0 + x1) / 2.0, (y0 + y1) / 2.0], um_per_px.max(1e-3))
}

const MAX_SLOTS: usize = 256;

pub struct Renderer {
    /// Normal, then the highlighted-net-only (`TOP`) variant.
    sdf: [wgpu::RenderPipeline; 2],
    tri: [wgpu::RenderPipeline; 2],
    bind: wgpu::BindGroup,
    view: wgpu::Buffer,
    colors: wgpu::Buffer,
    /// Each slot's `scene::copper`, which the shader dims by.
    copper: wgpu::Buffer,
    shader: wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    srgb: bool,
    gpu: Option<Buffers>,
    /// The 3D view's pipelines and geometry, made on first use (`upload3d`).
    three: Option<three::Three>,
}

struct Buffers {
    insts: wgpu::Buffer,
    verts: wgpu::Buffer,
    indices: wgpu::Buffer,
    draws: Vec<Draw>,
    stackup: usize,
    /// Each slot's `scene::copper`, and the outer copper layers' slots (F.Cu, B.Cu).
    copper: Vec<u32>,
    outer: [Option<usize>; 2],
}

impl Renderer {
    /// Pipelines for a single-sample colour target of `format`, without depth; the 3D view
    /// renders into its own target of that format.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let uniform = |size: usize| {
            let usage = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
            device.create_buffer(&wgpu::BufferDescriptor { label: None, size: size as u64, usage, mapped_at_creation: false })
        };
        let (view, colors, copper) = (uniform(size_of::<View>()), uniform(MAX_SLOTS * 16), uniform(MAX_SLOTS * 4));
        let entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        // Binding 6: the 3D view's shaders, in the same module, take 2 to 5.
        let layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[entry(0), entry(1), entry(6)] });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: view.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: colors.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: copper.as_entire_binding() },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |vs: &str, fs: &str, topology, buffer: &wgpu::VertexBufferLayout, top: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("TOP", if top { 1.0 } else { 0.0 })],
                        ..Default::default()
                    },
                    buffers: &[Some(buffer.clone())],
                },
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
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
        let sdf = [false, true].map(|top| pipeline("vs_sdf", "fs_sdf", wgpu::PrimitiveTopology::TriangleStrip, &inst, top));
        let tri = [false, true].map(|top| pipeline("vs_tri", "fs_tri", wgpu::PrimitiveTopology::TriangleList, &vert, top));
        Self { sdf, tri, bind, view, colors, copper, shader, format, srgb: format.is_srgb(), gpu: None, three: None }
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene) {
        let buf = |data: &[u8], usage| {
            // Empty buffers can't be created; draws skip empty ranges anyway.
            let contents = if data.is_empty() { &[0u8; 16][..] } else { data };
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents, usage })
        };
        let mut raw = [[0.0; 4]; MAX_SLOTS];
        for (dst, l) in raw.iter_mut().zip(&scene.layers) {
            *dst = l.color;
        }
        for i in 0..HEAT_STEPS {
            raw[(HEAT_SLOT + i) as usize] = heat_color(i as f32 / (HEAT_STEPS - 1) as f32);
        }
        raw[ADDED_SLOT as usize] = ADDED_COLOR;
        raw[REMOVED_SLOT as usize] = REMOVED_COLOR;
        let lin = srgb_to_linear;
        let colors = raw.map(|[r, g, b, a]| if self.srgb { [lin(r), lin(g), lin(b), a] } else { [r, g, b, a] });
        queue.write_buffer(&self.colors, 0, bytemuck::bytes_of(&colors));
        let copper: Vec<u32> = scene.layers.iter().map(|l| scene::copper(&l.name)).collect();
        let mut raw = [0u32; MAX_SLOTS];
        raw.iter_mut().zip(&copper).for_each(|(dst, &c)| *dst = c);
        queue.write_buffer(&self.copper, 0, bytemuck::bytes_of(&raw));
        let outer = ["F.Cu", "B.Cu"].map(|n| scene.layers.iter().position(|l| l.name == n));
        self.gpu = Some(Buffers {
            insts: buf(bytemuck::cast_slice(&scene.insts), wgpu::BufferUsages::VERTEX),
            verts: buf(bytemuck::cast_slice(&scene.verts), wgpu::BufferUsages::VERTEX),
            indices: buf(bytemuck::cast_slice(&scene.indices), wgpu::BufferUsages::INDEX),
            draws: scene.draws.clone(),
            stackup: scene.stackup,
            copper,
            outer,
        });
    }

    pub fn set_view(&self, queue: &wgpu::Queue, view: &View) {
        queue.write_buffer(&self.view, 0, bytemuck::bytes_of(view));
    }

    /// Draws the slots whose `visible` entry is true (or missing) as `view` (the one `set_view`
    /// gave) has them: flipped or not, its active copper on top; with its net highlighted, draws
    /// that net again on top so it shows through the layers above it.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, visible: &[bool], view: &View) {
        let Some(g) = &self.gpu else { return };
        let (stride, active) = (size_of::<Inst>() as u64, view.active.checked_sub(1).map(|s| s as usize));
        pass.set_bind_group(0, &self.bind, &[]);
        for (sdf, tri) in self.sdf.iter().zip(&self.tri).take(1 + (view.net != 0) as usize) {
            let order = scene::draw_order(&g.draws, g.stackup, view.flipped(), active, |s| g.copper[s] != 0);
            for d in order.filter(|d| visible.get(d.slot) != Some(&false)) {
                for tris in [&d.fills, &d.indices].into_iter().filter(|r| !r.is_empty()) {
                    pass.set_pipeline(tri);
                    pass.set_vertex_buffer(0, g.verts.slice(..));
                    pass.set_index_buffer(g.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(tris.clone(), 0, 0..1);
                }
                if !d.insts.is_empty() {
                    // Offset the buffer instead of using first_instance, which WebGL lacks.
                    pass.set_pipeline(sdf);
                    pass.set_vertex_buffer(0, g.insts.slice(d.insts.start as u64 * stride..d.insts.end as u64 * stride));
                    pass.draw(0..4, 0..d.insts.len() as u32);
                }
            }
        }
    }
}

/// Renders `bounds` of `scene` without a window, returning `(width, height, rgba_bytes)`.
pub fn render_rgba(scene: &Scene, bounds: [f32; 4], width: u32, visible: &[bool], net: u32, flip: bool) -> (u32, u32, Vec<u8>) {
    pollster::block_on(async {
        let (device, queue) = headless().await;
        let mut r = Renderer::new(&device, HEADLESS);
        r.upload(&device, &queue, scene);
        r.shot(&device, &queue, bounds, width, visible, net, flip)
    })
}

impl Renderer {
    /// Draws `bounds` of the scene uploaded, `width` pixels wide, into a texture of its own and
    /// reads it back: `(width, height, rgba_bytes)`, the outer copper of the side in view active.
    /// A window's renderer can, for thumbnails: the next frame's `set_view` puts its view back.
    #[expect(clippy::too_many_arguments, reason = "render_rgba's, with the GPU's")]
    pub fn shot(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: [f32; 4],
        width: u32,
        visible: &[bool],
        net: u32,
        flip: bool,
    ) -> (u32, u32, Vec<u8>) {
        let [x0, y0, x1, y1] = bounds;
        let w = width.clamp(1, 8192);
        let h = (w as f32 * (y1 - y0).max(1.0) / (x1 - x0).max(1.0)).ceil().clamp(1.0, 8192.0) as u32;
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let (center, um_per_px) = fit(bounds, [w as f32, h as f32]);
        let side = self.gpu.as_ref().and_then(|g| g.outer[flip as usize]);
        let mut v = View::new(center, um_per_px, [w as f32, h as f32]).flip(flip).active(side);
        v.net = net;
        self.set_view(queue, &v);
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let view = tex.create_view(&Default::default());
            let [r_, g, b, _] = BACKGROUND.map(|c| f64::from(if self.srgb { srgb_to_linear(c) } else { c }));
            let ops = wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: r_, g, b, a: 1.0 }), store: wgpu::StoreOp::Store };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw(&mut pass, visible, &v);
        }
        let mut pixels = read_back(device, queue, enc, &tex);
        if matches!(self.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            pixels.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.swap(0, 2));
        }
        (w, h, pixels)
    }
}

/// Renders the 3D view of `board` (its 2D scene `scene`, its footprints' 3D `models`) from
/// `orbit` without a window: `size` pixels of RGBA.
pub fn render_rgba3d(scene: &Scene, board: &Board, models: &Models, orbit: &Orbit, size: [u32; 2]) -> Vec<u8> {
    pollster::block_on(async {
        let (device, queue) = headless().await;
        let mut r = Renderer::new(&device, HEADLESS);
        r.upload(&device, &queue, scene);
        r.upload3d(&device, &queue, board, scene, models);
        let mut enc = device.create_command_encoder(&Default::default());
        r.render3d(&device, &queue, &mut enc, size, orbit);
        read_back(&device, &queue, enc, r.shot3d().expect("rendered above"))
    })
}

const HEADLESS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// An sRGB channel in linear light, as an sRGB target wants its colours.
fn srgb_to_linear(u: f32) -> f32 {
    if u <= 0.04045 { u / 12.92 } else { ((u + 0.055) / 1.055).powf(2.4) }
}

/// A GPU without a window.
async fn headless() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let opts = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
    let adapter = instance.request_adapter(&opts).await.expect("no GPU adapter");
    adapter.request_device(&wgpu::DeviceDescriptor::default()).await.expect("no GPU device")
}

/// Runs `enc`, then reads `tex` (RGBA, 4 bytes a pixel) back, row after row.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, mut enc: wgpu::CommandEncoder, tex: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (tex.width(), tex.height());
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let usage = wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ;
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage, mapped_at_creation: false });
    let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None };
    enc.copy_texture_to_buffer(tex.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &readback, layout }, tex.size());
    queue.submit([enc.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map readback"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let data = readback.slice(..).get_mapped_range().expect("mapped readback");
    (0..h as usize).flat_map(|y| &data[y * row as usize..][..w as usize * 4]).copied().collect()
}

#[cfg(test)]
mod tests {
    use wgpu::naga;

    #[test]
    fn uniforms_match_rust_and_fit_webgl() {
        let module = naga::front::wgsl::parse_str(include_str!("shader.wgsl")).unwrap();
        let mut layout = naga::proc::Layouter::default();
        layout.update(module.to_ctx()).unwrap();
        let size = |name| {
            let (_, g) = module.global_variables.iter().find(|(_, g)| g.name.as_deref() == Some(name)).unwrap();
            layout[g.ty].size as usize
        };
        // WebGL binds uniform buffers only in whole 16-byte blocks.
        assert_eq!((size("view"), size("view") % 16), (size_of::<super::View>(), 0));
        assert_eq!((size("colors"), size("colors") % 16), (super::MAX_SLOTS * 16, 0));
        assert_eq!((size("copper"), size("copper") % 16), (super::MAX_SLOTS * 4, 0));
        assert_eq!((size("cam"), size("cam") % 16), (size_of::<super::three::Cam>(), 0));
        assert_eq!((size("paint"), size("paint") % 16), (size_of::<super::three::Paint>(), 0));
    }
}
