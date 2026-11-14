use crate::engine::css::values::{CssValue, Unit};

use super::css_resolve::round_to_step;
use super::{resolve_css_color, resolve_css_len};
use crate::engine::layouter::types::{
    Background, BackgroundDimension, BackgroundImage, BackgroundLayer, BackgroundOffset,
    BackgroundPosition, BackgroundPositionAxis, BackgroundRepeat, BackgroundSize, Color,
    ColorScheme, ColorStop, ContainerStyle, Gradient, GradientKind, GradientSpread, RadialShape,
    RadialSizeKind, TextFlowStyle, TextStyle,
};

use ui_layout::Length;

fn background_dimension(value: &CssValue, font_size: f32) -> Option<BackgroundDimension> {
    match value {
        CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("auto") => {
            Some(BackgroundDimension::Auto)
        }
        CssValue::Number(value) if *value == 0.0 => Some(BackgroundDimension::Length(0.0)),
        CssValue::Length(value, Unit::Px) => Some(BackgroundDimension::Length(*value)),
        CssValue::Length(value, Unit::Em) => Some(BackgroundDimension::Length(*value * font_size)),
        CssValue::Length(value, Unit::Rem) => Some(BackgroundDimension::Length(*value * 16.0)),
        CssValue::Length(value, Unit::Percent) => {
            Some(BackgroundDimension::Percent(*value / 100.0))
        }
        _ => None,
    }
}

/// Split a `CssValue::List` into comma-separated groups. Returns `None` if the
/// value is not a `List`. Each group is a `Vec<&CssValue>` of the items between
/// `Separator` entries. A single value without commas yields one group with all
/// items.
fn split_on_separator<'a>(value: &'a CssValue) -> Option<Vec<Vec<&'a CssValue>>> {
    let items: Vec<&'a CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        _ => return None,
    };
    let mut groups: Vec<Vec<&'a CssValue>> = Vec::new();
    let mut current: Vec<&'a CssValue> = Vec::new();
    for item in items {
        if matches!(item, CssValue::Separator) {
            if !current.is_empty() {
                groups.push(current);
                current = Vec::new();
            }
        } else {
            current.push(item);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    if groups.is_empty() {
        groups.push(Vec::new());
    }
    Some(groups)
}

/// Like [`split_on_separator`] but returns `Some(vec![value])` for non-List
/// values, so callers can uniformly handle both single and multi-layer values.
fn split_on_separator_or_single<'a>(value: &'a CssValue) -> Vec<Vec<&'a CssValue>> {
    match value {
        CssValue::List(_) => split_on_separator(value).unwrap_or_default(),
        _ => vec![vec![value]],
    }
}

pub fn parse_background_size(value: &CssValue, font_size: f32) -> Vec<BackgroundSize> {
    let groups = split_on_separator_or_single(value);
    if groups.is_empty() {
        return vec![];
    }
    // If there's exactly one group, try the existing single-layer heuristic
    // (two dimensions → [width, height] for one layer).
    if groups.len() == 1 {
        let items = &groups[0];
        if items.len() == 2 {
            if let (Some(w), Some(h)) = (
                background_dimension(items[0], font_size),
                background_dimension(items[1], font_size),
            ) {
                return vec![BackgroundSize::Explicit {
                    width: w,
                    height: h,
                }];
            }
        }
        return items
            .iter()
            .filter_map(|v| parse_single_background_size(v, font_size))
            .collect();
    }
    // Multiple comma-separated groups: parse each as one layer's size.
    groups
        .iter()
        .map(|group| {
            // A group like [width, height] → Explicit; [size] → Explicit with
            // height Auto; [keyword] → Contain/Cover/Auto.
            if group.len() == 2 {
                if let (Some(w), Some(h)) = (
                    background_dimension(group[0], font_size),
                    background_dimension(group[1], font_size),
                ) {
                    return BackgroundSize::Explicit {
                        width: w,
                        height: h,
                    };
                }
            }
            if group.len() == 1 {
                return parse_single_background_size(group[0], font_size)
                    .unwrap_or(BackgroundSize::Auto);
            }
            BackgroundSize::Auto
        })
        .collect()
}

fn parse_single_background_size(value: &CssValue, font_size: f32) -> Option<BackgroundSize> {
    match value {
        CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("contain") => {
            Some(BackgroundSize::Contain)
        }
        CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("cover") => {
            Some(BackgroundSize::Cover)
        }
        CssValue::Keyword(keyword) if keyword.eq_ignore_ascii_case("auto") => {
            Some(BackgroundSize::Auto)
        }
        _ => background_dimension(value, font_size).map(|width| BackgroundSize::Explicit {
            width,
            height: BackgroundDimension::Auto,
        }),
    }
}

fn background_offset(value: &CssValue, font_size: f32) -> Option<BackgroundOffset> {
    match background_dimension(value, font_size)? {
        BackgroundDimension::Length(value) => Some(BackgroundOffset::Length(value)),
        BackgroundDimension::Percent(value) => Some(BackgroundOffset::Percent(value)),
        BackgroundDimension::Auto => None,
    }
}

