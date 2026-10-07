use std::time::Instant;

use bytemuck::{Pod, Zeroable};

const BASE: &str = include_str!("background.wgsl");

const CUSTOM_MAIN: &str = "
@fragment
fn fs_custom(in: FullOut) -> @location(0) vec4<f32> {
    return zhell_finish(background(in.clip.xy, in.uv), in.clip.xy);
}
";

pub const BUILTIN_SHADERS: &[(&str, &str)] = &[
    ("aurora", include_str!("shaders/aurora.wgsl")),
    ("grid", include_str!("shaders/grid.wgsl")),
    ("nebula", include_str!("shaders/nebula.wgsl")),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    #[default]
    Cover,
    Contain,
    Stretch,
    Tile,
    Center,
}

pub struct Background<'a> {
    pub image: Option<(u32, u32, &'a [u8])>,
    pub fit: Fit,

    pub shader: Option<&'a str>,

    pub dim: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    time: f32,
    dim: f32,
    tint: [f32; 4],
    image: [f32; 4],
}

pub(crate) struct Layers {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    image_size: [f32; 2],
    fit: Fit,
    dim: f32,
    tint: [f32; 4],
    background: Option<wgpu::RenderPipeline>,

    animated: bool,
    crt: Option<wgpu::RenderPipeline>,
    start: Instant,
}

pub fn validate(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
        .validate(&module)
        .map_err(|e| e.emit_to_string(source))?;
    Ok(())
}

impl Layers {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layers"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
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
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("layers"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("layers"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let image = Self::texture(device, 1, 1);
        let bind_group = Self::bind(device, &layout, &uniforms, &image, &sampler);
        Self {
            format,
            layout,
            pipeline_layout,
            uniforms,
            sampler,
            bind_group,
            image_size: [1.0, 1.0],
            fit: Fit::Cover,
            dim: 0.0,
            tint: [0.0; 4],
            background: None,
            animated: false,
            crt: None,
            start: Instant::now(),
        }
    }

    fn texture(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("background image"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, uniforms: &wgpu::Buffer, image: &wgpu::Texture, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layers"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&image.create_view(&wgpu::TextureViewDescriptor::default())),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    fn pipeline(&self, device: &wgpu::Device, source: &str, entry: &str) -> Result<wgpu::RenderPipeline, String> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("layer"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        match pollster::block_on(scope.pop()) {
            Some(e) => Err(e.to_string()),
            None => Ok(pipeline),
        }
    }

    pub fn set_background(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bg: Option<Background>) -> Result<(), String> {
        let Some(bg) = bg else {
            self.background = None;
            self.animated = false;
            return Ok(());
        };
        let (source, entry) = match bg.shader {
            Some(user) => {
                let full = format!("{user}\n{BASE}\n{CUSTOM_MAIN}");

                validate(&full)?;
                (full, "fs_custom")
            }
            None if bg.image.is_some() => (BASE.to_owned(), "fs_image"),
            None => {
                self.background = None;
                return Ok(());
            }
        };
        let pipeline = self.pipeline(device, &source, entry)?;
        let (w, h, pixels) = bg.image.unwrap_or((1, 1, &[0, 0, 0, 0]));
        let image = Self::texture(device, w, h);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &image, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.bind_group = Self::bind(device, &self.layout, &self.uniforms, &image, &self.sampler);
        self.image_size = [w as f32, h as f32];
        self.fit = bg.fit;
        self.dim = bg.dim.clamp(0.0, 1.0);
        self.animated = bg.shader.is_some_and(|s| s.contains("zhell.time"));
        self.background = Some(pipeline);
        Ok(())
    }

    pub fn set_crt(&mut self, device: &wgpu::Device, on: bool) {
        self.crt = match (on, self.crt.take()) {
            (false, _) => None,
            (true, Some(p)) => Some(p),
            (true, None) => self.pipeline(device, BASE, "fs_crt").map_err(|e| log::error!("crt effect: {e}")).ok(),
        };
    }

    pub fn has_background(&self) -> bool {
        self.background.is_some()
    }

    pub fn animated(&self) -> bool {
        self.background.is_some() && self.animated
    }

    pub fn set_tint(&mut self, tint: [f32; 4]) {
        self.tint = tint;
    }

    pub fn prepare(&self, queue: &wgpu::Queue, size: [f32; 2], radius: f32) {
        if self.background.is_none() && self.crt.is_none() {
            return;
        }
        let u = Uniforms {
            resolution: size,
            time: self.start.elapsed().as_secs_f32() % 3600.0,
            dim: self.dim,
            tint: self.tint,
            image: [self.image_size[0], self.image_size[1], self.fit as u32 as f32, radius],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
    }

    pub fn draw_background(&self, pass: &mut wgpu::RenderPass) {
        if let Some(p) = &self.background {
            pass.set_pipeline(p);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    pub fn draw_overlay(&self, pass: &mut wgpu::RenderPass, size: (u32, u32)) {
        if let Some(p) = &self.crt {
            pass.set_scissor_rect(0, 0, size.0, size.1);
            pass.set_pipeline(p);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_shaders_validate() {
        for (name, src) in BUILTIN_SHADERS {
            validate(&format!("{src}\n{BASE}\n{CUSTOM_MAIN}")).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        validate(BASE).unwrap();
    }

    #[test]
    fn errors_point_at_user_lines() {
        let e = validate(&format!("fn background(px: vec2<f32>, uv: vec2<f32>) -> vec4<f32> {{\n  return vec4<f32>(nope);\n}}\n{BASE}\n{CUSTOM_MAIN}")).unwrap_err();
        assert!(e.contains("nope") && e.contains(":2:"), "{e}");
    }
}
