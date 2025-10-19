use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph;
use fontdue::Font as FontDue;
use image::{GrayImage, Luma};
use wgpu::util::{DeviceExt, TextureDataOrder};

#[allow(dead_code)]
pub struct FontAtlas {
    pub texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub glyph_map: HashMap<char, PackedGlyphInfo>,
    pub width: u32,
    pub height: u32,
}

#[allow(dead_code)]
pub struct PackedGlyphInfo {
    pub uv_rect: [f32; 4], // [u0, v0, u1, v1]
    pub size: [f32; 2],    // pixel size in atlas
    pub bearing: [f32; 2], // left, top (bearingY positive upwards)
    pub advance: f32,
}

#[allow(dead_code)]
pub struct FontLoader {
    faces: HashMap<String, Arc<Vec<u8>>>,
    fontdue_cache: HashMap<String, FontDue>,
}

#[allow(dead_code)]
impl FontLoader {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            faces: HashMap::new(),
            fontdue_cache: HashMap::new(),
        })
    }

    pub fn load_from_bytes(
        &mut self,
        id: &str,
        data: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = Arc::new(data.to_vec());
        let fontdue = FontDue::from_bytes(&bytes[..], fontdue::FontSettings::default())?;
        self.fontdue_cache.insert(id.to_string(), fontdue);
        self.faces.insert(id.to_string(), bytes);
        Ok(())
    }

    pub fn build_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_id: &str,
        pixel_size: f32,
        charset: &str,
    ) -> Result<(FontAtlas, ab_glyph::FontArc), Box<dyn std::error::Error>> {
        let font_bytes = self
            .faces
            .get(font_id)
            .ok_or_else(|| format!("font id '{font_id}' not loaded"))?
            .clone();

        let fontdue = self
            .fontdue_cache
            .get(font_id)
            .ok_or_else(|| format!("fontdue font for '{font_id}' not found"))?;

        struct GlyphBitmap {
            ch: char,
            img: GrayImage,
            left: i32,
            top: i32,
            advance: f32,
        }

        let mut glyph_bitmaps: Vec<GlyphBitmap> = Vec::new();

        for ch in charset.chars() {
            let (metrics, bitmap) = fontdue.rasterize(ch, pixel_size);
            let w = metrics.width as u32;
            let h = metrics.height as u32;
            if w == 0 || h == 0 {
                continue;
            }
            let mut img = GrayImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let v = bitmap[(y * w + x) as usize];
                    img.put_pixel(x, y, Luma([v]));
                }
            }

            let left = metrics.xmin;
            // use ymin as bearing/top offset relative to baseline (allow negative values)
            let top = metrics.ymin;
            let advance = metrics.advance_width;
            glyph_bitmaps.push(GlyphBitmap {
                ch,
                img,
                left,
                top,
                advance,
            });
        }

        if glyph_bitmaps.is_empty() {
            return Err("no glyphs rasterized".into());
        }

        let width = 1024u32;
        let height = 1024u32;
        let mut atlas_image = GrayImage::new(width, height);

        let mut cursor_x = 0u32;
        let mut cursor_y = 0u32;
        let mut row_h = 0u32;

        let mut packed_infos: HashMap<char, PackedGlyphInfo> = HashMap::new();

        for g in glyph_bitmaps.into_iter() {
            let w = g.img.width();
            let h = g.img.height();
            if cursor_x + w > width {
                cursor_x = 0;
                cursor_y += row_h + 1;
                row_h = 0;
            }
            if cursor_y + h > height {
                return Err("atlas too small".into());
            }
            // blit
            for y in 0..h {
                for x in 0..w {
                    let p = g.img.get_pixel(x, y)[0];
                    atlas_image.put_pixel(cursor_x + x, cursor_y + y, Luma([p]));
                }
            }
            let u0 = cursor_x as f32 / width as f32;
            let v0 = cursor_y as f32 / height as f32;
            let u1 = (cursor_x + w) as f32 / width as f32;
            let v1 = (cursor_y + h) as f32 / height as f32;

            packed_infos.insert(
                g.ch,
                PackedGlyphInfo {
                    uv_rect: [u0, v0, u1, v1],
                    size: [w as f32, h as f32],
                    bearing: [g.left as f32, g.top as f32],
                    advance: g.advance,
                },
            );

            cursor_x += w + 1;
            row_h = row_h.max(h);
        }

        let unpadded_bytes_per_row = width as usize;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize; // usually 256
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

        let atlas_raw = atlas_image.into_raw();
        let mut padded: Vec<u8> = vec![0u8; padded_bytes_per_row * height as usize];
        for row in 0..height as usize {
            let src_start = row * unpadded_bytes_per_row;
            let dst_start = row * padded_bytes_per_row;
            padded[dst_start..dst_start + unpadded_bytes_per_row]
                .copy_from_slice(&atlas_raw[src_start..src_start + unpadded_bytes_per_row]);
        }

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("font_atlas"),
                size: texture_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &padded,
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());

        let atlas = FontAtlas {
            texture,
            texture_view,
            sampler,
            glyph_map: packed_infos,
            width,
            height,
        };

        let font_arc = ab_glyph::FontArc::try_from_vec((*font_bytes).clone())?;

        Ok((atlas, font_arc))
    }
}