pub fn parse_background_position(value: &CssValue, font_size: f32) -> Vec<BackgroundPosition> {
    let groups = split_on_separator_or_single(value);
    if groups.is_empty() {
        return vec![];
    }
    // Multiple comma-separated groups: parse each as one layer's position.
    if groups.len() > 1 {
        return groups
            .iter()
            .map(|group| {
                let group_value = if group.len() == 1 {
                    group[0].clone()
                } else {
                    CssValue::List(group.iter().map(|v| (*v).clone()).collect())
                };
                parse_single_position(&group_value, font_size).unwrap_or_default()
            })
            .collect();
    }
    // Single group: existing single-layer logic.
    let items = &groups[0];
    // First try parsing as a single position (existing behavior)
    if let Some(pos) = parse_single_position(
        &CssValue::List(items.iter().map(|v| (*v).clone()).collect()),
        font_size,
    ) {
        return vec![pos];
    }
    // If that fails, split dimension items into pairs for multiple layers
    // (e.g. `0 0, 8px 8px` → [(0,0), (8px,8px)]).
    if items.len() < 4 || items.len() % 2 != 0 {
        return vec![];
    }
    if !items
        .iter()
        .all(|v| matches!(v, CssValue::Length(_, _)) || matches!(v, CssValue::Number(0.0)))
    {
        return vec![];
    }
    let mut positions = Vec::with_capacity(items.len() / 2);
    for chunk in items.chunks(2) {
        let Some(pos) = parse_position_from_lengths(chunk[0], chunk[1], font_size) else {
            return vec![];
        };
        positions.push(pos);
    }
    positions
}

/// Parse a single background position from the full value (existing logic).
fn parse_single_position(value: &CssValue, font_size: f32) -> Option<BackgroundPosition> {
    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        _ => vec![value],
    };
    if values.is_empty() || values.len() > 4 {
        return None;
    }

    // The 3/4-value syntax consists of edge-and-offset pairs and may put the
    // vertical pair first (Scratch uses `bottom 32px right 50%`).
    if values.len() >= 3 {
        let mut position = BackgroundPosition::default();
        let mut has_x = false;
        let mut has_y = false;
        let mut index = 0;
        while index < values.len() {
            let CssValue::Keyword(edge) = values[index] else {
                return None;
            };
            let offset = values
                .get(index + 1)
                .and_then(|value| background_offset(value, font_size));
            let consumed_offset = offset.is_some();
            let offset = offset.unwrap_or_default();
            match edge.to_ascii_lowercase().as_str() {
                "left" if !has_x => {
                    position.x = BackgroundPositionAxis::Start(offset);
                    has_x = true;
                }
                "right" if !has_x => {
                    position.x = BackgroundPositionAxis::End(offset);
                    has_x = true;
                }
                "top" if !has_y => {
                    position.y = BackgroundPositionAxis::Start(offset);
                    has_y = true;
                }
                "bottom" if !has_y => {
                    position.y = BackgroundPositionAxis::End(offset);
                    has_y = true;
                }
                "center" if !has_x => {
                    position.x = BackgroundPositionAxis::Center(offset);
                    has_x = true;
                }
                "center" if !has_y => {
                    position.y = BackgroundPositionAxis::Center(offset);
                    has_y = true;
                }
                _ => return None,
            }
            index += if consumed_offset { 2 } else { 1 };
        }
        if !has_x {
            position.x = BackgroundPositionAxis::Center(BackgroundOffset::Zero);
        }
        if !has_y {
            position.y = BackgroundPositionAxis::Center(BackgroundOffset::Zero);
        }
        return Some(position);
    }

    fn keyword_axis(keyword: &str, horizontal: bool) -> Option<BackgroundPositionAxis> {
        let zero = BackgroundOffset::Zero;
        match (keyword, horizontal) {
            ("left", true) | ("top", false) => Some(BackgroundPositionAxis::Start(zero)),
            ("right", true) | ("bottom", false) => Some(BackgroundPositionAxis::End(zero)),
            ("center", _) => Some(BackgroundPositionAxis::Center(zero)),
            _ => None,
        }
    }

    if values.len() == 1 {
        return match values[0] {
            CssValue::Keyword(keyword) => {
                let keyword = keyword.to_ascii_lowercase();
                if matches!(keyword.as_str(), "top" | "bottom") {
                    Some(BackgroundPosition {
                        x: BackgroundPositionAxis::Center(BackgroundOffset::Zero),
                        y: keyword_axis(&keyword, false)?,
                    })
                } else {
                    Some(BackgroundPosition {
                        x: keyword_axis(&keyword, true)?,
                        y: BackgroundPositionAxis::Center(BackgroundOffset::Zero),
                    })
                }
            }
            value => Some(BackgroundPosition {
                x: BackgroundPositionAxis::Start(background_offset(value, font_size)?),
                y: BackgroundPositionAxis::Center(BackgroundOffset::Zero),
            }),
        };
    }

    let first_keyword = match values[0] {
        CssValue::Keyword(keyword) => Some(keyword.to_ascii_lowercase()),
        _ => None,
    };
    let second_keyword = match values[1] {
        CssValue::Keyword(keyword) => Some(keyword.to_ascii_lowercase()),
        _ => None,
    };
    let reversed = first_keyword
        .as_deref()
        .is_some_and(|keyword| matches!(keyword, "top" | "bottom"))
        && second_keyword
            .as_deref()
            .is_some_and(|keyword| matches!(keyword, "left" | "right" | "center"));
    let axis = |value: &CssValue, horizontal: bool| match value {
        CssValue::Keyword(keyword) => keyword_axis(&keyword.to_ascii_lowercase(), horizontal),
        value => Some(BackgroundPositionAxis::Start(background_offset(
            value, font_size,
        )?)),
    };
    if reversed {
        Some(BackgroundPosition {
            x: axis(values[1], true)?,
            y: axis(values[0], false)?,
        })
    } else {
        Some(BackgroundPosition {
            x: axis(values[0], true)?,
            y: axis(values[1], false)?,
        })
    }
}

