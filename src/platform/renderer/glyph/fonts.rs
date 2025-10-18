use std::collections::HashMap;
use std::sync::Arc;

use image::{GrayImage, Luma};
use wgpu::util::{DeviceExt, TextureDataOrder};
use wgpu_glyph::ab_glyph;
use fontdue::Font as FontDue;

pub struct FontAtlas {
    pub texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub glyph_map: HashMap<char, PackedGlyphInfo>,
    pub width: u32,
    pub height: u32,
}

pub struct PackedGlyphInfo {
    pub uv_rect: [f32; 4], // [u0, v0, u1, v1]
    pub size: [f32; 2],     // pixel size in atlas
    pub bearing: [f32; 2],  // left, top (bearingY positive upwards)
    pub advance: f32,
}

pub struct FontLoader {
    // store loaded font bytes so they live long enough
    faces: HashMap<String, Arc<Vec<u8>>>,
    // fontdue font cache
    fontdue_cache: HashMap<String, FontDue>,
}

impl FontLoader {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self { faces: HashMap::new(), fontdue_cache: HashMap::new() })
    }

    pub fn load_from_bytes(&mut self, id: &str, data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = Arc::new(data.to_vec());
        // create fontdue Font (pass as slice from the Arc<Vec<u8>>)
        let fontdue = FontDue::from_bytes(&bytes[..], fontdue::FontSettings::default())?;
        self.fontdue_cache.insert(id.to_string(), fontdue);
        self.faces.insert(id.to_string(), bytes);
        Ok(())
    }

    // Build atlas for a set of characters at a given pixel size. Returns FontAtlas and an ab_glyph::FontArc
    pub fn build_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_id: &str,
        pixel_size: f32,
        charset: &str,
    ) -> Result<(FontAtlas, ab_glyph::FontArc), Box<dyn std::error::Error>> {
        let font_bytes = self.faces.get(font_id)
            .ok_or_else(|| format!("font id '{}' not loaded", font_id))?
            .clone();

        let fontdue = self.fontdue_cache.get(font_id)
            .ok_or_else(|| format!("fontdue font for '{}' not found", font_id))?;

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
            // fontdue metrics: bitmap_left (xmin) and height. We use xmin as left bearing.
            // fontdue::Metrics doesn't expose `ymax`; use height as an approximation for top bearing.
            let left = metrics.xmin as i32;
            let top = metrics.height as i32; // approximate
            let advance = metrics.advance_width;
            glyph_bitmaps.push(GlyphBitmap { ch, img, left, top, advance });
        }

        if glyph_bitmaps.is_empty() {
            return Err("no glyphs rasterized".into());
        }

        // Simple row-based packing into a power-of-two atlas (prototype)
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
            // compute uv rect
            let u0 = cursor_x as f32 / width as f32;
            let v0 = cursor_y as f32 / height as f32;
            let u1 = (cursor_x + w) as f32 / width as f32;
            let v1 = (cursor_y + h) as f32 / height as f32;

            packed_infos.insert(g.ch, PackedGlyphInfo {
                uv_rect: [u0, v0, u1, v1],
                size: [w as f32, h as f32],
                bearing: [g.left as f32, g.top as f32],
                advance: g.advance,
            });

            cursor_x += w + 1;
            row_h = row_h.max(h);
        }

        // Prepare raw data with COPY_BYTES_PER_ROW_ALIGNMENT padding per row as required by wgpu
        let unpadded_bytes_per_row = width as usize; // R8Unorm -> 1 byte per texel
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize; // usually 256
        let padded_bytes_per_row = ((unpadded_bytes_per_row + align - 1) / align) * align;

        let atlas_raw = atlas_image.into_raw();
        let mut padded: Vec<u8> = vec![0u8; padded_bytes_per_row * height as usize];
        for row in 0..height as usize {
            let src_start = row * unpadded_bytes_per_row;
            let dst_start = row * padded_bytes_per_row;
            padded[dst_start..dst_start + unpadded_bytes_per_row]
                .copy_from_slice(&atlas_raw[src_start..src_start + unpadded_bytes_per_row]);
        }

        // Create texture and upload data using DeviceExt helper. Provide TextureDataOrder.
        let texture_size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
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

        // Create ab_glyph FontArc from the original bytes so wgpu_glyph can use it
        let font_arc = ab_glyph::FontArc::try_from_vec((*font_bytes).clone())?;

        Ok((atlas, font_arc))
    }
}
