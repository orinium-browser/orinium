//! Shared helpers for translating boxes after `ui_layout` has positioned them.
//!
//! Several post-layout passes move boxes the engine already laid out:
//! [`table_layout`](super::table_layout) spreads column widths across rows, and
//! the anchor-positioning fix-ups in
//! [`builder::layout_fix`](super::builder) shift anchored elements. They must
//! translate a box model and its inline line spans exactly the same way, so the
//! algorithm lives in one place.
//!
//! Inline boxes carry an extra per-line position ([`ui_layout::LineSpan`]): the
//! box model describes the outermost box, while each span records where that
//! line actually sits. Moving an inline box without moving its spans would
//! leave the rendered text behind, so every helper here shifts both.

use ui_layout::{BoxModel, LayoutBox};

/// Translates every layer (border, padding, content, children) of `model` by
/// `dx`/`dy`.
pub(crate) fn translate_box(model: &mut BoxModel, dx: f32, dy: f32) {
    model.border_box.x += dx;
    model.padding_box.x += dx;
    model.content_box.x += dx;
    model.children_box.x += dx;
    model.border_box.y += dy;
    model.padding_box.y += dy;
    model.content_box.y += dy;
    model.children_box.y += dy;
}

/// Translates a layout box — its box model together with any inline line
/// spans — by `dx`/`dy`.
pub(crate) fn shift_box(layout_box: &mut LayoutBox, dx: f32, dy: f32) {
    match layout_box {
        LayoutBox::None => {}
        LayoutBox::BlockBox(model) => translate_box(model, dx, dy),
        LayoutBox::InlineBox(inline) => {
            translate_box(&mut inline.box_model, dx, dy);
            for span in &mut inline.line_spans {
                span.line_pos.0 += dx;
                span.line_pos.1 += dy;
            }
        }
    }
}

/// Moves `layout_box` so its border box's left edge lands on `target_x`.
///
/// The delta is derived from the box's current border edge, so this positions
/// the box at an absolute coordinate rather than nudging it by a relative
/// offset.
pub(crate) fn move_box_x(layout_box: &mut LayoutBox, target_x: f32) {
    let dx = match layout_box {
        LayoutBox::None => return,
        LayoutBox::BlockBox(model) => target_x - model.border_box.x,
        LayoutBox::InlineBox(inline) => target_x - inline.box_model.border_box.x,
    };
    shift_box(layout_box, dx, 0.0);
}

/// Grows `layout_box` to `target_border_width` (never shrinks it).
pub(crate) fn grow_box_width(layout_box: &mut LayoutBox, target_border_width: f32) {
    let model = match layout_box {
        LayoutBox::None => return,
        LayoutBox::BlockBox(model) => model,
        LayoutBox::InlineBox(inline) => &mut inline.box_model,
    };
    let delta = (target_border_width - model.border_box.width).max(0.0);
    model.border_box.width += delta;
    model.padding_box.width += delta;
    model.content_box.width += delta;
    model.children_box.width += delta;
}

#[cfg(test)]
mod tests {
    use ui_layout::{InlineBox, LineSpan, Rect};

    use super::*;

    fn model(x: f32, y: f32) -> BoxModel {
        let rect = Rect {
            x,
            y,
            width: 40.0,
            height: 10.0,
        };
        rect.into()
    }

    #[test]
    fn shift_block_box_moves_every_layer() {
        let mut layout_box = LayoutBox::BlockBox(model(5.0, 7.0));
        shift_box(&mut layout_box, 3.0, -2.0);
        let b = layout_box.iter().next().unwrap();
        assert_eq!(b.border_box.x, 8.0);
        assert_eq!(b.padding_box.y, 5.0);
        assert_eq!(b.content_box.x, 8.0);
        assert_eq!(b.children_box.y, 5.0);
    }

    #[test]
    fn shift_inline_box_moves_line_spans_too() {
        let mut layout_box = LayoutBox::InlineBox(InlineBox {
            box_model: model(5.0, 7.0),
            line_spans: vec![LineSpan {
                x_range: 0.0..10.0,
                line_pos: (5.0, 7.0),
                line_index: 0,
            }],
        });
        shift_box(&mut layout_box, 3.0, 4.0);
        let LayoutBox::InlineBox(inline) = &layout_box else {
            panic!("expected InlineBox");
        };
        assert_eq!(inline.box_model.border_box.x, 8.0);
        assert_eq!(inline.line_spans[0].line_pos, (8.0, 11.0));
    }

    #[test]
    fn move_box_x_uses_absolute_target() {
        let mut layout_box = LayoutBox::BlockBox(model(5.0, 7.0));
        move_box_x(&mut layout_box, 100.0);
        let b = layout_box.iter().next().unwrap();
        assert_eq!(b.border_box.x, 100.0);
        assert_eq!(b.border_box.y, 7.0);
    }

    #[test]
    fn grow_box_width_never_shrinks() {
        let mut layout_box = LayoutBox::BlockBox(model(0.0, 0.0));
        grow_box_width(&mut layout_box, 100.0);
        assert_eq!(layout_box.iter().next().unwrap().border_box.width, 100.0);
        grow_box_width(&mut layout_box, 20.0);
        assert_eq!(layout_box.iter().next().unwrap().border_box.width, 100.0);
    }
}