/// Parse a position from two length values (x, y).
fn parse_position_from_lengths(
    x_val: &CssValue,
    y_val: &CssValue,
    font_size: f32,
) -> Option<BackgroundPosition> {
    let x_offset = match x_val {
        CssValue::Number(0.0) => BackgroundOffset::Length(0.0),
        _ => background_offset(x_val, font_size)?,
    };
    let y_offset = match y_val {
        CssValue::Number(0.0) => BackgroundOffset::Length(0.0),
        _ => background_offset(y_val, font_size)?,
    };
    Some(BackgroundPosition {
        x: BackgroundPositionAxis::Start(x_offset),
        y: BackgroundPositionAxis::Start(y_offset),
    })
}

pub fn parse_background_repeat(value: &CssValue) -> Vec<BackgroundRepeat> {
    let groups = split_on_separator_or_single(value);
    groups
        .iter()
        .filter_map(|group| {
            // A group may be a single keyword or two keywords (repeat-x repeat-y).
            if group.len() == 1 {
                parse_single_background_repeat(group[0])
            } else if group.len() == 2 {
                // Two-value syntax: [repeat-x, repeat-y]
                let rx = parse_single_background_repeat(group[0]);
                let ry = parse_single_background_repeat(group[1]);
                match (rx, ry) {
                    (Some(r), _) => Some(r),
                    (None, Some(r)) => Some(r),
                    _ => None,
                }
            } else {
                None
            }
        })
        .collect()
}

fn parse_single_background_repeat(value: &CssValue) -> Option<BackgroundRepeat> {
    let keyword = match value {
        CssValue::Keyword(keyword) => keyword.as_str(),
        _ => return None,
    };
    match keyword.to_ascii_lowercase().as_str() {
        "repeat" => Some(BackgroundRepeat::Repeat),
        "repeat-x" => Some(BackgroundRepeat::RepeatX),
        "repeat-y" => Some(BackgroundRepeat::RepeatY),
        "no-repeat" => Some(BackgroundRepeat::NoRepeat),
        _ => None,
    }
}

pub fn apply_background_shorthand_geometry(
    value: &CssValue,
    font_size: f32,
    container_style: &mut ContainerStyle,
) {
    // Handle url() function: each arg sublist is already a per-layer group.
    if let CssValue::Function(fn_name, args) = value
        && fn_name.eq_ignore_ascii_case("url")
    {
        // Per-layer repeat from url() sublists.
        for (i, arg_group) in args.iter().enumerate() {
            if let Some(repeat) = arg_group
                .iter()
                .find_map(|v| parse_single_background_repeat(v))
            {
                if let Some(layer) = container_style.background.layers.get_mut(i) {
                    layer.repeat = repeat;
                }
            }
        }
        // Per-layer size from url() sublists (after slash within each sublist).
        for (i, arg_group) in args.iter().enumerate() {
            if let Some(slash) = arg_group
                .iter()
                .position(|v| matches!(v, CssValue::Keyword(keyword) if keyword.as_str() == "/"))
            {
                let size_values: Vec<CssValue> = arg_group[slash + 1..]
                    .iter()
                    .take_while(|v| {
                        background_dimension(v, font_size).is_some()
                            || matches!(
                                v,
                                CssValue::Keyword(keyword)
                                    if matches!(
                                        keyword.to_ascii_lowercase().as_str(),
                                        "contain" | "cover" | "auto"
                                    )
                            )
                    })
                    .map(|v| (*v).clone())
                    .collect();
                if !size_values.is_empty() {
                    let size_value = if size_values.len() == 1 {
                        size_values[0].clone()
                    } else {
                        CssValue::List(size_values)
                    };
                    let sizes = parse_background_size(&size_value, font_size);
                    if let Some(layer) = container_style.background.layers.get_mut(i) {
                        layer.size = sizes.get(0).copied().unwrap_or_default();
                    }
                }
            }
        }
        // Per-layer position from url() sublists (before slash).
        for (i, arg_group) in args.iter().enumerate() {
            let slash = arg_group
                .iter()
                .position(|v| matches!(v, CssValue::Keyword(keyword) if keyword.as_str() == "/"));
            let pos_items: Vec<CssValue> = arg_group[..slash.unwrap_or(arg_group.len())]
                .iter()
                .filter(|v| {
                    background_offset(v, font_size).is_some()
                        || matches!(
                            v,
                            CssValue::Keyword(keyword)
                                if matches!(
                                    keyword.to_ascii_lowercase().as_str(),
                                    "left" | "right" | "top" | "bottom" | "center"
                                )
                        )
                })
                .map(|v| (*v).clone())
                .collect();
            if !pos_items.is_empty() {
                let pos_value = if pos_items.len() == 1 {
                    pos_items[0].clone()
                } else {
                    CssValue::List(pos_items)
                };
                let positions = parse_background_position(&pos_value, font_size);
                if let Some(layer) = container_style.background.layers.get_mut(i) {
                    layer.position = positions.get(0).copied().unwrap_or_default();
                }
            }
        }
        return;
    }

    // Top-level list: split on comma Separator into per-layer groups.
    let groups = split_on_separator_or_single(value);

    // Per-layer repeat: parse repeat from each group.
    for (i, group) in groups.iter().enumerate() {
        let repeat = group.iter().find_map(|v| parse_single_background_repeat(v));
        if let Some(r) = repeat {
            if let Some(layer) = container_style.background.layers.get_mut(i) {
                layer.repeat = r;
            }
        }
    }

    // Per-layer size: find slash within each group.
    for (i, group) in groups.iter().enumerate() {
        if let Some(slash) = group
            .iter()
            .position(|v| matches!(v, CssValue::Keyword(keyword) if keyword.as_str() == "/"))
        {
            let size_values: Vec<CssValue> = group[slash + 1..]
                .iter()
                .take_while(|v| {
                    background_dimension(v, font_size).is_some()
                        || matches!(
                            v,
                            CssValue::Keyword(keyword)
                                if matches!(
                                    keyword.to_ascii_lowercase().as_str(),
                                    "contain" | "cover" | "auto"
                                )
                        )
                })
                .map(|v| (*v).clone())
                .collect();
            if !size_values.is_empty() {
                let size_value = if size_values.len() == 1 {
                    size_values[0].clone()
                } else {
                    CssValue::List(size_values)
                };
                let sizes = parse_background_size(&size_value, font_size);
                if let Some(layer) = container_style.background.layers.get_mut(i) {
                    layer.size = sizes.get(0).copied().unwrap_or_default();
                }
            }
        }
    }

    // Per-layer position: position values before slash within each group.
    for (i, group) in groups.iter().enumerate() {
        let slash = group
            .iter()
            .position(|v| matches!(v, CssValue::Keyword(keyword) if keyword.as_str() == "/"));
        let pos_items: Vec<CssValue> = group[..slash.unwrap_or(group.len())]
            .iter()
            .filter(|v| {
                background_offset(v, font_size).is_some()
                    || matches!(
                        v,
                        CssValue::Keyword(keyword)
                            if matches!(
                                keyword.to_ascii_lowercase().as_str(),
                                "left" | "right" | "top" | "bottom" | "center"
                            )
                    )
            })
            .map(|v| (*v).clone())
            .collect();
        if !pos_items.is_empty() {
            let pos_value = if pos_items.len() == 1 {
                pos_items[0].clone()
            } else {
                CssValue::List(pos_items)
            };
            let positions = parse_background_position(&pos_value, font_size);
            if let Some(layer) = container_style.background.layers.get_mut(i) {
                layer.position = positions.get(0).copied().unwrap_or_default();
            }
        }
    }
}

