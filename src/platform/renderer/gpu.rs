//! wgpuを使用してGPUで描画するためのコンテキストと処理を提供するモジュール

use crate::engine::layouter::types::{
    Color, ColorStop, Gradient, GradientKind, RadialShape, RadialSizeKind,
};
use crate::engine::renderer_model::{AffineTransform, Brush, DrawCommand, Rect};
use anyhow::Result;
use std::sync::Arc;
use std::{env, fmt::Debug};
use wgpu::util::DeviceExt;
use winit::window::Window;

use super::text::global_font;
use super::text::text::{TextRenderer, TextSection};

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
    /// 頂点
    vertices: Vec<Vertex>,
    /// 頂点数
    num_vertices: u32,

    /// テキスト描画用ラッパー
    text_renderer: Option<TextRenderer>,

    /// テキストカリングを有効にする
    enable_text_culling: bool,
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

/// An axis-aligned clip region in logical (pre-scale-factor) coordinates.
#[derive(Clone, Copy)]
struct ClipRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl GpuRenderer {
    /// 新しいGPUレンダラーを作成
    pub async fn new(window: Arc<Window>, font_path: Option<&str>) -> Result<Self> {
        let size = window.inner_size();
        let scale_factor = window.scale_factor();

        // GPUドライバとの通信インスタンス
        // wgpuインスタンスの作成
        //
        // [`InstanceDescriptor::new_with_out_display_hundler`] の実装を参考に
        // backends 選択は [`select_wgpu_backends`] を使った実装。
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: select_wgpu_backends(),
            flags: Default::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });

        // OSウィンドウとGPUの描画対象（サーフェス）を関連付ける
        // サーフェスの作成
        let surface = instance.create_surface(Arc::clone(&window))?;

        // 利用可能なGPU（物理デバイス）アダプターの取得
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                // This is currently only used for the browser's rendering backend, but we
                // enable limit bucketing preemptively in case WebGPU is exposed to web content
                // in the future.
                apply_limit_buckets: true,
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
            // Use automatic color space selection until browser-level color
            // management and CSS color spaces are implemented.
            color_space: wgpu::SurfaceColorSpace::Auto,
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
                immediate_size: 0,
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            cache: None,
            vertex: wgpu::VertexState {
                module: &main_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(Vertex::desc())],
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
            multiview_mask: None,
        });
        // --- レンダーパイプライン作成終了 ---

        // テキスト描画用ラッパーの初期化。引数で渡されたフォントパスがあればグローバルフォントシステムに追加。
        if let Some(p) = font_path {
            global_font::load_global_font_path(p);
        }
        let text_renderer = match TextRenderer::new(&device, &queue, config.format) {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!(target:"PRender::gpu::font" ,"no system font found for text renderer: {}", e);
                None
            }
        };

        // Enable text culling by default, allow override by env var
        let enable_text_culling = std::env::var("ORINIUM_TEXT_CULL").map_or(true, |v| v != "0");

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            scale_factor,
            render_pipeline,
            vertex_buffer: None,
            vertices: vec![],
            num_vertices: 0,
            text_renderer,
            enable_text_culling,
        })
    }

    /// ウィンドウサイズが変更された時の処理
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            log::info!(target:"PRender::gpu::resized", "Resized: {}x{}", new_size.width, new_size.height);

            let old_size = self.size;

            self.size = new_size;

            self.config.width = new_size.width;
            self.config.height = new_size.height;

            self.surface.configure(&self.device, &self.config);

            self.update_vertices(old_size, new_size);

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
    pub fn parse_draw_commands(&mut self, commands: &[DrawCommand]) {
        let screen_width = self.size.width as f32;
        let screen_height = self.size.height as f32;

        // --- 頂点データ ---
        let mut vertices = Vec::new();
        // --- Text ---
        let mut sections: Vec<TextSection> = Vec::new();
        // --- scale_factor ---
        let sf = self.scale_factor as f32;
        // --- transform stack ---
        // Each entry stores the composition of all transforms up to and
        // including that level, so the current transform is always the last
        // entry (O(1) lookup instead of recomposing the whole stack).
        let mut transform_stack: Vec<AffineTransform> = vec![AffineTransform::identity()];
        let current_transform =
            |stack: &Vec<AffineTransform>| -> AffineTransform { *stack.last().unwrap() };
        // --- clip stack ---
        let mut clip_stack: Vec<ClipRect> = vec![ClipRect {
            x: 0.0,
            y: 0.0,
            w: screen_width,
            h: screen_height,
        }];
        let current_clip = |stack: &Vec<ClipRect>| -> ClipRect { *stack.last().unwrap() };

        for command in commands {
            match command {
                // Transform (Push / Pop)
                DrawCommand::PushTransform { transform } => {
                    let parent = current_transform(&transform_stack);
                    transform_stack.push(parent.then(transform));
                }
                DrawCommand::PopTransform => {
                    if transform_stack.len() > 1 {
                        transform_stack.pop();
                    }
                }

                // Clip (Push / Pop)
                DrawCommand::PushClip { path, rule: _ } => {
                    let t = current_transform(&transform_stack);
                    let b = path.bounding_box().unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
                    let (x, y, w, h) = (b.x, b.y, b.width, b.height);
                    let (cx, cy) = t.apply(x, y);
                    let new_clip = ClipRect { x: cx, y: cy, w, h };

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

                // Fill
                DrawCommand::Fill {
                    path,
                    paint,
                    rule: _,
                } => {
                    let t = current_transform(&transform_stack);
                    let Some(raw_points) = path.as_polygon_vertices() else {
                        continue;
                    };

                    let transformed_points: Vec<(f32, f32)> = raw_points
                        .iter()
                        .map(|(px, py)| {
                            let (tx, ty) = t.apply(*px, *py);
                            (tx * sf, ty * sf)
                        })
                        .collect();

                    let clip = current_clip(&clip_stack);
                    let clip_l = clip.x * sf;
                    let clip_t = clip.y * sf;
                    let clip_r = (clip.x + clip.w) * sf;
                    let clip_b = (clip.y + clip.h) * sf;

                    // Quick reject by bounding box
                    let mut min_x = f32::INFINITY;
                    let mut min_y = f32::INFINITY;
                    let mut max_x = f32::NEG_INFINITY;
                    let mut max_y = f32::NEG_INFINITY;
                    for (x, y) in transformed_points.iter() {
                        min_x = min_x.min(*x);
                        min_y = min_y.min(*y);
                        max_x = max_x.max(*x);
                        max_y = max_y.max(*y);
                    }
                    if max_x <= clip_l || min_x >= clip_r || max_y <= clip_t || min_y >= clip_b {
                        continue;
                    }

                    match &paint.brush {
                        Brush::Solid(color) => {
                            let mut color_arr = color.to_linear_f32_array();
                            color_arr[3] *= paint.opacity;
                            emit_clipped_polygon_fill(
                                &mut vertices,
                                &transformed_points,
                                &clip,
                                sf,
                                screen_width,
                                screen_height,
                                color_arr,
                            );
                        }
                        Brush::Gradient(gradient) => {
                            let mut x1 = min_x;
                            let mut y1 = min_y;
                            let mut x2 = max_x;
                            let mut y2 = max_y;

                            x1 = x1.max(clip_l);
                            y1 = y1.max(clip_t);
                            x2 = x2.min(clip_r);
                            y2 = y2.min(clip_b);

                            let logical_corners = [
                                (min_x, min_y),
                                (min_x, max_y),
                                (max_x, min_y),
                                (max_x, max_y),
                            ];

                            match &gradient.kind {
                                GradientKind::Linear { .. } => {
                                    let visible_corners = [(x1, y1), (x1, y2), (x2, y1), (x2, y2)];

                                    let corner_colors = compute_gradient_corner_colors_extent(
                                        gradient,
                                        &logical_corners,
                                        &visible_corners,
                                    );
                                    let colors_lin = [
                                        corner_colors[0].to_linear_f32_array(),
                                        corner_colors[1].to_linear_f32_array(),
                                        corner_colors[2].to_linear_f32_array(),
                                        corner_colors[3].to_linear_f32_array(),
                                    ];
                                    emit_rect_vertices(
                                        &mut vertices,
                                        x1,
                                        y1,
                                        x2,
                                        y2,
                                        screen_width,
                                        screen_height,
                                        colors_lin,
                                    );
                                }
                                GradientKind::Radial { .. } => {
                                    let (cx, cy, rx, ry) =
                                        compute_radial_params(&gradient.kind, &logical_corners);
                                    emit_radial_gradient_vertices(
                                        &mut vertices,
                                        x1,
                                        y1,
                                        x2,
                                        y2,
                                        screen_width,
                                        screen_height,
                                        cx,
                                        cy,
                                        rx,
                                        ry,
                                        &gradient.stops,
                                    );
                                }
                            }
                        }
                    }
                }

                // Text
                DrawCommand::DrawText { x, y, text, style } => {
                    let t = current_transform(&transform_stack);
                    let (tdx, tdy) = (t.dx, t.dy);

                    let clip = current_clip(&clip_stack);

                    let tw = clip.w;
                    let th = clip.h;

                    let font_size = &style.font_size;

                    // Text culling: if enabled and the text's bounding box is fully outside current clip, skip creating buffer
                    let mut skip_text = false;
                    if self.enable_text_culling {
                        // compute screen-space bbox
                        let sx1 = (x + tdx) * sf;
                        let sy1 = (y + tdy) * sf;
                        // if width/height are zero or NaN, estimate from font size and line count
                        let est_w = if !tw.is_finite() || tw <= 0.0 {
                            // fall back: estimate width as font_size * 10.0 * approximate_chars
                            (*font_size * sf) * (text.len().max(1) as f32) * 0.5
                        } else {
                            tw * sf
                        };
                        let est_h = if !th.is_finite() || th <= 0.0 {
                            // estimate height as font_size * 1.2 * lines
                            (*font_size * sf) * 1.2 * (text.lines().count() as f32).max(1.0)
                        } else {
                            th * sf
                        };
                        let sx2 = sx1 + est_w;
                        let sy2 = sy1 + est_h;

                        let clip_l = clip.x * sf;
                        let clip_t = clip.y * sf;
                        let clip_r = (clip.x + clip.w) * sf;
                        let clip_b = (clip.y + clip.h) * sf;

                        if sx2 <= clip_l || sx1 >= clip_r || sy2 <= clip_t || sy1 >= clip_b {
                            skip_text = true;
                        }
                    }

                    if skip_text {
                        continue;
                    }

                    // Use TextRenderer helper to create a Buffer with correct FontSystem handling
                    let section = if let Some(tr) = &mut self.text_renderer {
                        let mut render_text_style = style.clone();
                        render_text_style.font_size = ((*font_size * sf) * 64.0).round() / 64.0;
                        let layout = tr.create_buffer_for_text(text, render_text_style);

                        TextSection {
                            screen_position: ((*x + tdx) * sf, (*y + tdy) * sf),
                            clip_origin: (clip.x * sf, clip.y * sf),
                            bounds: (tw * sf, th * sf),
                            layout,
                        }
                    } else {
                        // No text renderer available; skip
                        continue;
                    };
                    sections.push(section);
                }

                DrawCommand::SystemUi { .. } => {
                    // TODO: implement composite/render system UI element
                }
            }
        }

        self.set_vertex_buffer(vertices);

        // テキストセクションをキューに追加
        if let Some(tr) = &mut self.text_renderer {
            tr.queue(&self.device, &self.queue, &sections).unwrap();
        }
    }

    /// フレームを描画
    ///
    /// TODO:
    /// [`wgpu::CurrentSurfaceTexture`] をよりよく処理する必要があります。
    /// 現在は、 Success 時以外の結果を無視し、Errorにまとめて返す挙動をします。
    pub fn render(&mut self) -> Result<()> {
        // 描画するフレームバッファを取得
        let current_surface_texture = self.surface.get_current_texture();

        let output = if let wgpu::CurrentSurfaceTexture::Success(frame) = current_surface_texture {
            frame
        } else {
            anyhow::bail!(
                "`surface.get_current_texture` hasn't succeeded: {:?}.",
                current_surface_texture
            );
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

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
                multiview_mask: None,
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
                multiview_mask: None,
            });
            tr.draw(&mut rpass);
        }

        // コマンドをGPUに送信
        self.queue.submit(std::iter::once(encoder.finish()));

        // フレームを画面に表示
        self.queue.present(output);

        Ok(())
    }

    fn update_vertices(
        &mut self,
        old_size: winit::dpi::PhysicalSize<u32>,
        new_size: winit::dpi::PhysicalSize<u32>,
    ) {
        let old_w = old_size.width as f32;
        let old_h = old_size.height as f32;
        let new_w = new_size.width as f32;
        let new_h = new_size.height as f32;

        let mut new_vertices = self.vertices.clone();

        for vertex in new_vertices.iter_mut() {
            // old NDC -> logical
            let logical_x = (vertex.position[0] + 1.0) / 2.0 * old_w;
            let logical_y = -(vertex.position[1] - 1.0) / 2.0 * old_h;

            // logical -> new NDC
            vertex.position[0] = (logical_x / new_w) * 2.0 - 1.0;
            vertex.position[1] = -((logical_y / new_h) * 2.0 - 1.0);
        }
        self.set_vertex_buffer(new_vertices);
    }

    fn set_vertex_buffer(&mut self, vertices: Vec<Vertex>) {
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
        self.vertices = vertices;
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }
}

