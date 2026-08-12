//! Engine-rendered controls for the HTML `<audio>` element.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use ui_layout::Style;

use crate::engine::layouter::types::{Color, TextFlowStyle, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Image, Paint, rect_path};
use crate::engine::ui::custom_node::{ContentSize, CustomNode, PointerEvent};
use crate::platform::audio::SoundManager;

const PLAYER_WIDTH: f32 = 170.0;
const PLAYER_HEIGHT: f32 = 32.0;
const BUTTON_WIDTH: f32 = 32.0;
const ICON_SIZE: f32 = 18.0;
const ICON_RASTER_SIZE: u32 = 64;
const TIMER_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

const PLAY_SVG: &[u8] = include_bytes!("../../../../resource/icons/audio_play.svg");
const STOP_SVG: &[u8] = include_bytes!("../../../../resource/icons/audio_stop.svg");

static PLAY_ICON: LazyLock<Result<Image, String>> =
    LazyLock::new(|| rasterize_svg(PLAY_SVG).map_err(|error| error.to_string()));
static STOP_ICON: LazyLock<Result<Image, String>> =
    LazyLock::new(|| rasterize_svg(STOP_SVG).map_err(|error| error.to_string()));

/// The compact play/stop control shown for an HTML `<audio>` element.
pub struct AudioComponent {
    source: String,
    data: Option<Arc<[u8]>>,
    sound: Arc<Mutex<SoundManager>>,
    loaded: AtomicBool,
    playing: AtomicBool,
    hovered: AtomicBool,
    pressed: AtomicBool,
    dirty: AtomicBool,
    last_timer_repaint: Mutex<Instant>,
}

impl std::fmt::Debug for AudioComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioComponent")
            .field("source", &self.source)
            .field("loaded", &self.loaded)
            .field("playing", &self.playing)
            .finish_non_exhaustive()
    }
}

impl AudioComponent {
    pub fn new(source: impl Into<String>, data: Option<Arc<[u8]>>) -> Self {
        let sound = SoundManager::init().expect("SoundManager initialization cannot fail");
        let loaded = data.as_ref().is_some_and(|data| {
            let result = sound
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .load_from_bytes(data);
            if let Err(error) = result {
                log::error!("Failed to decode <audio> data: {error}");
                false
            } else {
                true
            }
        });
        Self {
            source: source.into(),
            data,
            sound,
            loaded: AtomicBool::new(loaded),
            playing: AtomicBool::new(false),
            hovered: AtomicBool::new(false),
            pressed: AtomicBool::new(false),
            dirty: AtomicBool::new(true),
            last_timer_repaint: Mutex::new(Instant::now()),
        }
    }

