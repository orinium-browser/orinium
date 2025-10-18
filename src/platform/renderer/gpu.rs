use crate::engine::renderer::DrawCommand;
use anyhow::Result;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;
use wgpu_text::{glyph_brush::{Section as TextSection, Text}, BrushBuilder, TextBrush};

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

    glyph_brush: Option<TextBrush<ab_glyph::FontArc>>,
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
            glyph_brush: None,
        })
    }

    /// ウィンドウサイズが変更された時の処理
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            if let Some(brush) = &mut self.glyph_brush {
                brush.resize_view(self.config.width as f32, self.config.height as f32, &self.queue);
            }
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

        if self.glyph_brush.is_none() {
            let mut font_data: Option<Vec<u8>> = None;
            let candidates = [
                "C:\\Windows\\Fonts\\arial.ttf",
                "C:\\Windows\\Fonts\\segoeui.ttf",
                "C:\\Windows\\Fonts\\seguisym.ttf",
            ];
            for p in &candidates {
                if let Ok(b) = std::fs::read(p) {
                    font_data = Some(b);
                    break;
                }
            }
            if let Some(bytes) = font_data {
                let font_arc = ab_glyph::FontArc::try_from_vec(bytes).unwrap();
                let brush = BrushBuilder::using_font(font_arc).build(&self.device, self.config.width, self.config.height, self.config.format);
                self.glyph_brush = Some(brush);
            }
        }

        let mut sections: Vec<TextSection> = Vec::new();
        for command in commands {
            if let DrawCommand::DrawText { x, y, text, font_size, color } = command {
                let s = TextSection {
                    screen_position: (*x, *y),
                    bounds: (self.size.width as f32, self.size.height as f32),
                    text: vec![Text::new(text).with_scale(*font_size).with_color([color.r, color.g, color.b, color.a])],
                    ..TextSection::default()
                };
                sections.push(s);
            }
        }

        if let Some(brush) = &mut self.glyph_brush {
            brush.queue(&self.device, &self.queue, &sections).unwrap();
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

        if let Some(brush) = &mut self.glyph_brush {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            brush.draw(&mut rpass);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

         Ok(())
     }
 }