/// Sutherland–Hodgman polygon clipping against one axis-aligned edge of the
/// clip rectangle.
///
/// `edge` selects the clip edge in physical (scaled) coordinates:
/// `0` = left, `1` = right, `2` = top, `3` = bottom.
fn clip_against_edge(
    poly: &[(f32, f32)],
    edge: u8,
    clip_l: f32,
    clip_t: f32,
    clip_r: f32,
    clip_b: f32,
) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::new();
    if poly.is_empty() {
        return out;
    }
    let len = poly.len();
    for i in 0..len {
        let (sx, sy) = poly[i];
        let (ex, ey) = poly[(i + 1) % len];
        let inside = |x: f32, y: f32| -> bool {
            match edge {
                0 => x >= clip_l,
                1 => x <= clip_r,
                2 => y >= clip_t,
                3 => y <= clip_b,
                _ => true,
            }
        };
        let s_in = inside(sx, sy);
        let e_in = inside(ex, ey);

        // Intersection of the segment (s..e) with the selected clip edge.
        let intersect = || -> (f32, f32) {
            match edge {
                0 | 1 => {
                    let x_edge = if edge == 0 { clip_l } else { clip_r };
                    let dx = ex - sx;
                    if dx.abs() < f32::EPSILON {
                        (x_edge, sy)
                    } else {
                        let t = (x_edge - sx) / dx;
                        (x_edge, sy + t * (ey - sy))
                    }
                }
                2 | 3 => {
                    let y_edge = if edge == 2 { clip_t } else { clip_b };
                    let dy = ey - sy;
                    if dy.abs() < f32::EPSILON {
                        (sx, y_edge)
                    } else {
                        let t = (y_edge - sy) / dy;
                        (sx + t * (ex - sx), y_edge)
                    }
                }
                _ => (ex, ey),
            }
        };

        if s_in && e_in {
            out.push((ex, ey));
        } else if s_in && !e_in {
            out.push(intersect());
        } else if !s_in && e_in {
            out.push(intersect());
            out.push((ex, ey));
        }
    }
    out
}

