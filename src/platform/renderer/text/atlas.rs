use std::num::NonZeroUsize;

use etagere::{BucketedAtlasAllocator, size2};
use lru::LruCache;
use orinium_text::{FontKey, fontdb};

/// (fontdb ID, glyph_id, font_size_bits)
type GlyphKey = (fontdb::ID, u32, u32);

/// (layer_index, alloc_id, rectangle_packed, actual_width, actual_height)
type GlyphCacheValue = (u32, etagere::AllocId, etagere::Rectangle, u32, u32);

/// Normalized UV rect: (layer, u, v, width, height)
pub type GlyphUVRect = (u32, f32, f32, f32, f32);

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
    cpu_data: Vec<u8>,
    dirty_layers: std::collections::HashSet<u32>,
}

const MAX_LAYERS: u32 = 16;
const INITIAL_SIZE: u32 = 1024;
/// Maximum number of glyph entries in the LRU cache.
/// Set to roughly match the pixel capacity of a full atlas
/// (16 layers × 1024² px ÷ ~400 px avg glyph area ≈ 40k).
/// 8192 is a safe upper bound that prevents unbounded growth.
const LRU_CAPACITY: usize = 8192;

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

        Self {
            texture,
            view,
            size,
            layers: 1,
            allocators: vec![allocator],
            glyph_map: LruCache::new(NonZeroUsize::new(LRU_CAPACITY).unwrap()),
            cpu_data: vec![0u8; (INITIAL_SIZE * INITIAL_SIZE * MAX_LAYERS) as usize],
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
    ) -> Option<GlyphUVRect> {
        let key = (font_key.0, glyph_id, font_size.to_bits());
        let (layer, _alloc_id, rect, aw, ah) = self.glyph_map.get(&key)?;
        let (u, v) = (
            rect.min.x as f32 / self.size as f32,
            rect.min.y as f32 / self.size as f32,
        );
        let (w, h) = (*aw as f32 / self.size as f32, *ah as f32 / self.size as f32);
        Some((*layer, u, v, w, h))
    }

    /// Evict a single LRU entry from the cache and free its atlas allocation.
    /// Returns `true` if an entry was evicted.
    fn evict_one(&mut self) -> bool {
        if let Some((_key, (layer, alloc_id, _rect, _aw, _ah))) = self.glyph_map.pop_lru() {
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
        alpha_mask: &[u8],
        mask_width: u32,
        mask_height: u32,
    ) -> GlyphUVRect {
        if mask_width == 0 || mask_height == 0 {
            return (0, 0.0, 0.0, 0.0, 0.0);
        }

        let key = (font_key.0, glyph_id, font_size.to_bits());

        // Check if already present (updates LRU position).
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
            return (0, 0.0, 0.0, 0.0, 0.0);
        };

        let rect = allocation.rectangle;
        let alloc_id = allocation.id;

        let li = layer_idx as u32;
        let cache_val = (li, alloc_id, rect, mask_width, mask_height);

        // Make room in the cache if full (key is guaranteed absent).
        if self.glyph_map.len() >= LRU_CAPACITY {
            if let Some((_k, (elayer, ealloc_id, _, _, _))) = self.glyph_map.pop_lru() {
                self.allocators[elayer as usize].deallocate(ealloc_id);
            }
        }
        self.glyph_map.put(key, cache_val);

        // Write glyph to CPU texture data cache
        let layer_offset = (li * self.size * self.size) as usize;
        for y in 0..mask_height {
            let src_start = (y * mask_width) as usize;
            let src_end = src_start + mask_width as usize;

            let dst_y = rect.min.y as u32 + y;
            let dst_start = layer_offset + (dst_y * self.size + rect.min.x as u32) as usize;

            self.cpu_data[dst_start..dst_start + mask_width as usize]
                .copy_from_slice(&alpha_mask[src_start..src_end]);
        }
        self.dirty_layers.insert(li);

        let (u, v) = (
            rect.min.x as f32 / self.size as f32,
            rect.min.y as f32 / self.size as f32,
        );
        let (w, h) = (
            mask_width as f32 / self.size as f32,
            mask_height as f32 / self.size as f32,
        );
        (li, u, v, w, h)
    }

    pub fn flush_uploads(&mut self, queue: &wgpu::Queue) {
        if self.dirty_layers.is_empty() {
            return;
        }

        let size = self.size;
        let layer_size = (size * size) as usize;

        for &layer in &self.dirty_layers {
            let offset = (layer * size * size) as usize;
            let layer_bytes = &self.cpu_data[offset..offset + layer_size];

            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let stride = size; // R8Unorm so bytes_per_pixel = 1, stride = size
            let padded_stride = ((stride + align - 1) / align) * align;

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
                        padded.extend(std::iter::repeat(0u8).take(pad));
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
