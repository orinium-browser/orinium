use std::{num::NonZeroUsize, sync::Arc};

use bytemuck::{Pod, Zeroable};
use lru::LruCache;
use orinium_text::{
    Color as OriColor, FontKey, FontStyle as OriFontStyle, FontSystem, FontWeight as OriFontWeight,
    TextLayout, TextLayouter, TextStyle as OriTextStyle, fontdb,
};
use swash::{
    FontRef,
    scale::{Render, ScaleContext, Source},
    zeno::{Format, Vector},
};
use wgpu::util::DeviceExt;

use super::atlas::{GlyphAtlas, GlyphKey, RasterizedMask};
use super::global_font;
use crate::platform::renderer::mesh::{self, TextSection};
use crate::{perf_scope, profile_log};
use engine::layouter::types::{FontStyle, LineHeight, TextFlowStyle, TextStyle};

fn quantize_font_size(px: f32) -> f32 {
    (px * 64.0).round() / 64.0
}

fn rasterizable_font_size(px: f32) -> bool {
    px.is_finite() && px > 0.0
}

const SUBPIXEL_PHASES_X: i32 = 4;
const BEARING_CACHE_CAPACITY: usize = 32_768;

fn quantize_subpixel_x(x: f32) -> (f32, u8) {
    let quantized = (x * SUBPIXEL_PHASES_X as f32).round() as i32;
    (
        quantized.div_euclid(SUBPIXEL_PHASES_X) as f32,
        quantized.rem_euclid(SUBPIXEL_PHASES_X) as u8,
    )
}

/// Pre-clip test for a glyph using layout metrics alone, before any atlas
/// lookup. `glyph.x` already includes the x-bearing and `width`/`height` are
/// ink sizes, so the ink rect is known without rasterization. A small slack
/// covers subpixel quantization and hinting differences between the layout
/// metrics and the rasterized mask.
fn glyph_fully_outside_clip(
    section: &TextSection,
    glyph_x: f32,
    glyph_y: f32,
    glyph_width: f32,
    glyph_height: f32,
) -> bool {
    const CLIP_SLACK_PX: f32 = 4.0;
    let base_x = section.screen_position.0;
    let base_y = section.screen_position.1;
    let clip_l = section.clip_origin.0;
    let clip_t = section.clip_origin.1;
    let clip_r = section.clip_origin.0 + section.bounds.0;
    let clip_b = section.clip_origin.1 + section.bounds.1;
    let ink_l = base_x + glyph_x - CLIP_SLACK_PX;
    let ink_t = base_y + glyph_y - CLIP_SLACK_PX;
    let ink_r = ink_l + glyph_width + 2.0 * CLIP_SLACK_PX;
    let ink_b = ink_t + glyph_height + 2.0 * CLIP_SLACK_PX;
    ink_r < clip_l || ink_l > clip_r || ink_b < clip_t || ink_t > clip_b
}

fn rasterize_glyph(
    scale_context: &mut ScaleContext,
    font_system: &mut FontSystem,
    font_key: FontKey,
    glyph_id: u32,
    font_size: f32,
    font_weight: u16,
    phase_x: u8,
) -> Option<RasterizedMask> {
    let face_index = font_system.db.face(font_key.0)?.index as usize;
    let data = font_system.get_font_data(font_key)?;
    let font = FontRef::from_index(data.as_slice(), face_index)?;
    let mut scaler = scale_context
        .builder(font)
        .size(font_size)
        .variations([("wght", font_weight as f32)])
        .hint(true)
        .build();
    let mut render = Render::new(&[Source::Outline]);
    render
        .format(Format::Alpha)
        .offset(Vector::new(phase_x as f32 / SUBPIXEL_PHASES_X as f32, 0.0));
    let image = render.render(&mut scaler, glyph_id as u16)?;
    Some(RasterizedMask {
        width: image.placement.width,
        height: image.placement.height,
        left: image.placement.left,
        top: image.placement.top,
        data: image.data,
    })
}