/// Clip a polygon against the current clip rectangle, triangulate it as a fan
/// from its first vertex, and emit the resulting vertices.
///
/// `points` must be in physical (scale-factor-multiplied) screen coordinates;
/// `clip` is in logical coordinates. `color` is the linear-space fill color.
fn emit_clipped_polygon_fill(
    vertices: &mut Vec<Vertex>,
    points: &[(f32, f32)],
    clip: &ClipRect,
    sf: f32,
    screen_width: f32,
    screen_height: f32,
    color: [f32; 4],
) {
    if points.len() < 3 {
        return;
    }

    let clip_l = clip.x * sf;
    let clip_t = clip.y * sf;
    let clip_r = (clip.x + clip.w) * sf;
    let clip_b = (clip.y + clip.h) * sf;

    let ndc = |v: f32, max: f32| (v / max) * 2.0 - 1.0;

    let v0 = points[0];
    for i in 1..(points.len() - 1) {
        let tri = vec![v0, points[i], points[i + 1]];
        let mut poly = tri;
        poly = clip_against_edge(&poly, 0, clip_l, clip_t, clip_r, clip_b);
        if poly.is_empty() {
            continue;
        }
        poly = clip_against_edge(&poly, 1, clip_l, clip_t, clip_r, clip_b);
        if poly.is_empty() {
            continue;
        }
        poly = clip_against_edge(&poly, 2, clip_l, clip_t, clip_r, clip_b);
        if poly.is_empty() {
            continue;
        }
        poly = clip_against_edge(&poly, 3, clip_l, clip_t, clip_r, clip_b);
        if poly.is_empty() {
            continue;
        }

        for j in 1..(poly.len() - 1) {
            let p1 = poly[0];
            let p2 = poly[j];
            let p3 = poly[j + 1];

            let px1 = ndc(p1.0, screen_width);
            let py1 = -ndc(p1.1, screen_height);
            let px2 = ndc(p2.0, screen_width);
            let py2 = -ndc(p2.1, screen_height);
            let px3 = ndc(p3.0, screen_width);
            let py3 = -ndc(p3.1, screen_height);

            vertices.push(Vertex {
                position: [px1, py1, 0.0],
                color,
            });
            vertices.push(Vertex {
                position: [px2, py2, 0.0],
                color,
            });
            vertices.push(Vertex {
                position: [px3, py3, 0.0],
                color,
            });
        }
    }
}

