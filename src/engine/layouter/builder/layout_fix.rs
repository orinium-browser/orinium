use crate::engine::layouter::text_layouter::TextFlowLayouter;
use crate::engine::layouter::types::{InfoNode, NodeKind, TextAlign};
use ui_layout::{
    AutoSizeBehavior, Display, GridTrack, InnerDisplay, LayoutChild, LayoutNode, Length,
    LengthOrAuto, OuterDisplay,
};

// ---------------------------------------------------------------------------
// orinium internal helpers
// ---------------------------------------------------------------------------

/// Return the largest fixed-width (`Length::Px`) used by any descendant, or
/// `None` if no descendant has a fixed width.
pub fn maximum_fixed_descendant_width(children: &[LayoutChild]) -> Option<f32> {
    children
        .iter()
        .filter_map(|child| match child {
            LayoutChild::Node(node) => {
                let own = match node.style.size.width {
                    LengthOrAuto::Length(Length::Px(width))
                        if width.is_finite() && width >= 0.0 =>
                    {
                        Some(width)
                    }
                    _ => None,
                };
                own.into_iter()
                    .chain(maximum_fixed_descendant_width(&node.children))
                    .max_by(f32::total_cmp)
            }
            _ => None,
        })
        .max_by(f32::total_cmp)
}

/// Returns `true` when `info` represents a whitespace-only text node that
/// renders as a single collapsible space.
pub fn is_collapsible_whitespace_info(info: &InfoNode) -> bool {
    matches!(&info.kind, NodeKind::Text { text, .. } if text.trim().is_empty())
}

/// Returns `true` when `child` is a block-level layout child — i.e. it has
/// `outer: Block` display, or it is a shrink-to-fit `FlowRoot` (an anonymous
/// flex/grid wrapper).
pub fn is_block_layout_child(child: &LayoutChild) -> bool {
    child.node().is_some_and(|node| {
        node.style.display.outer() == Some(OuterDisplay::Block)
            || (node.style.display.inner() == Some(InnerDisplay::FlowRoot)
                && node.style.size.auto_behavior == AutoSizeBehavior::ShrinkToFit)
    })
}

// ---------------------------------------------------------------------------
// ui_layout bug fixes
// ---------------------------------------------------------------------------

/// **Bug fix:** `grid-template-columns`, `width`
///
/// During the intrinsic grid pass `ui_layout` measures block flex containers
/// with their containing width. In a template such as `1fr auto 1fr`, that
/// makes the auto track take the entire grid and leaves both fraction tracks
/// at zero. The first layout still records the flex contents' actual extent in
/// `children_box`, so use that intrinsic width and let a second layout resolve
/// the tracks correctly.
pub fn constrain_auto_grid_track_items(node: &mut LayoutNode) -> bool {
    let mut changed = false;
    for child in &mut node.children {
        if let LayoutChild::Node(child) = child {
            changed |= constrain_auto_grid_track_items(child);
        }
    }

    if node.style.display.inner() != Some(InnerDisplay::Grid)
        || node.style.grid_template_columns.is_empty()
    {
        return changed;
    }

    let mut item_index = 0usize;
    for child in &mut node.children {
        let LayoutChild::Node(child) = child else {
            continue;
        };
        if child.style.display == Display::None || child.style.position.kind.is_out_of_flow() {
            continue;
        }

        let auto_track = node
            .style
            .grid_template_columns
            .get(item_index)
            .is_some_and(|track| matches!(track, GridTrack::Breadth(LengthOrAuto::Auto)));
        item_index += 1;
        let self_aligned = child.style.spacing.margin_left == LengthOrAuto::Auto
            || child.style.spacing.margin_right == LengthOrAuto::Auto;
        if (!auto_track && !self_aligned) || child.style.size.width != LengthOrAuto::Auto {
            continue;
        }

        let Some(model) = child.layout_box.iter().next() else {
            continue;
        };
        let intrinsic_width = model.children_box.width.max(0.0);
        if intrinsic_width > 0.0 && intrinsic_width + 0.5 < model.content_box.width {
            child.style.size.width = LengthOrAuto::Length(Length::Px(intrinsic_width));
            changed = true;
        }
    }

    changed
}

