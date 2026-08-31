use crate::engine::css::values::{CssValue, Unit};

use super::{resolve_css_color, resolve_css_len};
use crate::engine::layouter::types::{
    Background, BackgroundDimension, BackgroundOffset, BackgroundPosition, BackgroundPositionAxis,
    BackgroundRepeat, BackgroundSize, Color, ColorScheme, ColorStop, ContainerStyle, Gradient,
    GradientKind, RadialShape, RadialSizeKind, TextFlowStyle, TextStyle,
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

pub fn parse_background_size(value: &CssValue, font_size: f32) -> Option<BackgroundSize> {
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
        CssValue::List(values) => match values.as_slice() {
            [width] => Some(BackgroundSize::Explicit {
                width: background_dimension(width, font_size)?,
                height: BackgroundDimension::Auto,
            }),
            [width, height] => Some(BackgroundSize::Explicit {
                width: background_dimension(width, font_size)?,
                height: background_dimension(height, font_size)?,
            }),
            _ => None,
        },
        _ => Some(BackgroundSize::Explicit {
            width: background_dimension(value, font_size)?,
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

pub fn parse_background_position(value: &CssValue, font_size: f32) -> Option<BackgroundPosition> {
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

pub fn parse_background_repeat(value: &CssValue) -> Option<BackgroundRepeat> {
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
    container_style.background_repeat = BackgroundRepeat::default();
    container_style.background_size = BackgroundSize::default();
    container_style.background_position = BackgroundPosition::default();

    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        _ => vec![value],
    };
    if let Some(repeat) = values
        .iter()
        .find_map(|value| parse_background_repeat(value))
    {
        container_style.background_repeat = repeat;
    }

    let slash = values
        .iter()
        .position(|value| matches!(value, CssValue::Keyword(keyword) if keyword.as_str() == "/"));
    if let Some(slash) = slash {
        let size_values: Vec<CssValue> = values[slash + 1..]
            .iter()
            .take_while(|value| {
                background_dimension(value, font_size).is_some()
                    || matches!(value, CssValue::Keyword(keyword) if matches!(keyword.to_ascii_lowercase().as_str(), "contain" | "cover" | "auto"))
            })
            .map(|value| (*value).clone())
            .collect();
        let size_value = match size_values.as_slice() {
            [value] => Some(value.clone()),
            [] => None,
            _ => Some(CssValue::List(size_values)),
        };
        if let Some(size) = size_value
            .as_ref()
            .and_then(|value| parse_background_size(value, font_size))
        {
            container_style.background_size = size;
        }
    }

    let position_values: Vec<CssValue> = values[..slash.unwrap_or(values.len())]
        .iter()
        .filter(|value| {
            background_offset(value, font_size).is_some()
                || matches!(
                    value,
                    CssValue::Keyword(keyword)
                        if matches!(keyword.to_ascii_lowercase().as_str(), "left" | "right" | "top" | "bottom" | "center")
                )
        })
        .map(|value| (*value).clone())
        .collect();
    let position_value = match position_values.as_slice() {
        [value] => Some(value.clone()),
        [] => None,
        _ => Some(CssValue::List(position_values)),
    };
    if let Some(position) = position_value
        .as_ref()
        .and_then(|value| parse_background_position(value, font_size))
    {
        container_style.background_position = position;
    }
}

pub fn parse_background_shorthand(
    name: &str,
    value: &CssValue,
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Background> {
    let items: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        _ => vec![value],
    };

    let mut maybe_color: Option<Color> = None;
    let mut maybe_gradient: Option<Gradient> = None;
    let mut maybe_image: Option<String> = None;

    for v in items {
        // inherit
        if let CssValue::Keyword(kw) = v {
            if kw.eq_ignore_ascii_case("inherit") {
                maybe_color = Some(text_style.color);
                continue;
            }
            if kw.eq_ignore_ascii_case("currentColor") {
                maybe_color = Some(text_style.color);
                continue;
            }
            if kw.eq_ignore_ascii_case("none")
                || kw.eq_ignore_ascii_case("initial")
                || kw.eq_ignore_ascii_case("unset")
            {
                maybe_color = Some(Color(0, 0, 0, 0));
                continue;
            }
        }

        if let CssValue::Number(0.0) = v {
            maybe_color = Some(Color(0, 0, 0, 0));
            continue;
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
            maybe_gradient = Some(parse_gradient(
                fn_name,
                &args.iter().flatten().cloned().collect::<Vec<_>>(),
                text_style,
                text_flow_style,
                color_scheme,
            )?);
            continue;
        }

        if let CssValue::Function(fn_name, args) = v
            && fn_name.eq_ignore_ascii_case("url")
        {
            maybe_image = args.iter().flatten().find_map(|value| match value {
                CssValue::String(source) => Some(source.clone()),
                CssValue::Keyword(source) => Some(source.to_string()),
                _ => None,
            });
            continue;
        }

        // Only color-shaped values should reach the color resolver. A
        // background shorthand also contains image, repeat, attachment,
        // position, size and box tokens, none of which are color errors.
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
        if is_color_value && let Some(c) = resolve_css_color(name, v, color_scheme) {
            maybe_color = Some(c);
        }
    }

    if let Some(g) = maybe_gradient {
        return Some(Background::Gradient(g));
    }
    if let Some(source) = maybe_image {
        return Some(Background::Image {
            source,
            image: None,
            color: maybe_color.unwrap_or(Color(0, 0, 0, 0)),
        });
    }
    if let Some(c) = maybe_color {
        return Some(Background::Color(c));
    }

    None
}

pub fn parse_gradient(
    fn_name: &str,
    args: &[CssValue],
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
    args: &[CssValue],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    if args.is_empty() {
        return None;
    }

    let (skip, angle) = parse_linear_direction(args);
    let angle = angle.unwrap_or(180.0);
    let stops = parse_color_stops(&args[skip..], text_style, text_flow_style, color_scheme)?;

    Some(Gradient {
        kind: GradientKind::Linear { angle },
        stops,
    })
}

/// Returns (number_of_consumed_args, optional_angle_in_degrees).
fn parse_linear_direction(args: &[CssValue]) -> (usize, Option<f32>) {
    if args.is_empty() {
        return (0, None);
    }

    // <angle>
    if let CssValue::Length(v, Unit::Deg) = &args[0] {
        return (1, Some(*v));
    }

    // "to" <side-or-corner>
    if let CssValue::Keyword(k) = &args[0]
        && k.as_str() == "to"
        && args.len() > 1
    {
        let mut idx = 1;
        let mut sides: Vec<&str> = Vec::new();
        while idx < args.len() {
            if let CssValue::Keyword(k) = &args[idx] {
                match k.as_str() {
                    "top" | "bottom" | "left" | "right" => {
                        sides.push(k.as_str());
                        idx += 1;
                    }
                    _ => break,
                }
            } else {
                break;
            }
        }
        if !sides.is_empty() {
            let angle = match sides.as_slice() {
                ["top"] => Some(0.0),
                ["top", "left"] => Some(315.0),
                ["top", "right"] => Some(45.0),
                ["bottom"] => Some(180.0),
                ["bottom", "left"] => Some(225.0),
                ["bottom", "right"] => Some(135.0),
                ["left"] => Some(270.0),
                ["right"] => Some(90.0),
                _ => None,
            };
            return (idx, angle);
        }
    }

    (0, None)
}

fn parse_radial_gradient(
    args: &[CssValue],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    let mut shape = RadialShape::Ellipse;
    let mut size = RadialSizeKind::default();
    let mut position = (0.5f32, 0.5f32);

    let mut idx = 0;

    // Consume known radial keywords before color stops
    while idx < args.len() {
        if let CssValue::Keyword(k) = &args[idx] {
            match k.as_str() {
                "circle" => {
                    shape = RadialShape::Circle;
                    idx += 1;
                    continue;
                }
                "ellipse" => {
                    shape = RadialShape::Ellipse;
                    idx += 1;
                    continue;
                }
                "closest-side" => {
                    size = RadialSizeKind::ClosestSide;
                    idx += 1;
                    continue;
                }
                "farthest-side" => {
                    size = RadialSizeKind::FarthestSide;
                    idx += 1;
                    continue;
                }
                "closest-corner" => {
                    size = RadialSizeKind::ClosestCorner;
                    idx += 1;
                    continue;
                }
                "farthest-corner" => {
                    size = RadialSizeKind::FarthestCorner;
                    idx += 1;
                    continue;
                }
                _ => break,
            }
        } else {
            break;
        }
    }

    // Optional "at <position>" — simplified to "at center" / "at top left" etc.
    if idx < args.len() && args[idx] == CssValue::Keyword("at".into()) {
        idx += 1; // skip "at"
        if idx < args.len()
            && let CssValue::Keyword(k) = &args[idx]
        {
            // Parse position keywords
            match k.as_str() {
                "center" => position = (0.5, 0.5),
                "top" => position = (0.5, 0.0),
                "bottom" => position = (0.5, 1.0),
                "left" => position = (0.0, 0.5),
                "right" => position = (1.0, 0.5),
                _ => {} // ignore unknown
            }
            idx += 1;
            // Optional second keyword (e.g. "top left")
            if idx < args.len()
                && let CssValue::Keyword(k2) = &args[idx]
            {
                match (k.as_str(), k2.as_str()) {
                    ("top", "left") | ("left", "top") => position = (0.0, 0.0),
                    ("top", "right") | ("right", "top") => position = (1.0, 0.0),
                    ("bottom", "left") | ("left", "bottom") => position = (0.0, 1.0),
                    ("bottom", "right") | ("right", "bottom") => position = (1.0, 1.0),
                    _ => {}
                }
                idx += 1;
            }
        }
    }

    let stops = parse_color_stops(&args[idx..], text_style, text_flow_style, color_scheme)?;
    if stops.is_empty() {
        return None;
    }
    Some(Gradient {
        kind: GradientKind::Radial {
            shape,
            size,
            position,
        },
        stops,
    })
}

fn parse_color_stops(
    args: &[CssValue],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Vec<ColorStop>> {
    let mut stops = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let color = match &args[i] {
            CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => text_style.color,
            _ => resolve_css_color("gradient", &args[i], color_scheme)?,
        };
        i += 1;

        // Consume up to two position lengths (a double-position stop is
        // equivalent to two stops at the same color).
        let mut positions: Vec<f32> = Vec::new();
        while positions.len() < 2 && i < args.len() {
            if let CssValue::Length(_v, Unit::Px) = &args[i] {
                // Absolute lengths are not resolvable here; treat as auto.
                i += 1;
                continue;
            }
            match resolve_gradient_position(&args[i], text_flow_style) {
                Some(position) => {
                    positions.push(position.clamp(0.0, 1.0));
                    i += 1;
                }
                None => break,
            }
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

    Some(stops)
}

fn parse_conic_gradient(
    args: &[CssValue],
    text_style: &TextStyle,
    text_flow_style: &TextFlowStyle,
    color_scheme: ColorScheme,
) -> Option<Gradient> {
    if args.is_empty() {
        return None;
    }

    let mut angle = 0.0f32;
    let mut position = (0.5f32, 0.5f32);
    let mut idx = 0;

    // Optional `from <angle>`
    if idx + 1 < args.len()
        && matches!(&args[idx], CssValue::Keyword(k) if k.eq_ignore_ascii_case("from"))
        && let CssValue::Length(v, Unit::Deg) = &args[idx + 1]
    {
        angle = *v;
        idx += 2;
    }

    // Optional `at <position>`
    if idx + 1 < args.len()
        && matches!(&args[idx], CssValue::Keyword(k) if k.eq_ignore_ascii_case("at"))
    {
        idx += 1;
        if idx < args.len()
            && let CssValue::Keyword(k) = &args[idx]
        {
            match k.as_str() {
                "center" => position = (0.5, 0.5),
                "top" => position = (0.5, 0.0),
                "bottom" => position = (0.5, 1.0),
                "left" => position = (0.0, 0.5),
                "right" => position = (1.0, 0.5),
                _ => {}
            }
            idx += 1;
            if idx < args.len()
                && let CssValue::Keyword(k2) = &args[idx]
            {
                match (k.as_str(), k2.as_str()) {
                    ("top", "left") | ("left", "top") => position = (0.0, 0.0),
                    ("top", "right") | ("right", "top") => position = (1.0, 0.0),
                    ("bottom", "left") | ("left", "bottom") => position = (0.0, 1.0),
                    ("bottom", "right") | ("right", "bottom") => position = (1.0, 1.0),
                    _ => {}
                }
                idx += 1;
            }
        }
    }

    let stops = parse_color_stops(&args[idx..], text_style, text_flow_style, color_scheme)?;
    if stops.is_empty() {
        return None;
    }
    Some(Gradient {
        kind: GradientKind::Conic { angle, position },
        stops,
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
        CssValue::Function(fn_name, args) if fn_name == "calc" => {
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
    }
}