/// Compute the 4 corner colors for a linear gradient rectangle.
///
/// `extent_corners` define the full gradient extent (min/max projection).
/// `sample_corners` are the actual corners to compute colors for (usually the clipped rect).
/// corners layout: [TL, BL, TR, BR] in physical (pre-NDC) screen coordinates.
fn compute_gradient_corner_colors_extent(
    gradient: &Gradient,
    extent_corners: &[(f32, f32); 4],
    sample_corners: &[(f32, f32); 4],
) -> [Color; 4] {
    let GradientKind::Linear { angle } = &gradient.kind else {
        return [Color(0, 0, 0, 0); 4];
    };
    let rad = angle.to_radians();
    let dir_x = rad.sin();
    let dir_y = -rad.cos();

    // Compute gradient extent from the full (unclipped) rectangle
    let extent_projs: Vec<f32> = extent_corners
        .iter()
        .map(|(cx, cy)| cx * dir_x + cy * dir_y)
        .collect();
    let min_p = extent_projs.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_p = extent_projs
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);
    let range = max_p - min_p;

    // Sample the gradient at each of the visible (clipped) corners
    let mut colors = [Color(0, 0, 0, 0); 4];
    for (i, (cx, cy)) in sample_corners.iter().enumerate() {
        let p = cx * dir_x + cy * dir_y;
        let t = if range > 0.0 {
            (p - min_p) / range
        } else {
            0.0
        };
        colors[i] = sample_gradient_stops(&gradient.stops, t);
    }
    colors
}

/// Returns the required ellipse radius `rx` (in the x direction) such that
/// an ellipse centered with aspect ratio `w/h` passes through a point at
/// offset `(dx, dy)` from its center.
///
/// The derivation:
///   (dx/rx)² + (dy/ry)² = 1   and   ry = rx * h/w
///   ⇒  rx² = dx² + dy² * w²/h²
fn ellipse_rx_for_corner(dx: f32, dy: f32, w: f32, h: f32) -> f32 {
    if h <= 0.0 || w <= 0.0 {
        return dx;
    }
    (dx * dx + dy * dy * (w / h) * (w / h)).sqrt()
}

fn compute_radial_params(kind: &GradientKind, corners: &[(f32, f32); 4]) -> (f32, f32, f32, f32) {
    let GradientKind::Radial {
        shape,
        size,
        position,
    } = kind
    else {
        return (0.0, 0.0, 1.0, 1.0);
    };

    let min_x = corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|c| c.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|c| c.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let w = max_x - min_x;
    let h = max_y - min_y;
    let cx = min_x + w * position.0;
    let cy = min_y + h * position.1;

    log::trace!(
        "compute_radial_params: corners=[({:.1},{:.1}),({:.1},{:.1}),({:.1},{:.1}),({:.1},{:.1})] \
         box=({:.1},{:.1}) center=({:.1},{:.1}) shape={:?} size={:?}",
        corners[0].0,
        corners[0].1,
        corners[1].0,
        corners[1].1,
        corners[2].0,
        corners[2].1,
        corners[3].0,
        corners[3].1,
        w,
        h,
        cx,
        cy,
        shape,
        size,
    );

    let (rx, ry) = match (shape, size) {
        (RadialShape::Circle, RadialSizeKind::ClosestSide) => {
            let d = (cx - min_x)
                .min(max_x - cx)
                .min((cy - min_y).min(max_y - cy));
            (d, d)
        }
        (RadialShape::Circle, RadialSizeKind::FarthestSide) => {
            let d = (cx - min_x)
                .max(max_x - cx)
                .max((cy - min_y).max(max_y - cy));
            (d, d)
        }
        (RadialShape::Circle, RadialSizeKind::ClosestCorner) => {
            let d = corners
                .iter()
                .map(|(px, py)| ((px - cx).powi(2) + (py - cy).powi(2)).sqrt())
                .fold(f32::INFINITY, f32::min);
            (d, d)
        }
        (RadialShape::Circle, RadialSizeKind::FarthestCorner) => {
            let d = corners
                .iter()
                .map(|(px, py)| ((px - cx).powi(2) + (py - cy).powi(2)).sqrt())
                .fold(0.0f32, f32::max);
            (d, d)
        }
        (RadialShape::Ellipse, RadialSizeKind::ClosestSide) => {
            let rx = (cx - min_x).min(max_x - cx);
            let ry = (cy - min_y).min(max_y - cy);
            (rx, ry)
        }
        (RadialShape::Ellipse, RadialSizeKind::FarthestSide) => {
            let rx = (cx - min_x).max(max_x - cx);
            let ry = (cy - min_y).max(max_y - cy);
            (rx, ry)
        }
        (RadialShape::Ellipse, RadialSizeKind::ClosestCorner) => {
            let mut best_rx = f32::INFINITY;
            let mut best_ry = f32::INFINITY;
            for (px, py) in corners.iter() {
                let dx = (px - cx).abs();
                let dy = (py - cy).abs();
                let rx = ellipse_rx_for_corner(dx, dy, w, h);
                let ry = rx * h / w;
                if rx < best_rx {
                    best_rx = rx;
                    best_ry = ry;
                }
            }
            (best_rx, best_ry)
        }
        (RadialShape::Ellipse, RadialSizeKind::FarthestCorner) => {
            let mut best_rx = 0.0f32;
            let mut best_ry = 0.0f32;
            for (px, py) in corners.iter() {
                let dx = (px - cx).abs();
                let dy = (py - cy).abs();
                let rx = ellipse_rx_for_corner(dx, dy, w, h);
                let ry = rx * h / w;
                if rx > best_rx {
                    best_rx = rx;
                    best_ry = ry;
                }
            }
            (best_rx, best_ry)
        }
    };

    let rx = rx.max(0.001);
    let ry = ry.max(0.001);

    log::trace!(
        "compute_radial_params: result cx={:.1} cy={:.1} rx={:.1} ry={:.1}",
        cx,
        cy,
        rx,
        ry,
    );

    (cx, cy, rx, ry)
}

