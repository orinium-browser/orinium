use std::num::NonZeroUsize;

use etagere::{BucketedAtlasAllocator, size2};
use lru::LruCache;
use orinium_text::{FontKey, fontdb};

/// (fontdb ID, glyph_id, font_size_bits, CSS weight, horizontal subpixel phase)
type GlyphKey = (fontdb::ID, u32, u32, u16, u8);

/// (layer, allocation, rectangle, bitmap width/height, placement left/top)
type GlyphCacheValue = (
    u32,
    etagere::AllocId,
    etagere::Rectangle,
    u32,
    u32,
    i32,
    i32,
);

#[derive(Debug, Clone, Copy)]
pub struct GlyphAtlasEntry {
    pub layer: u32,
    pub u: f32,
    pub v: f32,
    pub uv_width: f32,
    pub uv_height: f32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub left: i32,
    pub top: i32,
}

/// A packed glyph atlas managed as a wgpu 2D array texture.
///
/// Each layer holds a single row of shelf-packed glyph bitmaps.
/// When a layer runs out of space, a new layer is added (up to
/// MAX_LAYERS).  When all layers are full, the LRU entry is evicted
/// to make room (its atlas allocation is freed and the slot reused).
pub struct GlyphAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: u32,
    layers: u32,
    allocators: Vec<BucketedAtlasAllocator>,
    glyph_map: LruCache<GlyphKey, GlyphCacheValue>,
    cpu_layers: Vec<Vec<u8>>,
    dirty_layers: std::collections::HashSet<u32>,
}

const MAX_LAYERS: u32 = 16;
const INITIAL_SIZE: u32 = 1024;
const GLYPH_GUTTER: i32 = 1;
/// Maximum number of glyph entries in the LRU cache.
/// Set to roughly match the pixel capacity of a full atlas
/// (16 layers × 1024² px ÷ ~400 px avg glyph area ≈ 40k).
/// 8192 is a safe upper bound that prevents unbounded growth.
const LRU_CAPACITY: usize = 8192;

