//! The 3D viewport renderer.
//!
//! Draws into its own off-screen texture (with 4x anti-aliasing and a depth
//! buffer), which egui then shows like an image. That keeps our 3D drawing
//! separate from egui's own drawing.

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use egui_wgpu::wgpu::util::DeviceExt;
use glam::{Mat4, Vec3, Vec4};
use ts_core::Scene;

use crate::camera::OrbitCamera;

fn msaa() -> u32 {
    std::env::var("TS_MSAA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4)
}
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.035,
    g: 0.038,
    b: 0.045,
    a: 1.0,
};

/// Body colour, garment colours (in order), reference body colour.
const BODY_COLOR: [f32; 3] = [0.58, 0.63, 0.72];
const GARMENT_COLORS: [[f32; 3]; 4] = [
    [0.82, 0.45, 0.32],
    [0.38, 0.66, 0.58],
    [0.86, 0.72, 0.38],
    [0.66, 0.50, 0.78],
];
const REFERENCE_COLOR: [f32; 3] = [0.45, 0.75, 0.45];

/// Per-part material settings for the shader (group 1, binding 2).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PartLook {
    /// hue turn (0..1), saturation, brightness, 1 if textured
    adjust: [f32; 4],
    /// roughness, metallic, unused, unused
    surface: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    light_dir: [f32; 4],
    wire_color: [f32; 4],
    shadow: [f32; 4],
    outline: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct MeshVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

impl LineVertex {
    pub fn new(p: Vec3, color: Vec4) -> Self {
        Self {
            pos: p.to_array(),
            color: color.to_array(),
        }
    }
}

/// What to draw this frame.
pub struct DrawSettings {
    pub wireframe: bool,
    pub shaded: bool,
    pub grid: bool,
    /// Lines drawn on top of everything (skeleton).
    pub overlay_lines: Vec<LineVertex>,
    /// Scene part to draw an outline around (the selected garment).
    pub outline_part: Option<usize>,
    /// Soft shadow on the floor: centre (floor height in y) and radius.
    pub shadow: Option<(Vec3, f32)>,
    /// A plain background colour instead of the usual gradient (for renders
    /// that go to the AI).
    pub plain_background: Option<[f64; 3]>,
}

struct GpuPart {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    edge_buffer: wgpu::Buffer,
    edge_count: u32,
    color: [f32; 3],
    visible: bool,
    /// Texture + material settings binding.
    material: wgpu::BindGroup,
    look_buffer: wgpu::Buffer,
    /// Colour from the part's role (used when it has no material colour).
    role_color: [f32; 3],
}

struct Targets {
    size: [u32; 2],
    msaa_color: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::TextureView,
    resolved_texture: wgpu::Texture,
}

pub struct Viewport {
    mesh_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    background_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    /// Uploaded textures by content hash, kept across scene uploads.
    texture_cache: std::collections::HashMap<u64, wgpu::TextureView>,
    wire_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    grid: Vec<LineVertex>,
    parts: Vec<GpuPart>,
    /// The live view of a model being made (drawn after the scene).
    preview: Option<GpuPart>,
    targets: Option<Targets>,
    texture_id: Option<egui::TextureId>,
}

