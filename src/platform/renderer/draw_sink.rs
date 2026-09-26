//! The seam between draw-command production and draw-command execution.
//!
//! The browser layer produces a `Vec<DrawCommand>` for a window and nothing
//! more; executing those commands is a platform concern. Handing the concrete
//! GPU renderer down through the call stack instead would make every layer
//! above know that rendering happens on `wgpu`, on a surface bound to an OS
//! window — which is exactly the assumption that breaks if the surface ever
//! has to live in another process.
//!
//! [`DrawSink`] is the contract. The shell owns the implementation; today that
//! is [`GpuRenderer`](super::gpu::GpuRenderer), but a recording sink, an
//! offscreen one, or a remote one are equally valid.

use std::fmt;

use crate::engine::renderer_model::DrawCommand;

/// Failures a [`DrawSink`] can report.
#[derive(Debug)]
pub enum DrawError {
    /// No drawable could be acquired (e.g. a lost or outdated surface).
    Surface(String),
    /// The sink refused the submitted command stream.
    Rejected(String),
}

impl fmt::Display for DrawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Surface(s) => write!(f, "Could not acquire a drawable: {s}"),
            Self::Rejected(s) => write!(f, "Draw commands rejected: {s}"),
        }
    }
}

impl std::error::Error for DrawError {}

/// A destination for one window's draw commands.
///
/// Callers produce commands with [`DrawCommand`] and hand them over; the sink
/// decides when and how they reach a display.
///
/// [`upload`](Self::upload) only records, and [`present`](Self::present)
/// submits, so a caller may feed several command sets before showing
/// anything — or record without ever presenting, which is what the
/// offscreen sinks do.
pub trait DrawSink {
    /// Records and uploads `commands` for the next frame.
    fn upload(&mut self, commands: &[DrawCommand]);

    /// Draws and presents whatever [`upload`](Self::upload) recorded.
    fn present(&mut self) -> Result<(), DrawError>;

    /// Notifies the sink that its drawable changed size, in physical pixels.
    fn resize(&mut self, size: (u32, u32));

    /// Notifies the sink that the display scale factor changed.
    fn set_scale_factor(&mut self, scale_factor: f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that only records, standing in for a real GPU. Its existence is
    /// the point of the port: the browser layer can be driven end to end
    /// without a window, a device, or `wgpu`.
    #[derive(Default)]
    struct RecordingSink {
        frames: Vec<Vec<DrawCommand>>,
        pending: Vec<DrawCommand>,
        size: (u32, u32),
        scale_factor: f64,
        present_err: Option<DrawError>,
    }

    impl DrawSink for RecordingSink {
        fn upload(&mut self, commands: &[DrawCommand]) {
            self.pending.extend_from_slice(commands);
        }

        fn present(&mut self) -> Result<(), DrawError> {
            match self.present_err.take() {
                Some(err) => Err(err),
                None => {
                    self.frames.push(std::mem::take(&mut self.pending));
                    Ok(())
                }
            }
        }

        fn resize(&mut self, size: (u32, u32)) {
            self.size = size;
        }

        fn set_scale_factor(&mut self, scale_factor: f64) {
            self.scale_factor = scale_factor;
        }
    }

    fn command(x: f32) -> DrawCommand {
        DrawCommand::PushTransform {
            transform: crate::engine::renderer_model::AffineTransform::translate(x, 0.0),
        }
    }

    #[test]
    fn upload_records_and_present_flushes() {
        let mut sink = RecordingSink::default();

        sink.upload(&[command(1.0)]);
        assert!(sink.frames.is_empty(), "upload must not present");

        sink.present().unwrap();
        assert_eq!(sink.frames.len(), 1);
        assert_eq!(sink.frames[0].len(), 1);
    }

    #[test]
    fn several_uploads_accumulate_into_one_frame() {
        let mut sink = RecordingSink::default();

        sink.upload(&[command(1.0), command(2.0)]);
        sink.upload(&[command(3.0)]);
        sink.present().unwrap();

        assert_eq!(sink.frames.len(), 1);
        assert_eq!(sink.frames[0].len(), 3);
    }

    #[test]
    fn present_starts_a_fresh_frame() {
        let mut sink = RecordingSink::default();

        sink.upload(&[command(1.0)]);
        sink.present().unwrap();
        sink.upload(&[command(2.0)]);
        sink.present().unwrap();

        assert_eq!(sink.frames.len(), 2);
        assert_eq!(sink.frames[1].len(), 1);
    }

    #[test]
    fn present_errors_surface_to_the_caller() {
        let mut sink = RecordingSink {
            present_err: Some(DrawError::Surface("lost".to_string())),
            ..Default::default()
        };

        let err = sink.present().unwrap_err();
        assert!(matches!(err, DrawError::Surface(_)));
        assert!(err.to_string().contains("lost"));
    }

    #[test]
    fn surface_changes_are_forwarded() {
        let mut sink = RecordingSink::default();
        sink.resize((1024, 768));
        sink.set_scale_factor(2.0);

        assert_eq!(sink.size, (1024, 768));
        assert_eq!(sink.scale_factor, 2.0);
    }
}
