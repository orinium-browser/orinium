//! Audio playback abstraction for the engine's media widgets.
//!
//! The engine lays out and drives UI widgets such as the `<audio>` control,
//! but it must not own an audio device: decoding, output device selection
//! and stream management are platform concerns. This module defines the
//! interface the engine uses to talk to whatever the host environment
//! provides, mirroring [`crate::engine::bridge::text`] for text measurement.
//!
//! # Responsibilities
//!
//! - Accept a media source (fetched bytes and/or a URI) and start playback
//! - Expose transport controls (pause/resume) and playback position
//! - Report whether the decoded source has finished
//!
//! # Non-Responsibilities
//!
//! - Deciding *when* to play (the engine's widget drives that)
//! - Fetching media bytes (that goes through the resource loader)
//!
//! # Data Flow
//!
//! ```text
//! <audio> element → AudioSinkFactory → AudioSink → platform audio device
//! ```

use std::fmt;
use std::sync::Arc;

/* ============================
 * Errors
 * ============================ */

/// Failures surfaced by an [`AudioSink`].
#[derive(Debug)]
pub enum AudioError {
    /// The media bytes could not be decoded.
    Decode(String),
    /// The platform audio device rejected the request.
    Device(String),
    /// Playback was requested before any source was loaded.
    NotLoaded,
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(s) => write!(f, "Failed to decode audio: {s}"),
            Self::Device(s) => write!(f, "Audio device error: {s}"),
            Self::NotLoaded => write!(f, "No audio source is loaded"),
        }
    }
}

impl std::error::Error for AudioError {}

/* ============================
 * Traits
 * ============================ */

/// A single playable audio source.
///
/// Implementations own whatever playback state the platform needs, so a sink
/// is **not** shared between two `<audio>` elements — use
/// [`AudioSinkFactory`] to obtain one instance per element.
pub trait AudioSink: Send + Sync {
    /// Decodes `data` and rewinds to the start, without starting playback.
    fn preload(&self, data: &[u8]) -> Result<(), AudioError>;

    /// Starts playback of `source`, restarting from the beginning if the
    /// source is not currently playing.
    ///
    /// `data` carries the already-fetched media bytes when they are
    /// available. Sources that the platform must resolve itself (for example
    /// `file:` or `resource:` URIs) may ignore `data` and load `source`.
    fn play(&self, source: &str, data: Option<&[u8]>) -> Result<(), AudioError>;

    /// Pauses playback, preserving the current position.
    fn pause(&self) -> Result<(), AudioError>;

    /// Resumes playback from the current position.
    fn resume(&self) -> Result<(), AudioError>;

    /// The current playback position, in seconds.
    fn current_seconds(&self) -> f32;

    /// The decoded duration in seconds, or `0.0` when nothing is loaded.
    fn duration_seconds(&self) -> f32;

    /// Whether the decoded source reached its end.
    fn is_finished(&self) -> bool;
}

/// Creates one [`AudioSink`] per media element.
///
/// Audio widgets are built off the UI thread during layout, so the engine
/// cannot construct a platform sink itself. The host injects this factory
/// through the layout task instead.
pub trait AudioSinkFactory: Send + Sync {
    /// Builds a sink for one `<audio>` element.
    fn create(&self) -> Arc<dyn AudioSink>;
}

/* ============================
 * Null (headless / test)
 * ============================ */

/// A factory producing sinks that decode and play nothing.
///
/// Keeps the engine usable in builds without an audio device, such as
/// headless test runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullAudioSinkFactory;

impl AudioSinkFactory for NullAudioSinkFactory {
    fn create(&self) -> Arc<dyn AudioSink> {
        Arc::new(NullAudioSink)
    }
}

#[derive(Debug)]
struct NullAudioSink;

impl AudioSink for NullAudioSink {
    fn preload(&self, _data: &[u8]) -> Result<(), AudioError> {
        Ok(())
    }

    fn play(&self, _source: &str, _data: Option<&[u8]>) -> Result<(), AudioError> {
        Ok(())
    }

    fn pause(&self) -> Result<(), AudioError> {
        Ok(())
    }

    fn resume(&self) -> Result<(), AudioError> {
        Ok(())
    }

    fn current_seconds(&self) -> f32 {
        0.0
    }

    fn duration_seconds(&self) -> f32 {
        0.0
    }

    fn is_finished(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_accepts_everything_and_reports_no_progress() {
        let factory = NullAudioSinkFactory;
        let sink = factory.create();

        assert!(sink.preload(b"not really audio").is_ok());
        assert!(sink.play("resource:///audio/birds.mp3", None).is_ok());
        assert!(sink.pause().is_ok());
        assert!(sink.resume().is_ok());

        assert_eq!(sink.current_seconds(), 0.0);
        assert_eq!(sink.duration_seconds(), 0.0);
        assert!(sink.is_finished());
    }

    #[test]
    fn each_created_sink_is_distinct() {
        let factory = NullAudioSinkFactory;
        assert!(!Arc::ptr_eq(&factory.create(), &factory.create()));
    }
}
