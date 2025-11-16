use crate::engine::renderer::DrawCommand;
use anyhow::Result;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use wgpu_text::glyph_brush::{Section as TextSection, Text};
use winit::window::Window;

use crate::platform::renderer::glyph::text::TextRenderer;
use crate::platform::renderer::scroll_bar::ScrollBar;

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
    /// RenderPipelin（頂点 to ピクセル）
    render_pipeline: wgpu::RenderPipeline,
    /// 頂点バッファ
    vertex_buffer: Option<wgpu::Buffer>,
    /// 頂点数
    num_vertices: u32,

    /// テキスト描画用ラッパー
    text_renderer: Option<TextRenderer>,
    /// テキスト垂直スクロールオフセット（ピクセル）
    text_scroll: f32,
    /// アニメーション用ターゲットスクロール位置
    target_text_scroll: f32,
    /// 最後のフレーム時刻（アニメーション計算用）
    last_frame: Option<std::time::Instant>,

    /// コンテンツの高さ（スクロール可能領域の高さ）
    content_height: f32,
    /// マウスオーバーしてるかどうか
    scrollbar_hover: bool,
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
                        log::warn!("failed to init text renderer from provided font: {}", e);
                        None
                    }
                },
                Err(e) => {
                    log::warn!("failed to read font path '{}': {}", p, e);
                    None
                }
            }
        } else {
            match TextRenderer::new_from_device(&device, config.width, config.height, config.format)
            {
                Ok(t) => Some(t),
                Err(e) => {
                    log::warn!("no system font found for text renderer: {}", e);
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
            render_pipeline,
            vertex_buffer: None,
            num_vertices: 0,
            text_renderer,
            text_scroll: 0.0,
            target_text_scroll: 0.0,
            last_frame: None,
            content_height: 0.0,
            scrollbar_hover: false,
        })
    }

    /// スクロールのターゲットを相対更新（アニメーションで実際のtext_scrollに反映される）
    pub fn scroll_text_by(&mut self, dy: f32) {
        // 使えるスクロール範囲に収める
        let max_offset = (self.content_height - self.size.height as f32).max(0.0);
        self.target_text_scroll = (self.target_text_scroll + dy).clamp(0.0, max_offset);
        self.last_frame = None;
    }

    /// テキストのスクロール位置ターゲットを設定（ピクセル）
    pub fn set_text_scroll(&mut self, offset: f32) {
        let max_offset = (self.content_height - self.size.height as f32).max(0.0);
        self.target_text_scroll = offset.clamp(0.0, max_offset);
        self.last_frame = None;
    }

    pub fn set_text_scroll_immediate(&mut self, offset: f32) {
        let max_offset = (self.content_height - self.size.height as f32).max(0.0);
        let v = offset.clamp(0.0, max_offset);
        self.target_text_scroll = v;
        self.text_scroll = v;
        self.last_frame = None;
    }

    /// 現在のテキストスクロールオフセットを返す
    pub fn text_scroll(&self) -> f32 {
        self.text_scroll
    }

    /// コンテンツ全体の高さを返す（UI がスクロールバーのヒットテストに使用）
    pub fn content_height(&self) -> f32 {
        self.content_height
    }

    /// ウィンドウサイズが変更された時の処理
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
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

    /// 描画命令から頂点データを生成
    fn generate_vertices(&self, commands: &[DrawCommand]) -> Vec<Vertex> {
        let mut vertices = Vec::new();
        let width = self.size.width as f32;
        let height = self.size.height as f32;

        for command in commands {
            #[allow(clippy::single_match)]
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
                _ => {}
            }
        }

        vertices
    }

    /// 描画命令を更新
    pub fn update_draw_commands(&mut self, commands: &[DrawCommand]) {
        let mut all_vertices = self.generate_vertices(commands);

        // Compute content height from commands (max y + approx height)
        let mut max_y: f32 = 0.0;
        for command in commands {
            match command {
                DrawCommand::DrawText {
                    x: _x,
                    y,
                    text: _t,
                    font_size,
                    ..
                } => {
                    let bottom = y + font_size * 1.2; // approximate line height
                    if bottom > max_y {
                        max_y = bottom;
                    }
                }
                DrawCommand::DrawRect {
                    x: _x,
                    y,
                    height: h,
                    ..
                } => {
                    let bottom = y + h;
                    if bottom > max_y {
                        max_y = bottom;
                    }
                }
            }
        }
        self.content_height = max_y.max(self.size.height as f32);

        let sb = ScrollBar::default();
        log::debug!(
            "update_draw_commands: viewport=({},{}), computed content_height={}",
            self.size.width,
            self.size.height,
            self.content_height
        );
        if let Some((x1, y1, x2, y2)) = sb.thumb_rect(
            self.size.width as f32,
            self.size.height as f32,
            self.content_height,
            self.text_scroll,
        ) {
            // サム矩形が得られたら角丸矩形ヘルパーで頂点を生成する
            log::debug!(
                "scrollbar thumb rect: x1={},y1={},x2={},y2={}",
                x1,
                y1,
                x2,
                y2
            );
            let vw = self.size.width as f32;
            let vh = self.size.height as f32;
            let base = sb.color;
            // ホバー時はやや暗めにする
            let color = if self.scrollbar_hover {
                [base[0] * 0.6, base[1] * 0.6, base[2] * 0.6, base[3]]
            } else {
                base
            };
            // 角丸の半径（ピクセル）
            let radius = 6.0_f32;
            self.push_rounded_rect_vertices(
                &mut all_vertices,
                vw,
                vh,
                x1,
                y1,
                x2,
                y2,
                radius,
                color,
            );
        } else {
            log::debug!("no scrollbar needed (content fits viewport)");
        }

        log::debug!(
            "update_draw_commands: total_vertices_after_scrollbar={}",
            all_vertices.len()
        );

        if !all_vertices.is_empty() {
            self.vertex_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Vertex Buffer"),
                    contents: bytemuck::cast_slice(&all_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
            self.num_vertices = all_vertices.len() as u32;
        }

        // DrawCommandからテキスト描画セクションに変換
        let mut sections: Vec<TextSection> = Vec::new();
        for command in commands {
            if let DrawCommand::DrawText {
                x,
                y,
                text,
                font_size,
                color,
            } = command
            {
                // スクロールオフセットを適用して Y 座標を移動
                let y_with_scroll = *y - self.text_scroll;
                let s = TextSection {
                    screen_position: (*x, y_with_scroll),
                    bounds: (self.size.width as f32, self.size.height as f32),
                    text: vec![
                        Text::new(text)
                            .with_scale(*font_size)
                            .with_color([color.r, color.g, color.b, color.a]),
                    ],
                    ..TextSection::default()
                };
                sections.push(s);
            }
        }

        if let Some(tr) = &mut self.text_renderer {
            // テキスト描画キューに追加
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
        let alpha = 1.0 - (-smoothing_speed * dt).exp();
        self.text_scroll += (self.target_text_scroll - self.text_scroll) * alpha;
        let animating = (self.target_text_scroll - self.text_scroll).abs() > 0.5;

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
    pub fn set_scrollbar_hover(&mut self, hover: bool) {
        self.scrollbar_hover = hover;
    }

    pub fn scrollbar_hover(&self) -> bool {
        self.scrollbar_hover
    }

    /// 指定したスクリーン座標の角丸長方形を頂点バッファ用の三角形列として追加する
    fn push_rounded_rect_vertices(
        &self,
        all_vertices: &mut Vec<Vertex>,
        vw: f32,
        vh: f32,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        radius: f32,
        color: [f32; 4],
    ) {
        // 角丸の半径は幅/高さに収める
        let r = radius.min((x2 - x1) * 0.5).min((y2 - y1) * 0.5);
        if r <= 0.0 {
            // 普通の長方形として追加
            all_vertices.extend_from_slice(&[
                Vertex {
                    position: [(x1 / vw) * 2.0 - 1.0, 1.0 - (y1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(x1 / vw) * 2.0 - 1.0, 1.0 - (y2 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(x2 / vw) * 2.0 - 1.0, 1.0 - (y1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(x2 / vw) * 2.0 - 1.0, 1.0 - (y1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(x1 / vw) * 2.0 - 1.0, 1.0 - (y2 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(x2 / vw) * 2.0 - 1.0, 1.0 - (y2 / vh) * 2.0, 0.0],
                    color,
                },
            ]);
            return;
        }

        // 中央矩形（左右に角丸分を除いた領域）
        let cx1 = x1 + r;
        let cx2 = x2 - r;
        let cy1 = y1;
        let cy2 = y2;
        all_vertices.extend_from_slice(&[
            Vertex {
                position: [(cx1 / vw) * 2.0 - 1.0, 1.0 - (cy1 / vh) * 2.0, 0.0],
                color,
            },
            Vertex {
                position: [(cx1 / vw) * 2.0 - 1.0, 1.0 - (cy2 / vh) * 2.0, 0.0],
                color,
            },
            Vertex {
                position: [(cx2 / vw) * 2.0 - 1.0, 1.0 - (cy1 / vh) * 2.0, 0.0],
                color,
            },
            Vertex {
                position: [(cx2 / vw) * 2.0 - 1.0, 1.0 - (cy1 / vh) * 2.0, 0.0],
                color,
            },
            Vertex {
                position: [(cx1 / vw) * 2.0 - 1.0, 1.0 - (cy2 / vh) * 2.0, 0.0],
                color,
            },
            Vertex {
                position: [(cx2 / vw) * 2.0 - 1.0, 1.0 - (cy2 / vh) * 2.0, 0.0],
                color,
            },
        ]);

        // 左側矩形（上下に角丸分を除いた領域）
        let lx1 = x1;
        let lx2 = x1 + r;
        let ly1 = y1 + r;
        let ly2 = y2 - r;
        if ly2 > ly1 {
            all_vertices.extend_from_slice(&[
                Vertex {
                    position: [(lx1 / vw) * 2.0 - 1.0, 1.0 - (ly1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(lx1 / vw) * 2.0 - 1.0, 1.0 - (ly2 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(lx2 / vw) * 2.0 - 1.0, 1.0 - (ly1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(lx2 / vw) * 2.0 - 1.0, 1.0 - (ly1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(lx1 / vw) * 2.0 - 1.0, 1.0 - (ly2 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(lx2 / vw) * 2.0 - 1.0, 1.0 - (ly2 / vh) * 2.0, 0.0],
                    color,
                },
            ]);
        }

        // 右側矩形
        let rx1 = x2 - r;
        let rx2 = x2;
        if ly2 > ly1 {
            all_vertices.extend_from_slice(&[
                Vertex {
                    position: [(rx1 / vw) * 2.0 - 1.0, 1.0 - (ly1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(rx1 / vw) * 2.0 - 1.0, 1.0 - (ly2 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(rx2 / vw) * 2.0 - 1.0, 1.0 - (ly1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(rx2 / vw) * 2.0 - 1.0, 1.0 - (ly1 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(rx1 / vw) * 2.0 - 1.0, 1.0 - (ly2 / vh) * 2.0, 0.0],
                    color,
                },
                Vertex {
                    position: [(rx2 / vw) * 2.0 - 1.0, 1.0 - (ly2 / vh) * 2.0, 0.0],
                    color,
                },
            ]);
        }

        // 角の四半円を三角形扇で近似する
        let segments_per_corner = 8usize; // 精度
        let inv = 1.0 / (segments_per_corner as f32);
        use std::f32::consts::PI;
        // 左上
        let cx = x1 + r;
        let cy = y1 + r;
        for i in 0..segments_per_corner {
            let a0 = PI * 1.0 + (i as f32) * (PI * 0.5) * inv;
            let a1 = PI * 1.0 + ((i + 1) as f32) * (PI * 0.5) * inv;
            let x00 = cx + a0.cos() * r;
            let y00 = cy + a0.sin() * r;
            let x01 = cx + a1.cos() * r;
            let y01 = cy + a1.sin() * r;
            all_vertices.push(Vertex {
                position: [(cx / vw) * 2.0 - 1.0, 1.0 - (cy / vh) * 2.0, 0.0],
                color,
            });
            all_vertices.push(Vertex {
                position: [(x00 / vw) * 2.0 - 1.0, 1.0 - (y00 / vh) * 2.0, 0.0],
                color,
            });
            all_vertices.push(Vertex {
                position: [(x01 / vw) * 2.0 - 1.0, 1.0 - (y01 / vh) * 2.0, 0.0],
                color,
            });
        }
        // 右上
        let cx = x2 - r;
        let cy = y1 + r;
        for i in 0..segments_per_corner {
            let a0 = PI * 1.5 + (i as f32) * (PI * 0.5) * inv;
            let a1 = PI * 1.5 + ((i + 1) as f32) * (PI * 0.5) * inv;
            let x00 = cx + a0.cos() * r;
            let y00 = cy + a0.sin() * r;
            let x01 = cx + a1.cos() * r;
            let y01 = cy + a1.sin() * r;
            all_vertices.push(Vertex {
                position: [(cx / vw) * 2.0 - 1.0, 1.0 - (cy / vh) * 2.0, 0.0],
                color,
            });
            all_vertices.push(Vertex {
                position: [(x00 / vw) * 2.0 - 1.0, 1.0 - (y00 / vh) * 2.0, 0.0],
                color,
            });
            all_vertices.push(Vertex {
                position: [(x01 / vw) * 2.0 - 1.0, 1.0 - (y01 / vh) * 2.0, 0.0],
                color,
            });
        }
        // 右下
        let cx = x2 - r;
        let cy = y2 - r;
        for i in 0..segments_per_corner {
            let a0 = 0.0 + (i as f32) * (PI * 0.5) * inv;
            let a1 = 0.0 + ((i + 1) as f32) * (PI * 0.5) * inv;
            let x00 = cx + a0.cos() * r;
            let y00 = cy + a0.sin() * r;
            let x01 = cx + a1.cos() * r;
            let y01 = cy + a1.sin() * r;
            all_vertices.push(Vertex {
                position: [(cx / vw) * 2.0 - 1.0, 1.0 - (cy / vh) * 2.0, 0.0],
                color,
            });
            all_vertices.push(Vertex {
                position: [(x00 / vw) * 2.0 - 1.0, 1.0 - (y00 / vh) * 2.0, 0.0],
                color,
            });
            all_vertices.push(Vertex {
                position: [(x01 / vw) * 2.0 - 1.0, 1.0 - (y01 / vh) * 2.0, 0.0],
                color,
            });
        }
        // 左下
        let cx = x1 + r;
        let cy = y2 - r;
        for i in 0..segments_per_corner {
            let a0 = PI * 0.5 + (i as f32) * (PI * 0.5) * inv;
            let a1 = PI * 0.5 + ((i + 1) as f32) * (PI * 0.5) * inv;
            let x00 = cx + a0.cos() * r;
            let y00 = cy + a0.sin() * r;
            let x01 = cx + a1.cos() * r;
            let y01 = cy + a1.sin() * r;
            all_vertices.push(Vertex {
                position: [(cx / vw) * 2.0 - 1.0, 1.0 - (cy / vh) * 2.0, 0.0],
                color,
            });
            all_vertices.push(Vertex {
                position: [(x00 / vw) * 2.0 - 1.0, 1.0 - (y00 / vh) * 2.0, 0.0],
                color,
            });
            all_vertices.push(Vertex {
                position: [(x01 / vw) * 2.0 - 1.0, 1.0 - (y01 / vh) * 2.0, 0.0],
                color,
            });
        }
    }
}
