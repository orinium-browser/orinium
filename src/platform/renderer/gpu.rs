use crate::engine::renderer::DrawCommand;
use anyhow::Result;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use wgpu_text::glyph_brush::{Section as TextSection, Text};
use winit::window::Window;

use super::glyph::text::TextRenderer;
// use super::scroll_bar::ScrollBar;

/// GPU描画コンテキスト
pub struct GpuRenderer {
    /// GPUの描画対象
    surface: wgpu::Surface<'static>,
    /// GPUの論理デバイス
    device: wgpu::Device,
    /// コマンド送信用キュー
    queue: wgpu::Queue,
    /// サーフェス設定、解像度・フォーマットなどのフレームバッファ設定
    config: wgpu::SurfaceConfiguration,
    /// WindowSize
    size: winit::dpi::PhysicalSize<u32>,
    /// ディスプレイ倍率
    scale_factor: f64,
    /// RenderPipelin（頂点 to ピクセル）
    render_pipeline: wgpu::RenderPipeline,
    /// 頂点バッファ
    vertex_buffer: Option<wgpu::Buffer>,
    /// 頂点数
    num_vertices: u32,

    /// テキスト描画用ラッパー
    text_renderer: Option<TextRenderer>,
    /// 最後のフレーム時刻（アニメーション計算用）
    last_frame: Option<std::time::Instant>,
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
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

impl GpuRenderer {
    /// 新しいGPUレンダラーを作成
    pub async fn new(window: Arc<Window>, font_path: Option<&str>) -> Result<Self> {
        let size = window.inner_size();
        let scale_factor = window.scale_factor();

        // GPUドライバとの通信インスタンス
        // wgpuインスタンスの作成
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // OSウィンドウとGPUの描画対象（サーフェス）を関連付ける
        // サーフェスの作成
        let surface = instance.create_surface(window.clone())?;

        // 利用可能なGPU（物理デバイス）アダプターの取得
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
        // フレームバッファ設定（解像度・フォーマットなど）
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

        // シェーダーの読み込み
        // シェーダーモジュールの作成
        // vertex/fragment for main pipeline
        let main_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Main Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader/main.wgsl").into()),
        });

        // --- レンダーパイプライン（頂点→ピクセル変換のルール）の作成 ---
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
                cull_mode: None, // 三角扇がカリングで消えちゃう...
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
        // --- レンダーパイプライン作成終了 ---

