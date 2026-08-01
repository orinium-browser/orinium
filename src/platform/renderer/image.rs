//! GPU resources and rendering for decoded page images.

use std::collections::HashMap;

use wgpu::util::DeviceExt;

use super::mesh::ImageSection;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ImageVertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
}

impl ImageVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}

struct TextureBinding {
    bind_group: wgpu::BindGroup,
}

struct PreparedImage {
    vertex_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Caches decoded images as GPU textures and prepares image draw calls.
pub struct ImageRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    textures: HashMap<u64, TextureBinding>,
    prepared: Vec<PreparedImage>,
}

impl ImageRenderer {
    /// Creates the textured image pipeline for the target surface format.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image_bind_group_layout"),
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
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("image_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Image Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader/image.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Image Render Pipeline"),
            layout: Some(&pipeline_layout),
            cache: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(ImageVertex::desc())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            textures: HashMap::new(),
            prepared: Vec::new(),
        }
    }

    /// Uploads uncached textures and prepares vertices for the current frame.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: &[ImageSection],
        screen_width: f32,
        screen_height: f32,
    ) {
        self.prepared.clear();

        for section in sections {
            if !self.textures.contains_key(&section.image.id()) {
                let size = wgpu::Extent3d {
                    width: section.image.width(),
                    height: section.image.height(),
                    depth_or_array_layers: 1,
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("page_image"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    section.image.rgba(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * section.image.width()),
                        rows_per_image: Some(section.image.height()),
                    },
                    size,
                );
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("image_sampler"),
                    address_mode_u: wgpu::AddressMode::ClampToEdge,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("image_bind_group"),
                    layout: &self.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });
                self.textures
                    .insert(section.image.id(), TextureBinding { bind_group });
            }

            let x1 = section.rect.x / screen_width * 2.0 - 1.0;
            let y1 = 1.0 - section.rect.y / screen_height * 2.0;
            let x2 = (section.rect.x + section.rect.width) / screen_width * 2.0 - 1.0;
            let y2 = 1.0 - (section.rect.y + section.rect.height) / screen_height * 2.0;
            let u1 = section.uv.x;
            let v1 = section.uv.y;
            let u2 = section.uv.x + section.uv.width;
            let v2 = section.uv.y + section.uv.height;
            let opacity = section.opacity;
            let vertices = [
                ImageVertex {
                    position: [x1, y1],
                    uv: [u1, v1],
                    opacity,
                },
                ImageVertex {
                    position: [x1, y2],
                    uv: [u1, v2],
                    opacity,
                },
                ImageVertex {
                    position: [x2, y2],
                    uv: [u2, v2],
                    opacity,
                },
                ImageVertex {
                    position: [x1, y1],
                    uv: [u1, v1],
                    opacity,
                },
                ImageVertex {
                    position: [x2, y2],
                    uv: [u2, v2],
                    opacity,
                },
                ImageVertex {
                    position: [x2, y1],
                    uv: [u2, v1],
                    opacity,
                },
            ];
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("image_vertex_buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let bind_group = self.textures[&section.image.id()].bind_group.clone();
            self.prepared.push(PreparedImage {
                vertex_buffer,
                bind_group,
            });
        }
    }

    /// Draws all prepared image sections.
    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.pipeline);
        for prepared in &self.prepared {
            render_pass.set_bind_group(0, &prepared.bind_group, &[]);
            render_pass.set_vertex_buffer(0, prepared.vertex_buffer.slice(..));
            render_pass.draw(0..6, 0..1);
        }
    }
}
