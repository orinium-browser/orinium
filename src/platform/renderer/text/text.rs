use bytemuck::{Pod, Zeroable};
use orinium_text::{
    Color as OriColor, FontStyle as OriFontStyle, FontSystem, FontWeight as OriFontWeight,
    TextLayouter, TextStyle as OriTextStyle, fontdb,
};
use wgpu::util::DeviceExt;

use super::atlas::GlyphAtlas;
use crate::engine::layouter::types::{FontStyle, LineHeight, TextStyle};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct GlyphVertex {
    position: [f32; 2],
    tex_coord: [f32; 2],
    layer: f32,
    color: [f32; 4],
}

/// テキストセクション位置・クリップ・描画範囲をまとめた構造体
pub struct TextSection {
    pub screen_position: (f32, f32),
    pub clip_origin: (f32, f32),
    pub bounds: (f32, f32),
    pub layout: orinium_text::TextLayout,
}

/// テキストレンダラー (orinium_text + wgpu グリフアトラス)
pub struct TextRenderer {
    font_sys: FontSystem,
    layouter: TextLayouter,
    atlas: GlyphAtlas,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    sampler: wgpu::Sampler,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    num_indices: u32,
    vertices: Vec<GlyphVertex>,
    indices: Vec<u32>,
    screen_width: f32,
    screen_height: f32,
}

impl TextRenderer {
    /// システムフォントから初期化する
    pub fn new_from_device(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        if let Ok(p) = std::env::var("ORINIUM_FONT")
            && let Ok(bytes) = std::fs::read(&p)
        {
            return Self::new_from_bytes(device, queue, format, bytes);
        }

        for p in crate::platform::font::system_font_candidates()? {
            if let Ok(bytes) = std::fs::read(p) {
                return Self::new_from_bytes(device, queue, format, bytes);
            }
        }

        anyhow::bail!("no system font found");
    }

