//! Canvas replaced-element rendering backed by recorded 2D commands.

use ui_layout::Style;

use crate::engine::layouter::types::{Color, TextFlowStyle, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Paint, rect_path};
use crate::engine::ui::custom_node::{ContentSize, CustomNode};

#[derive(Debug, Clone, PartialEq)]
pub enum CanvasCommand {
    FillRect {
        color: Color,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    ClearRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    StrokeRect {
        color: Color,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
}

#[derive(Debug)]
pub struct CanvasComponent {
    width: f32,
    height: f32,
    commands: Vec<CanvasCommand>,
}

impl CanvasComponent {
    pub fn new(width: f32, height: f32, source: &str) -> Self {
        Self {
            width,
            height,
            commands: parse_commands(source),
        }
    }
}

impl CustomNode for CanvasComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        _text_style: &TextStyle,
        _text_flow_style: &TextFlowStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        let scale_x = if self.width > 0.0 {
            size.width / self.width
        } else {
            1.0
        };
        let scale_y = if self.height > 0.0 {
            size.height / self.height
        } else {
            1.0
        };
        for command in &self.commands {
            match *command {
                CanvasCommand::FillRect {
                    color,
                    x,
                    y,
                    width,
                    height,
                } => draw_rect(cmd_buf, color, x, y, width, height, scale_x, scale_y),
                CanvasCommand::ClearRect { .. } => {
                    // TODO: Add a retained transparent pixel surface so clearRect can erase
                    // previously drawn canvas content.
                }
                CanvasCommand::StrokeRect {
                    color,
                    x,
                    y,
                    width,
                    height,
                } => {
                    let line = 1.0;
                    draw_rect(cmd_buf, color, x, y, width, line, scale_x, scale_y);
                    draw_rect(
                        cmd_buf,
                        color,
                        x,
                        y + height - line,
                        width,
                        line,
                        scale_x,
                        scale_y,
                    );
                    draw_rect(cmd_buf, color, x, y, line, height, scale_x, scale_y);
                    draw_rect(
                        cmd_buf,
                        color,
                        x + width - line,
                        y,
                        line,
                        height,
                        scale_x,
                        scale_y,
                    );
                }
            }
        }
    }

    fn intrinsic_size(&self) -> ContentSize {
        ContentSize {
            width: self.width,
            height: self.height,
        }
    }

    fn role(&self) -> Option<&'static str> {
        Some("img")
    }
}

fn draw_rect(
    cmd_buf: &mut Vec<DrawCommand>,
    color: Color,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    scale_x: f32,
    scale_y: f32,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    cmd_buf.push(DrawCommand::Fill {
        path: rect_path(x * scale_x, y * scale_y, width * scale_x, height * scale_y),
        paint: Paint {
            brush: Brush::Solid(color),
            opacity: 1.0,
        },
        rule: FillRule::NonZero,
    });
}

fn parse_commands(source: &str) -> Vec<CanvasCommand> {
    source.lines().filter_map(parse_command).collect()
}

fn parse_command(source: &str) -> Option<CanvasCommand> {
    let mut parts = source.split('|');
    let name = parts.next()?;
    let color = parse_color(parts.next().unwrap_or(""));
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    let width = parts.next()?.parse().ok()?;
    let height = parts.next()?.parse().ok()?;
    match name {
        "fillRect" => Some(CanvasCommand::FillRect {
            color: color?,
            x,
            y,
            width,
            height,
        }),
        "clearRect" => Some(CanvasCommand::ClearRect {
            x,
            y,
            width,
            height,
        }),
        "strokeRect" => Some(CanvasCommand::StrokeRect {
            color: color?,
            x,
            y,
            width,
            height,
        }),
        _ => None,
    }
}

fn parse_color(source: &str) -> Option<Color> {
    let source = source.trim().to_ascii_lowercase();
    match source.as_str() {
        "black" => Some(Color(0, 0, 0, 255)),
        "white" => Some(Color(255, 255, 255, 255)),
        "red" => Some(Color(255, 0, 0, 255)),
        "green" => Some(Color(0, 128, 0, 255)),
        "blue" => Some(Color(0, 0, 255, 255)),
        "orange" => Some(Color(255, 165, 0, 255)),
        "transparent" => Some(Color(0, 0, 0, 0)),
        _ if source.len() == 7 && source.starts_with('#') => Some(Color(
            u8::from_str_radix(&source[1..3], 16).ok()?,
            u8::from_str_radix(&source[3..5], 16).ok()?,
            u8::from_str_radix(&source[5..7], 16).ok()?,
            255,
        )),
        _ if source.len() == 4 && source.starts_with('#') => {
            let mut digits = source[1..].chars();
            let expand = |digit: char| u8::from_str_radix(&format!("{digit}{digit}"), 16).ok();
            Some(Color(
                expand(digits.next()?)?,
                expand(digits.next()?)?,
                expand(digits.next()?)?,
                255,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_rectangles_parse_into_canvas_commands() {
        let canvas = CanvasComponent::new(
            150.0,
            100.0,
            "fillRect|orange|10|10|130|80\nstrokeRect|#0000ff|0|0|150|100",
        );
        assert_eq!(canvas.commands.len(), 2);
        assert!(matches!(
            canvas.commands[0],
            CanvasCommand::FillRect {
                color: Color(255, 165, 0, 255),
                ..
            }
        ));
    }
}