fn unhinted_bearings(
    font_system: &mut FontSystem,
    font_key: FontKey,
    glyph_id: u32,
    font_size: f32,
) -> Option<(f32, f32)> {
    let face_index = font_system.db.face(font_key.0)?.index;
    let data = font_system.get_font_data(font_key)?;
    let face = ttf_parser::Face::parse(data.as_slice(), face_index).ok()?;
    let bbox = face.glyph_bounding_box(ttf_parser::GlyphId(glyph_id as u16))?;
    let scale = font_size / face.units_per_em() as f32;
    Some((bbox.x_min as f32 * scale, bbox.y_max as f32 * scale))
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct QuadVertex {
    position: [f32; 2],
    tex_coord: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct GlyphInstance {
    pos: [f32; 2],
    size: u32,
    uv_off: u32,
    uv_size: u32,
    layer: u32,
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
    layer
}

fn pack_color(c: &OriColor) -> u32 {
    ((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | (c.3 as u32)
}

pub fn build_family_list<'a>(families: &'a [String]) -> Vec<fontdb::Family<'a>> {
    if families.is_empty() {
        return vec![fontdb::Family::SansSerif, fontdb::Family::Serif];
    }
    let mut list: Vec<fontdb::Family<'a>> = families
        .iter()
        .map(|f| {
            let lower = f.to_ascii_lowercase();
            match lower.as_str() {
                "serif" => fontdb::Family::Serif,
                "sans-serif" => fontdb::Family::SansSerif,
                "monospace" => fontdb::Family::Monospace,
                "cursive" => fontdb::Family::Cursive,
                "fantasy" => fontdb::Family::Fantasy,
                _ => fontdb::Family::Name(f.as_str()),
            }
        })
        .collect();
    list.push(fontdb::Family::SansSerif);
    list.push(fontdb::Family::Serif);
    list.push(fontdb::Family::Monospace);
    list
}

/// テキストセクション位置・クリップ・描画範囲をまとめた構造体
/// (定義は [`mesh::TextSection`] を参照)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CachedLineHeight {
    Normal,
    Number(u32),
    Px(u32),
}

impl CachedLineHeight {
    fn from_line_height(line_height: LineHeight) -> Self {
        match line_height {
            LineHeight::Normal => Self::Normal,
            LineHeight::Number(value) => Self::Number(value.to_bits()),
            LineHeight::Px(value) => Self::Px(value.to_bits()),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedLayout {
    text: String,
    font_size_bits: u32,
    color: u32,
    font_weight: u16,
    font_style: FontStyle,
    line_height: CachedLineHeight,
    font_families: Vec<String>,
    layout: Arc<TextLayout>,
}

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
        h ^= s.font_weight as u64;
        h ^= Arc::as_ptr(&s.layout) as u64;
    }
    h
}

/// テキストレンダラー (global FontSystem + wgpu グリフアトラス)
pub struct TextRenderer {
    layouter: TextLayouter,
    scale_context: ScaleContext,
    atlas: GlyphAtlas,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    sampler: wgpu::Sampler,
    atlas_dirty: bool,

    quad_vertex_buffer: wgpu::Buffer,
    quad_index_buffer: wgpu::Buffer,

    instance_buffer: Option<wgpu::Buffer>,
    instance_capacity: usize,
    num_instances: u32,
    instances: Vec<GlyphInstance>,

    uniform_buffer: wgpu::Buffer,
    screen_size: [f32; 2],

    prev_sections_hash: u64,
    prev_sections_count: usize,

    /// Per-section instance ranges `(start, count)` into `instances`, recorded
    /// in the same order as the sections. Enables drawing a subset of sections
    /// while preserving interleaved draw order.
    section_ranges: Vec<(u32, u32)>,

    layout_cache: Vec<CachedLayout>,
    bearing_cache: LruCache<(fontdb::ID, u32, u32, u16), (f32, f32)>,
}

impl TextRenderer {
    /// グローバル FontSystem を使用して初期化する
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        if !global_font::global_font_system_ready() {
            anyhow::bail!("no system font found");
        }
        Self::new_with_device(device, queue, format)
    }

    fn new_with_device(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
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
            layouter,
            scale_context: ScaleContext::with_max_entries(128),
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
            section_ranges: Vec::new(),
            layout_cache: Vec::new(),
            bearing_cache: LruCache::new(NonZeroUsize::new(BEARING_CACHE_CAPACITY).unwrap()),
        })
    }

    pub fn create_buffer_for_text(
        &mut self,
        text: &str,
        style: TextStyle,
        mut flow_style: TextFlowStyle,
    ) -> Arc<TextLayout> {
        flow_style.font_size = quantize_font_size(flow_style.font_size);

        let font_size_bits = flow_style.font_size.to_bits();
        let color = ((style.color.0 as u32) << 24)
            | ((style.color.1 as u32) << 16)
            | ((style.color.2 as u32) << 8)
            | (style.color.3 as u32);
        let font_weight = style.font_weight.0;
        let font_style = style.font_style;
        let line_height = CachedLineHeight::from_line_height(flow_style.line_height);
        let font_families = style.font_families.clone();

        for i in 0..self.layout_cache.len() {
            let cached = &self.layout_cache[i];

            if cached.text == text
                && cached.font_size_bits == font_size_bits
                && cached.color == color
                && cached.font_weight == font_weight
                && cached.font_style == font_style
                && cached.line_height == line_height
                && cached.font_families == font_families
            {
                let entry = self.layout_cache.remove(i);
                let layout = Arc::clone(&entry.layout);
                self.layout_cache.insert(0, entry);
                return layout;
            }
        }

        let layout = Arc::new(self.create_buffer_for_text_inner(text, &style, &flow_style));

        self.layout_cache.insert(
            0,
            CachedLayout {
                text: text.to_string(),
                font_size_bits,
                color,
                font_weight,
                font_style,
                line_height,
                font_families,
                layout: Arc::clone(&layout),
            },
        );

        if self.layout_cache.len() > 128 {
            self.layout_cache.pop();
        }

        layout
    }

    fn create_buffer_for_text_inner(
        &mut self,
        text: &str,
        style: &TextStyle,
        flow_style: &TextFlowStyle,
    ) -> TextLayout {
        perf_scope!(total);

        let font_size = flow_style.font_size;
        let line_height_ratio = match flow_style.line_height {
            LineHeight::Normal => 1.2,
            LineHeight::Number(n) => n,
            LineHeight::Px(px) => px / font_size,
        };

        let font_families = build_family_list(&style.font_families);

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
            font_families,
            exact_fonts: Vec::new(),
            variant: orinium_text::FontVariant::Normal,
        };

        perf_scope!(shape);
        let shaped = global_font::with_global_font_system(|fs| {
            self.layouter.shape_text(fs, text, &ori_style)
        });
        #[cfg(any(feature = "profile", debug_assertions))]
        let shape_time = shape.elapsed();

        perf_scope!(lines);
        let line_ranges: Vec<(usize, usize)> = text
            .split('\n')
            .scan(0usize, |offset, line| {
                let start = *offset;
                *offset += line.len() + 1;
                let end = start + line.len();
                Some((start, end))
            })
            .collect();

        let layout = global_font::with_global_font_system(|fs| {
            self.layouter
                .layout_lines(fs, &shaped, &line_ranges, &ori_style)
        });
        #[cfg(any(feature = "profile", debug_assertions))]
        let lines_time = lines.elapsed();

        profile_log!(
            target: "TextRenderer",
            log::Level::Info,
            "  create_buffer: text={:?} len={} font_size={}  shape={:?}  layout={:?}  total={:?}",
            crate::profile::text_preview(text),
            text.len(),
            font_size,
            shape_time,
            lines_time,
            total.elapsed(),
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

    pub fn queue(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: &[TextSection],
    ) -> anyhow::Result<()> {
        perf_scope!(total);

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
        self.section_ranges.clear();

        #[cfg(any(feature = "profile", debug_assertions))]
        let mut glyph_count = 0u32;
        #[cfg(any(feature = "profile", debug_assertions))]
        let mut culled_count = 0u32;
        #[cfg(any(feature = "profile", debug_assertions))]
        let mut atlas_miss_count = 0u32;
        #[cfg(any(feature = "profile", debug_assertions))]
        let mut atlas_lookup_time = std::time::Duration::ZERO;
        #[cfg(any(feature = "profile", debug_assertions))]
        let mut atlas_rasterize_time = std::time::Duration::ZERO;

        global_font::with_global_font_system(|fs| {
            for section in sections {
                let range_start = self.instances.len() as u32;
                let layout = &section.layout;

                let base_x = section.screen_position.0;
                let base_y = section.screen_position.1;

                let clip_l = section.clip_origin.0;
                let clip_t = section.clip_origin.1;
                let clip_r = section.clip_origin.0 + section.bounds.0;
                let clip_b = section.clip_origin.1 + section.bounds.1;

                for line in &layout.lines {
                    for glyph in &line.glyphs {
                        // Authors commonly use `font-size: 0` to remove the
                        // whitespace between inline-blocks. Swash does not
                        // define useful raster output for a zero-sized scaler
                        // and some fonts return enormous bogus masks.
                        if !rasterizable_font_size(glyph.font_size) {
                            continue;
                        }
                        if glyph_fully_outside_clip(
                            section,
                            glyph.x,
                            glyph.y,
                            glyph.width,
                            glyph.height,
                        ) {
                            #[cfg(any(feature = "profile", debug_assertions))]
                            {
                                culled_count += 1;
                            }
                            continue;
                        }

                        let Some(font_key) = glyph.font_key else {
                            continue;
                        };

                        let bearing_key = (
                            font_key.0,
                            glyph.glyph_id,
                            glyph.font_size.to_bits(),
                            section.font_weight,
                        );
                        let (bearing_x, bearing_y) = if let Some(bearings) =
                            self.bearing_cache.get(&bearing_key)
                        {
                            *bearings
                        } else {
                            let bearings =
                                unhinted_bearings(fs, font_key, glyph.glyph_id, glyph.font_size)
                                    .unwrap_or((0.0, 0.0));
                            self.bearing_cache.put(bearing_key, bearings);
                            bearings
                        };

                        let glyph_origin_x = base_x + glyph.x - bearing_x;
                        let glyph_baseline_y = (base_y + glyph.y + bearing_y).round();
                        let (origin_x, phase_x) = quantize_subpixel_x(glyph_origin_x);

                        perf_scope!(atlas_lap);
                        let glyph_key: GlyphKey = (
                            font_key.0,
                            glyph.glyph_id,
                            glyph.font_size.to_bits(),
                            section.font_weight,
                            phase_x,
                        );
                        let atlas_entry = match self.atlas.lookup(glyph_key) {
                            Some(entry) => {
                                #[cfg(any(feature = "profile", debug_assertions))]
                                {
                                    atlas_lookup_time += atlas_lap.elapsed();
                                }
                                entry
                            }
                            None => {
                                #[cfg(any(feature = "profile", debug_assertions))]
                                {
                                    atlas_miss_count += 1;
                                }
                                perf_scope!(rasterize_lap);
                                if let Some(mask) = rasterize_glyph(
                                    &mut self.scale_context,
                                    fs,
                                    font_key,
                                    glyph.glyph_id,
                                    glyph.font_size,
                                    section.font_weight,
                                    phase_x,
                                ) {
                                    #[cfg(any(feature = "profile", debug_assertions))]
                                    {
                                        atlas_rasterize_time += rasterize_lap.elapsed();
                                    }
                                    if mask.width == 0 || mask.height == 0 {
                                        continue;
                                    }
                                    self.atlas_dirty = true;
                                    #[cfg(any(feature = "profile", debug_assertions))]
                                    {
                                        atlas_lookup_time += atlas_lap.elapsed();
                                    }
                                    self.atlas.upload(device, queue, glyph_key, &mask)
                                } else {
                                    continue;
                                }
                            }
                        };
                        #[cfg(any(feature = "profile", debug_assertions))]
                        {
                            glyph_count += 1;
                        }

                        let gx = origin_x + atlas_entry.left as f32;
                        let gy = glyph_baseline_y - atlas_entry.top as f32;
                        let gw = atlas_entry.pixel_width as f32;
                        let gh = atlas_entry.pixel_height as f32;

                        let vis_l = gx.max(clip_l);
                        let vis_t = gy.max(clip_t);
                        let vis_r = (gx + gw).min(clip_r);
                        let vis_b = (gy + gh).min(clip_b);

                        if vis_l >= vis_r || vis_t >= vis_b {
                            continue;
                        }

                        let u0 = atlas_entry.u + (vis_l - gx) / gw * atlas_entry.uv_width;
                        let u1 = atlas_entry.u + (vis_r - gx) / gw * atlas_entry.uv_width;
                        let v0 = atlas_entry.v + (vis_t - gy) / gh * atlas_entry.uv_height;
                        let v1 = atlas_entry.v + (vis_b - gy) / gh * atlas_entry.uv_height;

                        let inst = GlyphInstance {
                            pos: pack_position(vis_l, vis_t),
                            size: pack_size(vis_r - vis_l, vis_b - vis_t),
                            uv_off: pack_uv_off(u0, v0),
                            uv_size: pack_uv_size(u1 - u0, v1 - v0),
                            layer: pack_layer(atlas_entry.layer),
                            color: pack_color(&glyph.color),
                        };
                        self.instances.push(inst);
                    }
                }
                self.section_ranges
                    .push((range_start, self.instances.len() as u32 - range_start));
            }
        });

        perf_scope!(flush);
        self.atlas.flush_uploads(queue);
        #[cfg(any(feature = "profile", debug_assertions))]
        let flush_time = flush.elapsed();

        self.num_instances = self.instances.len() as u32;

        perf_scope!(buf);
        if self.num_instances > 0 {
            let needed = self.instances.len();
            let instance_bytes: &[u8] = bytemuck::cast_slice(&self.instances);
            if needed > self.instance_capacity {
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
                queue.write_buffer(self.instance_buffer.as_ref().unwrap(), 0, instance_bytes);
            }
        } else {
            self.instance_buffer = None;
            self.instance_capacity = 0;
        }
        #[cfg(any(feature = "profile", debug_assertions))]
        let buf_time = buf.elapsed();

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

        profile_log!(
            target: "TextRenderer",
            log::Level::Info,
            "queue: {} sections, {} glyphs ({} culled, {} atlas misses), lookup={:?} rasterize={:?} flush={:?} buf={:?} total={:?}",
            sections.len(),
            glyph_count,
            culled_count,
            atlas_miss_count,
            atlas_lookup_time,
            atlas_rasterize_time,
            flush_time,
            buf_time,
            total.elapsed(),
        );

        Ok(())
    }

    /// Returns the `(start, count)` instance range for the section at `index`,
    /// if it was queued this frame.
    pub fn section_range(&self, index: usize) -> Option<(u32, u32)> {
        self.section_ranges.get(index).copied()
    }

    /// Draws instances `start..start + count` with the glyph pipeline.
    pub fn draw_range<'a>(&mut self, rpass: &mut wgpu::RenderPass<'a>, start: u32, count: u32) {
        let Some(ref bind_group) = self.bind_group else {
            return;
        };
        if count == 0 {
            return;
        }

        rpass.set_pipeline(&self.pipeline);
        rpass.set_bind_group(0, bind_group, &[]);
        rpass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        if let Some(ref ib) = self.instance_buffer {
            rpass.set_vertex_buffer(1, ib.slice(..));
        }
        rpass.set_index_buffer(self.quad_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        rpass.draw_indexed(0..6, 0, start..start + count);
    }
}