impl Viewport {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let mesh_layout_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh layout"),
            bind_group_layouts: &[Some(&bind_layout), Some(&material_layout)],
            immediate_size: 0,
        });

        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x2];
        let mesh_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &mesh_attrs,
        };
        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &line_attrs,
        };

        let make_with = |pl: &wgpu::PipelineLayout,
                    cull: Option<wgpu::Face>,
                    label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    topology: wgpu::PrimitiveTopology,
                    depth_write: bool,
                    depth_compare: wgpu::CompareFunction,
                    blend: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: msaa(),
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    topology: wgpu::PrimitiveTopology,
                    depth_write: bool,
                    depth_compare: wgpu::CompareFunction,
                    blend: bool| {
            let pl = if buffers.first().and_then(|b| b.as_ref()).is_some_and(|b| b.attributes.len() == 4) {
                &mesh_layout_pl
            } else {
                &layout
            };
            make_with(pl, None, label, vs, fs, buffers, topology, depth_write, depth_compare, blend)
        };
        use wgpu::CompareFunction as C;
        use wgpu::PrimitiveTopology as T;
        let outline_pipeline = make_with(
            &mesh_layout_pl,
            Some(wgpu::Face::Front),
            "outline",
            "vs_outline",
            "fs_flat",
            &[Some(mesh_layout.clone())],
            T::TriangleList,
            true,
            C::Less,
            false,
        );
        let background_pipeline = make("background", "vs_bg", "fs_bg", &[], T::TriangleList, false, C::Always, false);
        let shadow_pipeline = make("shadow", "vs_shadow", "fs_shadow", &[], T::TriangleList, false, C::LessEqual, true);
        let mesh_pipeline = make(
            "mesh",
            "vs_mesh",
            "fs_mesh",
            &[Some(mesh_layout.clone())],
            T::TriangleList,
            true,
            C::Less,
            false,
        );
        let wire_pipeline = make(
            "wire",
            "vs_wire",
            "fs_wire",
            &[Some(mesh_layout)],
            T::LineList,
            false,
            C::LessEqual,
            true,
        );
        let line_pipeline = make(
            "lines",
            "vs_line",
            "fs_line",
            &[Some(line_layout.clone())],
            T::LineList,
            true,
            C::Less,
            true,
        );
        let overlay_pipeline = make(
            "overlay",
            "vs_line",
            "fs_line",
            &[Some(line_layout)],
            T::LineList,
            false,
            C::Always,
            true,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("texture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let white = make_texture(device, queue, &[[255, 255, 255, 255]], 1, 1);
        Self {
            mesh_pipeline,
            outline_pipeline,
            background_pipeline,
            shadow_pipeline,
            material_layout,
            sampler,
            white,
            texture_cache: Default::default(),
            wire_pipeline,
            line_pipeline,
            overlay_pipeline,
            globals_buffer,
            bind_group,
            grid: grid_lines(10.0, 0.5),
            parts: Vec::new(),
            preview: None,
            targets: None,
            texture_id: None,
        }
    }

    /// Upload a newly loaded scene (in its bind pose).
    pub fn set_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        joint_world: &[Mat4],
    ) {
        self.parts.clear();
        let mut garment_count = 0;
        for part in scene.parts.iter() {
            let role_color = match part.role {
                ts_core::PartRole::Body | ts_core::PartRole::Head => BODY_COLOR,
                ts_core::PartRole::Reference => REFERENCE_COLOR,
                ts_core::PartRole::Garment => {
                    garment_count += 1;
                    GARMENT_COLORS[(garment_count - 1) % GARMENT_COLORS.len()]
                }
            };
            let (color, look) = part_look(part, role_color);
            let view = self.part_texture(device, queue, part);
            let look_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("part look"),
                contents: bytemuck::bytes_of(&look),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let material = self.material_bind_group(device, &view, &look_buffer);
            let vertices = build_vertices(&part.mesh, joint_world, color);
            let edges = part.mesh.edges();
            self.parts.push(GpuPart {
                vertex_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("part vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                }),
                index_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("part indices"),
                    contents: bytemuck::cast_slice(&part.mesh.indices),
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                }),
                index_count: part.mesh.indices.len() as u32,
                edge_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("part edges"),
                    contents: bytemuck::cast_slice(&edges),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                edge_count: edges.len() as u32,
                color,
                visible: part.visible,
                material,
                look_buffer,
                role_color,
            });
        }
    }

    /// Show (or remove) the live view of a model being made.
    pub fn set_preview(&mut self, device: &wgpu::Device, geometry: Option<&crate::live_view::PreviewGeometry>) {
        self.preview = None;
        let Some(g) = geometry.filter(|g| !g.indices.is_empty() && !g.positions.is_empty()) else {
            return;
        };
        let vertices: Vec<MeshVertex> = g
            .positions
            .iter()
            .zip(&g.normals)
            .map(|(p, n)| MeshVertex { pos: p.to_array(), normal: n.to_array(), color: g.color, uv: [0.0, 0.0] })
            .collect();
        let mut edges = Vec::with_capacity(g.indices.len() * 2);
        for t in g.indices.chunks_exact(3) {
            edges.extend_from_slice(&[t[0], t[1], t[1], t[2], t[2], t[0]]);
        }
        let look = PartLook { adjust: [0.0, 1.0, 1.0, 0.0], surface: [0.75, 0.0, 0.0, 0.0] };
        let look_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("preview look"),
            contents: bytemuck::bytes_of(&look),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let material = self.material_bind_group(device, &self.white.clone(), &look_buffer);
        self.preview = Some(GpuPart {
            vertex_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("preview vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            index_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("preview indices"),
                contents: bytemuck::cast_slice(&g.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            index_count: g.indices.len() as u32,
            edge_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("preview edges"),
                contents: bytemuck::cast_slice(&edges),
                usage: wgpu::BufferUsages::INDEX,
            }),
            edge_count: edges.len() as u32,
            color: g.color,
            visible: true,
            material,
            look_buffer,
            role_color: g.color,
        });
    }

    /// The part's base colour texture (white when it has none).
    fn part_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, part: &ts_core::MeshPart) -> wgpu::TextureView {
        match part_texture_of(part) {
            Some(t) => self.texture_view(device, queue, t),
            None => self.white.clone(),
        }
    }

    fn material_bind_group(&self, device: &wgpu::Device, view: &wgpu::TextureView, look: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("part material"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: look.as_entire_binding() },
            ],
        })
    }

    /// The part's material changed (colour, sliders or texture): update it.
    pub fn refresh_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        index: usize,
        joint_world: &[Mat4],
    ) {
        let Some(part) = scene.parts.get(index) else { return };
        let Some(role_color) = self.parts.get(index).map(|g| g.role_color) else { return };
        let (color, look) = part_look(part, role_color);
        let view = self.part_texture(device, queue, part);
        let gpu = &self.parts[index];
        queue.write_buffer(&gpu.look_buffer, 0, bytemuck::bytes_of(&look));
        let material = self.material_bind_group(device, &view, &self.parts[index].look_buffer);
        let gpu = &mut self.parts[index];
        gpu.material = material;
        gpu.color = color;
        let vertices = build_vertices(&part.mesh, joint_world, color);
        queue.write_buffer(&gpu.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }

    /// The GPU texture for a texture (uploaded once, then reused).
    fn texture_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &ts_core::Texture,
    ) -> wgpu::TextureView {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        texture.bytes.hash(&mut h);
        let key = h.finish();
        if let Some(b) = self.texture_cache.get(&key) {
            return b.clone();
        }
        let binding = match image::load_from_memory(&texture.bytes) {
            Ok(img) => {
                let mut img = img.to_rgba8();
                let max = device.limits().max_texture_dimension_2d.min(4096);
                if img.width() > max || img.height() > max {
                    img = image::imageops::resize(&img, img.width().min(max), img.height().min(max), image::imageops::FilterType::Triangle);
                }
                let (w, h) = img.dimensions();
                let pixels: Vec<[u8; 4]> = img.pixels().map(|p| p.0).collect();
                make_texture(device, queue, &pixels, w, h)
            }
            Err(e) => {
                log::warn!("Couldn't read texture {}: {e}", texture.name);
                self.white.clone()
            }
        };
        self.texture_cache.insert(key, binding.clone());
        binding
    }

    /// Re-skin all parts for a new pose and update visibility.
    pub fn update_pose(&mut self, queue: &wgpu::Queue, scene: &Scene, joint_world: &[Mat4]) {
        for (gpu, part) in self.parts.iter_mut().zip(&scene.parts) {
            gpu.visible = part.visible;
            if part.mesh.skin.is_some() {
                let vertices = build_vertices(&part.mesh, joint_world, gpu.color);
                queue.write_buffer(&gpu.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            }
        }
    }

    /// Re-upload one part whose vertices moved (same vertex count), e.g.
    /// while sculpting or moving a garment.
    pub fn update_part(
        &mut self,
        queue: &wgpu::Queue,
        scene: &Scene,
        index: usize,
        joint_world: &[Mat4],
    ) {
        if let (Some(gpu), Some(part)) = (self.parts.get_mut(index), scene.parts.get(index)) {
            let vertices = build_vertices(&part.mesh, joint_world, gpu.color);
            queue.write_buffer(&gpu.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            gpu.visible = part.visible;
        }
    }

    /// Hide some triangles of a part (e.g. body skin under the clothes);
    /// `None` shows them all again.
    pub fn set_hidden_triangles(&mut self, queue: &wgpu::Queue, scene: &Scene, index: usize, hidden: Option<&[bool]>) {
        let (Some(gpu), Some(part)) = (self.parts.get(index), scene.parts.get(index)) else { return };
        let mut indices = part.mesh.indices.clone();
        if indices.len() as u32 != gpu.index_count {
            return;
        }
        if let Some(hidden) = hidden {
            for (t, tri) in indices.chunks_exact_mut(3).enumerate() {
                if hidden.get(t).copied().unwrap_or(false) {
                    // A zero-area triangle draws nothing.
                    tri[1] = tri[0];
                    tri[2] = tri[0];
                }
            }
        }
        queue.write_buffer(&gpu.index_buffer, 0, bytemuck::cast_slice(&indices));
    }

    pub fn set_visibility(&mut self, scene: &Scene) {
        for (gpu, part) in self.parts.iter_mut().zip(&scene.parts) {
            gpu.visible = part.visible;
        }
    }

    pub fn clear_scene(&mut self) {
        self.parts.clear();
    }

    /// Copy the last rendered frame back from the GPU as RGBA8 pixels
    /// (used for outfit thumbnails).
    pub fn read_pixels(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.targets.as_ref()?;
        let [w, h] = t.size;
        let row = w * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.resolved_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = buffer.slice(..).get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((row * h) as usize);
        for y in 0..h as usize {
            let start = y * padded as usize;
            out.extend_from_slice(&data[start..start + row as usize]);
        }
        drop(data);
        buffer.unmap();
        Some((w, h, out))
    }

    /// Draw a frame and return the egui texture showing it.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut egui_wgpu::Renderer,
        size: [u32; 2],
        camera: &OrbitCamera,
        settings: &DrawSettings,
    ) -> egui::TextureId {
        let size = [size[0].clamp(1, 8192), size[1].clamp(1, 8192)];
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            let targets = Targets::new(device, size);
            match self.texture_id {
                Some(id) => renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &targets.resolved,
                    wgpu::FilterMode::Linear,
                    id,
                ),
                None => {
                    self.texture_id = Some(renderer.register_native_texture(
                        device,
                        &targets.resolved,
                        wgpu::FilterMode::Linear,
                    ))
                }
            }
            self.targets = Some(targets);
        }
        let targets = self.targets.as_ref().unwrap();

        let aspect = size[0] as f32 / size[1] as f32;
        let eye = camera.eye();
        // Key light comes from over the camera's right shoulder.
        let view = camera.view();
        let light =
            (view.row(0).truncate() * 0.5 + Vec3::Y * 0.8 + (eye - camera.target).normalize())
                .normalize();
        let globals = Globals {
            view_proj: camera.view_proj(aspect).to_cols_array_2d(),
            camera_pos: eye.extend(1.0).to_array(),
            light_dir: light.extend(0.0).to_array(),
            wire_color: [0.05, 0.75, 1.0, if settings.shaded { 0.55 } else { 0.9 }],
            shadow: match settings.shadow {
                Some((c, r)) => [c.x, c.y, c.z, r],
                None => [0.0; 4],
            },
            outline: [1.0, 0.62, 0.12, 0.0016],
        };
        queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        // Lines are cut where they pass behind the camera. Some GPUs (and
        // software renderers) draw junk for lines that cross the camera.
        let forward = (camera.target - eye).normalize();
        let near = camera.distance * 0.02;
        let line_buffer = |label: &str, lines: &[LineVertex]| {
            let clipped = clip_lines(lines, eye, forward, near);
            (!clipped.is_empty()).then(|| {
                let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(&clipped),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                (buf, clipped.len() as u32)
            })
        };
        let grid_buffer = if settings.grid {
            line_buffer("grid", &self.grid)
        } else {
            None
        };
        let overlay_buffer = line_buffer("overlay lines", &settings.overlay_lines);

        let clear = match settings.plain_background {
            Some([r, g, b]) => wgpu::Color { r, g, b, a: 1.0 },
            None => BACKGROUND,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport pass"),
                color_attachments: &[Some(if msaa() > 1 {
                    wgpu::RenderPassColorAttachment {
                        view: &targets.msaa_color,
                        depth_slice: None,
                        resolve_target: Some(&targets.resolved),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Discard,
                        },
                    }
                } else {
                    wgpu::RenderPassColorAttachment {
                        view: &targets.resolved,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: wgpu::StoreOp::Store,
                        },
                    }
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);

            if settings.plain_background.is_none() {
                pass.set_pipeline(&self.background_pipeline);
                pass.draw(0..3, 0..1);
            }
            if settings.shadow.is_some() && settings.shaded {
                pass.set_pipeline(&self.shadow_pipeline);
                pass.draw(0..6, 0..1);
            }

            if settings.shaded {
                pass.set_pipeline(&self.mesh_pipeline);
                for p in self.parts.iter().chain(self.preview.as_ref()).filter(|p| p.visible) {
                    pass.set_bind_group(1, &p.material, &[]);
                    pass.set_vertex_buffer(0, p.vertex_buffer.slice(..));
                    pass.set_index_buffer(p.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..p.index_count, 0, 0..1);
                }
                if let Some(p) = settings.outline_part.and_then(|i| self.parts.get(i)).filter(|p| p.visible) {
                    pass.set_pipeline(&self.outline_pipeline);
                    pass.set_bind_group(1, &p.material, &[]);
                    pass.set_vertex_buffer(0, p.vertex_buffer.slice(..));
                    pass.set_index_buffer(p.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..p.index_count, 0, 0..1);
                }
            }
            if let Some((buf, count)) = &grid_buffer {
                pass.set_pipeline(&self.line_pipeline);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..*count, 0..1);
            }
            if settings.wireframe {
                pass.set_pipeline(&self.wire_pipeline);
                for p in self.parts.iter().chain(self.preview.as_ref()).filter(|p| p.visible) {
                    pass.set_bind_group(1, &p.material, &[]);
                    pass.set_vertex_buffer(0, p.vertex_buffer.slice(..));
                    pass.set_index_buffer(p.edge_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..p.edge_count, 0, 0..1);
                }
            }
            if let Some((buf, count)) = &overlay_buffer {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..*count, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
        self.texture_id.unwrap()
    }
}