    /// FontSystem を受け取って生成する
    pub fn new_with_fontsys(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_sys: FontSystem,
    ) -> anyhow::Result<Self> {
        let atlas = GlyphAtlas::new(device);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Glyph Atlas Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Text Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Text Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shader/text.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Text Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Text Render Pipeline"),
            layout: Some(&pipeline_layout),
            cache: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: size_of::<GlyphVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: size_of::<[f32; 2]>() as wgpu::BufferAddress,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: (size_of::<[f32; 2]>() * 2) as wgpu::BufferAddress,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: (size_of::<[f32; 2]>() * 2 + size_of::<f32>())
                                as wgpu::BufferAddress,
                            shader_location: 3,
                        },
                    ],
                }],
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
            multiview_mask: None,
        });

        let layouter = TextLayouter::new();

        Ok(Self {
            font_sys,
            layouter,
            atlas,
            pipeline,
            bind_group_layout,
            bind_group: None,
            sampler,
            vertex_buffer: None,
            index_buffer: None,
            num_indices: 0,
            vertices: Vec::new(),
            indices: Vec::new(),
            screen_width: 800.0,
            screen_height: 600.0,
        })
    }

    /// フォントバイト列から生成する
    pub fn new_from_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        font_bytes: Vec<u8>,
    ) -> anyhow::Result<Self> {
        let font_sys = FontSystem::new_with_fonts(vec![font_bytes]);
        Self::new_with_fontsys(device, queue, format, font_sys)
    }

    /// Create an orinium_text `TextLayout` for the given text using the internal `FontSystem`.
    pub fn create_buffer_for_text(
        &mut self,
        text: &str,
        style: TextStyle,
    ) -> orinium_text::TextLayout {
        let font_size = style.font_size;
        let line_height_ratio = match style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n,
            LineHeight::Px(px) => px / font_size,
        };

        let ori_style = OriTextStyle {
            font_size,
            color: OriColor(style.color.0, style.color.1, style.color.2, style.color.3),
            font_weight: OriFontWeight(style.font_weight.0),
            font_style: match style.font_style {
                FontStyle::Normal => OriFontStyle::Normal,
                FontStyle::Italic => OriFontStyle::Italic,
                FontStyle::Oblique => OriFontStyle::Oblique,
            },
            line_height: line_height_ratio,
            bidi_mode: orinium_text::BidiMode::Auto,
            font_families: vec![fontdb::Family::SansSerif],
            exact_fonts: self.font_sys.font_keys(),
        };

        let shaped = self
            .layouter
            .shape_text(&mut self.font_sys, text, &ori_style);

        let line_ranges: Vec<(usize, usize)> = text
            .split('\n')
            .scan(0usize, |offset, line| {
                let start = *offset;
                *offset += line.len() + 1;
                let end = start + line.len();
                Some((start, end))
            })
            .collect();

        self.layouter
            .layout_lines(&mut self.font_sys, &shaped, &line_ranges, &ori_style)
    }

    pub fn resize_view(&mut self, width: f32, height: f32, _queue: &wgpu::Queue) {
        self.screen_width = width;
        self.screen_height = height;
    }

    /// 指定されたセクション群をギリフォン用の TextArea に変換して Atlas に転送する
    pub fn queue(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: &[TextSection],
    ) -> anyhow::Result<()> {
        self.vertices.clear();
        self.indices.clear();

        let ndc_x = |sx: f32| (sx / self.screen_width) * 2.0 - 1.0;
        let ndc_y = |sy: f32| -((sy / self.screen_height) * 2.0 - 1.0);

        for section in sections {
            let layout = &section.layout;

            let base_x = section.screen_position.0;
            let base_y = section.screen_position.1;

            let clip_l = section.clip_origin.0;
            let clip_t = section.clip_origin.1;
            let clip_r = section.clip_origin.0 + section.bounds.0;
            let clip_b = section.clip_origin.1 + section.bounds.1;

            for line in &layout.lines {
                for glyph in &line.glyphs {
                    let Some(font_key) = glyph.font_key else {
                        continue;
                    };

                    let gx = base_x + glyph.x;
                    let gy = base_y + glyph.y;
                    let gw = glyph.width;
                    let gh = glyph.height;

                    // Compute visible portion against clip rect
                    let vis_l = gx.max(clip_l);
                    let vis_t = gy.max(clip_t);
                    let vis_r = (gx + gw).min(clip_r);
                    let vis_b = (gy + gh).min(clip_b);

                    if vis_l >= vis_r || vis_t >= vis_b {
                        continue;
                    }

                    let (layer, u, v, uw, uh) =
                        match self.atlas.lookup(font_key, glyph.glyph_id, glyph.font_size) {
                            Some(uv) => uv,
                            None => {
                                if let Some((metrics, alpha_mask)) =
                                    self.font_sys.get_or_rasterize_with_bitmap(
                                        font_key,
                                        glyph.glyph_id,
                                        glyph.font_size,
                                    )
                                {
                                    if metrics.width == 0 || metrics.height == 0 {
                                        continue;
                                    }
                                    let mask_w = metrics.width;
                                    let mask_h = metrics.height;
                                    self.atlas.upload(
                                        device,
                                        queue,
                                        font_key,
                                        glyph.glyph_id,
                                        glyph.font_size,
                                        &alpha_mask,
                                        mask_w,
                                        mask_h,
                                    )
                                } else {
                                    continue;
                                }
                            }
                        };

                    // Convert visible rect to NDC
                    let quad_x1 = ndc_x(vis_l);
                    let quad_y1 = ndc_y(vis_t);
                    let quad_x2 = ndc_x(vis_r);
                    let quad_y2 = ndc_y(vis_b);

                    // NDC Y is flipped: upper-left becomes min, lower-right becomes max
                    let (qy1, qy2) = if quad_y1 < quad_y2 {
                        (quad_y1, quad_y2)
                    } else {
                        (quad_y2, quad_y1)
                    };

                    // Compute UVs for the visible portion
                    let u0 = u + (vis_l - gx) / gw * uw;
                    let u1 = u + (vis_r - gx) / gw * uw;
                    let v0 = v + (vis_b - gy) / gh * uh;
                    let v1 = v + (vis_t - gy) / gh * uh;

                    let idx = self.vertices.len() as u32;
                    let layer_f = layer as f32;

                    let color_arr = crate::engine::layouter::types::Color(
                        glyph.color.0,
                        glyph.color.1,
                        glyph.color.2,
                        glyph.color.3,
                    )
                    .to_linear_f32_array();

                    self.vertices.extend_from_slice(&[
                        GlyphVertex {
                            position: [quad_x1, qy1],
                            tex_coord: [u0, v0],
                            layer: layer_f,
                            color: color_arr,
                        },
                        GlyphVertex {
                            position: [quad_x2, qy1],
                            tex_coord: [u1, v0],
                            layer: layer_f,
                            color: color_arr,
                        },
                        GlyphVertex {
                            position: [quad_x2, qy2],
                            tex_coord: [u1, v1],
                            layer: layer_f,
                            color: color_arr,
                        },
                        GlyphVertex {
                            position: [quad_x1, qy2],
                            tex_coord: [u0, v1],
                            layer: layer_f,
                            color: color_arr,
                        },
                    ]);

                    self.indices
                        .extend_from_slice(&[idx, idx + 1, idx + 2, idx, idx + 2, idx + 3]);
                }
            }
        }

        self.atlas.flush_uploads(queue);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Text Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(self.atlas.texture_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.bind_group = Some(bind_group);

        self.num_indices = self.indices.len() as u32;
        if !self.vertices.is_empty() {
            self.vertex_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Text Vertex Buffer"),
                    contents: bytemuck::cast_slice(&self.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ));
        } else {
            self.vertex_buffer = None;
        }
        if !self.indices.is_empty() {
            self.index_buffer = Some(device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Text Index Buffer"),
                    contents: bytemuck::cast_slice(&self.indices),
                    usage: wgpu::BufferUsages::INDEX,
                },
            ));
        } else {
            self.index_buffer = None;
        }

        Ok(())
    }

    /// テキストをレンダリングする
    pub fn draw<'a>(&mut self, rpass: &mut wgpu::RenderPass<'a>) {
        let Some(ref bind_group) = self.bind_group else {
            return;
        };
        let Some(ref vertex_buffer) = self.vertex_buffer else {
            return;
        };
        let Some(ref index_buffer) = self.index_buffer else {
            return;
        };
        if self.num_indices == 0 {
            return;
        }

        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.set_vertex_buffer(0, vertex_buffer.slice(..));
        rpass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(0..self.num_indices, 0, 0..1);
    }
}
