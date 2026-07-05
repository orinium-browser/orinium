use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use orinium_text::{
    Color as OriColor, FontStyle as OriFontStyle, FontWeight as OriFontWeight, TextLayout,
    TextLayouter, TextStyle as OriTextStyle, fontdb,
};
use wgpu::util::DeviceExt;

use super::atlas::GlyphAtlas;
use super::global_font;
use crate::engine::layouter::types::{FontStyle, LineHeight, TextStyle};

fn quantize_font_size(px: f32) -> f32 {
    (px * 64.0).round() / 64.0
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
    layer as u32
}

fn pack_color(c: &OriColor) -> u32 {
    ((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | (c.3 as u32)
}

fn build_family_list<'a>(families: &'a [String]) -> Vec<fontdb::Family<'a>> {
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
pub struct TextSection {
    pub screen_position: (f32, f32),
    pub clip_origin: (f32, f32),
    pub bounds: (f32, f32),
    pub layout: Arc<TextLayout>,
}

#[derive(Debug, Clone)]
struct CachedLayout {
    text: String,
    font_size_bits: u32,
    color: u32,
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
        h ^= Arc::as_ptr(&s.layout) as u64;
    }
    h
}

/// テキストレンダラー (global FontSystem + wgpu グリフアトラス)
pub struct TextRenderer {
    layouter: TextLayouter,
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

    layout_cache: Vec<CachedLayout>,
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

    pub fn create_buffer_for_text(&mut self, text: &str, mut style: TextStyle) -> Arc<TextLayout> {
        style.font_size = quantize_font_size(style.font_size);

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
        };

        let _t_shape = std::time::Instant::now();
        let shaped = global_font::with_global_font_system(|fs| {
            self.layouter.shape_text(fs, text, &ori_style)
        });
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

        let layout = global_font::with_global_font_system(|fs| {
            self.layouter
                .layout_lines(fs, &shaped, &line_ranges, &ori_style)
        });
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

    pub fn queue(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: &[TextSection],
    ) -> anyhow::Result<()> {
        let _t0 = std::time::Instant::now();

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

        global_font::with_global_font_system(|fs| {
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
                                    if let Some((metrics, alpha_mask)) = fs
                                        .get_or_rasterize_with_bitmap(
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
        });

        let _t_flush = std::time::Instant::now();
        self.atlas.flush_uploads(queue);
        let t_flush = _t_flush.elapsed();

        self.num_instances = self.instances.len() as u32;

        let _t_buf = std::time::Instant::now();
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
        let t_buf = _t_buf.elapsed();

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