/// **Bug fix:** `display`, `margin-left`, `margin-right`
///
/// `ui_layout` advances past an inline flow-root using its content width.
/// CSS inline-blocks advance by their margin-box width instead. Adjacent
/// atomic inline boxes end up overlapping their padding or horizontal margins.
pub fn correct_atomic_inline_spacing(node: &mut LayoutNode) {
    correct_atomic_inline_spacing_impl(node, None);
}

/// Like [`correct_atomic_inline_spacing`], but accepts an [`InfoNode`] so that
/// per-child `text-align` information is available during correction.
pub fn correct_atomic_inline_spacing_with_info(node: &mut LayoutNode, info: &InfoNode) {
    correct_atomic_inline_spacing_impl(node, Some(info));
}

/// Core implementation of atomic-inline spacing correction. When `info` is
/// supplied, `text-align` from the container is respected for the first item on
/// each line.
fn correct_atomic_inline_spacing_impl(node: &mut LayoutNode, info: Option<&InfoNode>) {
    let containing_rect = node.layout_box.iter().next().map(|model| model.content_box);
    let containing_width = containing_rect.map(|rect| rect.width);
    let text_align = info
        .and_then(|info| match &info.kind {
            NodeKind::Container { style, .. } => Some(style.text_align),
            _ => None,
        })
        .unwrap_or_default();
    let wraps_inline_content = matches!(
        node.style.display.inner(),
        Some(InnerDisplay::Flow | InnerDisplay::FlowRoot)
    );
    let mut previous: Option<(f32, f32)> = None;
    let mut line_y: Option<f32> = None;
    let mut line_start_x = 0.0;
    let mut line_bottom = 0.0;
    let mut line_margin_bottom = 0.0;
    let mut preceding_block_bottom: Option<(f32, f32)> = None;

    for (child_index, child) in node.children.iter_mut().enumerate() {
        let LayoutChild::Node(child) = child else {
            continue;
        };

        let child_info = info.and_then(|info| info.children.get(child_index));
        correct_atomic_inline_spacing_impl(child, child_info);

        let is_atomic_inline = child.style.display.outer() == Some(OuterDisplay::Inline)
            && child.style.display.inner() != Some(InnerDisplay::Flow);

        if is_atomic_inline && let Some(model) = child.layout_box.iter().next() {
            let rect = model.border_box;
            let margin_left = fixed_nonnegative_px(&child.style.spacing.margin_left);
            let margin_right = fixed_nonnegative_px(&child.style.spacing.margin_right);
            let margin_top = fixed_nonnegative_px(&child.style.spacing.margin_top);
            let margin_bottom = fixed_nonnegative_px(&child.style.spacing.margin_bottom);

            if line_y.is_none_or(|y| (y - rect.y).abs() >= 0.5) {
                previous = None;
                line_y = Some(rect.y);
                line_start_x = rect.x;
                line_bottom = rect.bottom();
                line_margin_bottom = margin_bottom;
            }

            let mut desired_x = match previous {
                Some((right, previous_margin_right)) => right + previous_margin_right + margin_left,
                None => {
                    let margin_width = margin_left + rect.width + margin_right;
                    let aligned_x = containing_rect.map_or(rect.x, |containing| {
                        let free_space = (containing.width - margin_width).max(0.0);
                        containing.x
                            + match text_align {
                                TextAlign::Left => 0.0,
                                TextAlign::Center => free_space / 2.0,
                                TextAlign::Right => free_space,
                            }
                    });
                    aligned_x + margin_left
                }
            };
            // ui_layout positions atomic inline boxes at the line origin but
            // does not include their vertical margins in that position. The
            // margin box, rather than the border box, is what participates in
            // inline formatting (notably a full-width inline-block <main>
            // placed below a fixed header).
            let mut desired_y = rect.y + margin_top;
            if previous.is_none()
                && let Some((block_bottom, block_margin_bottom)) = preceding_block_bottom
            {
                desired_y = desired_y.max(block_bottom + block_margin_bottom + margin_top);
            }
            let exceeds_line = previous.is_some()
                && wraps_inline_content
                && containing_width
                    .is_some_and(|width| desired_x + rect.width + margin_right > width + 0.5);
            if exceeds_line {
                desired_x = line_start_x + margin_left;
                desired_y = line_bottom + line_margin_bottom + margin_top;
                line_y = Some(desired_y);
                line_bottom = desired_y + rect.height;
                line_margin_bottom = margin_bottom;
            }

            let shift_x = desired_x - rect.x;
            if shift_x.abs() >= 0.01 {
                shift_layout_box_x(&mut child.layout_box, shift_x);
            }
            let shift_y = desired_y - rect.y;
            if shift_y.abs() >= 0.01 {
                shift_layout_box_y(&mut child.layout_box, shift_y);
            }

            line_bottom = line_bottom.max(desired_y + rect.height);
            line_margin_bottom = line_margin_bottom.max(margin_bottom);
            previous = Some((desired_x + rect.width, margin_right));
        } else if matches!(child.layout_box, ui_layout::LayoutBox::BlockBox(_)) {
            previous = None;
            line_y = None;
            if !child.style.position.kind.is_out_of_flow()
                && let Some(model) = child.layout_box.iter().next()
            {
                preceding_block_bottom = Some((
                    model.border_box.bottom(),
                    fixed_nonnegative_px(&child.style.spacing.margin_bottom),
                ));
            }
        }
    }

    expand_auto_inline_width_to_children(node);
    correct_single_row_grid_inline_alignment(node);
}

