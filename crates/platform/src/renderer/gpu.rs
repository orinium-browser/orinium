//! wgpuを使用してGPUで描画するためのコンテキストと処理を提供するモジュール

use anyhow::Result;
use engine::renderer_model::DrawCommand;
use std::env;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

use super::image::ImageRenderer;
use super::mesh::{self, MeshBuilder, Vertex};
use super::text::global_font;
use super::text::text_renderer::TextRenderer;

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
    /// 描画命令から頂点・テキストセクションを生成するジオメトリ層
    mesh_builder: MeshBuilder,
    /// コマンド順を保持する描画項目
    draw_items: Vec<mesh::DrawItem>,

    /// テキスト描画用ラッパー
    text_renderer: Option<TextRenderer>,

    /// Decoded page image renderer.
    image_renderer: ImageRenderer,

    /// テキストカリングを有効にする
    enable_text_culling: bool,
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
        let image_renderer = ImageRenderer::new(&device, config.format);

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
            mesh_builder: MeshBuilder::new(
                size.width as f32,
                size.height as f32,
                scale_factor as f32,
            ),
            draw_items: Vec::new(),
            text_renderer,
            image_renderer,
            enable_text_culling,
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
    pub fn parse_draw_commands(&mut self, commands: &[DrawCommand]) {
        self.mesh_builder
            .set_screen_size(self.size.width as f32, self.size.height as f32);
        self.mesh_builder.set_scale_factor(self.scale_factor as f32);
        self.mesh_builder.set_text_culling(self.enable_text_culling);

        let mut text_source: Option<&mut dyn mesh::TextLayoutSource> = match &mut self.text_renderer
        {
            Some(tr) => Some(tr),
            None => None,
        };
        let built = self.mesh_builder.build(commands, &mut text_source);
        let vertices = built.vertices.clone();
        let sections = built.sections.clone();
        let images = built.images.clone();
        self.draw_items.clone_from(&built.draw_items);

        self.set_vertex_buffer(&vertices);
        self.image_renderer.prepare(
            &self.device,
            &self.queue,
            &images,
            self.size.width as f32,
            self.size.height as f32,
        );

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

            self.draw_in_order(&mut render_pass);
        }

        // コマンドをGPUに送信
        self.queue.submit(std::iter::once(encoder.finish()));

        // フレームを画面に表示
        self.queue.present(output);

        Ok(())
    }

    /// Draws every [`mesh::DrawItem`] in command order within a single render
    /// pass, merging adjacent same-kind items to minimize pipeline switches.
    fn draw_in_order<'a>(&mut self, render_pass: &mut wgpu::RenderPass<'a>) {
        let mut i = 0;
        while i < self.draw_items.len() {
            match self.draw_items[i] {
                mesh::DrawItem::Fill {
                    vertex_start,
                    vertex_count,
                } => {
                    let start = vertex_start;
                    let mut count = vertex_count;
                    i += 1;
                    while i < self.draw_items.len() {
                        if let mesh::DrawItem::Fill {
                            vertex_start: s,
                            vertex_count: c,
                        } = self.draw_items[i]
                        {
                            if s == start + count {
                                count += c;
                                i += 1;
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    if let Some(ref vertex_buffer) = self.vertex_buffer {
                        render_pass.set_pipeline(&self.render_pipeline);
                        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                        render_pass.draw(start..start + count, 0..1);
                    }
                }
                mesh::DrawItem::Image(index) => {
                    self.image_renderer.draw_at(render_pass, index);
                    i += 1;
                }
                mesh::DrawItem::Text(_) => {
                    let mut start = 0u32;
                    let mut end = 0u32;
                    let mut any = false;
                    while i < self.draw_items.len() {
                        if let mesh::DrawItem::Text(idx) = self.draw_items[i] {
                            if let Some(tr) = &self.text_renderer
                                && let Some((s, c)) = tr.section_range(idx)
                            {
                                if !any {
                                    start = s;
                                    any = true;
                                }
                                end = s + c;
                            }
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    if let Some(tr) = &mut self.text_renderer
                        && any
                        && end > start
                    {
                        tr.draw_range(render_pass, start, end - start);
                    }
                }
            }
        }
    }

    fn set_vertex_buffer(&mut self, vertices: &[Vertex]) {
        // 頂点バッファを登録
        if !vertices.is_empty() {
            self.vertex_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Vertex Buffer"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
        } else {
            self.vertex_buffer = None;
        }
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }
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
