use crate::engine::layouter::shift::shift_box;
use crate::engine::layouter::text_layouter::TextFlowLayouter;
use crate::engine::layouter::types::{InfoNode, NodeKind, TextAlign, WhiteSpace};
use ui_layout::{
    AutoSizeBehavior, Display, GridPlacement, GridTrack, InnerDisplay, LayoutChild, LayoutNode,
    Length, LengthOrAuto, OuterDisplay,
};

// ---------------------------------------------------------------------------
// orinium internal helpers
// ---------------------------------------------------------------------------

/// Returns `true` when `info` represents a whitespace-only text node that
/// renders as a single collapsible space.
///
/// `white_space` is the computed `white-space` of the element that owns the
/// text. Per CSS, `pre`, `pre-wrap`, and `break-spaces` preserve whitespace
/// and must not be collapsed even at block boundaries.
pub fn is_collapsible_whitespace_info(info: &InfoNode, white_space: WhiteSpace) -> bool {
    // Preserved-whitespace modes keep all whitespace, including at boundaries.
    if !matches!(white_space, WhiteSpace::Normal | WhiteSpace::Nowrap) {
        return false;
    }
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

    changed |= stretch_fraction_rows_to_min_height(node);
    changed |= include_constrained_grid_item_height(node);

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

/// **Supplement:** constrained grid items, implicit rows
///
/// The grid backend can leave an auto-sized grid shorter than a placed item
/// whose explicit `min-height` exceeds the row's intrinsic contribution. Use
/// the laid-out item extent to grow the auto-sized grid, then let the normal
/// second layout pass recompute downstream positions.
fn include_constrained_grid_item_height(node: &mut LayoutNode) -> bool {
    if node.style.display.inner() != Some(InnerDisplay::Grid)
        || node.style.size.height != LengthOrAuto::Auto
    {
        return false;
    }

    let Some(current_height) = node
        .layout_box
        .iter()
        .next()
        .map(|model| model.border_box.height)
    else {
        return false;
    };
    let required_height = node
        .children
        .iter()
        .filter_map(LayoutChild::node)
        .filter(|child| {
            child.style.display != Display::None
                && !child.style.position.kind.is_out_of_flow()
                && (child.style.size.height != LengthOrAuto::Auto
                    || child.style.size.min_height != LengthOrAuto::Auto
                    || child.style.size.max_height != LengthOrAuto::Auto)
        })
        .filter_map(|child| {
            child
                .layout_box
                .iter()
                .next()
                .map(|model| model.border_box.y + model.border_box.height)
        })
        .fold(current_height, f32::max);

    if required_height > current_height + 0.5 {
        node.style.size.height = LengthOrAuto::Length(Length::Px(required_height));
        true
    } else {
        false
    }
}

/// **Supplement:** `min-height`, fractional grid rows
///
/// `ui_layout` lays out fractional rows using `height`, but does not use a
/// definite `min-height` as the available grid size. Promote the minimum to
/// the same height constraint when the current grid tracks leave unused
/// space. Keeping the original length preserves viewport-relative sizing.
fn stretch_fraction_rows_to_min_height(node: &mut LayoutNode) -> bool {
    if node.style.display.inner() != Some(InnerDisplay::Grid)
        || node.style.size.height != LengthOrAuto::Auto
        || !node
            .style
            .grid_template_rows
            .iter()
            .any(|track| matches!(track, GridTrack::Flex(factor) if *factor > 0.0))
    {
        return false;
    }

    let LengthOrAuto::Length(min_height) = &node.style.size.min_height else {
        return false;
    };
    let Some(model) = node.layout_box.iter().next() else {
        return false;
    };
    if model.children_box.height + 0.5 >= model.content_box.height {
        return false;
    }

    node.style.size.height = LengthOrAuto::Length(min_height.clone());
    true
}

/// **Supplement:** `grid-template-columns`
///
/// `ui_layout` sizes an implicit `auto` column to its max-content contribution
/// but does not stretch it into the remaining inline space. For a grid with no
/// explicit columns and only auto-placed items, model the single implicit
/// column as `minmax(auto, 1fr)`:
/// it keeps the intrinsic minimum and consumes the available space, as the
/// implicit auto track should with the default `justify-content: normal`.
pub fn stretch_single_implicit_grid_column(node: &mut LayoutNode) {
    for child in &mut node.children {
        if let LayoutChild::Node(child) = child {
            stretch_single_implicit_grid_column(child);
        }
    }

    if node.style.display.inner() != Some(InnerDisplay::Grid)
        || !node.style.grid_template_columns.is_empty()
        || !node.style.grid_template_areas.is_empty()
    {
        return;
    }

    let auto_placed_items_only = node.children.iter().all(|child| {
        child.node().is_none_or(|child| {
            child.style.display == Display::None
                || child.style.position.kind.is_out_of_flow()
                || child.style.grid_column == GridPlacement::default()
        })
    });
    if auto_placed_items_only {
        node.style.grid_template_columns = vec![GridTrack::MinMax(
            Box::new(GridTrack::Breadth(LengthOrAuto::Auto)),
            Box::new(GridTrack::Flex(1.0)),
        )];
    }
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
                        // Child coordinates are already relative to this content origin.
                        match text_align {
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
                shift_box(&mut child.layout_box, shift_x, 0.0);
            }
            let shift_y = desired_y - rect.y;
            if shift_y.abs() >= 0.01 {
                shift_box(&mut child.layout_box, 0.0, shift_y);
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
                    float_space: None,
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
/// grid items land in a single implicit column, resolve auto left/right
/// margins against that column so items are horizontally centered or
/// right-aligned.
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
    let one_column = node.style.grid_template_columns.len() == 1
        && item_indices.iter().all(|index| {
            node.children[*index]
                .node()
                .is_some_and(|child| child.style.grid_column == GridPlacement::default())
        });
    if one_column {
        for index in item_indices {
            let child = node.children[index].node_mut().expect("grid item");
            let left_auto = child.style.spacing.margin_left == LengthOrAuto::Auto;
            let right_auto = child.style.spacing.margin_right == LengthOrAuto::Auto;
            if !left_auto && !right_auto {
                continue;
            }
            let Some(model) = child.layout_box.iter().next() else {
                continue;
            };
            let free_space = (content_box.width - model.border_box.width).max(0.0);
            let offset = match (left_auto, right_auto) {
                (true, true) => free_space / 2.0,
                (true, false) => free_space,
                _ => 0.0,
            };
            shift_box(&mut child.layout_box, offset, 0.0);
        }
        return;
    }

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
        shift_box(&mut child.layout_box, offset, 0.0);
    }
}

// ---------------------------------------------------------------------------
// utilities
// ---------------------------------------------------------------------------

/// Extract a non-negative pixel value from a `LengthOrAuto`, returning `0.0`
/// for `Auto` or non-pixel lengths.
fn fixed_nonnegative_px(value: &LengthOrAuto) -> f32 {
    match value {
        LengthOrAuto::Length(Length::Px(value)) => value.max(0.0),
        _ => 0.0,
    }
}
