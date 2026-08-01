//! Replaced-element component for decoded HTML images.

use crate::engine::layouter::types::TextStyle;
use crate::engine::renderer_model::{Brush, DrawCommand, FillRule, Image, Paint, rect_path};
use crate::engine::ui::custom_node::CustomNode;

/// A decoded image participating in layout as a replaced element.
#[derive(Debug)]
pub struct ImageComponent {
    pub image: Option<Image>,
}

impl CustomNode for ImageComponent {
    fn draw(&self, cmd_buf: &mut Vec<DrawCommand>, text_style: &TextStyle) {
        let size = self.intrinsic_size();
        self.draw_sized(cmd_buf, text_style, size);
    }

    fn draw_sized(
        &self,
        cmd_buf: &mut Vec<DrawCommand>,
        _text_style: &TextStyle,
        size: (f32, f32),
    ) {
        let Some(image) = &self.image else {
            return;
        };
        cmd_buf.push(DrawCommand::Fill {
            path: rect_path(0.0, 0.0, size.0, size.1),
            paint: Paint {
                brush: Brush::Image(image.clone()),
                opacity: 1.0,
            },
            rule: FillRule::NonZero,
        });
    }

    fn intrinsic_size(&self) -> (f32, f32) {
        self.image.as_ref().map_or((0.0, 0.0), |image| {
            (image.width() as f32, image.height() as f32)
        })
    }

    fn preserves_intrinsic_aspect_ratio(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_component_uses_intrinsic_size_and_draw_target() {
        let image = Image::from_rgba(2, 3, vec![255; 24]).unwrap();
        let component = ImageComponent { image: Some(image) };
        assert_eq!(component.intrinsic_size(), (2.0, 3.0));

        let mut commands = Vec::new();
        component.draw_sized(&mut commands, &TextStyle::default(), (20.0, 30.0));
        let DrawCommand::Fill { path, paint, .. } = &commands[0] else {
            panic!("expected image fill");
        };
        assert!(matches!(&paint.brush, Brush::Image(_)));
        let points = path.subpaths().remove(0);
        assert!(points.iter().any(|point| *point == (20.0, 30.0)));
    }
}