impl Targets {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let make = |label: &str, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let resolved_texture = make(
            "viewport image",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        Self {
            size,
            msaa_color: make(
                "msaa color",
                COLOR_FORMAT,
                msaa(),
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .create_view(&Default::default()),
            depth: make(
                "depth",
                DEPTH_FORMAT,
                msaa(),
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .create_view(&Default::default()),
            resolved: resolved_texture.create_view(&Default::default()),
            resolved_texture,
        }
    }
}

fn build_vertices(mesh: &ts_core::Mesh, joint_world: &[Mat4], color: [f32; 3]) -> Vec<MeshVertex> {
    let (pos, nrm) = mesh.skinned(joint_world);
    let has_uv = mesh.uvs.len() == pos.len();
    pos.iter()
        .zip(nrm.iter().chain(std::iter::repeat(&Vec3::Y)))
        .enumerate()
        .map(|(i, (p, n))| MeshVertex {
            pos: p.to_array(),
            normal: n.to_array(),
            color,
            uv: if has_uv { mesh.uvs[i].to_array() } else { [0.0, 0.0] },
        })
        .collect()
}

/// Keep only the parts of each line that are in front of the camera.
fn clip_lines(lines: &[LineVertex], eye: Vec3, forward: Vec3, near: f32) -> Vec<LineVertex> {
    let mut out = Vec::with_capacity(lines.len());
    for seg in lines.chunks_exact(2) {
        let a = Vec3::from(seg[0].pos);
        let b = Vec3::from(seg[1].pos);
        let da = (a - eye).dot(forward) - near;
        let db = (b - eye).dot(forward) - near;
        if da < 0.0 && db < 0.0 {
            continue; // fully behind
        }
        let mut s = [seg[0], seg[1]];
        if da < 0.0 || db < 0.0 {
            let t = da / (da - db);
            let cut = a.lerp(b, t).to_array();
            if da < 0.0 {
                s[0].pos = cut
            } else {
                s[1].pos = cut
            }
        }
        out.extend_from_slice(&s);
    }
    out
}

/// Floor grid on the ground plane, with red X and blue Z axis lines.
/// Long lines are split into short pieces: very long lines that reach far
/// past the camera can draw incorrectly on some GPUs.
fn grid_lines(half_size: f32, step: f32) -> Vec<LineVertex> {
    let mut v = Vec::new();
    let steps = (half_size / step).round() as i32;
    let mut push_split = |a: Vec3, b: Vec3, c: Vec4| {
        let pieces = ((b - a).length() / step).ceil().max(1.0) as i32;
        for k in 0..pieces {
            v.push(LineVertex::new(a.lerp(b, k as f32 / pieces as f32), c));
            v.push(LineVertex::new(
                a.lerp(b, (k + 1) as f32 / pieces as f32),
                c,
            ));
        }
    };
    for i in -steps..=steps {
        let t = i as f32 * step;
        let major = i % 2 == 0;
        let c = if major {
            Vec4::new(1.0, 1.0, 1.0, 0.10)
        } else {
            Vec4::new(1.0, 1.0, 1.0, 0.05)
        };
        let (cx, cz) = if i == 0 {
            (
                Vec4::new(0.85, 0.25, 0.25, 0.8),
                Vec4::new(0.25, 0.45, 0.9, 0.8),
            )
        } else {
            (c, c)
        };
        // Line along X (at z = t) and along Z (at x = t).
        push_split(
            Vec3::new(-half_size, 0.0, t),
            Vec3::new(half_size, 0.0, t),
            cx,
        );
        push_split(
            Vec3::new(t, 0.0, -half_size),
            Vec3::new(t, 0.0, half_size),
            cz,
        );
    }
    v
}

/// Upload an RGBA image with a full mipmap chain.
fn make_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixels: &[[u8; 4]],
    width: u32,
    height: u32,
) -> wgpu::TextureView {
    // Mip levels, each half the size of the one before.
    let mut levels: Vec<(u32, u32, Vec<u8>)> = vec![(width, height, pixels.iter().flatten().copied().collect())];
    while levels.last().is_some_and(|l| l.0 > 1 || l.1 > 1) {
        let (pw, ph, prev) = levels.last().unwrap();
        let img = image::RgbaImage::from_raw(*pw, *ph, prev.clone()).unwrap();
        let (nw, nh) = ((*pw / 2).max(1), (*ph / 2).max(1));
        let next = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
        levels.push((nw, nh, next.into_raw()));
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("part texture"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, (w, h, data)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
            wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
        );
    }
    texture.create_view(&Default::default())
}

/// The base colour texture the viewport uses (needs UVs).
fn part_texture_of(part: &ts_core::MeshPart) -> Option<&ts_core::Texture> {
    part.material
        .as_ref()
        .and_then(|m| m.base_color_texture.as_ref())
        .filter(|_| part.mesh.uvs.len() == part.mesh.positions.len())
}

/// Vertex colour and shader settings for a part.
fn part_look(part: &ts_core::MeshPart, role_color: [f32; 3]) -> ([f32; 3], PartLook) {
    let texture = part_texture_of(part);
    let color = match (&part.material, texture) {
        (Some(m), Some(_)) => [m.base_color[0], m.base_color[1], m.base_color[2]],
        // A plain colour from the file, unless it's just "white".
        (Some(m), None) if m.base_color[..3].iter().any(|c| *c < 0.98) || m.has_color_change() => {
            [m.base_color[0], m.base_color[1], m.base_color[2]]
        }
        _ => role_color,
    };
    let look = match &part.material {
        Some(m) => PartLook {
            adjust: [m.hue / 360.0, m.saturation, m.brightness, if texture.is_some() { 1.0 } else { 0.0 }],
            surface: [m.roughness, m.metallic, 0.0, 0.0],
        },
        None => PartLook { adjust: [0.0, 1.0, 1.0, 0.0], surface: [0.8, 0.0, 0.0, 0.0] },
    };
    (color, look)
}
