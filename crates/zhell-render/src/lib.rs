mod atlas;
mod boxdraw;
mod fontscan;
mod layers;

pub use atlas::{AtlasGlyph, CellMetrics, FontConfig, GlyphStyle, preload_fonts};
pub use layers::{BUILTIN_SHADERS, Background, Fit, validate as validate_shader};

pub fn is_builtin_glyph(ch: char) -> bool {
    boxdraw::is_builtin(ch)
}
use atlas::{ATLAS_SIZE, GlyphAtlas};

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no suitable GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("surface: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameOutcome {
    Presented,

    Skipped,

    Rebuild,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    kind: u32,
    uv_size: [f32; 2],
}

const IMAGE_ATLAS: u32 = 4096;

#[derive(Default)]
struct ImageCache {
    placed: std::collections::HashMap<u64, (u32, u32)>,
    uploads: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
}

impl ImageCache {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn place(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > IMAGE_ATLAS || h > IMAGE_ATLAS {
            return None;
        }
        if self.shelf_x + w > IMAGE_ATLAS {
            self.shelf_y += self.shelf_h;
            self.shelf_x = 0;
            self.shelf_h = 0;
        }
        if self.shelf_y + h > IMAGE_ATLAS {
            return None;
        }
        let pos = (self.shelf_x, self.shelf_y);
        self.shelf_x += w + 1;
        self.shelf_h = self.shelf_h.max(h + 1);
        Some(pos)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    atlas: [f32; 2],
    params: [f32; 4],
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    atlas_tex: wgpu::Texture,
    instance_buf: wgpu::Buffer,
    instance_cap: usize,
    instances: Vec<Instance>,
    atlas: GlyphAtlas,

    window_radius: f32,
    transparent: bool,
    image_tex: wgpu::Texture,
    images: ImageCache,
    images_overflowed: bool,

    clips: Vec<(usize, Option<[u32; 4]>)>,
    layers: layers::Layers,

    fade: (f32, f32),
}

impl Renderer {
    pub async fn new(
        window: Arc<impl wgpu::DisplayAndWindowHandle + 'static>,
        width: u32,
        height: u32,
        font: FontConfig,
    ) -> Result<Self, RenderError> {
        let t = std::time::Instant::now();
        let (surface, adapter) = match Self::adapter(window.clone(), wgpu::Backends::PRIMARY).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("no native GPU backend ({e}); falling back to OpenGL");
                Self::adapter(window, wgpu::Backends::GL).await?
            }
        };
        log::info!("GPU: {:?}", adapter.get_info());
        log::debug!("gpu: adapter {} ms", t.elapsed().as_millis());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("zhell"),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await?;

