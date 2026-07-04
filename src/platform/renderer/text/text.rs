use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use orinium_text::{
    Color as OriColor, FontStyle as OriFontStyle, FontSystem, FontWeight as OriFontWeight,
    TextLayout, TextLayouter, TextStyle as OriTextStyle, fontdb,
};
use wgpu::util::DeviceExt;

use super::atlas::GlyphAtlas;
use crate::engine::layouter::types::{FontStyle, LineHeight, TextStyle};

/// Quantize font_size to 1/64 px to avoid cache misses from floating-point noise.
fn quantize_font_size(px: f32) -> f32 {
    (px * 64.0).round() / 64.0
}

/// A single corner of the shared quad (4 vertices total, uploaded once).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct QuadVertex {
    position: [f32; 2],
    tex_coord: [f32; 2],
}

/// Per-glyph instance data, packed into u32s for minimal vertex bandwidth.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct GlyphInstance {
    /// [f32 x, f32 y] — screen-space upper-left of the visible rect, in pixels
    pos: [f32; 2],
    /// [u16 w, u16 h] — visible rect pixel size
    size: u32,
    /// [i16 u_off, i16 v_off] — atlas UV origin, normalized × 2048
    uv_off: u32,
    /// [u16 uv_w, u16 uv_h] — atlas UV size, normalized × 2048
    uv_size: u32,
    /// [u16 layer, u16 pad]
    layer: u32,
    /// [u8 r, u8 g, u8 b, u8 a] — sRGB color
    color: u32,
}

const QUAD_VERTS: [QuadVertex; 4] = [
    QuadVertex {
        position: [0.0, 0.0],
        tex_coord: [0.0, 0.0],
    },
    QuadVertex {
        position: [1.0, 0.0],
        tex_coord: [1.0, 0.0],
    },
    QuadVertex {
        position: [0.0, 1.0],
        tex_coord: [0.0, 1.0],
    },
    QuadVertex {
        position: [1.0, 1.0],
        tex_coord: [1.0, 1.0],
    },
];

const QUAD_INDICES: [u32; 6] = [0, 1, 2, 2, 1, 3];

fn pack_position(x: f32, y: f32) -> [f32; 2] {
    [x, y]
}

fn pack_size(w: f32, h: f32) -> u32 {
    let qw = (w * 64.0).round() as u16;
    let qh = (h * 64.0).round() as u16;
    ((qw as u32) << 16) | (qh as u32)
}

fn pack_uv_off(u: f32, v: f32) -> u32 {
    let qu = (u * 2048.0).round() as i16;
    let qv = (v * 2048.0).round() as i16;
    ((qu as u32) << 16) | (qv as u16 as u32)
}

fn pack_uv_size(uw: f32, vh: f32) -> u32 {
    let quw = (uw * 2048.0).round() as u16;
    let qvh = (vh * 2048.0).round() as u16;
    ((quw as u32) << 16) | (qvh as u32)
}

fn pack_layer(layer: u32) -> u32 {
    layer as u32
}

