use crate::engine::renderer::DrawCommand;
use anyhow::Result;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;
use crate::platform::renderer::glyph::fonts::{FontLoader, FontAtlas};

/// GPU描画コンテキスト
pub struct GpuRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: Option<wgpu::Buffer>,
    num_vertices: u32,

    // Text rendering resources
    font_loader: Option<FontLoader>,
    font_atlas: Option<FontAtlas>,
    text_pipeline: Option<wgpu::RenderPipeline>,
    font_bind_group: Option<wgpu::BindGroup>,
    text_vertex_buffer: Option<wgpu::Buffer>,
    num_text_vertices: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

impl Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

impl GpuRenderer {
    /// 新しいGPUレンダラーを作成
    pub async fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();

        // wgpuインスタンスの作成
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // サーフェスの作成
        let surface = instance.create_surface(window.clone())?;

        // アダプターの取得
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await?;

        // デバイスとキューの作成
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: Default::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: Default::default(),
            })
            .await?;

        // サーフェス設定
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // シェーダーモジュールの作成
        // vertex/fragment for main pipeline
        let main_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Main Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader/main.wgsl").into()),
        });

        // レンダーパイプラインの作成
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            cache: None,
            vertex: wgpu::VertexState {
                module: &main_shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &main_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            render_pipeline,
            vertex_buffer: None,
            num_vertices: 0,
            font_loader: None,
            font_atlas: None,
            text_pipeline: None,
            font_bind_group: None,
            text_vertex_buffer: None,
            num_text_vertices: 0,
        })
    }

    /// ウィンドウサイズが変更された時の処理
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// 描画命令から頂点データを生成
    fn generate_vertices(&self, commands: &[DrawCommand]) -> Vec<Vertex> {
        let mut vertices = Vec::new();
        let width = self.size.width as f32;
        let height = self.size.height as f32;

        for command in commands {
            match command {
                DrawCommand::DrawRect {
                    x,
                    y,
                    width: w,
                    height: h,
                    color,
                } => {
                    // スクリーン座標からNDC座標に変換
                    let x1 = (x / width) * 2.0 - 1.0;
                    let y1 = 1.0 - (y / height) * 2.0;
                    let x2 = ((x + w) / width) * 2.0 - 1.0;
                    let y2 = 1.0 - ((y + h) / height) * 2.0;

                    let color_array = [color.r, color.g, color.b, color.a];

                    // 矩形を2つの三角形で構成
                    vertices.extend_from_slice(&[
                        Vertex {
                            position: [x1, y1, 0.0],
                            color: color_array,
                        },
                        Vertex {
                            position: [x1, y2, 0.0],
                            color: color_array,
                        },
                        Vertex {
                            position: [x2, y1, 0.0],
                            color: color_array,
                        },
                        Vertex {
                            position: [x2, y1, 0.0],
                            color: color_array,
                        },
                        Vertex {
                            position: [x1, y2, 0.0],
                            color: color_array,
                        },
                        Vertex {
                            position: [x2, y2, 0.0],
                            color: color_array,
                        },
                    ]);
                }
                _ => {

                }
            }
        }

        vertices
    }

    /// 描画命令を更新
    pub fn update_draw_commands(&mut self, commands: &[DrawCommand]) {
        let vertices = self.generate_vertices(commands);
        self.num_vertices = vertices.len() as u32;

        if !vertices.is_empty() {
            self.vertex_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Vertex Buffer"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
        }

        if self.font_atlas.is_none() {
            let loader = FontLoader::new().ok();
            if let Some(mut loader) = loader {
                let candidates = [
                    "C:\\Windows\\Fonts\\arial.ttf",
                    "C:\\Windows\\Fonts\\segoeui.ttf",
                    "C:\\Windows\\Fonts\\seguisym.ttf",
                ];
                for path in &candidates {
                    if let Ok(bytes) = std::fs::read(path) {
                        if loader.load_from_bytes("sys", &bytes).is_ok() {
                            let charset: String = (32u8..127u8).map(|b| b as char).collect();
                            if let Ok((atlas, _fontarc)) = loader.build_atlas(&self.device, &self.queue, "sys", 32.0, &charset) {
                                let bind_group_layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                                    entries: &[
                                        wgpu::BindGroupLayoutEntry {
                                            binding: 0,
                                            visibility: wgpu::ShaderStages::FRAGMENT,
                                            ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Float { filterable: true } },
                                            count: None,
                                        },
                                        wgpu::BindGroupLayoutEntry {
                                            binding: 1,
                                            visibility: wgpu::ShaderStages::FRAGMENT,
                                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                            count: None,
                                        },
                                    ],
                                    label: Some("font_bind_group_layout"),
                                });

                                let font_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    layout: &bind_group_layout,
                                    entries: &[
                                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&atlas.texture_view) },
                                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&atlas.sampler) },
                                    ],
                                    label: Some("font_bind_group"),
                                });

                                let text_shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                                    label: Some("Text Shader"),
                                    source: wgpu::ShaderSource::Wgsl(include_str!("shader/text.wgsl").into()),
                                });

                                let text_pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                                    label: Some("Text Pipeline Layout"),
                                    bind_group_layouts: &[&bind_group_layout],
                                    push_constant_ranges: &[],
                                });

                                let text_pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                                    label: Some("Text Pipeline"),
                                    layout: Some(&text_pipeline_layout),
                                    cache: None,
                                    vertex: wgpu::VertexState {
                                        module: &text_shader,
                                        entry_point: Some("vs_text"),
                                        buffers: &[wgpu::VertexBufferLayout {
                                            array_stride: std::mem::size_of::<[f32; 9]>() as wgpu::BufferAddress, // pos(3) + uv(2) + color(4)
                                            step_mode: wgpu::VertexStepMode::Vertex,
                                            attributes: &[
                                                wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                                                wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32x2 },
                                                wgpu::VertexAttribute { offset: 20, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
                                            ],
                                        }],
                                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                                    },
                                    fragment: Some(wgpu::FragmentState {
                                        module: &text_shader,
                                        entry_point: Some("fs_text"),
                                        targets: &[Some(wgpu::ColorTargetState { format: self.config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                                    }),
                                    primitive: wgpu::PrimitiveState::default(),
                                    depth_stencil: None,
                                    multisample: wgpu::MultisampleState::default(),
                                    multiview: None,
                                });

                                self.font_loader = Some(loader);
                                self.font_atlas = Some(atlas);
                                self.text_pipeline = Some(text_pipeline);
                                self.font_bind_group = Some(font_bind_group);
                                break;
                            }
                        }
                    }
                }
            }
        }

        if let Some(atlas) = &self.font_atlas {
            let mut text_vertices: Vec<f32> = Vec::new();
            for command in commands {
                if let DrawCommand::DrawText { x, y, text, font_size, color } = command {
                    let mut pen_x = *x;
                    for ch in text.chars() {
                        if let Some(g) = atlas.glyph_map.get(&ch) {
                            let w = g.size[0] * (*font_size / 32.0);
                            let h = g.size[1] * (*font_size / 32.0);
                            let x0 = pen_x;
                            let y0 = *y - g.bearing[1];
                            let x1 = x0 + w;
                            let y1 = y0 + h;

                            let u0 = g.uv_rect[0];
                            let v0 = g.uv_rect[1];
                            let u1 = g.uv_rect[2];
                            let v1 = g.uv_rect[3];

                            let nx0 = (x0 / self.size.width as f32) * 2.0 - 1.0;
                            let ny0 = 1.0 - (y0 / self.size.height as f32) * 2.0;
                            let nx1 = (x1 / self.size.width as f32) * 2.0 - 1.0;
                            let ny1 = 1.0 - (y1 / self.size.height as f32) * 2.0;

                            let col = [color.r, color.g, color.b, color.a];

                            let mut push_vertex = |vx: f32, vy: f32, ux: f32, uy: f32| {
                                text_vertices.push(vx); text_vertices.push(vy); text_vertices.push(0.0);
                                text_vertices.push(ux); text_vertices.push(uy);
                                text_vertices.push(col[0]); text_vertices.push(col[1]); text_vertices.push(col[2]); text_vertices.push(col[3]);
                            };

                            // tri 1
                            push_vertex(nx0, ny0, u0, v0);
                            push_vertex(nx0, ny1, u0, v1);
                            push_vertex(nx1, ny0, u1, v0);
                            // tri 2
                            push_vertex(nx1, ny0, u1, v0);
                            push_vertex(nx0, ny1, u0, v1);
                            push_vertex(nx1, ny1, u1, v1);

                            pen_x += g.advance * (*font_size / 32.0);
                        } else {
                            pen_x += *font_size * 0.6;
                        }
                    }
                }
            }

            if !text_vertices.is_empty() {
                self.num_text_vertices = (text_vertices.len() / 9) as u32;
                // create buffer
                let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Text Vertex Buffer"),
                    contents: bytemuck::cast_slice(&text_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                self.text_vertex_buffer = Some(buf);
            } else {
                self.text_vertex_buffer = None;
                self.num_text_vertices = 0;
            }
        }
    }

    /// フレームを描画
    pub fn render(&mut self) -> Result<()> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            if let Some(ref vertex_buffer) = self.vertex_buffer {
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.draw(0..self.num_vertices, 0..1);
            }
        }

        if let (Some(pipeline), Some(buf), Some(bind)) = (&self.text_pipeline, &self.text_vertex_buffer, &self.font_bind_group) {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Text Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, bind, &[]);
            rp.set_vertex_buffer(0, buf.slice(..));
            rp.draw(0..self.num_text_vertices, 0..1);
        }

         self.queue.submit(std::iter::once(encoder.finish()));
         output.present();

         Ok(())
     }
 }