/// **Supplement:** `width`
///
/// `ui_layout` does not handle `width: auto` for inline containers wrapping
/// block children. Grow the inline container's width to accommodate the
/// widest child's margin box.
fn expand_auto_inline_width_to_children(node: &mut LayoutNode) {
    if node.style.display.outer() != Some(OuterDisplay::Inline)
        || node.style.size.width != LengthOrAuto::Auto
    {
        return;
    }
    // Use the margin-box width of the widest child.
    let required_width = node
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .filter_map(|child| {
            child.layout_box.iter().next().map(|model| {
                model.border_box.right() + fixed_nonnegative_px(&child.style.spacing.margin_right)
            })
        })
        .fold(0.0, f32::max);
    if required_width <= 0.0 {
        return;
    }
    if let Some(model) = node.layout_box.iter().next() {
        let extra = required_width - model.content_box.width;
        if extra > 0.0 {
            match &mut node.layout_box {
                ui_layout::LayoutBox::BlockBox(model) => {
                    model.content_box.width += extra;
                    model.padding_box.width += extra;
                    model.border_box.width += extra;
                    model.children_box.width = model.children_box.width.max(required_width);
                }
                ui_layout::LayoutBox::InlineBox(inline) => {
                    inline.box_model.content_box.width += extra;
                    inline.box_model.padding_box.width += extra;
                    inline.box_model.border_box.width += extra;
                    inline.box_model.children_box.width =
                        inline.box_model.children_box.width.max(required_width);
                    if let Some(last) = inline.line_spans.last_mut() {
                        last.x_range.end += extra;
                    }
                }
                ui_layout::LayoutBox::None => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ui_layout feature supplements
// ---------------------------------------------------------------------------

/// **Supplement:** custom flex item `layout` method
///
/// `ui_layout` measures and positions direct custom flex items but does not
/// call their `layout` method. Render-time text-flow lookup therefore finds no
/// spans for text directly inside an `inline-flex` element. Walk the tree and
/// invoke `layout` for any custom flex item whose result is missing.
pub fn refresh_missing_text_layout_results(
    layout: &mut LayoutNode,
    info: &InfoNode,
    viewport: (f32, f32),
) {
    let containing = layout
        .layout_box
        .iter()
        .next()
        .map(|model| (model.content_box.width, model.content_box.height))
        .unwrap_or(viewport);

    for (layout_child, info_child) in layout.children.iter_mut().zip(&info.children) {
        match (layout_child, &info_child.kind) {
            (LayoutChild::Node(child), _) => {
                refresh_missing_text_layout_results(child, info_child, viewport);
            }
            (LayoutChild::Custom(custom), NodeKind::Text { text_id, .. })
                if TextFlowLayouter::get_result(*text_id).is_none() =>
            {
                let Some(box_model) = custom.result().map(|result| result.box_model.clone()) else {
                    continue;
                };
                let line_height = custom
                    .style()
                    .line_height
                    .resolve_with(Some(containing.0), viewport.0, viewport.1)
                    .unwrap_or(box_model.border_box.height);
                let _ = custom.layouter_mut().layout(&ui_layout::LayoutContext {
                    containing_block_width: Some(containing.0),
                    containing_block_height: Some(containing.1),
                    start_pos: (box_model.border_box.x, box_model.border_box.y),
                    available_inline_size: box_model.border_box.width.max(1.0),
                    line_height,
                    viewport_width: viewport.0,
                    viewport_height: viewport.1,
                });
            }
            _ => {}
        }
    }
}

/// **Supplement:** `margin-left`, `margin-right`, `column-gap`
///
/// `ui_layout` does not implement `margin-inline: auto` for grid items. When
/// all grid items land in a single row, resolve auto left/right margins within
/// each track so that items are horizontally centered or right-aligned.
fn correct_single_row_grid_inline_alignment(node: &mut LayoutNode) {
    if node.style.display.inner() != Some(InnerDisplay::Grid) {
        return;
    }
    let Some(content_box) = node.layout_box.iter().next().map(|model| model.content_box) else {
        return;
    };
    let column_gap = fixed_nonnegative_px(&node.style.column_gap);
    let item_indices: Vec<usize> = node
        .children
        .iter()
        .enumerate()
        .filter_map(|(index, child)| {
            child.node().and_then(|child| {
                (child.style.display != Display::None
                    && !child.style.position.kind.is_out_of_flow())
                .then_some(index)
            })
        })
        .collect();
    if item_indices.len() > node.style.grid_template_columns.len() {
        return;
    }

    for (position, index) in item_indices.iter().copied().enumerate() {
        let next_x = item_indices
            .get(position + 1)
            .and_then(|next| node.children[*next].node())
            .and_then(|next| next.layout_box.iter().next())
            .map(|model| model.border_box.x - column_gap)
            .unwrap_or(content_box.width);
        let child = node.children[index].node_mut().expect("grid item");
        let left_auto = child.style.spacing.margin_left == LengthOrAuto::Auto;
        let right_auto = child.style.spacing.margin_right == LengthOrAuto::Auto;
        if !left_auto && !right_auto {
            continue;
        }
        let Some(model) = child.layout_box.iter().next() else {
            continue;
        };
        let track_start = model.border_box.x;
        let free_space = (next_x - track_start - model.border_box.width).max(0.0);
        let offset = match (left_auto, right_auto) {
            (true, true) => free_space / 2.0,
            (true, false) => free_space,
            _ => 0.0,
        };
        shift_layout_box_x(&mut child.layout_box, offset);
    }
}

// ---------------------------------------------------------------------------
// utilities
// ---------------------------------------------------------------------------

/// Shift every layer (content, padding, border, children) of a layout box
/// horizontally by `shift_x` pixels.
fn shift_layout_box_x(layout_box: &mut ui_layout::LayoutBox, shift_x: f32) {
    let shift_model = |model: &mut ui_layout::BoxModel| {
        model.border_box.x += shift_x;
        model.padding_box.x += shift_x;
        model.content_box.x += shift_x;
        model.children_box.x += shift_x;
    };
    match layout_box {
        ui_layout::LayoutBox::None => {}
        ui_layout::LayoutBox::BlockBox(model) => shift_model(model),
        ui_layout::LayoutBox::InlineBox(inline) => {
            shift_model(&mut inline.box_model);
            for span in &mut inline.line_spans {
                span.line_pos.0 += shift_x;
            }
        }
    }
}

/// Shift every layer (content, padding, border, children) of a layout box
/// vertically by `shift_y` pixels.
fn shift_layout_box_y(layout_box: &mut ui_layout::LayoutBox, shift_y: f32) {
    let shift_model = |model: &mut ui_layout::BoxModel| {
        model.border_box.y += shift_y;
        model.padding_box.y += shift_y;
        model.content_box.y += shift_y;
        model.children_box.y += shift_y;
    };
    match layout_box {
        ui_layout::LayoutBox::None => {}
        ui_layout::LayoutBox::BlockBox(model) => shift_model(model),
        ui_layout::LayoutBox::InlineBox(inline) => {
            shift_model(&mut inline.box_model);
            for span in &mut inline.line_spans {
                span.line_pos.1 += shift_y;
            }
        }
    }
}

/// Extract a non-negative pixel value from a `LengthOrAuto`, returning `0.0`
/// for `Auto` or non-pixel lengths.
fn fixed_nonnegative_px(value: &LengthOrAuto) -> f32 {
    match value {
        LengthOrAuto::Length(Length::Px(value)) => value.max(0.0),
        _ => 0.0,
    }
}
