//! Renders geometric shapes directly through [`DrawCommand`]s, bypassing the
//! browser: a circle (正円) with a radial gradient, a rounded square
//! (丸角の正方形) with a linear gradient, and a small solid circle.
//!
//! ```sh
//! cargo run --example draw_shapes
//! ```

#[path = "common/mod.rs"]
mod common;

use anyhow::Result;
use common::{EventOutcome, ShapeWindow};
use orinium_browser::engine::layouter::types::{
    Color, ColorStop, Gradient, GradientKind, RadialShape, RadialSizeKind,
};
use orinium_browser::engine::renderer_model::{
    Brush, DrawCommand, FillRule, Paint, Path, ellipse_path, rounded_rect_path,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// Builds a `Fill` draw command with the given brush and full opacity.
fn fill(path: Path, brush: Brush) -> DrawCommand {
    DrawCommand::Fill {
        path,
        paint: Paint {
            brush,
            opacity: 1.0,
        },
        rule: FillRule::NonZero,
    }
}

fn gradient(positions: Vec<(Color, f32)>, kind: GradientKind) -> Brush {
    Brush::Gradient(Gradient {
        kind,
        stops: positions
            .into_iter()
            .map(|(color, position)| ColorStop {
                color,
                position: Some(position),
            })
            .collect(),
    })
}

fn build_draw_commands() -> Vec<DrawCommand> {
    vec![
        // 正円: radial gradient (yellow → green). Gradient fills now follow
        // the path shape, so the circle stays round.
        fill(
            ellipse_path(200.0, 250.0, 120.0, 120.0),
            gradient(
                vec![
                    (Color(250, 220, 90, 255), 0.0),
                    (Color(90, 200, 130, 255), 1.0),
                ],
                GradientKind::Radial {
                    shape: RadialShape::Circle,
                    size: RadialSizeKind::FarthestCorner,
                    position: (0.5, 0.5),
                },
            ),
        ),
        // 丸角の正方形: linear gradient (red → blue, left to right), rounded
        // corners preserved by the path-shaped fill.
        fill(
            rounded_rect_path(
                430.0,
                130.0,
                240.0,
                240.0,
                (48.0, 48.0),
                (48.0, 48.0),
                (48.0, 48.0),
                (48.0, 48.0),
            ),
            gradient(
                vec![
                    (Color(235, 90, 90, 255), 0.0),
                    (Color(80, 140, 235, 255), 1.0),
                ],
                GradientKind::Linear { angle: 90.0 },
            ),
        ),
        // 単色の小円: solid coral fill as a solid-color reference.
        fill(
            ellipse_path(200.0, 500.0, 40.0, 40.0),
            Brush::Solid(Color(235, 90, 90, 255)),
        ),
    ]
}

struct ShapeApp {
    window: Option<ShapeWindow>,
    commands: Vec<DrawCommand>,
}

impl ApplicationHandler for ShapeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            match ShapeWindow::new(event_loop, "Orinium Shape Demo", WIDTH, HEIGHT) {
                Ok(window) => {
                    window.request_redraw();
                    self.window = Some(window);
                }
                Err(err) => {
                    log::error!("failed to create window: {err:#}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(window) = &mut self.window
            && window.handle_event(event, &self.commands) == EventOutcome::Close
        {
            event_loop.exit();
        }
    }
}

fn main() -> Result<()> {
    env_logger::init();

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = ShapeApp {
        window: None,
        commands: build_draw_commands(),
    };
    event_loop.run_app(&mut app)?;

    Ok(())
}