        let caps = surface.get_capabilities(&adapter);

        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            caps.alpha_modes[0]
        };
        let transparent = alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied;
        log::info!("surface alpha modes {:?}, using {alpha_mode:?}", caps.alpha_modes);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        log::debug!("gpu: device {} ms", t.elapsed().as_millis());
        surface.configure(&device, &config);
        log::debug!("gpu: surface {} ms", t.elapsed().as_millis());

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let image_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image atlas"),
            size: wgpu::Extent3d { width: IMAGE_ATLAS, height: IMAGE_ATLAS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &atlas_tex.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&image_tex.create_view(&wgpu::TextureViewDescriptor::default())),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quads"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quads"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x4, 4 => Uint32, 5 => Float32x2
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let instance_cap = 16 * 1024;
        let instance_buf = Self::make_instance_buf(&device, instance_cap);
        log::debug!("gpu: pipeline {} ms", t.elapsed().as_millis());
        let layers = layers::Layers::new(&device, format);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            bind_group,
            globals,
            atlas_tex,
            instance_buf,
            instance_cap,
            instances: Vec::new(),
            atlas: GlyphAtlas::new(font),
            window_radius: 0.0,
            transparent,
            image_tex,
            images: ImageCache::default(),
            images_overflowed: false,
            clips: Vec::new(),
            layers,
            fade: (1.0, 0.0),
        })
    }

    async fn adapter<W: wgpu::DisplayAndWindowHandle + 'static>(
        window: Arc<W>,
        backends: wgpu::Backends,
    ) -> Result<(wgpu::Surface<'static>, wgpu::Adapter), RenderError> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::from_env().unwrap_or(backends);
        let instance = wgpu::Instance::new(desc);
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await?;
        Ok((surface, adapter))
    }

    fn make_instance_buf(device: &wgpu::Device, cap: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (cap * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn transparent(&self) -> bool {
        self.transparent
    }

    pub fn set_window_radius(&mut self, radius: f32) {
        self.window_radius = if self.transparent { radius.max(0.0) } else { 0.0 };
    }

    pub fn set_background(&mut self, bg: Option<Background>) -> Result<(), String> {
        self.layers.set_background(&self.device, &self.queue, bg)
    }

    pub fn has_background(&self) -> bool {
        self.layers.has_background()
    }

    pub fn background_animated(&self) -> bool {
        self.layers.animated()
    }

    pub fn set_background_tint(&mut self, tint: [f32; 4]) {
        self.layers.set_tint(tint);
    }

    pub fn set_crt(&mut self, on: bool) {
        self.layers.set_crt(&self.device, on);
    }

    pub fn max_texture_size(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn set_font(&mut self, font: FontConfig) {
        self.atlas.set_font(font);
    }

    pub fn upgrade_fonts(&mut self) -> bool {
        self.atlas.upgrade_fonts()
    }

    pub fn cell_metrics(&self) -> CellMetrics {
        self.atlas.metrics
    }

    pub fn begin(&mut self) {
        self.instances.clear();
        self.clips.clear();
        self.fade = (1.0, 0.0);
        self.atlas.overflowed = false;
        self.images_overflowed = false;
    }

    pub fn image(&mut self, key: u64, width: u32, height: u32, rgba: &[u8], src: [f32; 4], dst: [f32; 4]) {
        let pos = match self.images.placed.get(&key) {
            Some(p) => *p,
            None => {
                let Some(p) = self.images.place(width, height) else {
                    self.images.clear();
                    self.images_overflowed = true;
                    return;
                };
                if rgba.len() >= (width * height * 4) as usize {
                    self.images.uploads.push((p.0, p.1, width, height, rgba[..(width * height * 4) as usize].to_vec()));
                }
                self.images.placed.insert(key, p);
                p
            }
        };
        self.push(Instance {
            pos: [dst[0], dst[1]],
            size: [dst[2], dst[3]],
            uv: [pos.0 as f32 + src[0], pos.1 as f32 + src[1]],
            color: [1.0; 4],
            kind: 5,
            uv_size: [src[2], src[3]],
        });
    }

    pub fn set_fade(&mut self, alpha: f32, dy: f32) {
        self.fade = (alpha.clamp(0.0, 1.0), dy);
    }

    fn push(&mut self, mut i: Instance) {
        let (alpha, dy) = self.fade;
        if alpha <= 0.0 {
            return;
        }
        i.pos[1] += dy;
        i.color[3] *= alpha;
        self.instances.push(i);
    }

    pub fn forget_image(&mut self, key: u64) {
        self.images.placed.remove(&key);
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.push(Instance { pos: [x, y], size: [w, h], uv: [0.0; 2], color, kind: 0, uv_size: [0.0; 2] });
    }

    pub fn rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: [f32; 4]) {
        self.push(Instance { pos: [x, y], size: [w, h], uv: [radius, 0.0], color, kind: 3, uv_size: [0.0; 2] });
    }

    pub fn clip(&mut self, rect: Option<[f32; 4]>) {
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let dy = self.fade.1;
        let r = rect.map(|[x, y, rw, rh]| {
            let y = y + dy;
            let x0 = x.max(0.0).min(w);
            let y0 = y.max(0.0).min(h);
            let x1 = (x + rw).max(0.0).min(w);
            let y1 = (y + rh).max(0.0).min(h);
            [x0 as u32, y0 as u32, (x1 - x0).max(0.0) as u32, (y1 - y0).max(0.0) as u32]
        });
        self.clips.push((self.instances.len(), r));
    }

    pub fn rounded_outline(&mut self, rect: [f32; 4], radius: f32, thickness: f32, color: [f32; 4]) {
        let [x, y, w, h] = rect;
        self.push(Instance { pos: [x, y], size: [w, h], uv: [radius, thickness], color, kind: 4, uv_size: [0.0; 2] });
    }

    pub fn glyph(&mut self, x: f32, y: f32, text: &str, style: GlyphStyle, color: [f32; 4]) {
        if let Some(g) = self.atlas.get(text, style) {
            self.push(Instance {
                pos: [x + g.offset[0], y + g.offset[1]],
                size: g.size,
                uv: g.uv,
                color,
                kind: if g.color { 2 } else { 1 },
                uv_size: g.size,
            });
        }
    }

    pub fn word(&mut self, x: f32, y: f32, text: &str, style: GlyphStyle, color: [f32; 4]) {
        match self.atlas.get_word(text, style) {
            Some(glyphs) => {
                for g in glyphs {
                    self.push(Instance {
                        pos: [x + g.offset[0], y + g.offset[1]],
                        size: g.size,
                        uv: g.uv,
                        color,
                        kind: if g.color { 2 } else { 1 },
                        uv_size: g.size,
                    });
                }
            }
            None => {
                let w = self.atlas.metrics.width;
                let mut buf = [0u8; 4];
                for (i, ch) in text.chars().enumerate() {
                    self.glyph(x + i as f32 * w, y, ch.encode_utf8(&mut buf), style, color);
                }
            }
        }
    }

    pub fn render(&mut self, clear: [f32; 4]) -> FrameOutcome {
        if self.atlas.overflowed || self.images_overflowed {
            return FrameOutcome::Rebuild;
        }
        for (x, y, w, h, data) in self.images.uploads.drain(..) {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.image_tex, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                &data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        for (x, y, w, h, data) in self.atlas.uploads.drain(..) {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.surface.configure(&self.device, &self.config);
                f
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return FrameOutcome::Skipped;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return FrameOutcome::Skipped,
        };

        if self.instances.len() > self.instance_cap {
            self.instance_cap = self.instances.len().next_power_of_two();
            self.instance_buf = Self::make_instance_buf(&self.device, self.instance_cap);
        }
        self.queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(&self.instances));
        let g = Globals {
            screen: [self.config.width as f32, self.config.height as f32],
            atlas: [ATLAS_SIZE as f32; 2],
            params: [self.window_radius, IMAGE_ATLAS as f32, 0.0, 0.0],
        };
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        self.layers.prepare(&self.queue, g.screen, self.window_radius);

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: clear[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.layers.draw_background(&mut pass);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buf.slice(..));

            let total = self.instances.len() as u32;
            let mut segments: Vec<(u32, Option<[u32; 4]>)> = vec![(0, None)];
            for (start, rect) in &self.clips {
                segments.push((*start as u32, *rect));
            }
            for (i, (start, rect)) in segments.iter().enumerate() {
                let end = segments.get(i + 1).map_or(total, |s| s.0);
                if end <= *start {
                    continue;
                }
                match rect {
                    Some([_, _, 0, _]) | Some([_, _, _, 0]) => continue,
                    Some([x, y, w, h]) => pass.set_scissor_rect(*x, *y, *w, *h),
                    None => pass.set_scissor_rect(0, 0, self.config.width, self.config.height),
                }
                pass.draw(0..4, *start..end);
            }
            self.layers.draw_overlay(&mut pass, (self.config.width, self.config.height));
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
        FrameOutcome::Presented
    }
}
