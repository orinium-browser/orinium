#![allow(dead_code)]

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ImageHandle {
    /// テクスチャID
    pub id: u64,
    /// 画像の幅
    pub width: u32,
    /// 画像の高さ
    pub height: u32,
    /// テクスチャビュー
    pub view: wgpu::TextureView,
    /// サンプラー
    pub sampler: wgpu::Sampler,
}

pub struct ImageManager {
    /// 画像IDカウンター
    counter: AtomicU64,
    /// 画像メタデータのマップ
    images: HashMap<u64, ImageMetadata>,
}

struct ImageMetadata {
    /// 画像の幅
    width: u32,
    /// 画像の高さ
    height: u32,
    /// テクスチャビュー
    view: wgpu::TextureView,
    /// サンプラー
    sampler: wgpu::Sampler,
}

impl ImageManager {
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
            images: HashMap::new(),
        }
    }

    /// URIから画像を読み込み、テクスチャとして登録する
    pub fn load_from_uri(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uri: &str,
        label: Option<&str>,
    ) -> Result<ImageHandle> {
        if !uri.starts_with("resource:///") {
            anyhow::bail!("Only resource:/// URIs are supported by ImageManager");
        }
        // strip scheme
        let rel = uri.trim_start_matches("resource:///");
        let mut path = PathBuf::from("resource");
        path.push(rel);

        let bytes = std::fs::read(&path)
            .with_context(|| format!("failed to read resource file: {}", path.display()))?;
        let img = image::load_from_memory(&bytes).context("failed to decode image")?;
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // write texture using new wgpu TexelCopy* API
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        self.images.insert(
            id,
            ImageMetadata {
                width,
                height,
                view: view.clone(),
                sampler: sampler.clone(),
            },
        );

        Ok(ImageHandle {
            id,
            width,
            height,
            view,
            sampler,
        })
    }

    /// 画像のサイズを取得
    pub fn get_size(&self, id: u64) -> Option<(u32, u32)> {
        self.images.get(&id).map(|m| (m.width, m.height))
    }

    /// テクスチャビューとサンプラーを取得する
    pub fn get_view_sampler(&self, id: u64) -> Option<(&wgpu::TextureView, &wgpu::Sampler)> {
        self.images.get(&id).map(|m| (&m.view, &m.sampler))
    }
}
