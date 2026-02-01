use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};
use crate::platform::io as platform_io;

/// 音声の管理を行う構造体
pub struct SoundManager {
    /// f32のインタリーブドサンプルバッファ
    samples: Arc<Mutex<Vec<f32>>>,
    /// 現在の再生位置（フレーム単位）
    play_pos: Arc<Mutex<usize>>,
    /// ソースのチャンネル数
    src_channels: usize,
    /// ソースのサンプルレート
    src_sample_rate: u32,
    /// cpalのストリーム
    stream: Option<cpal::Stream>,
}

impl SoundManager {
    /// 初期化
    pub fn init() -> Result<Arc<Mutex<Self>>> {
        let manager = SoundManager {
            samples: Arc::new(Mutex::new(Vec::new())),
            play_pos: Arc::new(Mutex::new(0)),
            src_channels: 0,
            src_sample_rate: 0,
            stream: None,
        };
        Ok(Arc::new(Mutex::new(manager)))
    }

    /// cpalストリームを確保する
    fn ensure_stream(&mut self) -> Result<()> {
        if self.stream.is_some() {
            return Ok(());
        }

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No default output device available")?;
        let supported_cfg = device.default_output_config().context("Failed to get default output config")?;
        let config: StreamConfig = supported_cfg.clone().into();
        let sample_format = supported_cfg.sample_format();
        let output_channels = config.channels as usize;

        let samples = self.samples.clone();
        let play_pos = self.play_pos.clone();
        let src_channels = self.src_channels;

        let err_fn = |err| log::error!("cpal stream error: {}", err);

        let latency = Some(Duration::from_millis(100));

        let stream = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    write_output_f32(data, src_channels, output_channels, &samples, &play_pos)
                },
                err_fn,
                latency,
            )?,
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    write_output_i16(data, src_channels, output_channels, &samples, &play_pos)
                },
                err_fn,
                latency,
            )?,
            SampleFormat::U16 => device.build_output_stream(
                &config,
                move |data: &mut [u16], _| {
                    write_output_u16(data, src_channels, output_channels, &samples, &play_pos)
                },
                err_fn,
                latency,
            )?,
            _ => return Err(anyhow::anyhow!("Unsupported sample format from output device")),
        };

        stream.play()?;
        self.stream = Some(stream);
        Ok(())
    }

    /// バイト列から音声を再生する
    pub fn play_from_bytes(&mut self, data: &[u8]) -> Result<()> {
        let (samples, channels, sample_rate) = decode(data)?;
        // replace buffer
        {
            let mut buf = match self.samples.lock() {
                Ok(x) => x,
                Err(_) => todo!(),
            };
            *buf = samples;
        }
        // reset position
        {
            let mut pos = match self.play_pos.lock() {
                Ok(x) => x,
                Err(_) => todo!(),
            };
            *pos = 0;
        }
        self.src_channels = channels;
        self.src_sample_rate = sample_rate;

        self.ensure_stream()?;

        Ok(())
    }

    /// ローカルファイルから音声を再生する
    pub fn play_from_file(&mut self, path: &str) -> Result<()> {
        let data = platform_io::load_local_file(path).with_context(|| format!("Failed to read local file: {}", path))?;
        self.play_from_bytes(&data)
    }

    /// URIから音声を再生する（resourceまたはfileスキームに対応）
    ///
    /// 通常の再生には `play_from_bytes` を使用してください。
    /// これはテスト用メソッドです
    pub fn play_from_local_uri(&mut self, uri: &str) -> Result<()> {
        if uri.starts_with("resource:") {
            let rel = uri
                .trim_start_matches("resource:///")
                .trim_start_matches("resource://")
                .trim_start_matches("resource:/")
                .trim_start_matches("resource:");
            let rel = rel.trim_start_matches('/');
            let data = platform_io::load_resource(rel).with_context(|| format!("Failed to load resource: {}", rel))?;
            return self.play_from_bytes(&data);
        }
        if uri.starts_with("file://") {
            let p = uri.trim_start_matches("file://");
            let data = platform_io::load_local_file(p).with_context(|| format!("Failed to read local file from URI: {}", p))?;
            return self.play_from_bytes(&data);
        }
        let data = platform_io::load_local_file(uri).with_context(|| format!("Failed to read local file: {}", uri))?;
        self.play_from_bytes(&data)
    }
}