fn allocation_fits_layer(atlas_size: u32, width: i32, height: i32) -> bool {
    width > 0 && height > 0 && width <= atlas_size as i32 && height <= atlas_size as i32
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let size = INITIAL_SIZE;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: MAX_LAYERS,
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
        let layer_size = (size * size) as usize;

        Self {
            texture,
            view,
            size,
            layers: 1,
            allocators: vec![allocator],
            glyph_map: LruCache::new(NonZeroUsize::new(LRU_CAPACITY).unwrap()),
            cpu_layers: vec![vec![0u8; layer_size]],
            dirty_layers: std::collections::HashSet::new(),
        }
    }

    pub fn texture_view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Returns cached (layer, u, v, w, h) if the glyph is already in the atlas.
    pub fn lookup(
        &mut self,
        font_key: FontKey,
        glyph_id: u32,
        font_size: f32,
        font_weight: u16,
        phase_x: u8,
    ) -> Option<GlyphAtlasEntry> {
        let key = (
            font_key.0,
            glyph_id,
            font_size.to_bits(),
            font_weight,
            phase_x,
        );
        let (layer, _alloc_id, rect, width, height, left, top) = self.glyph_map.get(&key)?;
        Some(Self::entry(
            self.size, *layer, *rect, *width, *height, *left, *top,
        ))
    }

    /// Evict a single LRU entry from the cache and free its atlas allocation.
    /// Returns `true` if an entry was evicted.
    fn evict_one(&mut self) -> bool {
        if let Some((_key, (layer, alloc_id, ..))) = self.glyph_map.pop_lru() {
            self.allocators[layer as usize].deallocate(alloc_id);
            true
        } else {
            false
        }
    }

    pub fn upload(
        &mut self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        font_key: FontKey,
        glyph_id: u32,
        font_size: f32,
        font_weight: u16,
        phase_x: u8,
        alpha_mask: &[u8],
        mask_width: u32,
        mask_height: u32,
        left: i32,
        top: i32,
    ) -> GlyphAtlasEntry {
        if mask_width == 0 || mask_height == 0 {
            return GlyphAtlasEntry {
                layer: 0,
                u: 0.0,
                v: 0.0,
                uv_width: 0.0,
                uv_height: 0.0,
                pixel_width: 0,
                pixel_height: 0,
                left,
                top,
            };
        }

        let key = (
            font_key.0,
            glyph_id,
            font_size.to_bits(),
            font_weight,
            phase_x,
        );

        // Check if already present (updates LRU position).
        if let Some((layer, _alloc_id, rect, width, height, left, top)) = self.glyph_map.get(&key) {
            return Self::entry(self.size, *layer, *rect, *width, *height, *left, *top);
        }

        let item_w = mask_width.max(1) as i32 + GLYPH_GUTTER * 2;
        let item_h = mask_height.max(1) as i32 + GLYPH_GUTTER * 2;

        // An oversized raster can never fit any layer. Do not enter the LRU
        // retry loop in that case: evicting cached glyphs cannot make a
        // too-large allocation fit and would corrupt all text already queued
        // for this frame.
        if !allocation_fits_layer(self.size, item_w, item_h) {
            log::warn!(
                target: "GlyphAtlas",
                "skipping oversized glyph raster {}x{} (atlas layer {}x{}, font size {})",
                mask_width,
                mask_height,
                self.size,
                self.size,
                font_size,
            );
            return GlyphAtlasEntry {
                layer: 0,
                u: 0.0,
                v: 0.0,
                uv_width: 0.0,
                uv_height: 0.0,
                pixel_width: 0,
                pixel_height: 0,
                left,
                top,
            };
        }

        // Try allocating (with eviction retries).
        let allocation = 'search: loop {
            // Try existing allocators.
            for (layer_idx, alloc) in self.allocators.iter_mut().enumerate() {
                if let Some(allocation) = alloc.allocate(size2(item_w, item_h)) {
                    break 'search Some((layer_idx, allocation));
                }
            }

            // Try adding a new layer.
            if self.layers < MAX_LAYERS {
                let new_layer = self.layers as usize;
                let mut alloc =
                    BucketedAtlasAllocator::new(size2(self.size as i32, self.size as i32));

                if let Some(allocation) = alloc.allocate(size2(item_w, item_h)) {
                    self.allocators.push(alloc);
                    self.cpu_layers
                        .push(vec![0u8; (self.size * self.size) as usize]);
                    self.layers += 1;

                    break 'search Some((new_layer, allocation));
                }
            }

            // Atlas full: evict LRU and retry.
            if self.evict_one() {
                continue;
            }

            break 'search None;
        };

        let Some((layer_idx, allocation)) = allocation else {
            log::warn!(target:"GlyphAtlas", "glyph atlas full ({} layers, no evictable entries)", self.layers);
            return GlyphAtlasEntry {
                layer: 0,
                u: 0.0,
                v: 0.0,
                uv_width: 0.0,
                uv_height: 0.0,
                pixel_width: 0,
                pixel_height: 0,
                left,
                top,
            };
        };

        let rect = allocation.rectangle;
        let alloc_id = allocation.id;

        let li = layer_idx as u32;
        let cache_val = (li, alloc_id, rect, mask_width, mask_height, left, top);

        // Make room in the cache if full (key is guaranteed absent).
        if self.glyph_map.len() >= LRU_CAPACITY
            && let Some((_k, (elayer, ealloc_id, ..))) = self.glyph_map.pop_lru()
        {
            self.allocators[elayer as usize].deallocate(ealloc_id);
        }
        self.glyph_map.put(key, cache_val);

        // Write glyph to CPU texture data cache
        let layer_data = &mut self.cpu_layers[li as usize];

        for y in rect.min.y..rect.max.y {
            let start = (y as u32 * self.size + rect.min.x as u32) as usize;
            layer_data[start..start + item_w as usize].fill(0);
        }

        for y in 0..mask_height {
            let src_start = (y * mask_width) as usize;
            let src_end = src_start + mask_width as usize;

            let dst_y = (rect.min.y + GLYPH_GUTTER) as u32 + y;
            let dst_x = (rect.min.x + GLYPH_GUTTER) as u32;
            let dst_start = (dst_y * self.size + dst_x) as usize;

            layer_data[dst_start..dst_start + mask_width as usize]
                .copy_from_slice(&alpha_mask[src_start..src_end]);
        }

        self.dirty_layers.insert(li);

        Self::entry(self.size, li, rect, mask_width, mask_height, left, top)
    }

    fn entry(
        atlas_size: u32,
        layer: u32,
        rect: etagere::Rectangle,
        width: u32,
        height: u32,
        left: i32,
        top: i32,
    ) -> GlyphAtlasEntry {
        let size = atlas_size as f32;
        GlyphAtlasEntry {
            layer,
            u: (rect.min.x + GLYPH_GUTTER) as f32 / size,
            v: (rect.min.y + GLYPH_GUTTER) as f32 / size,
            uv_width: width as f32 / size,
            uv_height: height as f32 / size,
            pixel_width: width,
            pixel_height: height,
            left,
            top,
        }
    }

    pub fn flush_uploads(&mut self, queue: &wgpu::Queue) {
        if self.dirty_layers.is_empty() {
            return;
        }

        let size = self.size;

        for &layer in &self.dirty_layers {
            let Some(layer_bytes) = self.cpu_layers.get(layer as usize) else {
                continue;
            };

            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let stride = size;
            let padded_stride = stride.div_ceil(align) * align;

            let layout = wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_stride),
                rows_per_image: Some(size),
            };

            if padded_stride == stride {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    layer_bytes,
                    layout,
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
            } else {
                let mut padded = Vec::with_capacity((padded_stride * size) as usize);

                for row in layer_bytes.chunks(stride as usize) {
                    padded.extend_from_slice(row);

                    let pad = (padded_stride - stride) as usize;
                    if pad > 0 {
                        padded.extend(std::iter::repeat_n(0u8, pad));
                    }
                }

                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &padded,
                    layout,
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }

        self.dirty_layers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::allocation_fits_layer;

    #[test]
    fn oversized_glyph_allocations_are_rejected_before_eviction() {
        assert!(allocation_fits_layer(1024, 1024, 32));
        assert!(!allocation_fits_layer(1024, 1025, 32));
        assert!(!allocation_fits_layer(1024, 32, 1025));
    }
}
