//! Replaced-element component for decoded HTML images.

use ui_layout::Style;

use crate::engine::layouter::types::{Color, TextStyle};
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Image, Paint, rect_path};
use crate::engine::ui::custom_node::{ContentSize, CustomNode};

/// A decoded image participating in layout as a replaced element.
///
/// When the image failed to decode (`image == None`), the component renders
/// a placeholder box with the `alt` text (if any).
#[derive(Debug)]
pub struct ImageComponent {
    pub image: Option<Image>,
    pub alt: String,
}

impl CustomNode for ImageComponent {
    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        _style: &Style,
        size: ContentSize,
    ) {
        match &self.image {
            Some(image) => cmd_buf.push(DrawCommand::Fill {
                path: rect_path(0.0, 0.0, size.width, size.height),
                paint: Paint {
                    brush: Brush::Image(image.clone()),
                    opacity: 1.0,
                },
                rule: FillRule::NonZero,
            }),
            None => self.draw_placeholder(cmd_buf, text_style, size),
        }
    }

    fn intrinsic_size(&self) -> ContentSize {
        match &self.image {
            Some(image) => ContentSize {
                width: image.width() as f32,
                height: image.height() as f32,
            },
            None => {
                // Placeholder sized to the alt text (or a fixed box when empty).
                let line_count = self.alt.lines().count().max(1);
                ContentSize {
                    width: 160.0,
                    height: (line_count * 16) as f32,
                }
            }
        }
    }

    fn preserves_intrinsic_aspect_ratio(&self) -> bool {
        true
    }

    fn role(&self) -> Option<&'static str> {
        self.image.is_some().then_some("img")
    }

    fn label(&self) -> Option<String> {
        (!self.alt.is_empty()).then(|| self.alt.clone())
    }
}

impl ImageComponent {
    /// Renders a placeholder box with a broken-image mark and the `alt` text.
    fn draw_placeholder(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        text_style: &TextStyle,
        size: ContentSize,
    ) {
        // Broken-image box (light fill with a thin border drawn as fills).
        let border = 1.0;
        cmd_buf.push(DrawCommand::Fill {
            path: rect_path(0.0, 0.0, size.width, size.height),
            paint: Paint {
                brush: Brush::Solid(Color(240, 240, 240, 255)),
                opacity: 1.0,
            },
            rule: FillRule::NonZero,
        });
        let border_rects = [
            [0.0, 0.0, size.width, border],
            [0.0, size.height - border, size.width, border],
            [0.0, 0.0, border, size.height],
            [size.width - border, 0.0, border, size.height],
        ];
        for [x, y, w, h] in border_rects {
            cmd_buf.push(DrawCommand::Fill {
                path: rect_path(x, y, w, h),
                paint: Paint {
                    brush: Brush::Solid(Color(180, 180, 180, 255)),
                    opacity: 1.0,
                },
                rule: FillRule::NonZero,
            });
        }

        if self.alt.is_empty() {
            return;
        }

        // Render the alt text, wrapping within the placeholder width.
        let mut style = text_style.clone();
        style.color = Color(90, 90, 90, 255);
        let max_width = size.width - 8.0;
        let x = 4.0;
        let mut y = 4.0 + style.font_size;
        let mut line = String::new();
        for ch in self.alt.chars() {
            if ch == '\n' || line.chars().count() * 8 >= max_width as usize {
                cmd_buf.push(DrawCommand::DrawText {
                    text: line.clone().into(),
                    x,
                    y,
                    style: style.clone(),
                });
                line = String::new();
                y += style.font_size + 2.0;
            }
            if ch != '\n' {
                line.push(ch);
            }
        }
        if !line.is_empty() {
            cmd_buf.push(DrawCommand::DrawText {
                text: line.into(),
                x,
                y,
                style,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_component_uses_intrinsic_size_and_draw_target() {
        let image = Image::from_rgba(2, 3, vec![255; 24]).unwrap();
        let component = ImageComponent {
            image: Some(image),
            alt: String::new(),
        };
        assert_eq!(
            component.intrinsic_size(),
            ContentSize {
                width: 2.0,
                height: 3.0
            }
        );

        let mut commands = Vec::new();
        component.draw_sized(
            &mut commands,
            &TextStyle::default(),
            &Style::default(),
            ContentSize {
                width: 20.0,
                height: 30.0,
            },
        );
        let DrawCommand::Fill { path, paint, .. } = &commands[0] else {
            panic!("expected image fill");
        };
        assert!(matches!(&paint.brush, Brush::Image(_)));
        let points = path.subpaths().remove(0);
        assert!(points.contains(&(20.0, 30.0)));
    }

    #[test]
    fn broken_image_renders_placeholder() {
        let component = ImageComponent {
            image: None,
            alt: "example".to_string(),
        };
        let mut commands = Vec::new();
        component.draw_sized(
            &mut commands,
            &TextStyle::default(),
            &Style::default(),
            ContentSize {
                width: 100.0,
                height: 50.0,
            },
        );
        // Fill + 4 border rects + alt text.
        assert!(commands.len() >= 6);
        assert!(
            commands
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::DrawText { .. }))
        );
    }

    #[test]
    fn broken_image_intrinsic_sized_to_alt() {
        let component = ImageComponent {
            image: None,
            alt: "hello".to_string(),
        };
        let size = component.intrinsic_size();
        assert_eq!(size.width, 160.0);
        assert_eq!(size.height, 16.0);
    }

    #[test]
    fn broken_image_exposes_alt_as_label() {
        let broken = ImageComponent {
            image: None,
            alt: "alt text".to_string(),
        };
        assert_eq!(broken.role(), None);
        assert_eq!(broken.label(), Some("alt text".to_string()));

        let ok = ImageComponent {
            image: Some(Image::from_rgba(1, 1, vec![255; 4]).unwrap()),
            alt: "alt text".to_string(),
        };
        assert_eq!(ok.role(), Some("img"));
    }
}