/// 音声をデコードする
fn decode(data: &[u8]) -> Result<(Vec<f32>, usize, u32)> {
    let cursor = Cursor::new(data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let hint = Hint::new();
    let probed = get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("Failed to probe media format")?;

    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow::anyhow!("No default audio track found"))?;
    let codec_params = &track.codec_params;
    let mut decoder = get_codecs().make(&codec_params, &DecoderOptions::default()).context("Failed to create decoder")?;

    let mut samples: Vec<f32> = Vec::new();
    let mut channels: usize = codec_params.channels.map(|c| c.count()).unwrap_or(1);
    let mut sample_rate: u32 = codec_params.sample_rate.unwrap_or(44100);

    loop {
        match format.next_packet() {
            Ok(packet) => match decoder.decode(&packet) {
                Ok(audio_buf) => match audio_buf {
                    AudioBufferRef::U8(buf) => {
                        let ab = buf.as_ref();
                        channels = ab.spec().channels.count();
                        sample_rate = ab.spec().rate;
                        let frames = ab.frames();
                        for f in 0..frames {
                            for ch in 0..channels {
                                let v = ab.chan(ch)[f] as f32;
                                samples.push((v - 128.0) / 128.0);
                            }
                        }
                    }
                    AudioBufferRef::U16(buf) => {
                        let ab = buf.as_ref();
                        channels = ab.spec().channels.count();
                        sample_rate = ab.spec().rate;
                        let frames = ab.frames();
                        for f in 0..frames {
                            for ch in 0..channels {
                                let v = ab.chan(ch)[f] as f32;
                                samples.push((v - 32768.0) / 32768.0);
                            }
                        }
                    }
                    AudioBufferRef::S16(buf) => {
                        let ab = buf.as_ref();
                        channels = ab.spec().channels.count();
                        sample_rate = ab.spec().rate;
                        let frames = ab.frames();
                        for f in 0..frames {
                            for ch in 0..channels {
                                let v = ab.chan(ch)[f] as f32;
                                samples.push(v / i16::MAX as f32);
                            }
                        }
                    }
                    AudioBufferRef::F32(buf) => {
                        let ab = buf.as_ref();
                        channels = ab.spec().channels.count();
                        sample_rate = ab.spec().rate;
                        let frames = ab.frames();
                        for f in 0..frames {
                            for ch in 0..channels {
                                let v = ab.chan(ch)[f];
                                samples.push(v);
                            }
                        }
                    }
                    AudioBufferRef::F64(buf) => {
                        let ab = buf.as_ref();
                        channels = ab.spec().channels.count();
                        sample_rate = ab.spec().rate;
                        let frames = ab.frames();
                        for f in 0..frames {
                            for ch in 0..channels {
                                let v = ab.chan(ch)[f];
                                samples.push(v as f32);
                            }
                        }
                    }
                    _ => {
                        // Unsupported format
                    }
                },
                Err(_) => { /* ignore */ }
            },
            Err(_) => break,
        }
    }

    Ok((samples, channels, sample_rate))
}

/// 出力バッファに音声データを書き込む（f32）
fn write_output_f32(output: &mut [f32], src_channels: usize, out_channels: usize, samples: &Arc<Mutex<Vec<f32>>>, pos: &Arc<Mutex<usize>>) {
    let mut p = pos.lock().unwrap();
    let buf = samples.lock().unwrap();
    let total_frames = if src_channels > 0 { buf.len() / src_channels } else { 0 };

    if out_channels == 0 {
        return;
    }
    let frames_to_write = output.len() / out_channels;

    for frame in 0..frames_to_write {
        if total_frames == 0 || *p >= total_frames {
            // zero out remaining
            for ch in 0..out_channels {
                output[frame * out_channels + ch] = 0.0;
            }
            continue;
        }
        for ch in 0..out_channels {
            let src_index = (*p * src_channels) + (ch % src_channels);
            if src_index < buf.len() {
                output[frame * out_channels + ch] = buf[src_index];
            } else {
                output[frame * out_channels + ch] = 0.0;
            }
        }
        *p += 1;
    }
}

/// 出力バッファに音声データを書き込む（i16）
fn write_output_i16(output: &mut [i16], src_channels: usize, out_channels: usize, samples: &Arc<Mutex<Vec<f32>>>, pos: &Arc<Mutex<usize>>) {
    let mut p = pos.lock().unwrap();
    let buf = samples.lock().unwrap();
    let total_frames = if src_channels > 0 { buf.len() / src_channels } else { 0 };

    if out_channels == 0 { return; }
    let frames_to_write = output.len() / out_channels;

    for frame in 0..frames_to_write {
        if total_frames == 0 || *p >= total_frames {
            for ch in 0..out_channels { output[frame * out_channels + ch] = 0; }
            continue;
        }
        for ch in 0..out_channels {
            let src_index = (*p * src_channels) + (ch % src_channels);
            if src_index < buf.len() {
                let v = buf[src_index].clamp(-1.0, 1.0);
                output[frame * out_channels + ch] = (v * i16::MAX as f32) as i16;
            } else {
                output[frame * out_channels + ch] = 0;
            }
        }
        *p += 1;
    }
}

/// 出力バッファに音声データを書き込む（u16）
fn write_output_u16(output: &mut [u16], src_channels: usize, out_channels: usize, samples: &Arc<Mutex<Vec<f32>>>, pos: &Arc<Mutex<usize>>) {
    let mut p = pos.lock().unwrap();
    let buf = samples.lock().unwrap();
    let total_frames = if src_channels > 0 { buf.len() / src_channels } else { 0 };

    if out_channels == 0 { return; }
    let frames_to_write = output.len() / out_channels;

    for frame in 0..frames_to_write {
        if total_frames == 0 || *p >= total_frames {
            for ch in 0..out_channels { output[frame * out_channels + ch] = 0; }
            continue;
        }
        for ch in 0..out_channels {
            let src_index = (*p * src_channels) + (ch % src_channels);
            if src_index < buf.len() {
                let v = buf[src_index].clamp(-1.0, 1.0);
                output[frame * out_channels + ch] = ((v * 0.5 + 0.5) * u16::MAX as f32) as u16;
            } else {
                output[frame * out_channels + ch] = 0;
            }
        }
        *p += 1;
    }
}
