//! Post-layout column alignment for HTML tables.
//!
//! Table elements currently use flex formatting in `ui_layout`. Flex rows
//! size their cells independently, while HTML tables share column widths
//! across every row group. This pass preserves the layout/info tree shape and
//! expands each column to the widest cell found anywhere in the table.

use ui_layout::{BoxModel, LayoutBox, LayoutNode};

use super::types::{ContainerRole, InfoNode, NodeKind};

pub fn align_table_columns(layout: &mut LayoutNode, info: &InfoNode) {
    for (child_layout, child_info) in layout.children.iter_mut().zip(&info.children) {
        if let Some(child_layout) = child_layout.node_mut() {
            align_table_columns(child_layout, child_info);
        }
    }

    if container_role(info) == Some(&ContainerRole::Table) {
        align_one_table(layout, info);
    }
}

fn align_one_table(layout: &mut LayoutNode, info: &InfoNode) {
    let mut widths: Vec<f32> = Vec::new();
    visit_rows(layout, info, &mut |row, row_info| {
        let mut column = 0;
        for (cell, cell_info) in row.children.iter().zip(&row_info.children) {
            if container_role(cell_info) != Some(&ContainerRole::TableCell) {
                continue;
            }
            let Some(cell) = cell.node() else {
                continue;
            };
            if widths.len() <= column {
                widths.resize(column + 1, 0.0);
            }
            widths[column] = widths[column].max(cell.layout_box.width_box());
            column += 1;
        }
    });

    if widths.is_empty() {
        return;
    }

    let table_width = widths.iter().sum();
    visit_rows_mut(layout, info, table_width, &widths);
    grow_box_width(&mut layout.layout_box, table_width);
}

fn visit_rows(
    layout: &LayoutNode,
    info: &InfoNode,
    visit: &mut impl FnMut(&LayoutNode, &InfoNode),
) {
    for (child, child_info) in layout.children.iter().zip(&info.children) {
        let Some(child) = child.node() else {
            continue;
        };
        match container_role(child_info) {
            Some(ContainerRole::TableRow) => visit(child, child_info),
            Some(ContainerRole::TableRowGroup) => visit_rows(child, child_info, visit),
            _ => {}
        }
    }
}

fn visit_rows_mut(layout: &mut LayoutNode, info: &InfoNode, table_width: f32, widths: &[f32]) {
    for (child, child_info) in layout.children.iter_mut().zip(&info.children) {
        let Some(child) = child.node_mut() else {
            continue;
        };
        match container_role(child_info) {
            Some(ContainerRole::TableRow) => align_row(child, child_info, widths),
            Some(ContainerRole::TableRowGroup) => {
                visit_rows_mut(child, child_info, table_width, widths);
                grow_box_width(&mut child.layout_box, table_width);
            }
            _ => {}
        }
    }
}

fn align_row(row: &mut LayoutNode, info: &InfoNode, widths: &[f32]) {
    let start_x = row
        .children
        .iter()
        .zip(&info.children)
        .find_map(|(cell, cell_info)| {
            if container_role(cell_info) != Some(&ContainerRole::TableCell) {
                return None;
            }
            Some(cell.node()?.layout_box.iter().next()?.border_box.x)
        })
        .unwrap_or(0.0);

    let mut column = 0;
    let mut x = start_x;
    for (cell, cell_info) in row.children.iter_mut().zip(&info.children) {
        if container_role(cell_info) != Some(&ContainerRole::TableCell) {
            continue;
        }
        let Some(cell) = cell.node_mut() else {
            continue;
        };
        let Some(width) = widths.get(column).copied() else {
            break;
        };
        move_box_x(&mut cell.layout_box, x);
        grow_box_width(&mut cell.layout_box, width);
        x += width;
        column += 1;
    }
    grow_box_width(&mut row.layout_box, x - start_x);
}

fn container_role(info: &InfoNode) -> Option<&ContainerRole> {
    match &info.kind {
        NodeKind::Container { role, .. } => Some(role),
        _ => None,
    }
}

fn move_box_x(layout_box: &mut LayoutBox, target_x: f32) {
    match layout_box {
        LayoutBox::None => {}
        LayoutBox::BlockBox(model) => translate_box_x(model, target_x - model.border_box.x),
        LayoutBox::InlineBox(inline) => {
            let dx = target_x - inline.box_model.border_box.x;
            translate_box_x(&mut inline.box_model, dx);
        }
    }
}

fn translate_box_x(model: &mut BoxModel, dx: f32) {
    model.border_box.x += dx;
    model.padding_box.x += dx;
    model.content_box.x += dx;
    model.children_box.x += dx;
}

fn grow_box_width(layout_box: &mut LayoutBox, target_border_width: f32) {
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
    use ui_layout::{LayoutNode, Rect, Style};

    use super::*;
    use crate::engine::layouter::types::ContainerStyle;

    fn info(role: ContainerRole, children: Vec<InfoNode>) -> InfoNode {
        InfoNode {
            kind: NodeKind::Container {
                scroll_x: false,
                scroll_y: false,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                style: ContainerStyle::default(),
                role,
            },
            children,
            dom_id: None,
        }
    }

    fn box_node(x: f32, width: f32, children: Vec<LayoutNode>) -> LayoutNode {
        let mut node = LayoutNode::with_children(Style::default(), children);
        node.layout_box = LayoutBox::BlockBox(
            Rect {
                x,
                width,
                height: 20.0,
                ..Default::default()
            }
            .into(),
        );
        node
    }

    #[test]
    fn shares_column_widths_across_row_groups() {
        let header = box_node(
            0.0,
            140.0,
            vec![box_node(0.0, 70.0, vec![]), box_node(70.0, 70.0, vec![])],
        );
        let body = box_node(
            0.0,
            100.0,
            vec![box_node(0.0, 60.0, vec![]), box_node(60.0, 40.0, vec![])],
        );
        let mut table = box_node(
            0.0,
            140.0,
            vec![
                box_node(0.0, 140.0, vec![header]),
                box_node(0.0, 100.0, vec![body]),
            ],
        );
        let row_info = || {
            info(
                ContainerRole::TableRow,
                vec![
                    info(ContainerRole::TableCell, vec![]),
                    info(ContainerRole::TableCell, vec![]),
                ],
            )
        };
        let table_info = info(
            ContainerRole::Table,
            vec![
                info(ContainerRole::TableRowGroup, vec![row_info()]),
                info(ContainerRole::TableRowGroup, vec![row_info()]),
            ],
        );

        align_table_columns(&mut table, &table_info);

        let body_group = table.children[1].node().unwrap();
        let body_row = body_group.children[0].node().unwrap();
        let first = body_row.children[0].node().unwrap();
        let second = body_row.children[1].node().unwrap();
        assert_eq!(first.layout_box.width_box(), 70.0);
        assert_eq!(second.layout_box.width_box(), 70.0);
        assert_eq!(second.layout_box.iter().next().unwrap().border_box.x, 70.0);
        assert_eq!(body_row.layout_box.width_box(), 140.0);
        assert_eq!(body_group.layout_box.width_box(), 140.0);
    }
}
