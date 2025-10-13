use crate::engine::renderer::DrawCommand;
use anyhow::Result;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
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
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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
                cull_mode: None,
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

        log::info!("Generating vertices for window size: {}x{}", width, height);

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

                    log::info!("DrawRect: screen({}, {}, {}, {}) -> NDC({}, {}, {}, {})",
                              x, y, w, h, x1, y1, x2, y2);

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
                DrawCommand::DrawText {
                    x,
                    y,
                    text,
                    font_size,
                    color,
                } => {
                    // 簡易的なテキスト描画（矩形で代用）
                    let char_width = font_size * 0.6;
                    for (i, _ch) in text.chars().enumerate() {
                        let char_x = x + (i as f32 * char_width);
                        let x1 = (char_x / width) * 2.0 - 1.0;
                        let y1 = 1.0 - (y / height) * 2.0;
                        let x2 = ((char_x + char_width) / width) * 2.0 - 1.0;
                        let y2 = 1.0 - ((y + font_size) / height) * 2.0;

                        let color_array = [color.r, color.g, color.b, color.a];

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
                }
            }
        }

        vertices
    }

    /// 描画命令を更新
    pub fn update_draw_commands(&mut self, commands: &[DrawCommand]) {
        log::info!("update_draw_commands called with {} commands", commands.len());
        let vertices = self.generate_vertices(commands);
        self.num_vertices = vertices.len() as u32;
        log::info!("Generated {} vertices from commands", self.num_vertices);

        if !vertices.is_empty() {
            self.vertex_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Vertex Buffer"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
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
                log::info!("Drawing {} vertices", self.num_vertices);
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.draw(0..self.num_vertices, 0..1);
            } else {
                log::warn!("No vertex buffer available for rendering");
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