        // テキスト描画用ラッパーの初期化。引数で渡されたフォントパスがあればそれを優先して読み込む。
        let text_renderer = if let Some(p) = font_path {
            match std::fs::read(p) {
                Ok(bytes) => match TextRenderer::new_from_bytes(
                    &device,
                    config.width,
                    config.height,
                    config.format,
                    bytes,
                ) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        log::warn!(target:"PRender::gpu::font" ,"failed to init text renderer from provided font: {}", e);
                        None
                    }
                },
                Err(e) => {
                    log::warn!(target:"PRender::gpu::font" ,"failed to read font path '{}': {}", p, e);
                    None
                }
            }
        } else {
            match TextRenderer::new_from_device(&device, config.width, config.height, config.format)
            {
                Ok(t) => Some(t),
                Err(e) => {
                    log::warn!(target:"PRender::gpu::font" ,"no system font found for text renderer: {}", e);
                    None
                }
            }
        };

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            scale_factor,
            render_pipeline,
            vertex_buffer: None,
            num_vertices: 0,
            text_renderer,
            last_frame: None,
        })
    }

    /// ウィンドウサイズが変更された時の処理
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            log::info!(target:"PRender::gpu::resized", "Resized: {}x{}", new_size.width, new_size.height);

            self.size = new_size;

            self.config.width = new_size.width;
            self.config.height = new_size.height;

            self.surface.configure(&self.device, &self.config);

            if let Some(tr) = &mut self.text_renderer {
                tr.resize_view(
                    self.config.width as f32,
                    self.config.height as f32,
                    &self.queue,
                );
            }
        }
    }

    /// 描画命令を解析して頂点バッファやテキストキューに登録
    /// Textのclippingはまだ未実装
    pub fn parse_draw_commands(&mut self, commands: &[DrawCommand]) {
        let width = self.size.width as f32;
        let height = self.size.height as f32;

        // --- 頂点データ ---
        let mut vertices = Vec::new();
        // --- Text ---
        let mut sections: Vec<TextSection> = Vec::new();
        // --- scale_factor ---
        let sf = self.scale_factor as f32;
        // --- transform stack ---
        let mut transform_stack: Vec<(f32, f32)> = vec![(0.0, 0.0)];
        let current_transform = |stack: &Vec<(f32, f32)>| -> (f32, f32) {
            let mut dx = 0.0;
            let mut dy = 0.0;
            for (x, y) in stack.iter() {
                dx += x;
                dy += y;
            }
            (dx, dy)
        };
        // --- clip stack ---
        #[derive(Clone, Copy)]
        struct ClipRect {
            x: f32,
            y: f32,
            w: f32,
            h: f32,
        }
        let mut clip_stack: Vec<ClipRect> = vec![ClipRect {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
        }];
        let current_clip = |stack: &Vec<ClipRect>| -> ClipRect { *stack.last().unwrap() };

        for command in commands {
            match command {
                // Transform (Push / Pop)
                DrawCommand::PushTransform { dx, dy } => {
                    transform_stack.push((*dx, *dy));
                }
                DrawCommand::PopTransform => {
                    if transform_stack.len() > 1 {
                        transform_stack.pop();
                    }
                }

                // Clip (Push / Pop)
                DrawCommand::PushClip {
                    x,
                    y,
                    width: w,
                    height: h,
                } => {
                    let (tdx, tdy) = current_transform(&transform_stack);
                    let new_clip = ClipRect {
                        x: x + tdx,
                        y: y + tdy,
                        w: *w,
                        h: *h,
                    };

                    // 現在の clip との AND を取る
                    let parent = current_clip(&clip_stack);

                    let x1 = new_clip.x.max(parent.x);
                    let y1 = new_clip.y.max(parent.y);
                    let x2 = (new_clip.x + new_clip.w).min(parent.x + parent.w);
                    let y2 = (new_clip.y + new_clip.h).min(parent.y + parent.h);

                    clip_stack.push(ClipRect {
                        x: x1,
                        y: y1,
                        w: (x2 - x1).max(0.0),
                        h: (y2 - y1).max(0.0),
                    });
                }

                DrawCommand::PopClip => {
                    if clip_stack.len() > 1 {
                        clip_stack.pop();
                    }
                }

                // Rectangle
                DrawCommand::DrawRect {
                    x,
                    y,
                    width: w,
                    height: h,
                    color,
                } => {
                    // transform
                    let (tdx, tdy) = current_transform(&transform_stack);
                    let mut x1 = (x + tdx) * sf;
                    let mut y1 = (y + tdy) * sf;
                    let mut x2 = x1 + w * sf;
                    let mut y2 = y1 + h * sf;

                    // clip 取得
                    let clip = current_clip(&clip_stack);

                    // 完全に外なら skip
                    if x2 <= clip.x * sf
                        || x1 >= (clip.x + clip.w) * sf
                        || y2 <= clip.y * sf
                        || y1 >= (clip.y + clip.h) * sf
                    {
                        continue;
                    }

                    // 部分クリップ
                    x1 = x1.max(clip.x * sf);
                    y1 = y1.max(clip.y * sf);
                    x2 = x2.min((clip.x + clip.w) * sf);
                    y2 = y2.min((clip.y + clip.h) * sf);

                    // NDC
                    let ndc = |v, max| (v / max) * 2.0 - 1.0;

                    let px1 = ndc(x1, width);
                    let py1 = -ndc(y1, height);
                    let px2 = ndc(x2, width);
                    let py2 = -ndc(y2, height);

                    let color = [color.r, color.g, color.b, color.a];

                    #[rustfmt::skip]
                    vertices.extend_from_slice(&[
                        Vertex { position: [px1, py1, 0.0], color },
                        Vertex { position: [px1, py2, 0.0], color },
                        Vertex { position: [px2, py1, 0.0], color },

                        Vertex { position: [px2, py1, 0.0], color },
                        Vertex { position: [px1, py2, 0.0], color },
                        Vertex { position: [px2, py2, 0.0], color },
                    ]);
                }

                // Text
                DrawCommand::DrawText {
                    x,
                    y,
                    text,
                    font_size,
                    color,
                } => {
                    let (tdx, tdy) = current_transform(&transform_stack);

                    let clip = current_clip(&clip_stack);
                    let (clip_x, clip_y) = (clip.x + clip.w, clip.y + clip.h);

                    let section = TextSection {
                        screen_position: ((*x + tdx) * sf, (*y + tdy) * sf),
                        bounds: (clip_x * sf, clip_y * sf),
                        text: vec![
                            Text::new(text)
                                .with_scale(*font_size * sf)
                                .with_color([color.r, color.g, color.b, color.a]),
                        ],
                        ..TextSection::default()
                    };
                    sections.push(section);
                }

                // Polygon
                #[allow(unused)]
                DrawCommand::DrawPolygon { points, color } => {
                    // transform
                    let (tdx, tdy) = current_transform(&transform_stack);
                    let mut transformed_points: Vec<(f32, f32)> =
                        points.iter().map(|(px, py)| (px + tdx, py + tdy)).collect();

                    // clip 取得
                    let clip = current_clip(&clip_stack);

                    todo!("Polygon drawing with clipping is not implemented yet");
                }

                // Ellipse
                #[allow(unused)]
                DrawCommand::DrawEllipse {
                    center,
                    radius_x,
                    radius_y,
                    color,
                } => {
                    // transform
                    let (tdx, tdy) = current_transform(&transform_stack);
                    let cx = center.0 + tdx;
                    let cy = center.1 + tdy;

                    // clip 取得
                    let clip = current_clip(&clip_stack);

                    todo!("Ellipse drawing with clipping is not implemented yet");
                }
            }
        }

        // 頂点バッファを登録
        if !vertices.is_empty() {
            self.vertex_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Vertex Buffer"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
            self.num_vertices = vertices.len() as u32;
        }

        // テキストセクションをキューに追加
        if let Some(tr) = &mut self.text_renderer {
            tr.queue(&self.device, &self.queue, &sections).unwrap();
        }
    }

    /// フレームを描画
    pub fn render(&mut self) -> Result<bool> {
        // 描画するフレームバッファを取得
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // text_scroll を target_text_scroll に向かって進める
        let now = std::time::Instant::now();
        let dt = if let Some(prev) = self.last_frame {
            now.duration_since(prev).as_secs_f32()
        } else {
            1.0 / 60.0
        };
        self.last_frame = Some(now);

        let smoothing_speed = 15.0_f32;
        let _alpha = 1.0 - (-smoothing_speed * dt).exp();

        let animating = false;

        // アニメーション中はテキストブラシが更新位置を反映できるようにセクションを再キューする必要がある
        // 補足: 呼び出し元（UI層）も各フレームで描画コマンドを再キューしているため、ここではアニメーション状態を返り値で通知するだけ

        // GPUコマンドのエンコーダーの作成
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        // 描画パスの開始
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // 背景色をクリア
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

            // 使用するシェーダー・設定をセット
            render_pass.set_pipeline(&self.render_pipeline);
            // 頂点バッファをセットして描画
            if let Some(ref vertex_buffer) = self.vertex_buffer {
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.draw(0..self.num_vertices, 0..1);
            }
        }

        // テキストをレンダリング
        if let Some(tr) = &mut self.text_renderer {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Text Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            tr.draw(&mut rpass);
        }

        // コマンドをGPUに送信
        self.queue.submit(std::iter::once(encoder.finish()));

        // フレームを画面に表示
        output.present();

        Ok(animating)
    }
}