fn color_at_point(cx: f32, cy: f32, rx: f32, ry: f32, px: f32, py: f32) -> f32 {
    let dx = px - cx;
    let dy = py - cy;
    ((dx / rx).powi(2) + (dy / ry).powi(2))
        .sqrt()
        .clamp(0.0, 1.0)
}

fn emit_rect_vertices(
    vertices: &mut Vec<Vertex>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    screen_width: f32,
    screen_height: f32,
    colors: [[f32; 4]; 4],
) {
    let ndc = |v: f32, max: f32| (v / max) * 2.0 - 1.0;
    let px1 = ndc(x1, screen_width);
    let py1 = -ndc(y1, screen_height);
    let px2 = ndc(x2, screen_width);
    let py2 = -ndc(y2, screen_height);

    #[rustfmt::skip]
    vertices.extend_from_slice(&[
        Vertex { position: [px1, py1, 0.0], color: colors[0] },
        Vertex { position: [px1, py2, 0.0], color: colors[1] },
        Vertex { position: [px2, py1, 0.0], color: colors[2] },
        Vertex { position: [px2, py1, 0.0], color: colors[2] },
        Vertex { position: [px1, py2, 0.0], color: colors[1] },
        Vertex { position: [px2, py2, 0.0], color: colors[3] },
    ]);
}

/// Subdivide a radial gradient rectangle into an NxN grid for proper rendering.
fn emit_radial_gradient_vertices(
    vertices: &mut Vec<Vertex>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    screen_width: f32,
    screen_height: f32,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    stops: &[ColorStop],
) {
    const SUBDIV: u32 = 32;
    let rect_w = x2 - x1;
    let rect_h = y2 - y1;
    let step_x = rect_w / SUBDIV as f32;
    let step_y = rect_h / SUBDIV as f32;

    for gy in 0..SUBDIV {
        for gx in 0..SUBDIV {
            let sx1 = x1 + gx as f32 * step_x;
            let sy1 = y1 + gy as f32 * step_y;
            let sx2 = sx1 + step_x;
            let sy2 = sy1 + step_y;

            let t_tl = color_at_point(cx, cy, rx, ry, sx1, sy1);
            let t_bl = color_at_point(cx, cy, rx, ry, sx1, sy2);
            let t_tr = color_at_point(cx, cy, rx, ry, sx2, sy1);
            let t_br = color_at_point(cx, cy, rx, ry, sx2, sy2);

            let colors = [
                sample_gradient_stops(stops, t_tl).to_linear_f32_array(),
                sample_gradient_stops(stops, t_bl).to_linear_f32_array(),
                sample_gradient_stops(stops, t_tr).to_linear_f32_array(),
                sample_gradient_stops(stops, t_br).to_linear_f32_array(),
            ];

            emit_rect_vertices(
                vertices,
                sx1,
                sy1,
                sx2,
                sy2,
                screen_width,
                screen_height,
                colors,
            );
        }
    }
}

/// Sample a gradient's color stops at normalized position `t` (0.0–1.0).
fn sample_gradient_stops(stops: &[ColorStop], t: f32) -> Color {
    if stops.is_empty() {
        return Color(0, 0, 0, 0);
    }
    if stops.len() == 1 {
        return stops[0].color;
    }

    let t = t.clamp(0.0, 1.0);
    let last = stops.len() - 1;

    for i in 0..last {
        let pos_i = stops[i]
            .position
            .unwrap_or(if i == 0 { 0.0 } else { i as f32 / last as f32 });
        let pos_j = stops[i + 1].position.unwrap_or(if i + 1 == last {
            1.0
        } else {
            (i + 1) as f32 / last as f32
        });

        if t >= pos_i && t <= pos_j {
            let local = if pos_j > pos_i {
                (t - pos_i) / (pos_j - pos_i)
            } else {
                0.0
            };
            return lerp_color(stops[i].color, stops[i + 1].color, local);
        }
    }

    if t <= stops[0].position.unwrap_or(0.0) {
        return stops[0].color;
    }
    stops[last].color
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    // Interpolate in linear RGB space for correct color mixing
    let al = a.to_linear_f32_array();
    let bl = b.to_linear_f32_array();
    Color::from_linear_f32_array([
        al[0] + (bl[0] - al[0]) * t,
        al[1] + (bl[1] - al[1]) * t,
        al[2] + (bl[2] - al[2]) * t,
        al[3] + (bl[3] - al[3]) * t,
    ])
}