impl mesh::TextLayoutSource for TextRenderer {
    fn layout_text(
        &mut self,
        text: &str,
        style: &TextStyle,
        flow_style: TextFlowStyle,
    ) -> Option<Arc<TextLayout>> {
        Some(self.create_buffer_for_text(text, style.clone(), flow_style))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use orinium_text::TextLayout;

    use super::{
        TextSection, glyph_fully_outside_clip, quantize_subpixel_x, rasterizable_font_size,
        sections_hash,
    };

    fn section(
        screen_position: (f32, f32),
        clip_origin: (f32, f32),
        bounds: (f32, f32),
    ) -> TextSection {
        TextSection {
            screen_position,
            clip_origin,
            bounds,
            font_weight: 400,
            layout: Arc::new(TextLayout {
                lines: Vec::new(),
                width: 0.0,
                height: 0.0,
            }),
        }
    }

    #[test]
    fn horizontal_subpixel_positions_use_four_phases() {
        assert_eq!(quantize_subpixel_x(10.0), (10.0, 0));
        assert_eq!(quantize_subpixel_x(10.24), (10.0, 1));
        assert_eq!(quantize_subpixel_x(10.51), (10.0, 2));
        assert_eq!(quantize_subpixel_x(10.76), (10.0, 3));
        assert_eq!(quantize_subpixel_x(10.99), (11.0, 0));
    }

    #[test]
    fn horizontal_subpixel_positions_handle_negative_coordinates() {
        assert_eq!(quantize_subpixel_x(-0.24), (-1.0, 3));
        assert_eq!(quantize_subpixel_x(-0.51), (-1.0, 2));
        assert_eq!(quantize_subpixel_x(-0.99), (-1.0, 0));
    }

    #[test]
    fn pre_clip_keeps_glyphs_inside_clip() {
        let s = section((0.0, 0.0), (0.0, 0.0), (100.0, 100.0));
        // Glyph fully inside the clip rect.
        assert!(!glyph_fully_outside_clip(&s, 10.0, 20.0, 8.0, 12.0));
        // Glyph straddling the clip boundary is kept (post-clip handles the split).
        assert!(!glyph_fully_outside_clip(&s, 98.0, 20.0, 8.0, 12.0));
    }

    #[test]
    fn section_hash_distinguishes_font_weights() {
        let regular = section((0.0, 0.0), (0.0, 0.0), (100.0, 20.0));
        let mut bold = regular.clone();
        bold.font_weight = 700;
        assert_ne!(sections_hash(&[regular]), sections_hash(&[bold]));
    }

    #[test]
    fn zero_and_non_finite_font_sizes_are_not_rasterized() {
        assert!(rasterizable_font_size(0.01));
        assert!(!rasterizable_font_size(0.0));
        assert!(!rasterizable_font_size(-1.0));
        assert!(!rasterizable_font_size(f32::NAN));
        assert!(!rasterizable_font_size(f32::INFINITY));
    }

    #[test]
    fn pre_clip_culls_glyphs_far_outside() {
        let s = section((0.0, 0.0), (0.0, 0.0), (100.0, 100.0));
        // Far left, far right, above, and below the clip rect are all culled.
        assert!(glyph_fully_outside_clip(&s, -1000.0, 20.0, 8.0, 12.0));
        assert!(glyph_fully_outside_clip(&s, 2000.0, 20.0, 8.0, 12.0));
        assert!(glyph_fully_outside_clip(&s, 10.0, -2000.0, 8.0, 12.0));
        assert!(glyph_fully_outside_clip(&s, 10.0, 3000.0, 8.0, 12.0));
    }

    #[test]
    fn pre_clip_keeps_glyphs_within_slack_of_boundary() {
        let s = section((0.0, 0.0), (0.0, 0.0), (100.0, 100.0));
        // Within 4px slack of the edge: the real (hinted/quantized) ink rect may
        // still touch the clip, so it must not be culled here.
        assert!(!glyph_fully_outside_clip(&s, 3.0, 20.0, 8.0, 12.0));
        assert!(!glyph_fully_outside_clip(&s, 96.0, 20.0, 8.0, 12.0));
        assert!(!glyph_fully_outside_clip(&s, 10.0, -3.0, 8.0, 12.0));
    }

    #[test]
    fn pre_clip_culls_once_beyond_slack() {
        let s = section((0.0, 0.0), (0.0, 0.0), (100.0, 100.0));
        // Just past the 4px slack from the boundary.
        assert!(glyph_fully_outside_clip(&s, -20.0, 20.0, 8.0, 12.0));
        assert!(glyph_fully_outside_clip(&s, 105.0, 20.0, 8.0, 12.0));
    }
}
