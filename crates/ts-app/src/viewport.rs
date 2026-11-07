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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    light_dir: [f32; 4],
    wire_color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct MeshVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
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
}

struct GpuPart {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    edge_buffer: wgpu::Buffer,
    edge_count: u32,
    color: [f32; 3],
    visible: bool,
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
    wire_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    grid: Vec<LineVertex>,
    parts: Vec<GpuPart>,
    targets: Option<Targets>,
    texture_id: Option<egui::TextureId>,
}

impl Viewport {
    pub fn new(device: &wgpu::Device) -> Self {
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

        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
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

        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    topology: wgpu::PrimitiveTopology,
                    depth_write: bool,
                    depth_compare: wgpu::CompareFunction,
                    blend: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
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
        use wgpu::CompareFunction as C;
        use wgpu::PrimitiveTopology as T;
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

        Self {
            mesh_pipeline,
            wire_pipeline,
            line_pipeline,
            overlay_pipeline,
            globals_buffer,
            bind_group,
            grid: grid_lines(10.0, 0.5),
            parts: Vec::new(),
            targets: None,
            texture_id: None,
        }
    }

    /// Upload a newly loaded scene (in its bind pose).
    pub fn set_scene(&mut self, device: &wgpu::Device, scene: &Scene, joint_world: &[Mat4]) {
        self.parts.clear();
        let mut garment_count = 0;
        for part in scene.parts.iter() {
            let color = match part.role {
                ts_core::PartRole::Body | ts_core::PartRole::Head => BODY_COLOR,
                ts_core::PartRole::Reference => REFERENCE_COLOR,
                ts_core::PartRole::Garment => {
                    garment_count += 1;
                    GARMENT_COLORS[(garment_count - 1) % GARMENT_COLORS.len()]
                }
            };
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
                    usage: wgpu::BufferUsages::INDEX,
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
            });
        }
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
                            load: wgpu::LoadOp::Clear(BACKGROUND),
                            store: wgpu::StoreOp::Discard,
                        },
                    }
                } else {
                    wgpu::RenderPassColorAttachment {
                        view: &targets.resolved,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(BACKGROUND),
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

            if settings.shaded {
                pass.set_pipeline(&self.mesh_pipeline);
                for p in self.parts.iter().filter(|p| p.visible) {
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
                for p in self.parts.iter().filter(|p| p.visible) {
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
    pos.iter()
        .zip(nrm.iter().chain(std::iter::repeat(&Vec3::Y)))
        .map(|(p, n)| MeshVertex {
            pos: p.to_array(),
            normal: n.to_array(),
            color,
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