pub fn parse_background_shorthand(
    _name: &str,
    value: &CssValue,
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Background {
    let mut color: Option<Color> = None;
    let mut layers: Vec<BackgroundLayer> = Vec::new();

    match value {
        CssValue::List(items) => {
            for v in items {
                parse_shorthand_item(
                    v,
                    text_style,
                    text_flow_style,
                    color_scheme,
                    &mut color,
                    &mut layers,
                );
            }
        }
        CssValue::Function(fn_name, args) if fn_name.eq_ignore_ascii_case("url") => {
            // CSS パーサーは url(...) の中にカンマ区切りの複数グループを
            // サブリストとして保持する。各サブリストを別レイヤーとして処理。
            for arg_group in args.iter() {
                parse_url_layer_group(arg_group, &mut color, &mut layers);
            }
        }
        _ => {
            parse_shorthand_item(
                value,
                text_style,
                text_flow_style,
                color_scheme,
                &mut color,
                &mut layers,
            );
        }
    }

    Background { color, layers }
}

/// Parse a single top-level shorthand item into either a background color or
/// an image/gradient layer.
fn parse_shorthand_item(
    v: &CssValue,
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
    color: &mut Option<Color>,
    layers: &mut Vec<BackgroundLayer>,
) {
    if let Some(layer) =
        parse_single_background_layer("", v, text_style, text_flow_style, color_scheme)
    {
        match layer {
            ParsedLayer::Color(c) => *color = Some(c),
            ParsedLayer::Image(layer) => layers.push(layer),
        }
    }
}

/// Parse one comma-separated group from inside a `url(...)` call. Each group
/// may contain an image source (String), a background color (Color), or nested
/// url() calls. Keywords (repeat/size/position) are handled later by
/// `apply_background_shorthand_geometry`.
fn parse_url_layer_group(
    group: &[CssValue],
    color: &mut Option<Color>,
    layers: &mut Vec<BackgroundLayer>,
) {
    let mut source: Option<String> = None;
    for item in group.iter() {
        match item {
            CssValue::String(s) => {
                source = Some(s.clone());
            }
            CssValue::Color(c) => {
                if let Some((r, g, b, a)) = CssValue::Color(c.clone()).to_rgba_tuple() {
                    *color = Some(Color(r, g, b, a));
                }
            }
            CssValue::Function(fn_name, args) if fn_name.eq_ignore_ascii_case("url") => {
                // Nested url() call: extract the image source recursively.
                for nested_group in args.iter() {
                    for nested_item in nested_group.iter() {
                        if let CssValue::String(s) = nested_item {
                            source = Some(s.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(src) = source {
        layers.push(BackgroundLayer {
            image: Some(BackgroundImage::Image {
                source: src,
                image: None,
            }),
            position: Default::default(),
            size: Default::default(),
            repeat: Default::default(),
        });
    }
}

/// Result of parsing a single shorthand item: either a background color or an image layer.
enum ParsedLayer {
    Color(Color),
    Image(BackgroundLayer),
}

fn parse_single_background_layer(
    name: &str,
    v: &CssValue,
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<ParsedLayer> {
    // inherit
    if let CssValue::Keyword(kw) = v {
        if kw.eq_ignore_ascii_case("inherit") {
            return Some(ParsedLayer::Color(text_style.color));
        }
        if kw.eq_ignore_ascii_case("currentColor") {
            return Some(ParsedLayer::Color(text_style.color));
        }
        if kw.eq_ignore_ascii_case("none")
            || kw.eq_ignore_ascii_case("initial")
            || kw.eq_ignore_ascii_case("unset")
        {
            return Some(ParsedLayer::Color(Color(0, 0, 0, 0)));
        }
    }

    if let CssValue::Number(0.0) = v {
        return Some(ParsedLayer::Color(Color(0, 0, 0, 0)));
    }

    // gradient
    if let CssValue::Function(fn_name, args) = v
        && matches!(
            fn_name.as_str(),
            "linear-gradient"
                | "repeating-linear-gradient"
                | "radial-gradient"
                | "repeating-radial-gradient"
                | "conic-gradient"
                | "repeating-conic-gradient"
        )
    {
        return Some(ParsedLayer::Image(BackgroundLayer {
            image: Some(BackgroundImage::Gradient(parse_gradient(
                fn_name,
                args,
                text_style,
                text_flow_style,
                color_scheme,
            )?)),
            position: Default::default(),
            size: Default::default(),
            repeat: Default::default(),
        }));
    }

    if let CssValue::Function(fn_name, args) = v
        && fn_name.eq_ignore_ascii_case("url")
    {
        let source = args.iter().flatten().find_map(|value| match value {
            CssValue::String(source) => Some(source.clone()),
            CssValue::Keyword(source) => Some(source.to_string()),
            _ => None,
        })?;
        return Some(ParsedLayer::Image(BackgroundLayer {
            image: Some(BackgroundImage::Image {
                source,
                image: None,
            }),
            position: Default::default(),
            size: Default::default(),
            repeat: Default::default(),
        }));
    }

    // Only color-shaped values should reach the color resolver.
    let is_color_value = matches!(v, CssValue::Color(_))
        || matches!(
            v,
            CssValue::Function(function, _)
                if matches!(
                    function.as_str(),
                    "rgb" | "rgba" | "hsl" | "hsla" | "light-dark" | "color-mix"
                )
        )
        || matches!(
            v,
            CssValue::Keyword(keyword)
                if !matches!(
                    keyword.to_ascii_lowercase().as_str(),
                    "none"
                        | "repeat"
                        | "repeat-x"
                        | "repeat-y"
                        | "no-repeat"
                        | "space"
                        | "round"
                        | "scroll"
                        | "fixed"
                        | "local"
                        | "left"
                        | "right"
                        | "top"
                        | "bottom"
                        | "center"
                        | "cover"
                        | "contain"
                        | "auto"
                        | "border-box"
                        | "padding-box"
                        | "content-box"
                        | "/"
                )
        );
    if is_color_value {
        return resolve_css_color(name, v, color_scheme).map(ParsedLayer::Color);
    }

    None
}

pub fn parse_gradient(
    fn_name: &str,
    args: &[Vec<CssValue>],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    match fn_name {
        "linear-gradient" | "repeating-linear-gradient" => {
            parse_linear_gradient(args, text_style, text_flow_style, color_scheme)
        }
        "radial-gradient" | "repeating-radial-gradient" => {
            parse_radial_gradient(args, text_style, text_flow_style, color_scheme)
        }
        "conic-gradient" | "repeating-conic-gradient" => {
            parse_conic_gradient(args, text_style, text_flow_style, color_scheme)
        }
        _ => None,
    }
}

fn parse_linear_gradient(
    args: &[Vec<CssValue>],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    // Optional leading direction group: `<angle>` or `to <side-or-corner>`,
    // taken only when it makes up the whole group; otherwise it is a stop.
    let direction = args.first().and_then(|group| {
        let (consumed, angle) = parse_linear_direction(group);
        match angle {
            Some(angle) if consumed == group.len() => Some(angle),
            _ => None,
        }
    });
    let stop_start = if direction.is_some() { 1 } else { 0 };

    let stops = parse_color_stops(
        &args[stop_start..],
        text_style,
        text_flow_style,
        color_scheme,
    )?;
    Some(Gradient {
        kind: GradientKind::Linear {
            angle: direction.unwrap_or(180.0),
        },
        stops,
        spread: GradientSpread::default(),
    })
}

/// Parse the leading `<angle>` / `to <side-or-corner>` of a linear gradient.
///
/// Returns `(consumed, angle_in_degrees)` where `angle` is `None` when no
/// direction is present (the caller should default to `180deg`). Only the
/// prelude portion is consumed; color stops must follow.
fn parse_linear_direction(args: &[CssValue]) -> (usize, Option<f32>) {
    let Some(first) = args.first() else {
        return (0, None);
    };

    // <angle>
    if let CssValue::Length(v, Unit::Deg) = first {
        return (1, Some(*v));
    }

    // "to" <side-or-corner>
    let CssValue::Keyword(k) = first else {
        return (0, None);
    };
    if !k.eq_ignore_ascii_case("to") {
        return (0, None);
    }

    // Consume the following side keywords (case-insensitive), up to two.
    let mut sides: Vec<&'static str> = Vec::new();
    let mut idx = 1;
    while idx < args.len() && sides.len() < 2 {
        let CssValue::Keyword(k) = &args[idx] else {
            break;
        };
        let side = match k.to_ascii_lowercase().as_str() {
            "top" => "top",
            "bottom" => "bottom",
            "left" => "left",
            "right" => "right",
            _ => break,
        };
        sides.push(side);
        idx += 1;
    }

    if sides.is_empty() {
        return (0, None);
    }
    // The angle exists only for the valid side/corner combinations.
    let angle = match sides.as_slice() {
        ["top"] => Some(0.0),
        ["top", "left"] | ["left", "top"] => Some(315.0),
        ["top", "right"] | ["right", "top"] => Some(45.0),
        ["bottom"] => Some(180.0),
        ["bottom", "left"] | ["left", "bottom"] => Some(225.0),
        ["bottom", "right"] | ["right", "bottom"] => Some(135.0),
        ["left"] => Some(270.0),
        ["right"] => Some(90.0),
        _ => None,
    };
    // When two side keywords appear they must be opposite axes (a corner);
    // unknown / contradictory pairs (e.g. `to top bottom`) are rejected.
    if angle.is_none() {
        return (0, None);
    }
    (idx, angle)
}

/// Parse an explicit radial size given as `<length-percentage>` values.
///
/// Returns `(consumed, size, inferred_shape)`.
///
/// An `<ellipse>` takes two values whose percentages resolve against the box's
/// width and height respectively; a `<circle>` takes a single `<length>`. A lone
/// percentage is ambiguous (no axis to resolve against) and therefore invalid.
/// Only the two-percentage (ellipse) form is representable here; mixed lengths
/// and single-length circle radii yield `None` and reject the gradient.
fn parse_explicit_radial_size(
    args: &[CssValue],
) -> Option<(usize, RadialSizeKind, Option<RadialShape>)> {
    let (CssValue::Length(rx, Unit::Percent), CssValue::Length(ry, Unit::Percent)) =
        (args.first()?, args.get(1)?)
    else {
        return None;
    };
    if *rx < 0.0 || *ry < 0.0 {
        return None;
    }
    Some((
        2,
        RadialSizeKind::Explicit {
            rx: *rx / 100.0,
            ry: *ry / 100.0,
        },
        Some(RadialShape::Ellipse),
    ))
}

/// The radial-gradient prelude — `[<ending-shape> || <size>]? [at <position>]?` —
/// as a single comma group. `None` means the group is not (fully) a prelude,
/// in which case the caller treats it as a color stop.
fn parse_radial_prelude(group: &[CssValue]) -> Option<(RadialShape, RadialSizeKind, (f32, f32))> {
    let mut shape = RadialShape::Ellipse;
    let mut size = RadialSizeKind::default();
    let mut position = (0.5, 0.5);
    let mut shape_seen = false;
    let mut size_seen = false;
    let mut idx = 0;

    // Consume the shape and size (in any order, case-insensitively) before a
    // possible "at <position>" clause. At most one shape and one size are valid;
    // duplicates are rejected.
    while idx < group.len() {
        // Explicit size (`<length-percentage>`): the size may precede the shape
        // keyword (e.g. `30% 50% ellipse`), so consume it here too.
        if matches!(&group[idx], CssValue::Length(_, Unit::Percent | Unit::Px)) {
            if size_seen {
                return None;
            }
            let (consumed, parsed_size, inferred_shape) =
                parse_explicit_radial_size(&group[idx..])?;
            size = parsed_size;
            size_seen = true;
            if let Some(shape_hint) = inferred_shape {
                if shape_seen && shape != shape_hint {
                    return None;
                }
                // Do not mark `shape_seen`: an explicit size carries its own
                // inferred shape, and a redundant matching shape keyword may
                // still follow (e.g. `30% 50% ellipse`).
                if !shape_seen {
                    shape = shape_hint;
                }
            }
            idx += consumed;
            continue;
        }
        let CssValue::Keyword(k) = &group[idx] else {
            break;
        };
        match k.to_ascii_lowercase().as_str() {
            "circle" => {
                if shape_seen || (size_seen && matches!(size, RadialSizeKind::Explicit { .. })) {
                    return None;
                }
                shape = RadialShape::Circle;
                shape_seen = true;
                idx += 1;
            }
            "ellipse" => {
                if shape_seen {
                    return None;
                }
                shape = RadialShape::Ellipse;
                shape_seen = true;
                idx += 1;
            }
            "closest-side" => {
                if size_seen {
                    return None;
                }
                size = RadialSizeKind::ClosestSide;
                size_seen = true;
                idx += 1;
            }
            "farthest-side" => {
                if size_seen {
                    return None;
                }
                size = RadialSizeKind::FarthestSide;
                size_seen = true;
                idx += 1;
            }
            "closest-corner" => {
                if size_seen {
                    return None;
                }
                size = RadialSizeKind::ClosestCorner;
                size_seen = true;
                idx += 1;
            }
            "farthest-corner" => {
                if size_seen {
                    return None;
                }
                size = RadialSizeKind::FarthestCorner;
                size_seen = true;
                idx += 1;
            }
            _ => break,
        }
    }

    // Optional "at <position>".
    if idx < group.len()
        && let CssValue::Keyword(k) = &group[idx]
        && k.eq_ignore_ascii_case("at")
    {
        let (consumed, parsed) = parse_gradient_position(&group[idx + 1..])?;
        if consumed == 0 {
            return None;
        }
        position = parsed;
        idx += 1 + consumed;
    }

    // A group is a prelude only when it starts with (and is fully consumed
    // by) shape/size/`at` clauses; anything else is a color stop.
    if idx == 0 || idx != group.len() {
        return None;
    }
    Some((shape, size, position))
}

fn parse_radial_gradient(
    args: &[Vec<CssValue>],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    let prelude = args.first().and_then(|group| parse_radial_prelude(group));
    let stop_start = if prelude.is_some() { 1 } else { 0 };
    let (shape, size, position) =
        prelude.unwrap_or_else(|| (RadialShape::Ellipse, RadialSizeKind::default(), (0.5, 0.5)));

    let stops = parse_color_stops(
        &args[stop_start..],
        text_style,
        text_flow_style,
        color_scheme,
    )?;
    Some(Gradient {
        kind: GradientKind::Radial {
            shape,
            size,
            position,
        },
        stops,
        spread: GradientSpread::default(),
    })
}

/// Parse a CSS `<position>` (as used by radial/conic `at`) into normalized
/// `(x, y)` fractions. Handles the single/two keyword and corner forms plus a
/// bare `<percentage>` value, ignoring keyword case.
///
/// Returns `(consumed, (x, y))`; `None` when no valid position is present.
/// Percentage/length offsets alongside a keyword (e.g. `at left 10px`) are not
/// representable and yield `None` (the caller rejects the gradient). The second
/// value may be absent when the position ends its group.
fn parse_gradient_position(args: &[CssValue]) -> Option<(usize, (f32, f32))> {
    // `(coordinate, is_vertical)` for a side keyword.
    fn keyword_axis(kw: &str) -> Option<(f32, bool)> {
        match kw {
            "left" => Some((0.0, false)),
            "right" => Some((1.0, false)),
            "top" => Some((0.0, true)),
            "bottom" => Some((1.0, true)),
            _ => None,
        }
    }
    fn percent_fraction(v: &CssValue) -> Option<f32> {
        match v {
            CssValue::Length(value, Unit::Percent) => Some(*value / 100.0),
            _ => None,
        }
    }

    let k0 = args.first()?;

    // A bare <length-percentage>. One value means x with y centered;
    // two values are (x, y) horizontal-then-vertical.
    if let Some(x) = percent_fraction(k0) {
        if let Some(y) = args.get(1).and_then(percent_fraction) {
            return Some((2, (x, y)));
        }
        return Some((1, (x, 0.5)));
    }

    let CssValue::Keyword(k0) = k0 else {
        return None;
    };
    let k0 = k0.to_ascii_lowercase();

    // center + optional bare percent → (0.5, y) or (0.5, 0.5).
    if k0 == "center" {
        if let Some(y) = args.get(1).and_then(percent_fraction) {
            return Some((2, (0.5, y)));
        }
        return Some((1, (0.5, 0.5)));
    }

    let (v0, is_v0) = keyword_axis(&k0)?;

    // keyword + bare <length-percentage> → perpendicular axis.
    if let Some(y) = args.get(1).and_then(percent_fraction) {
        return if is_v0 {
            Some((2, (y, v0))) // k0 vertical, percentage is horizontal
        } else {
            Some((2, (v0, y))) // k0 horizontal, percentage is vertical
        };
    }

    // Two keywords forming a corner (one horizontal, one vertical).
    if let Some(CssValue::Keyword(k1)) = args.get(1) {
        let k1 = k1.to_ascii_lowercase();
        if let Some((v1, is_v1)) = keyword_axis(&k1)
            && is_v0 != is_v1
        {
            return if is_v0 {
                Some((2, (v1, v0))) // k0 vertical (y), k1 horizontal (x)
            } else {
                Some((2, (v0, v1)))
            };
        }
    }

    // Single side keyword.
    Some(if is_v0 {
        (1, (0.5, v0))
    } else {
        (1, (v0, 0.5))
    })
}

fn parse_color_stops(
    args: &[Vec<CssValue>],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Vec<ColorStop>> {
    let mut stops = Vec::new();

    for group in args {
        let (first, rest) = group.split_first()?;
        let color = match first {
            CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => text_style.color,
            _ => resolve_css_color("gradient", first, color_scheme)?,
        };

        // Consume up to two position lengths (a double-position stop is
        // equivalent to two stops at the same color).
        let mut positions: Vec<f32> = Vec::new();
        for value in rest {
            if positions.len() == 2 {
                return None;
            }
            // Absolute lengths (px/vw/vh/…) cannot be resolved without the
            // gradient box; they are dropped here and treated as auto stops.
            if matches!(
                value,
                CssValue::Length(_, Unit::Px | Unit::Em | Unit::Rem | Unit::Vw | Unit::Vh)
            ) {
                continue;
            }
            // Math functions (including round()) resolve through the shared
            // length resolver below; anything unresolvable (absolute lengths,
            // unsupported functions, zero steps) is not a position, so the
            // gradient is rejected.
            let position = resolve_gradient_position(value, text_flow_style)?;
            positions.push(position.clamp(0.0, 1.0));
        }

        if positions.is_empty() {
            stops.push(ColorStop {
                color,
                position: None,
            });
        } else {
            for p in positions {
                stops.push(ColorStop {
                    color,
                    position: Some(p),
                });
            }
        }
    }

    // A gradient must define at least two color stops per the CSS spec.
    if stops.len() < 2 {
        return None;
    }
    Some(stops)
}

/// The conic-gradient prelude — `from <angle>? && at <position>?` — as a
/// single comma group. `None` means the group is not (fully) a prelude, in
/// which case the caller treats it as a color stop.
fn parse_conic_prelude(group: &[CssValue]) -> Option<(f32, (f32, f32))> {
    let mut angle = 0.0;
    let mut position = (0.5, 0.5);
    let mut idx = 0;

    // Optional `from <angle>`
    if let Some(CssValue::Keyword(k)) = group.get(idx)
        && k.eq_ignore_ascii_case("from")
    {
        match group.get(idx + 1) {
            Some(CssValue::Length(v, Unit::Deg)) => {
                angle = *v;
                idx += 2;
            }
            _ => return None,
        }
    }

    // Optional `at <position>`
    if let Some(CssValue::Keyword(k)) = group.get(idx)
        && k.eq_ignore_ascii_case("at")
    {
        let (consumed, parsed) = parse_gradient_position(&group[idx + 1..])?;
        if consumed == 0 {
            return None;
        }
        position = parsed;
        idx += 1 + consumed;
    }

    // A group is a prelude only when it starts with (and is fully consumed
    // by) a `from`/`at` clause; anything else is a color stop.
    if idx == 0 || idx != group.len() {
        return None;
    }
    Some((angle, position))
}

fn parse_conic_gradient(
    args: &[Vec<CssValue>],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    let prelude = args.first().and_then(|group| parse_conic_prelude(group));
    let stop_start = if prelude.is_some() { 1 } else { 0 };
    let (angle, position) = prelude.unwrap_or((0.0, (0.5, 0.5)));

    let stops = parse_color_stops(
        &args[stop_start..],
        text_style,
        text_flow_style,
        color_scheme,
    )?;
    Some(Gradient {
        kind: GradientKind::Conic { angle, position },
        stops,
        spread: GradientSpread::default(),
    })
}

/// Resolve a gradient stop position into a normalized fraction of the
/// gradient length (100% == 1.0). `None` means the value is not resolvable
/// at this stage and should be treated as an auto position.
fn resolve_gradient_position(value: &CssValue, text_flow_style: &TextFlowStyle) -> Option<f32> {
    match value {
        CssValue::Length(v, Unit::Percent) => Some(*v / 100.0),
        CssValue::Length(v, Unit::Deg) => Some(*v / 360.0),
        CssValue::Number(0.0) => Some(0.0),
        // calc()/min()/max()/clamp()/round() and other math functions reduce to a
        // Length via the shared resolver, then to a gradient fraction.
        CssValue::Function(fn_name, args)
            if matches!(fn_name.as_str(), "calc" | "min" | "max" | "clamp" | "round") =>
        {
            let value = CssValue::Function(fn_name.clone(), args.clone());
            let length =
                resolve_css_len("gradient", std::slice::from_ref(&value), text_flow_style)?;
            length_to_fraction(&length)
        }
        _ => None,
    }
}

/// Convert a resolved [`Length`] into a normalized gradient fraction. Percent
/// values are relative to the gradient length (100% == 1.0). Absolute lengths
/// (px/vw/vh) cannot be resolved without the gradient box and yield `None`.
fn length_to_fraction(length: &Length) -> Option<f32> {
    match length {
        Length::Percent(v) => Some(*v / 100.0),
        Length::Add(a, b) => Some(length_to_fraction(a)? + length_to_fraction(b)?),
        Length::Sub(a, b) => Some(length_to_fraction(a)? - length_to_fraction(b)?),
        Length::Mul(a, factor) => Some(length_to_fraction(a)? * factor),
        Length::Div(a, factor) => {
            if *factor == 0.0 {
                None
            } else {
                Some(length_to_fraction(a)? / factor)
            }
        }
        Length::Min(a, b) => Some(length_to_fraction(a)?.min(length_to_fraction(b)?)),
        Length::Max(a, b) => Some(length_to_fraction(a)?.max(length_to_fraction(b)?)),
        Length::Clamp { min, val, max } => {
            Some(length_to_fraction(val)?.clamp(length_to_fraction(min)?, length_to_fraction(max)?))
        }
        Length::Px(_) | Length::Vw(_) | Length::Vh(_) => None,
        // `round()` of percentage-valued stops resolves in the normalized
        // fraction space (100% == 1.0) used throughout this function.
        Length::Round {
            strategy,
            value,
            step,
        } => {
            let value = length_to_fraction(value.as_ref())?;
            let step = length_to_fraction(step.as_ref())?;
            if step == 0.0 {
                None
            } else {
                Some(round_to_step(*strategy, value, step))
            }
        }
    }
}
