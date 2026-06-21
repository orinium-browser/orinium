use std::collections::HashMap;

use etagere::{BucketedAtlasAllocator, size2};
use orinium_text::{FontKey, fontdb};

/// (fontdb ID, glyph_id, font_size_bits)
type GlyphKey = (fontdb::ID, u32, u32);

/// (layer_index, alloc_id, rectangle_packed, actual_width, actual_height)
type GlyphCacheValue = (u32, etagere::AllocId, etagere::Rectangle, u32, u32);

/// (layer_index, rectangle, alpha_mask, actual_width, actual_height)
type PendingUpload = (u32, etagere::Rectangle, Vec<u8>, u32, u32);

/// Normalized UV rect: (layer, u, v, width, height)
pub type GlyphUVRect = (u32, f32, f32, f32, f32);

/// A packed glyph atlas managed as a wgpu 2D array texture.
///
/// Each layer holds a single row of shelf-packed glyph bitmaps.
/// When a layer runs out of space, a new layer is added (up to
/// MAX_LAYERS).  When all layers are full, the atlas is grown by
/// doubling the texture size and re-packing every existing glyph.
pub struct GlyphAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: u32,
    layers: u32,
    allocators: Vec<BucketedAtlasAllocator>,
    glyph_map: HashMap<GlyphKey, GlyphCacheValue>,
    pending_uploads: Vec<PendingUpload>,
}

const MAX_LAYERS: u32 = 16;
const INITIAL_SIZE: u32 = 1024;

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let size = INITIAL_SIZE;
        let layers = 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Glyph Atlas View"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let allocator = BucketedAtlasAllocator::new(size2(size as i32, size as i32));

        Self {
            texture,
            view,
            size,
            layers,
            allocators: vec![allocator],
            glyph_map: HashMap::new(),
            pending_uploads: Vec::new(),
        }
    }

    pub fn texture_view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Returns cached (layer, u, v, w, h) if the glyph is already in the atlas.
    pub fn lookup(&self, font_key: FontKey, glyph_id: u32, font_size: f32) -> Option<GlyphUVRect> {
        let key = (font_key.0, glyph_id, font_size.to_bits());
        let (layer, _alloc_id, rect, aw, ah) = self.glyph_map.get(&key)?;
        let (u, v) = (
            rect.min.x as f32 / self.size as f32,
            rect.min.y as f32 / self.size as f32,
        );
        let (w, h) = (*aw as f32 / self.size as f32, *ah as f32 / self.size as f32);
        Some((*layer, u, v, w, h))
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_key: FontKey,
        glyph_id: u32,
        font_size: f32,
        alpha_mask: &[u8],
        mask_width: u32,
        mask_height: u32,
    ) -> GlyphUVRect {
        if mask_width == 0 || mask_height == 0 {
            return (0, 0.0, 0.0, 0.0, 0.0);
        }

        let key = (font_key.0, glyph_id, font_size.to_bits());

        if let Some((layer, _alloc_id, rect, aw, ah)) = self.glyph_map.get(&key) {
            let (u, v) = (
                rect.min.x as f32 / self.size as f32,
                rect.min.y as f32 / self.size as f32,
            );
            let (w, h) = (*aw as f32 / self.size as f32, *ah as f32 / self.size as f32);
            return (*layer, u, v, w, h);
        }

        let item_w = mask_width.max(1) as i32;
        let item_h = mask_height.max(1) as i32;

        for (layer_idx, alloc) in self.allocators.iter_mut().enumerate() {
            if let Some(allocation) = alloc.allocate(size2(item_w, item_h)) {
                let rect = allocation.rectangle;
                let alloc_id = allocation.id;
                self.glyph_map.insert(
                    key,
                    (layer_idx as u32, alloc_id, rect, mask_width, mask_height),
                );
                self.pending_uploads.push((
                    layer_idx as u32,
                    rect,
                    alpha_mask.to_vec(),
                    mask_width,
                    mask_height,
                ));
                let (u, v) = (
                    rect.min.x as f32 / self.size as f32,
                    rect.min.y as f32 / self.size as f32,
                );
                let (w, h) = (
                    mask_width as f32 / self.size as f32,
                    mask_height as f32 / self.size as f32,
                );
                return (layer_idx as u32, u, v, w, h);
            }
        }

        if self.layers < MAX_LAYERS {
            let new_layer = self.layers;
            self.layers += 1;
            let mut alloc = BucketedAtlasAllocator::new(size2(self.size as i32, self.size as i32));
            if let Some(allocation) = alloc.allocate(size2(item_w, item_h)) {
                let rect = allocation.rectangle;
                let alloc_id = allocation.id;
                self.allocators.push(alloc);
                self.glyph_map
                    .insert(key, (new_layer, alloc_id, rect, mask_width, mask_height));
                self.pending_uploads.push((
                    new_layer,
                    rect,
                    alpha_mask.to_vec(),
                    mask_width,
                    mask_height,
                ));

                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Glyph Atlas"),
                    size: wgpu::Extent3d {
                        width: self.size,
                        height: self.size,
                        depth_or_array_layers: self.layers,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let mut encoder =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                for old_layer in 0..new_layer {
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &self.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: old_layer,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: old_layer,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width: self.size,
                            height: self.size,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                queue.submit(std::iter::once(encoder.finish()));

                self.texture = texture;
                self.view = self.texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("Glyph Atlas View"),
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                });

                self.flush_uploads(queue);

                let (u, v) = (
                    rect.min.x as f32 / self.size as f32,
                    rect.min.y as f32 / self.size as f32,
                );
                let (w, h) = (
                    mask_width as f32 / self.size as f32,
                    mask_height as f32 / self.size as f32,
                );
                return (new_layer, u, v, w, h);
            }
        }

        log::warn!(target:"GlyphAtlas", "glyph atlas full ({} layers used)", self.layers);
        (0, 0.0, 0.0, 0.0, 0.0)
    }

    pub fn flush_uploads(&mut self, queue: &wgpu::Queue) {
        for (layer, rect, data, actual_w, actual_h) in self.pending_uploads.drain(..) {
            let width = actual_w.max(1);
            let height = actual_h.max(1);

            let bytes_per_pixel = 1u32;
            let stride = width * bytes_per_pixel;
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded_stride = ((stride + align - 1) / align) * align;

            let layout = wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_stride),
                rows_per_image: Some(height),
            };

            if padded_stride == stride {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: rect.min.x as u32,
                            y: rect.min.y as u32,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &data,
                    layout,
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            } else {
                let mut padded = Vec::with_capacity((padded_stride * height) as usize);
                for row in data.chunks(stride as usize) {
                    padded.extend_from_slice(row);
                    let pad = (padded_stride - stride) as usize;
                    if pad > 0 {
                        padded.extend(std::iter::repeat(0u8).take(pad));
                    }
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: rect.min.x as u32,
                            y: rect.min.y as u32,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &padded,
                    layout,
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
}