fn select_wgpu_backends() -> wgpu::Backends {
    if let Ok(value) = env::var("ORINIUM_WGPU_BACKEND") {
        match value.to_lowercase().as_str() {
            "gl" | "opengl" => return wgpu::Backends::GL,
            "vulkan" | "vk" => return wgpu::Backends::VULKAN,
            "metal" => return wgpu::Backends::METAL,
            "dx12" | "d3d12" => return wgpu::Backends::DX12,
            "primary" => return wgpu::Backends::PRIMARY,
            _ => {}
        }
    }

    let is_wsl = env::var_os("WSL_DISTRO_NAME").is_some() || env::var_os("WSL_INTEROP").is_some();
    let is_wayland = env::var_os("WAYLAND_DISPLAY").is_some();

    if is_wsl && is_wayland {
        // WSLg + Wayland can be unstable with Vulkan; prefer GL by default.
        return wgpu::Backends::GL;
    }

    wgpu::Backends::PRIMARY
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::layouter::types::{
        ColorStop, Gradient, GradientKind, RadialShape, RadialSizeKind,
    };

    // ── ellipse_rx_for_corner ──────────────────────────────────────────

    #[test]
    fn test_ellipse_rx_for_corner_centered() {
        // Box 200x100, center (100,50), corner (200,100)
        // dx=100, dy=50, w=200, h=100
        // rx = sqrt(100^2 + 50^2 * (200/100)^2) = sqrt(10000 + 2500*4) = sqrt(20000) ≈ 141.42
        let rx = ellipse_rx_for_corner(100.0, 50.0, 200.0, 100.0);
        let expected = (20000.0f32).sqrt();
        assert!(
            (rx - expected).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected
        );
    }

    #[test]
    fn test_ellipse_rx_for_corner_square() {
        // Box 100x100, center (0,0), corner (100,100)
        // dx=100, dy=100, w=100, h=100
        // rx = sqrt(100^2 + 100^2 * (100/100)^2) = sqrt(20000) ≈ 141.42
        let rx = ellipse_rx_for_corner(100.0, 100.0, 100.0, 100.0);
        let expected = (20000.0f32).sqrt();
        assert!(
            (rx - expected).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected
        );
    }

    #[test]
    fn test_ellipse_rx_for_corner_zero_box() {
        // Degenerate box: h=0
        assert_eq!(ellipse_rx_for_corner(50.0, 30.0, 100.0, 0.0), 50.0);
        assert_eq!(ellipse_rx_for_corner(50.0, 30.0, 0.0, 100.0), 50.0);
    }

    // ── color_at_point ─────────────────────────────────────────────────

    #[test]
    fn test_color_at_point_center() {
        let d = color_at_point(100.0, 100.0, 50.0, 50.0, 100.0, 100.0);
        assert!((d - 0.0).abs() < 1e-6, "d={}", d);
    }

    #[test]
    fn test_color_at_point_on_edge() {
        let d = color_at_point(100.0, 100.0, 50.0, 50.0, 150.0, 100.0);
        assert!((d - 1.0).abs() < 1e-6, "d={}", d);
    }

    #[test]
    fn test_color_at_point_outside() {
        let d = color_at_point(100.0, 100.0, 50.0, 50.0, 200.0, 200.0);
        assert!((d - 1.0).abs() < 1e-6, "d={}", d);
    }

    #[test]
    fn test_color_at_point_ellipse() {
        // Point (50, 25) on ellipse rx=100, ry=50
        // distance = sqrt((50/100)^2 + (25/50)^2) = sqrt(0.25+0.25) = sqrt(0.5) ≈ 0.707
        let d = color_at_point(0.0, 0.0, 100.0, 50.0, 50.0, 25.0);
        assert!((d - (0.5f32).sqrt()).abs() < 1e-6, "d={}", d);
    }

    // ── compute_radial_params ──────────────────────────────────────────

    fn make_radial(shape: RadialShape, size: RadialSizeKind, position: (f32, f32)) -> GradientKind {
        GradientKind::Radial {
            shape,
            size,
            position,
        }
    }

    fn corners_from_rect(x: f32, y: f32, w: f32, h: f32) -> [(f32, f32); 4] {
        [(x, y), (x, y + h), (x + w, y), (x + w, y + h)]
    }

    #[test]
    fn test_radial_circle_closest_side_centered() {
        // 200x100 box, center (100,50)
        let kind = make_radial(RadialShape::Circle, RadialSizeKind::ClosestSide, (0.5, 0.5));
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        // closest side = min(100, 100, 50, 50) = 50
        assert!((rx - 50.0).abs() < 0.01, "rx={}", rx);
        assert!((ry - 50.0).abs() < 0.01, "ry={}", ry);
    }

    #[test]
    fn test_radial_circle_farthest_corner_centered() {
        // 200x100 box, center (100,50). Distance to any corner = sqrt(100^2+50^2) ≈ 111.80
        let kind = make_radial(
            RadialShape::Circle,
            RadialSizeKind::FarthestCorner,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected = ((100.0f32 * 100.0 + 50.0 * 50.0) as f32).sqrt();
        assert!(
            (rx - expected).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected
        );
        assert!((ry - expected).abs() < 0.01, "ry={}", ry);
    }

    #[test]
    fn test_radial_ellipse_closest_side_centered() {
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::ClosestSide,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        assert!((rx - 100.0).abs() < 0.01, "rx={}", rx);
        assert!((ry - 50.0).abs() < 0.01, "ry={}", ry);
    }

    #[test]
    fn test_radial_ellipse_farthest_side_centered() {
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::FarthestSide,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        assert!((rx - 100.0).abs() < 0.01, "rx={}", rx);
        assert!((ry - 50.0).abs() < 0.01, "ry={}", ry);
    }

    #[test]
    fn test_radial_ellipse_closest_corner_centered() {
        // All corners equidistant from center: rx = sqrt(100^2 + 50^2 * (200/100)^2) = sqrt(20000) ≈ 141.42
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::ClosestCorner,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected_rx = (20000.0f32).sqrt();
        let expected_ry = expected_rx * 100.0 / 200.0;
        assert!(
            (rx - expected_rx).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected_rx
        );
        assert!(
            (ry - expected_ry).abs() < 0.01,
            "ry={} expected={}",
            ry,
            expected_ry
        );
    }

    #[test]
    fn test_radial_ellipse_farthest_corner_centered() {
        // Same as closest-corner for centered position (all corners equidistant)
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::FarthestCorner,
            (0.5, 0.5),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected_rx = (20000.0f32).sqrt();
        let expected_ry = expected_rx * 100.0 / 200.0;
        assert!(
            (rx - expected_rx).abs() < 0.01,
            "rx={} expected={}",
            rx,
            expected_rx
        );
        assert!(
            (ry - expected_ry).abs() < 0.01,
            "ry={} expected={}",
            ry,
            expected_ry
        );
    }

    #[test]
    fn test_radial_ellipse_closest_corner_offset() {
        // Box 200x100, position (0.2, 0.3) → center at (40, 30)
        // Closest corner = TL (0,0): dx=40, dy=30
        // rx = sqrt(40^2 + 30^2 * (200/100)^2) = sqrt(1600 + 900*4) = sqrt(5200) ≈ 72.11
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::ClosestCorner,
            (0.2, 0.3),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected_rx = (5200.0f32).sqrt();
        let expected_ry = expected_rx * 100.0 / 200.0;
        assert!(
            (rx - expected_rx).abs() < 0.1,
            "rx={} expected={}",
            rx,
            expected_rx
        );
        assert!(
            (ry - expected_ry).abs() < 0.1,
            "ry={} expected={}",
            ry,
            expected_ry
        );
    }

    #[test]
    fn test_radial_ellipse_farthest_corner_offset() {
        // Box 200x100, position (0.2, 0.3) → center at (40, 30)
        // Farthest corner = BR (200,100): dx=160, dy=70
        // rx = sqrt(160^2 + 70^2 * (200/100)^2) = sqrt(25600 + 4900*4) = sqrt(45200) ≈ 212.60
        let kind = make_radial(
            RadialShape::Ellipse,
            RadialSizeKind::FarthestCorner,
            (0.2, 0.3),
        );
        let (_cx, _cy, rx, ry) =
            compute_radial_params(&kind, &corners_from_rect(0.0, 0.0, 200.0, 100.0));
        let expected_rx = (45200.0f32).sqrt();
        let expected_ry = expected_rx * 100.0 / 200.0;
        assert!(
            (rx - expected_rx).abs() < 0.1,
            "rx={} expected={}",
            rx,
            expected_rx
        );
        assert!(
            (ry - expected_ry).abs() < 0.1,
            "ry={} expected={}",
            ry,
            expected_ry
        );
    }

    // ── sample_gradient_stops / lerp_color ─────────────────────────────

    #[test]
    fn test_lerp_color_srgb_vs_linear() {
        // Interpolation between red and blue at t=0.5 should differ
        // between sRGB and linear space.
        let red = Color(255, 0, 0, 255);
        let blue = Color(0, 0, 255, 255);
        let mixed = lerp_color(red, blue, 0.5);
        // In sRGB space: (128, 0, 128)
        // In linear space: ~ (188, 0, 188)  (perceptually mid-way)
        assert!(
            mixed.0 > 128,
            "R should be >128 in linear lerp, got {}",
            mixed.0
        );
        assert!(
            mixed.2 > 128,
            "B should be >128 in linear lerp, got {}",
            mixed.2
        );
    }

    #[test]
    fn test_sample_gradient_single_stop() {
        let stops = vec![ColorStop {
            color: Color(255, 0, 0, 255),
            position: None,
        }];
        let c = sample_gradient_stops(&stops, 0.5);
        assert_eq!(c, Color(255, 0, 0, 255));
    }

    #[test]
    fn test_sample_gradient_two_stops() {
        let stops = vec![
            ColorStop {
                color: Color(255, 0, 0, 255),
                position: None,
            },
            ColorStop {
                color: Color(0, 0, 255, 255),
                position: None,
            },
        ];
        let c0 = sample_gradient_stops(&stops, 0.0);
        assert_eq!(c0, Color(255, 0, 0, 255));
        let c1 = sample_gradient_stops(&stops, 1.0);
        assert_eq!(c1, Color(0, 0, 255, 255));
        // Middle should be a linear-space blend of red and blue
        let mid = sample_gradient_stops(&stops, 0.5);
        assert!(mid.0 > 0 && mid.0 < 255);
        assert!(mid.2 > 0 && mid.2 < 255);
    }

    // ── compute_gradient_corner_colors_extent ──────────────────────────

    #[test]
    fn test_linear_gradient_extent_clipped() {
        let gradient = Gradient {
            kind: GradientKind::Linear { angle: 90.0 }, // left→right
            stops: vec![
                ColorStop {
                    color: Color(255, 0, 0, 255),
                    position: None,
                },
                ColorStop {
                    color: Color(0, 0, 255, 255),
                    position: None,
                },
            ],
        };
        // Full rect: x=0, y=0, w=200, h=100
        let full = [(0.0, 0.0), (0.0, 100.0), (200.0, 0.0), (200.0, 100.0)];
        // Visible rect: right portion x=120..200 (t=0.6..1.0), all corners should be more blue than red
        let visible = [(120.0, 0.0), (120.0, 100.0), (200.0, 0.0), (200.0, 100.0)];

        let colors = compute_gradient_corner_colors_extent(&gradient, &full, &visible);
        for (i, c) in colors.iter().enumerate() {
            assert!(
                c.2 > c.0,
                "Corner {} should be more blue (got r={}, b={})",
                i,
                c.0,
                c.2
            );
        }
        // TL/BL should be at t=0.6 (more red than TR/BR at t=1.0)
        assert!(colors[0].0 > colors[2].0, "TL should have more red than TR");
    }

    #[test]
    fn test_linear_gradient_extent_unclipped() {
        let gradient = Gradient {
            kind: GradientKind::Linear { angle: 90.0 },
            stops: vec![
                ColorStop {
                    color: Color(255, 0, 0, 255),
                    position: None,
                },
                ColorStop {
                    color: Color(0, 0, 255, 255),
                    position: None,
                },
            ],
        };
        let corners = [(0.0, 0.0), (0.0, 100.0), (200.0, 0.0), (200.0, 100.0)];
        let colors = compute_gradient_corner_colors_extent(&gradient, &corners, &corners);
        // TL/BL should be red, TR/BR should be blue
        assert_eq!(colors[0], Color(255, 0, 0, 255)); // TL
        assert_eq!(colors[1], Color(255, 0, 0, 255)); // BL
        assert_eq!(colors[2], Color(0, 0, 255, 255)); // TR
        assert_eq!(colors[3], Color(0, 0, 255, 255)); // BR
    }

    // ── clip_against_edge / emit_clipped_polygon_fill ───────────────────

    #[test]
    fn test_clip_against_edge_left_keeps_points_inside() {
        let poly = [(10.0, 0.0), (20.0, 0.0), (20.0, 10.0), (10.0, 10.0)];
        let clipped = clip_against_edge(&poly, 0, 5.0, 0.0, 100.0, 100.0);
        assert_eq!(clipped.len(), poly.len());
        for (x, _) in &clipped {
            assert!(*x >= 5.0);
        }
    }

    #[test]
    fn test_emit_clipped_polygon_fill_fully_inside() {
        let mut vertices = Vec::new();
        let points = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        let clip = ClipRect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        emit_clipped_polygon_fill(&mut vertices, &points, &clip, 1.0, 200.0, 200.0, [1.0; 4]);
        // Two triangles for the quad.
        assert_eq!(vertices.len(), 6);
    }

    #[test]
    fn test_emit_clipped_polygon_fill_fully_outside() {
        let mut vertices = Vec::new();
        let points = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let clip = ClipRect {
            x: 20.0,
            y: 20.0,
            w: 10.0,
            h: 10.0,
        };
        emit_clipped_polygon_fill(&mut vertices, &points, &clip, 1.0, 200.0, 200.0, [1.0; 4]);
        assert!(vertices.is_empty());
    }

    #[test]
    fn test_emit_clipped_polygon_fill_partially_inside() {
        let mut vertices = Vec::new();
        let points = [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 50.0)];
        let clip = ClipRect {
            x: 10.0,
            y: 10.0,
            w: 30.0,
            h: 30.0,
        };
        emit_clipped_polygon_fill(&mut vertices, &points, &clip, 1.0, 100.0, 100.0, [1.0; 4]);
        assert!(!vertices.is_empty());
        assert_eq!(vertices.len() % 3, 0);
        for v in &vertices {
            // Recover screen coordinates from NDC and ensure they stay within
            // the clip rect [10, 40].
            let sx = (v.position[0] + 1.0) / 2.0 * 100.0;
            let sy = -(v.position[1] - 1.0) / 2.0 * 100.0;
            assert!((10.0 - 1e-3..=40.0 + 1e-3).contains(&sx), "sx={sx}");
            assert!((10.0 - 1e-3..=40.0 + 1e-3).contains(&sy), "sy={sy}");
        }
    }

    // ── AffineTransform ─────────────────────────────────────────────────

    #[test]
    fn test_affine_transform_composition_and_inverse() {
        let t = AffineTransform::translate(3.0, 4.0).then(&AffineTransform::translate(5.0, 6.0));
        assert_eq!(t.apply(1.0, 1.0), (9.0, 11.0));

        let inv = t.inverse().unwrap();
        let (x, y) = t.apply(10.0, 20.0);
        let (rx, ry) = inv.apply(x, y);
        assert!((rx - 10.0).abs() < 1e-5, "rx={rx}");
        assert!((ry - 20.0).abs() < 1e-5, "ry={ry}");
    }
}