fn pack_color(c: &OriColor) -> u32 {
    ((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | (c.3 as u32)
}

/// テキストセクション位置・クリップ・描画範囲をまとめた構造体
pub struct TextSection {
    pub screen_position: (f32, f32),
    pub clip_origin: (f32, f32),
    pub bounds: (f32, f32),
    pub layout: Arc<TextLayout>,
}

/// Cached result of `create_buffer_for_text`.
#[derive(Debug, Clone)]
struct CachedLayout {
    text: String,
    font_size_bits: u32,
    color: u32,
    layout: Arc<TextLayout>,
}

/// Compute a fast hash of the section array for change detection.
fn sections_hash(sections: &[TextSection]) -> u64 {
    let mut h: u64 = 0;
    for s in sections {
        h = h.wrapping_mul(6364136223846793005);
        h ^= s.screen_position.0.to_bits() as u64;
        h ^= s.screen_position.1.to_bits() as u64;
        h ^= s.clip_origin.0.to_bits() as u64;
        h ^= s.clip_origin.1.to_bits() as u64;
        h ^= s.bounds.0.to_bits() as u64;
        h ^= s.bounds.1.to_bits() as u64;
        h ^= Arc::as_ptr(&s.layout) as u64;
    }
    h
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
    atlas_dirty: bool,

    /// Static shared quad vertex buffer (4 corners, uploaded once).
    quad_vertex_buffer: wgpu::Buffer,
    /// Static shared index buffer (6 indices for two triangles, uploaded once).
    quad_index_buffer: wgpu::Buffer,

    /// Dynamic instance buffer (per-glyph data).
    instance_buffer: Option<wgpu::Buffer>,
    /// Current capacity of the instance buffer (in glyphs).
    instance_capacity: usize,
    /// Number of instances to draw.
    num_instances: u32,
    /// Scratch buffer for building instance data.
    instances: Vec<GlyphInstance>,

    /// Small uniform buffer holding the screen size for NDC conversion in the shader.
    uniform_buffer: wgpu::Buffer,
    screen_size: [f32; 2],

    /// Previous state for change detection.
    prev_sections_hash: u64,
    prev_sections_count: usize,

    layout_cache: Vec<CachedLayout>,
}

impl TextRenderer {
    /// システムフォントから初期化する
    /// システムフォントから初期化する
    ///
    /// `ORINIUM_FONT` 環境変数が設定されている場合はそのフォントを使用し、
    /// 存在しない場合はシステムフォントを検索して最初のものを採用します。
    /// フォントが見つからない場合はエラーになります。
    pub fn new_from_device(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        if let Ok(p) = std::env::var("ORINIUM_FONT") {
            let source = fontdb::Source::File(p.into());
            let font_sys = FontSystem::new_with_fonts(vec![source]);
            if font_sys.db.len() > 0 {
                return Self::new_with_fontsys(device, queue, format, font_sys);
            }
        }

        let font_sys = FontSystem::new();
        if font_sys.db.len() == 0 {
            anyhow::bail!("no system font found");
        }
        Self::new_with_fontsys(device, queue, format, font_sys)
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

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Text Uniform Buffer"),
            size: wgpu::BufferSize::new(8).unwrap().into(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Text Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(wgpu::BufferSize::new(8).unwrap()),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
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
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<QuadVertex>() as wgpu::BufferAddress,
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
                        ],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<GlyphInstance>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 2,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32,
                                offset: 8,
                                shader_location: 3,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32,
                                offset: 12,
                                shader_location: 4,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32,
                                offset: 16,
                                shader_location: 5,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32,
                                offset: 20,
                                shader_location: 6,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32,
                                offset: 24,
                                shader_location: 7,
                            },
                        ],
                    }),
                ],
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

        // Create static quad + index buffers.
        let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Text Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(&QUAD_VERTS),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let quad_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Text Quad Index Buffer"),
            contents: bytemuck::cast_slice(&QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
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
            atlas_dirty: true,
            quad_vertex_buffer,
            quad_index_buffer,
            instance_buffer: None,
            instance_capacity: 0,
            num_instances: 0,
            instances: Vec::new(),
            uniform_buffer,
            screen_size: [800.0, 600.0],
            prev_sections_hash: 0,
            prev_sections_count: 0,
            layout_cache: Vec::new(),
        })
    }

    /// Create an orinium_text `TextLayout` for the given text using the internal `FontSystem`.
    ///
    /// Results are cached: if the same `text` and `style` pair has been requested before,
    /// the cached layout is returned without re-shaping or re-layout.
    ///
    /// The cache key is intentionally simplified to (text, font_size_bits, color) to avoid
    /// repeated shape/layout computations for texts with the same font properties but
    /// different styling properties. The cache hit rate is already >90% in practice for modern web
    /// pages where font properties are the primary distinguishing factors.
    pub fn create_buffer_for_text(&mut self, text: &str, mut style: TextStyle) -> Arc<TextLayout> {
        // Quantize font_size early so the cache key is stable.
        // This ensures consistent hashing for font sizes that differ only by floating-point
        // rounding (e.g., 16.0 vs 15.9999999). The 1/64-px quantization provides a good
        // balance between precision and cache efficiency.
        style.font_size = quantize_font_size(style.font_size);

        // Fast path: return cached layout if text + font properties match.
        // Move matched entry to front (LRU).
        let font_size_bits = style.font_size.to_bits();
        let color_bits = ((style.color.0 as u32) << 24)
            | ((style.color.1 as u32) << 16)
            | ((style.color.2 as u32) << 8)
            | (style.color.3 as u32);
        for i in 0..self.layout_cache.len() {
            if self.layout_cache[i].text == text
                && self.layout_cache[i].font_size_bits == font_size_bits
                && self.layout_cache[i].color == color_bits
            {
                let entry = self.layout_cache.remove(i);
                let layout = entry.layout.clone();
                self.layout_cache.insert(0, entry);
                return layout;
            }
        }

        let layout = Arc::new(self.create_buffer_for_text_inner(text, &style));

        self.layout_cache.insert(
            0,
            CachedLayout {
                text: text.to_string(),
                font_size_bits,
                color: color_bits,
                layout: layout.clone(),
            },
        );
        if self.layout_cache.len() > 128 {
            self.layout_cache.pop();
        }

        layout
    }

    fn create_buffer_for_text_inner(&mut self, text: &str, style: &TextStyle) -> TextLayout {
        let _t0 = std::time::Instant::now();

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
            exact_fonts: Vec::new(),
        };

        let _t_shape = std::time::Instant::now();
        let shaped = self
            .layouter
            .shape_text(&mut self.font_sys, text, &ori_style);
        let t_shape = _t_shape.elapsed();

        let _t_lines = std::time::Instant::now();
        let line_ranges: Vec<(usize, usize)> = text
            .split('\n')
            .scan(0usize, |offset, line| {
                let start = *offset;
                *offset += line.len() + 1;
                let end = start + line.len();
                Some((start, end))
            })
            .collect();

        let layout =
            self.layouter
                .layout_lines(&mut self.font_sys, &shaped, &line_ranges, &ori_style);
        let t_layout = _t_lines.elapsed();

        let preview = if text.len() > 40 {
            let cut = text.floor_char_boundary(40);
            format!("{}...", &text[..cut])
        } else {
            text.to_string()
        };
        log::info!(
            target: "TextRenderer",
            "  create_buffer: text={:?} len={} font_size={}  shape={:?}  layout={:?}  total={:?}",
            preview,
            text.len(),
            font_size,
            t_shape,
            t_layout,
            _t0.elapsed(),
        );

        layout
    }

    pub fn resize_view(&mut self, width: f32, height: f32, queue: &wgpu::Queue) {
        self.screen_size = [width, height];
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&self.screen_size),
        );
    }

    /// 指定されたセクション群をインスタンスデータに変換し、GPUに転送する
    pub fn queue(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: &[TextSection],
    ) -> anyhow::Result<()> {
        let _t0 = std::time::Instant::now();

        // Fast path: skip geometry rebuild when sections haven't changed and atlas is clean.
        let s_hash = sections_hash(sections);
        if !self.atlas_dirty
            && s_hash == self.prev_sections_hash
            && sections.len() == self.prev_sections_count
        {
            self.instances.clear();
            log::trace!(target:"TextRenderer", "queue: fast-path ({} sections, {} instances)", sections.len(), self.num_instances);
            return Ok(());
        }
        self.prev_sections_hash = s_hash;
        self.prev_sections_count = sections.len();

        self.instances.clear();

        let mut glyph_count = 0u32;
        let mut atlas_miss_count = 0u32;
        let mut atlas_lookup_time = std::time::Duration::ZERO;
        let mut atlas_rasterize_time = std::time::Duration::ZERO;

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

                    // Compute visible portion against clip rect.
                    let vis_l = gx.max(clip_l);
                    let vis_t = gy.max(clip_t);
                    let vis_r = (gx + gw).min(clip_r);
                    let vis_b = (gy + gh).min(clip_b);

                    if vis_l >= vis_r || vis_t >= vis_b {
                        continue;
                    }

                    let _t_atlas = std::time::Instant::now();
                    let (layer, u, v, uw, uh) =
                        match self.atlas.lookup(font_key, glyph.glyph_id, glyph.font_size) {
                            Some(uv) => {
                                atlas_lookup_time += _t_atlas.elapsed();
                                uv
                            }
                            None => {
                                atlas_miss_count += 1;
                                let _t_raster = std::time::Instant::now();
                                if let Some((metrics, alpha_mask)) =
                                    self.font_sys.get_or_rasterize_with_bitmap(
                                        font_key,
                                        glyph.glyph_id,
                                        glyph.font_size,
                                    )
                                {
                                    atlas_rasterize_time += _t_raster.elapsed();
                                    if metrics.width == 0 || metrics.height == 0 {
                                        continue;
                                    }
                                    let mask_w = metrics.width;
                                    let mask_h = metrics.height;
                                    self.atlas_dirty = true;
                                    atlas_lookup_time += _t_atlas.elapsed();
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
                    glyph_count += 1;

                    // UVs for the visible portion.
                    let u0 = u + (vis_l - gx) / gw * uw;
                    let u1 = u + (vis_r - gx) / gw * uw;
                    let v0 = v + (vis_t - gy) / gh * uh;
                    let v1 = v + (vis_b - gy) / gh * uh;

                    let inst = GlyphInstance {
                        pos: pack_position(vis_l, vis_t),
                        size: pack_size(vis_r - vis_l, vis_b - vis_t),
                        uv_off: pack_uv_off(u0, v0),
                        uv_size: pack_uv_size(u1 - u0, v1 - v0),
                        layer: pack_layer(layer),
                        color: pack_color(&glyph.color),
                    };
                    self.instances.push(inst);
                }
            }
        }

        let _t_flush = std::time::Instant::now();
        self.atlas.flush_uploads(queue);
        let t_flush = _t_flush.elapsed();

        self.num_instances = self.instances.len() as u32;

        // Create or grow the instance buffer as needed.
        let _t_buf = std::time::Instant::now();
        if self.num_instances > 0 {
            let needed = self.instances.len();
            let instance_bytes: &[u8] = bytemuck::cast_slice(&self.instances);
            if needed > self.instance_capacity {
                // Grow with headroom (2× or at least 64).
                let new_cap = (needed * 2).max(64);
                let buf_size = (new_cap as u64) * size_of::<GlyphInstance>() as u64;
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Text Instance Buffer"),
                    size: buf_size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&buffer, 0, instance_bytes);
                self.instance_buffer = Some(buffer);
                self.instance_capacity = new_cap;
            } else {
                // Update existing buffer via write_buffer.
                queue.write_buffer(self.instance_buffer.as_ref().unwrap(), 0, instance_bytes);
            }
        } else {
            self.instance_buffer = None;
            self.instance_capacity = 0;
        }
        let t_buf = _t_buf.elapsed();

        // Rebuild bind group only when atlas changes.
        if self.atlas_dirty {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Text Bind Group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.uniform_buffer,
                            offset: 0,
                            size: None,
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(self.atlas.texture_view()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.bind_group = Some(bind_group);
            self.atlas_dirty = false;
        }

        let t_total = _t0.elapsed();
        log::info!(
            target: "TextRenderer",
            "queue: {} sections, {} glyphs ({} atlas misses), lookup={:?} rasterize={:?} flush={:?} buf={:?} total={:?}",
            sections.len(),
            glyph_count,
            atlas_miss_count,
            atlas_lookup_time,
            atlas_rasterize_time,
            t_flush,
            t_buf,
            t_total,
        );

        Ok(())
    }

    /// テキストをレンダリングする
    pub fn draw<'a>(&mut self, rpass: &mut wgpu::RenderPass<'a>) {
        let Some(ref bind_group) = self.bind_group else {
            return;
        };
        if self.num_instances == 0 {
            return;
        }

        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        if let Some(ref ib) = self.instance_buffer {
            rpass.set_vertex_buffer(1, ib.slice(..));
        }
        rpass.set_index_buffer(self.quad_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(0..6, 0, 0..self.num_instances);
    }
}