    fn toggle_playback(&self) {
        if self.source.is_empty() {
            log::warn!("Cannot play <audio> without a media source");
            return;
        }

        let mut sound = self.sound.lock().unwrap_or_else(|e| e.into_inner());
        let result = if !self.loaded.load(Ordering::Relaxed) || sound.is_finished() {
            if let Some(data) = &self.data {
                sound.play_from_bytes(data)
            } else {
                sound.play_from_local_uri(&self.source)
            }
        } else if self.playing.load(Ordering::Relaxed) {
            sound.pause()
        } else {
            sound.resume()
        };

        match result {
            Ok(()) => {
                if !self.loaded.load(Ordering::Relaxed) || sound.is_finished() {
                    self.loaded.store(true, Ordering::Relaxed);
                }
                self.playing.fetch_xor(true, Ordering::Relaxed);
            }
            Err(error) => log::error!("Failed to toggle <audio> playback: {error}"),
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn playback_times(&self) -> (f32, f32) {
        let sound = self.sound.lock().unwrap_or_else(|e| e.into_inner());
        (sound.current_seconds(), sound.duration_seconds())
    }

    fn update_finished_state(&self) {
        if !self.playing.load(Ordering::Relaxed) {
            return;
        }
        let finished = self
            .sound
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_finished();
        if finished && self.playing.swap(false, Ordering::Relaxed) {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }
}

impl CustomNode for AudioComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        text_flow_style: &TextFlowStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        self.update_finished_state();

        let button_color = if self.pressed.load(Ordering::Relaxed) {
            Color(45, 45, 45, 255)
        } else if self.hovered.load(Ordering::Relaxed) {
            Color(85, 85, 85, 255)
        } else {
            Color(65, 65, 65, 255)
        };
        cmd_buf.push(solid_fill(
            rect_path(0.0, 0.0, BUTTON_WIDTH.min(size.width), size.height),
            button_color,
        ));

        let icon = if self.playing.load(Ordering::Relaxed) {
            STOP_ICON.as_ref()
        } else {
            PLAY_ICON.as_ref()
        };
        if let Ok(icon) = icon {
            let icon_size = ICON_SIZE.min(size.height);
            cmd_buf.push(DrawCommand::Fill {
                path: rect_path(
                    (BUTTON_WIDTH - icon_size) * 0.5,
                    (size.height - icon_size) * 0.5,
                    icon_size,
                    icon_size,
                ),
                paint: Paint {
                    brush: Brush::Image(icon.clone()),
                    opacity: 1.0,
                },
                rule: FillRule::NonZero,
            });
        }

        let mut time_style = text_style.clone();
        time_style.color = Color(40, 40, 40, 255);
        let (current, duration) = self.playback_times();
        cmd_buf.push(DrawCommand::DrawText {
            x: BUTTON_WIDTH + 10.0,
            y: ((size.height - text_flow_style.font_size) * 0.5).max(0.0),
            text: format!(
                "{} / {}",
                format_media_time(current),
                format_media_time(duration)
            )
            .into(),
            style: time_style,
            flow_style: *text_flow_style,
        });
    }

    fn intrinsic_size(&self) -> ContentSize {
        ContentSize {
            width: PLAYER_WIDTH,
            height: PLAYER_HEIGHT,
        }
    }

    fn on_pointer_event(&self, event: PointerEvent) -> bool {
        match event {
            PointerEvent::Move { x, .. } => {
                self.set_hovered(x < BUTTON_WIDTH);
                x < BUTTON_WIDTH
            }
            PointerEvent::Down { x, .. } if x < BUTTON_WIDTH => {
                self.pressed.store(true, Ordering::Relaxed);
                self.dirty.store(true, Ordering::Relaxed);
                true
            }
            PointerEvent::Up { x, .. } => {
                let clicked = self.pressed.swap(false, Ordering::Relaxed) && x < BUTTON_WIDTH;
                if clicked {
                    self.toggle_playback();
                }
                self.dirty.store(true, Ordering::Relaxed);
                clicked
            }
            PointerEvent::Leave => {
                self.set_hovered(false);
                self.pressed.store(false, Ordering::Relaxed);
                false
            }
            PointerEvent::Down { .. } => false,
        }
    }

    fn set_hovered(&self, hovered: bool) {
        if self.hovered.swap(hovered, Ordering::Relaxed) != hovered {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    fn is_hovered(&self) -> bool {
        self.hovered.load(Ordering::Relaxed)
    }

    fn needs_repaint(&self) -> bool {
        self.update_finished_state();
        if self.playing.load(Ordering::Relaxed) {
            let mut last_repaint = self
                .last_timer_repaint
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if last_repaint.elapsed() >= TIMER_REPAINT_INTERVAL {
                *last_repaint = Instant::now();
                return true;
            }
        }
        self.dirty.swap(false, Ordering::Relaxed)
    }

    fn role(&self) -> Option<&'static str> {
        Some("group")
    }

    fn label(&self) -> Option<String> {
        Some("Audio player".to_string())
    }
}

fn solid_fill(path: crate::engine::renderer_model::Path, color: Color) -> DrawCommand {
    DrawCommand::Fill {
        path,
        paint: Paint {
            brush: Brush::Solid(color),
            opacity: 1.0,
        },
        rule: FillRule::NonZero,
    }
}

fn format_media_time(seconds: f32) -> String {
    let total_seconds = seconds.max(0.0).floor() as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn rasterize_svg(svg: &[u8]) -> anyhow::Result<Image> {
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(svg, &options)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(ICON_RASTER_SIZE, ICON_RASTER_SIZE)
        .ok_or_else(|| anyhow::anyhow!("failed to allocate SVG icon pixmap"))?;
    let size = tree.size();
    let transform = resvg::tiny_skia::Transform::from_scale(
        ICON_RASTER_SIZE as f32 / size.width(),
        ICON_RASTER_SIZE as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // tiny-skia stores premultiplied RGBA, while the renderer model accepts
    // straight-alpha RGBA. Convert once when the static icon is initialized.
    let mut rgba = pixmap.data().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3] as u16;
        if alpha == 0 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u16 * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
    Image::from_rgba(ICON_RASTER_SIZE, ICON_RASTER_SIZE, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_assets_decode_to_renderer_images() {
        assert!(PLAY_ICON.is_ok());
        assert!(STOP_ICON.is_ok());
    }

    #[test]
    fn audio_control_draws_button_icon_and_seconds() {
        let component = AudioComponent::new("resource:///audio/birds.mp3", None);
        let mut commands = Vec::new();
        component.draw(
            &mut commands,
            &TextStyle::default(),
            &TextFlowStyle::default(),
        );
        assert!(matches!(commands.first(), Some(DrawCommand::Fill { .. })));
        assert!(commands.iter().any(
            |command| matches!(command, DrawCommand::DrawText { text, .. } if text == "0:00 / 0:00")
        ));
    }

    #[test]
    fn media_time_uses_minutes_and_hours() {
        assert_eq!(format_media_time(0.0), "0:00");
        assert_eq!(format_media_time(83.9), "1:23");
        assert_eq!(format_media_time(3_661.0), "1:01:01");
    }

    #[test]
    fn only_left_button_accepts_pointer_down() {
        let component = AudioComponent::new("", None);
        assert!(!component.on_pointer_event(PointerEvent::Down { x: 80.0, y: 10.0 }));
        assert!(component.on_pointer_event(PointerEvent::Down { x: 10.0, y: 10.0 }));
    }
}
