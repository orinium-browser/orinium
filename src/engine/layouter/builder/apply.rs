use std::sync::LazyLock;

use smol_str::SmolStr;

use crate::engine::css::values::CssValue;
use crate::engine::layouter::types::CursorStyle;

use ui_layout::{
    AlignContent, AlignItems, BoxSizing, Display, FlexDirection, FlexWrap, JustifyContent,
    JustifyItems, Length, LengthOrAuto, OuterDisplay, Position, Style,
};

use crate::engine::layouter::types::{
    Background, BorderRadius, BorderStyle, Color, ColorScheme, ContainerStyle, CornerRadius,
    CssFloat, FontStyle, FontWeight, LineHeight, Overflow, TextAlign, TextDecoration,
    TextFlowStyle, TextStyle, TextTransform, VerticalAlign, Visibility, WhiteSpace,
};

use super::{
    apply_background_shorthand_geometry, extract_font_families, length_to_px, one_or_two_values,
    parse_background_position, parse_background_repeat, parse_background_shorthand,
    parse_background_size, parse_grid_line, parse_grid_line_end, parse_grid_placement,
    parse_grid_template_areas, parse_grid_tracks, resolve_css_color, resolve_css_len,
    resolve_css_len_auto, resolve_font_size_px,
};

macro_rules! apply_property {
    ($field:ident, $target:ident, $parent:ident, $default:expr, $value:expr, $parse:expr) => {
        $target.$field = if let CssValue::Keyword(v) = $value
            && v == "initial"
        {
            $default.$field.clone()
        } else if let CssValue::Keyword(v) = $value
            && v == "inherit"
        {
            $parent.$field.clone()
        } else {
            $parse
        };
    };

    ($field:ident, $target:ident, $parent:ident, $default:expr, $value:expr, $pattern:pat, $parse:expr) => {
        $target.$field = if let CssValue::Keyword(v) = $value
            && v == "initial"
        {
            $default.$field.clone()
        } else if let CssValue::Keyword(v) = $value
            && v == "inherit"
        {
            $parent.$field.clone()
        } else if let $pattern = $value {
            $parse
        } else {
            return None;
        };
    };
}

static DEFAULT_STYLE: LazyLock<Style> = LazyLock::new(Style::default);
static DEFAULT_CONTAINER_STYLE: LazyLock<ContainerStyle> = LazyLock::new(ContainerStyle::default);
static DEFAULT_TEXT_STYLE: LazyLock<TextStyle> = LazyLock::new(TextStyle::default);
static DEFAULT_TEXT_FLOW_STYLE: LazyLock<TextFlowStyle> = LazyLock::new(TextFlowStyle::default);

pub fn blockify_out_of_flow_positioned(style: &mut Style) {
    if style.position.kind.is_out_of_flow() && style.display.outer == OuterDisplay::Inline {
        style.display.outer = OuterDisplay::Block;
    }
}

fn resolve_flex_shorthand(
    value: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<(f32, f32, LengthOrAuto)> {
    let zero_basis = LengthOrAuto::Length(Length::Percent(0.0));

    match value {
        CssValue::Keyword(keyword) => match keyword.as_str() {
            "none" => Some((0.0, 0.0, LengthOrAuto::Auto)),
            "auto" => Some((1.0, 1.0, LengthOrAuto::Auto)),
            "initial" => Some((0.0, 1.0, LengthOrAuto::Auto)),
            _ => None,
        },
        CssValue::Number(grow) if *grow >= 0.0 => Some((*grow, 1.0, zero_basis)),
        CssValue::Length(_, _) => Some((
            1.0,
            1.0,
            resolve_css_len_auto("flex", value, text_flow_style)?,
        )),
        CssValue::List(values) => match values.as_slice() {
            [CssValue::Number(grow), CssValue::Number(shrink)]
                if *grow >= 0.0 && *shrink >= 0.0 =>
            {
                Some((*grow, *shrink, zero_basis))
            }
            [CssValue::Number(grow), basis] if *grow >= 0.0 => Some((
                *grow,
                1.0,
                resolve_css_len_auto("flex", basis, text_flow_style)?,
            )),
            [CssValue::Number(grow), CssValue::Number(shrink), basis]
                if *grow >= 0.0 && *shrink >= 0.0 =>
            {
                Some((
                    *grow,
                    *shrink,
                    resolve_css_len_auto("flex", basis, text_flow_style)?,
                ))
            }
            _ => None,
        },
        _ => None,
    }
}

fn flex_direction_keyword(keyword: &str) -> Option<FlexDirection> {
    match keyword {
        "row" => Some(FlexDirection::Row),
        "column" => Some(FlexDirection::Column),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

fn flex_wrap_keyword(keyword: &str) -> Option<FlexWrap> {
    match keyword {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

fn resolve_flex_flow(value: &CssValue) -> Option<(FlexDirection, FlexWrap)> {
    let values = match value {
        CssValue::Keyword(keyword) if keyword == "initial" => {
            return Some((FlexDirection::Row, FlexWrap::NoWrap));
        }
        CssValue::Keyword(_) => std::slice::from_ref(value),
        CssValue::List(values) if (1..=2).contains(&values.len()) => values.as_slice(),
        _ => return None,
    };
    let mut direction = None;
    let mut wrap = None;
    for value in values {
        let CssValue::Keyword(keyword) = value else {
            return None;
        };
        if let Some(parsed) = flex_direction_keyword(keyword) {
            if direction.replace(parsed).is_some() {
                return None;
            }
        } else {
            let parsed = flex_wrap_keyword(keyword)?;
            if wrap.replace(parsed).is_some() {
                return None;
            }
        }
    }

    Some((
        direction.unwrap_or(FlexDirection::Row),
        wrap.unwrap_or(FlexWrap::NoWrap),
    ))
}

fn resolve_justify_items(keyword: &str) -> Option<JustifyItems> {
    match keyword {
        "stretch" => Some(JustifyItems::Stretch),
        "flex-start" | "start" => Some(JustifyItems::Start),
        "center" => Some(JustifyItems::Center),
        "flex-end" | "end" => Some(JustifyItems::End),
        _ => None,
    }
}

fn resolve_align_items(keyword: &str) -> Option<AlignItems> {
    match keyword {
        "stretch" => Some(AlignItems::Stretch),
        "flex-start" | "start" => Some(AlignItems::Start),
        "center" => Some(AlignItems::Center),
        "flex-end" | "end" => Some(AlignItems::End),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn apply_declaration(
    name: &str,
    value: &CssValue,
    style: &mut Style,
    container_style: &mut ContainerStyle,
    text_style: &mut TextStyle,
    text_flow_style: &mut TextFlowStyle,
    parent_style: &Style,
    parent_container_style: &ContainerStyle,
    parent_text_style: &TextStyle,
    parent_text_flow_style: &TextFlowStyle,
    overflow: &mut Overflow,
    color_scheme: ColorScheme,
) -> Option<()> {
    fn expand_box<T: Clone, F>(
        name: &str,
        value: &CssValue,
        text_flow_style: &TextFlowStyle,
        resolve: &impl Fn(&str, &CssValue, &TextFlowStyle) -> Option<T>,
        mut set: F,
    ) -> Option<()>
    where
        F: FnMut(T, T, T, T),
    {
        let resolve = |v: &CssValue| -> Option<T> { resolve(name, v, text_flow_style) };

        match value {
            CssValue::List(values) => {
                let vals: Vec<T> = values.iter().map(resolve).collect::<Option<_>>()?;

                match vals.as_slice() {
                    [a] => set(a.clone(), a.clone(), a.clone(), a.clone()),
                    [v, h] => set(v.clone(), h.clone(), v.clone(), h.clone()),
                    [t, h, b] => set(t.clone(), h.clone(), b.clone(), h.clone()),
                    [t, r, b, l] => set(t.clone(), r.clone(), b.clone(), l.clone()),
                    _ => return None,
                }
            }

            _ => {
                let v = resolve(value)?;
                set(v.clone(), v.clone(), v.clone(), v);
            }
        }

        Some(())
    }

    /// Split a border-radius value list on the `Keyword("/")` separator into
    /// its horizontal and vertical components. Without a slash the vertical
    /// list is empty (meaning "use the horizontal value").
    fn split_radius_lists<'a>(value: &'a CssValue) -> (Vec<&'a CssValue>, Vec<&'a CssValue>) {
        match value {
            CssValue::List(vals) => {
                let mut horiz: Vec<&'a CssValue> = Vec::new();
                let mut vert: Vec<&'a CssValue> = Vec::new();
                let mut after_slash = false;
                for v in vals {
                    if let CssValue::Keyword(k) = v
                        && k == "/"
                    {
                        after_slash = true;
                        continue;
                    }
                    if after_slash {
                        vert.push(v);
                    } else {
                        horiz.push(v);
                    }
                }
                (horiz, vert)
            }
            _ => (vec![value], Vec::new()),
        }
    }

    /// Expand a 1/2/3/4-value list to per-corner lengths in CSS order
    /// [top-left, top-right, bottom-right, bottom-left].
    fn expand_radius_axis(
        name: &str,
        values: &[&CssValue],
        text_flow_style: &TextFlowStyle,
    ) -> Option<[Length; 4]> {
        let vals: Vec<Length> = values
            .iter()
            .map(|v| resolve_css_len(name, v, text_flow_style))
            .collect::<Option<_>>()?;
        match vals.as_slice() {
            [a] => Some([a.clone(), a.clone(), a.clone(), a.clone()]),
            [v, h] => Some([v.clone(), h.clone(), v.clone(), h.clone()]),
            [t, h, b] => Some([t.clone(), h.clone(), b.clone(), h.clone()]),
            [t, r, b, l] => Some([t.clone(), r.clone(), b.clone(), l.clone()]),
            _ => None,
        }
    }

    /// Parse a `border-radius` shorthand value (1-4 lengths per axis, optional
    /// elliptical `/` form) into the four corners.
    fn parse_border_radius_shorthand(
        name: &str,
        value: &CssValue,
        text_flow_style: &TextFlowStyle,
    ) -> Option<(CornerRadius, CornerRadius, CornerRadius, CornerRadius)> {
        let (horiz, vert) = split_radius_lists(value);
        let h = expand_radius_axis(name, &horiz, text_flow_style)?;
        let v = if vert.is_empty() {
            h.clone()
        } else {
            expand_radius_axis(name, &vert, text_flow_style)?
        };
        Some((
            CornerRadius {
                x: h[0].clone(),
                y: v[0].clone(),
            },
            CornerRadius {
                x: h[1].clone(),
                y: v[1].clone(),
            },
            CornerRadius {
                x: h[2].clone(),
                y: v[2].clone(),
            },
            CornerRadius {
                x: h[3].clone(),
                y: v[3].clone(),
            },
        ))
    }

    /// Parse a single-corner radius value: one length, two lengths (`rx ry`)
    /// or the elliptical `rx / ry` form.
    fn parse_corner_radius(
        name: &str,
        value: &CssValue,
        text_flow_style: &TextFlowStyle,
    ) -> Option<CornerRadius> {
        let (horiz, vert) = split_radius_lists(value);
        let x = expand_radius_axis(name, &horiz, text_flow_style)?;
        let y = if vert.is_empty() {
            x.clone()
        } else {
            expand_radius_axis(name, &vert, text_flow_style)?
        };
        Some(CornerRadius {
            x: x[0].clone(),
            y: y[0].clone(),
        })
    }

    /// Whether a single `overflow` keyword enables scrolling on an axis.
    fn overflow_scrollable(keyword: &str) -> Option<bool> {
        match keyword {
            "visible" | "clip" => Some(false),
            "hidden" | "scroll" | "auto" => Some(true),
            _ => None,
        }
    }

    /// Expand an `overflow` shorthand (1 or 2 keywords) into per-axis flags.
    fn overflow_flags(value: &CssValue) -> Option<(bool, bool)> {
        match value {
            CssValue::Keyword(k) => overflow_scrollable(k).map(|b| (b, b)),
            CssValue::List(l) => {
                let x = match l.first()? {
                    CssValue::Keyword(k) => overflow_scrollable(k)?,
                    _ => return None,
                };
                let y = match l.get(1) {
                    Some(CssValue::Keyword(k)) => overflow_scrollable(k)?,
                    Some(_) => return None,
                    None => x,
                };
                Some((x, y))
            }
            _ => None,
        }
    }

    fn parse_border_shorthand(
        name: &str,
        value: &CssValue,
        text_flow_style: &TextFlowStyle,
        color_scheme: ColorScheme,
    ) -> Option<(Option<Length>, Option<BorderStyle>, Option<Color>)> {
        let mut width: Option<Length> = None;
        let mut style_v: Option<BorderStyle> = None;
        let mut color_v: Option<Color> = None;

        let items: Vec<&CssValue> = match value {
            CssValue::List(values) => values.iter().collect(),
            _ => vec![value],
        };

        for v in items {
            let token = v;

            // try as length (numeric lengths)
            if width.is_none()
                && let Some(l) = resolve_css_len(name, token, text_flow_style)
            {
                width = Some(l);
                continue;
            }

            // try as width keyword (thin/medium/thick). Check keywords before style keywords.
            if width.is_none()
                && let CssValue::Keyword(s) = token
            {
                match s.as_str().to_ascii_lowercase().as_str() {
                    "thin" => {
                        width = Some(Length::Px(1.0));
                        continue;
                    }
                    "medium" => {
                        width = Some(Length::Px(3.0));
                        continue;
                    }
                    "midium" => {
                        width = Some(Length::Px(3.0));
                        continue;
                    } // common misspelling
                    "thick" => {
                        width = Some(Length::Px(5.0));
                        continue;
                    }
                    _ => {}
                }
            }

            // try as style keyword
            if style_v.is_none()
                && let CssValue::Keyword(s) = token
            {
                let s_lower = s.as_str();
                let parsed = match s_lower {
                    "none" => Some(BorderStyle::None),
                    "solid" => Some(BorderStyle::Solid),
                    "dashed" => Some(BorderStyle::Dashed),
                    "dotted" => Some(BorderStyle::Dotted),
                    "inset" | "outset" | "groove" | "ridge" | "double" | "hidden" => {
                        // stub
                        style_v = Some(BorderStyle::Solid);
                        continue;
                    }
                    _ => None,
                };

                if let Some(p) = parsed {
                    style_v = Some(p);
                    continue;
                }
            }

            // try as color
            if color_v.is_none()
                && let Some(c) = resolve_css_color(name, token, color_scheme)
            {
                color_v = Some(c);
                continue;
            }

            // unknown token: ignore
        }

        Some((width, style_v, color_v))
    }

    match (name, value) {
        /* ======================
         * Display
         * ====================== */
        ("display", _) => {
            apply_property!(
                display,
                style,
                parent_style,
                DEFAULT_STYLE,
                value,
                CssValue::Keyword(v),
                Display::from_css_name(v.as_str())?
            );
        }

        ("z-index", _) => {
            let f = |v: &CssValue| match v {
                CssValue::Number(v) => {
                    if v.is_finite() && v.fract().abs() < f32::EPSILON {
                        Some(Some(*v as i32))
                    } else {
                        None
                    }
                }
                CssValue::Keyword(v) if v.eq_ignore_ascii_case("auto") => Some(None),
                _ => None,
            };

            apply_property!(
                z_index,
                container_style,
                parent_container_style,
                DEFAULT_CONTAINER_STYLE,
                value,
                f(value)?
            );
        }

        ("visibility", _) => {
            let f = |v: &SmolStr| match v.to_ascii_lowercase().as_str() {
                "visible" => Some(Visibility::Visible),
                "hidden" => Some(Visibility::Hidden),
                "collapse" => Some(Visibility::Collapse),
                _ => None,
            };

            apply_property!(
                visibility,
                container_style,
                parent_container_style,
                DEFAULT_CONTAINER_STYLE,
                value,
                CssValue::Keyword(v),
                f(v)?
            );
        }

        ("float", _) => {
            let f = |v: &SmolStr| match v.to_ascii_lowercase().as_str() {
                "left" => Some(CssFloat::Left),
                "right" => Some(CssFloat::Right),
                "none" => Some(CssFloat::None),
                _ => None,
            };

            apply_property!(
                css_float,
                container_style,
                parent_container_style,
                DEFAULT_CONTAINER_STYLE,
                value,
                CssValue::Keyword(v),
                f(v)?
            );
        }

        ("cursor", _) => {
            let f = |v: &SmolStr| match v.as_str() {
                "auto" => Some(CursorStyle::Auto),
                "default" => Some(CursorStyle::Default),
                "none" => Some(CursorStyle::None),
                "pointer" => Some(CursorStyle::Pointer),
                "text" => Some(CursorStyle::Text),
                "move" => Some(CursorStyle::Move),
                "not-allowed" => Some(CursorStyle::NotAllowed),
                "wait" => Some(CursorStyle::Wait),
                "crosshair" => Some(CursorStyle::Crosshair),
                "grab" => Some(CursorStyle::Grab),
                "grabbing" => Some(CursorStyle::Grabbing),
                _ => None,
            };

            apply_property!(
                cursor,
                container_style,
                parent_container_style,
                DEFAULT_CONTAINER_STYLE,
                value,
                CssValue::Keyword(v),
                f(v)?
            );
        }

        /* ======================
         * Color / Text
         * ====================== */
        ("background-color", _) => {
            let color = match value {
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => text_style.color,
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => {
                    text_style.color
                }
                CssValue::Keyword(kw)
                    if kw.eq_ignore_ascii_case("initial") || kw.eq_ignore_ascii_case("unset") =>
                {
                    Color(0, 0, 0, 0)
                }
                _ => resolve_css_color(name, value, color_scheme)?,
            };
            match &mut container_style.background {
                Background::Image {
                    color: image_color, ..
                } => *image_color = color,
                _ => container_style.background = Background::Color(color),
            };
        }

        ("background", _) => {
            container_style.background =
                parse_background_shorthand(name, value, text_style, text_flow_style, color_scheme)?;
            apply_background_shorthand_geometry(value, text_flow_style.font_size, container_style);
        }

        ("background-image", _) => {
            let existing_color = match &container_style.background {
                Background::Color(color) | Background::Image { color, .. } => *color,
                Background::Gradient(_) => Color(0, 0, 0, 0),
            };
            let parsed =
                parse_background_shorthand(name, value, text_style, text_flow_style, color_scheme)?;
            container_style.background = match parsed {
                Background::Image { source, image, .. } => Background::Image {
                    source,
                    image,
                    color: existing_color,
                },
                Background::Color(Color(_, _, _, 0)) => Background::Color(existing_color),
                other => other,
            };
        }

        ("background-repeat", _) => {
            apply_property!(
                background_repeat,
                container_style,
                parent_container_style,
                DEFAULT_CONTAINER_STYLE,
                value,
                parse_background_repeat(value)?
            );
        }

        ("background-size", _) => {
            container_style.background_size =
                parse_background_size(value, text_flow_style.font_size)?;
        }

        ("background-position", _) => {
            container_style.background_position =
                parse_background_position(value, text_flow_style.font_size)?;
        }

        ("color", _) => {
            text_style.color = match value {
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => {
                    // inherit: use parent's color
                    text_style.color
                }
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("currentColor") => {
                    text_style.color
                }
                _ => resolve_css_color(name, value, color_scheme)?,
            }
        }

        // `color-scheme` is resolved separately above.
        ("color-scheme", _) => {}

        ("font-size", _) => {
            let len = resolve_css_len(name, value, text_flow_style)?;
            let px = resolve_font_size_px(&len, text_flow_style.font_size)?;
            text_flow_style.font_size = px;
        }

        ("line-height", CssValue::Number(factor)) => {
            text_flow_style.line_height = LineHeight::Number(*factor);
        }
        ("line-height", CssValue::Keyword(v)) if v == "normal" => {
            text_flow_style.line_height = LineHeight::Normal;
        }
        ("line-height", _) => {
            let len = resolve_css_len(name, value, text_flow_style)?;
            text_flow_style.line_height =
                LineHeight::Px(length_to_px(&len, text_flow_style.font_size));
        }

        ("font-weight", CssValue::Keyword(v)) => {
            text_style.font_weight = match v.as_str() {
                "normal" => FontWeight::NORMAL,
                "bold" => FontWeight::BOLD,
                _ => text_style.font_weight,
            };
        }
        ("font-weight", CssValue::Number(v)) => {
            text_style.font_weight = FontWeight(*v as u16);
        }

        ("font-style", _) => {
            apply_property!(
                font_style,
                text_style,
                parent_text_style,
                DEFAULT_TEXT_STYLE,
                value,
                CssValue::Keyword(v),
                match v.as_str() {
                    "normal" => FontStyle::Normal,
                    "italic" => FontStyle::Italic,
                    "oblique" => FontStyle::Oblique,
                    _ => text_style.font_style,
                }
            );
        }

        ("font-family", _) => {
            let families = extract_font_families(value);
            if !families.is_empty() {
                text_style.font_families = families;
            }
        }

        ("font", v) => {
            // CSS `font` shorthand:
            // [ [ <'font-style'> || <'font-variant'> || <'font-weight'> ] font-size [/ line-height]? font-family ]
            let values: Vec<&CssValue> = match v {
                CssValue::List(list) => list.iter().collect(),
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("inherit") => {
                    text_style.font_style = parent_text_style.font_style;
                    text_style.font_weight = parent_text_style.font_weight;
                    text_style.font_families = parent_text_style.font_families.clone();
                    text_flow_style.font_size = parent_text_flow_style.font_size;
                    text_flow_style.line_height = parent_text_flow_style.line_height.clone();
                    return Some(());
                }
                CssValue::Keyword(kw) if kw.eq_ignore_ascii_case("initial") => {
                    text_style.font_style = DEFAULT_TEXT_STYLE.font_style;
                    text_style.font_weight = DEFAULT_TEXT_STYLE.font_weight;
                    text_style.font_families = DEFAULT_TEXT_STYLE.font_families.clone();
                    text_flow_style.font_size = DEFAULT_TEXT_FLOW_STYLE.font_size;
                    text_flow_style.line_height = DEFAULT_TEXT_FLOW_STYLE.line_height.clone();
                    return Some(());
                }
                _ => return None,
            };

            if values.is_empty() {
                return None;
            }

            // font-size (first <length>) is required.
            let font_size_idx = values
                .iter()
                .position(|v| matches!(v, CssValue::Length(_, _)))?;

            // Resolve font-size.
            let len = resolve_css_len(name, values[font_size_idx], text_flow_style)?;
            let px = resolve_font_size_px(&len, text_flow_style.font_size)?;
            text_flow_style.font_size = px;

            // Everything before font-size is font-style / font-weight keywords.
            for v in &values[..font_size_idx] {
                if let CssValue::Keyword(kw) = v {
                    match kw.as_str() {
                        "italic" => text_style.font_style = FontStyle::Italic,
                        "oblique" => text_style.font_style = FontStyle::Oblique,
                        "normal" => text_style.font_style = FontStyle::Normal,
                        "bold" => text_style.font_weight = FontWeight::BOLD,
                        "bolder" | "lighter" | "small-caps" => {} // simplified: skip
                        s if s.len() <= 3 && s.bytes().all(|b| b.is_ascii_digit()) => {
                            if let Ok(w) = s.parse::<u16>()
                                && (100..=900).contains(&w)
                            {
                                text_style.font_weight = FontWeight(w);
                            }
                        }
                        _ => {}
                    }
                } else if let CssValue::Number(n) = v {
                    if *n >= 100.0 && *n <= 900.0 && n.fract().abs() < f32::EPSILON {
                        text_style.font_weight = FontWeight(*n as u16);
                    }
                }
            }

            // After font-size: optional `/` line-height, then font-family.
            let after = &values[font_size_idx + 1..];
            let family_start = if after
                .first()
                .is_some_and(|v| matches!(v, CssValue::Keyword(k) if k == "/"))
            {
                if after.len() > 1 {
                    match after[1] {
                        CssValue::Number(factor) => {
                            text_flow_style.line_height = LineHeight::Number(*factor);
                        }
                        _ => {
                            if let Some(lh_len) = resolve_css_len(name, after[1], text_flow_style) {
                                text_flow_style.line_height = LineHeight::Px(length_to_px(
                                    &lh_len,
                                    text_flow_style.font_size,
                                ));
                            }
                        }
                    }
                    2 // skip `/` and line-height value
                } else {
                    1 // skip bare `/`
                }
            } else {
                0
            };

            // Rest is font-family.
            let family_values = &values[font_size_idx + 1 + family_start..];
            if !family_values.is_empty() {
                let families: Vec<String> = family_values
                    .iter()
                    .filter_map(|v| match v {
                        CssValue::Keyword(k) if !k.is_empty() => Some(k.to_string()),
                        CssValue::String(s) if !s.is_empty() => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                if !families.is_empty() {
                    text_style.font_families = families;
                }
            }

            return Some(());
        }

        ("text-decoration", v) => {
            let items: Vec<&CssValue> = match v {
                CssValue::List(list) => list.iter().collect(),
                _ => vec![v],
            };
            for item in items {
                match item {
                    CssValue::Keyword(k) => match k.as_str() {
                        "none" => text_style.text_decoration = TextDecoration::None,
                        "underline" => text_style.text_decoration = TextDecoration::Underline,
                        "line-through" => text_style.text_decoration = TextDecoration::LineThrough,
                        "overline" => text_style.text_decoration = TextDecoration::Overline,
                        _ => {}
                    },
                    _ => {
                        if let Some(c) = resolve_css_color(name, item, color_scheme) {
                            text_style.text_decoration_color = Some(c);
                        }
                    }
                }
            }
        }

        ("text-decoration-color", _) => {
            if let Some(c) = resolve_css_color(name, value, color_scheme) {
                text_style.text_decoration_color = Some(c);
            }
        }

        ("vertical-align", _) => {
            apply_property!(
                vertical_align,
                text_flow_style,
                parent_text_flow_style,
                DEFAULT_TEXT_FLOW_STYLE,
                value,
                CssValue::Keyword(v),
                match v.as_str() {
                    "sub" => VerticalAlign::Sub,
                    "super" | "sup" => VerticalAlign::Super,
                    _ => text_flow_style.vertical_align,
                }
            );
        }

        ("text-transform", _) => {
            apply_property!(
                text_transform,
                text_style,
                parent_text_style,
                DEFAULT_TEXT_STYLE,
                value,
                CssValue::Keyword(v),
                match v.as_str() {
                    "none" => TextTransform::None,
                    "uppercase" => TextTransform::Uppercase,
                    "lowercase" => TextTransform::Lowercase,
                    _ => TextTransform::None,
                }
            );
        }

        ("text-align", _) => {
            apply_property!(
                text_align,
                text_flow_style,
                parent_text_flow_style,
                DEFAULT_TEXT_FLOW_STYLE,
                value,
                CssValue::Keyword(v),
                match v.as_str() {
                    "left" => TextAlign::Left,
                    "center" => TextAlign::Center,
                    "right" => TextAlign::Right,
                    _ => return None,
                }
            );
        }

        ("white-space", _) => {
            apply_property!(
                white_space,
                text_flow_style,
                parent_text_flow_style,
                DEFAULT_TEXT_FLOW_STYLE,
                value,
                CssValue::Keyword(v),
                match v.as_str() {
                    "normal" => WhiteSpace::Normal,
                    "nowrap" => WhiteSpace::Nowrap,
                    "pre" => WhiteSpace::Pre,
                    "pre-wrap" => WhiteSpace::PreWrap,
                    "pre-line" => WhiteSpace::PreLine,
                    "break-spaces" => WhiteSpace::BreakSpaces,
                    _ => text_flow_style.white_space,
                }
            );
        }

        /* ======================
         * Box Model
         * ====================== */
        ("box-sizing", _) => {
            apply_property!(
                box_sizing,
                style,
                parent_style,
                DEFAULT_STYLE,
                value,
                CssValue::Keyword(v),
                match v.as_str() {
                    "content-box" => BoxSizing::ContentBox,
                    "border-box" => BoxSizing::BorderBox,
                    _ => BoxSizing::ContentBox,
                }
            );
        }

        ("margin", v) => {
            expand_box(
                name,
                v,
                text_flow_style,
                &|_, cv, tfs| match cv {
                    CssValue::Keyword(s) if s == "auto" => Some(ui_layout::LengthOrAuto::Auto),
                    _ => resolve_css_len(name, cv, tfs).map(ui_layout::LengthOrAuto::Length),
                },
                |t, r, b, l| {
                    style.spacing.margin_top = t;
                    style.spacing.margin_right = r;
                    style.spacing.margin_bottom = b;
                    style.spacing.margin_left = l;
                },
            )?;
        }
        ("margin-top", _) => {
            style.spacing.margin_top = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-right", _) => {
            style.spacing.margin_right = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-bottom", _) => {
            style.spacing.margin_bottom = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-left", _) => {
            style.spacing.margin_left = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-inline", _) => {
            let values = one_or_two_values(value)?;
            style.spacing.margin_left = resolve_css_len_auto(name, values.0, text_flow_style)?;
            style.spacing.margin_right = resolve_css_len_auto(name, values.1, text_flow_style)?;
        }
        ("margin-inline-start", _) => {
            style.spacing.margin_left = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("margin-inline-end", _) => {
            style.spacing.margin_right = resolve_css_len_auto(name, value, text_flow_style)?;
        }

        ("border", v) => {
            let (maybe_width, maybe_style, maybe_color) = if let CssValue::Keyword(k) = v
                && (k.eq_ignore_ascii_case("inset") || k.eq_ignore_ascii_case("initial"))
            {
                (Some(Length::Px(0.0)), None, None)
            } else {
                parse_border_shorthand(name, v, text_flow_style, color_scheme)?
            };

            if let Some(w) = maybe_width {
                style.spacing.border_top = w.clone();
                style.spacing.border_right = w.clone();
                style.spacing.border_bottom = w.clone();
                style.spacing.border_left = w;
            }

            if let Some(s) = maybe_style {
                container_style.border_style.top = s;
                container_style.border_style.right = s;
                container_style.border_style.bottom = s;
                container_style.border_style.left = s;
            }

            if let Some(c) = maybe_color {
                container_style.border_color.top = c;
                container_style.border_color.right = c;
                container_style.border_color.bottom = c;
                container_style.border_color.left = c;
            }
        }
        ("border-top", _) => {
            let (maybe_width, maybe_style, maybe_color) =
                parse_border_shorthand(name, value, text_flow_style, color_scheme)?;
            if let Some(w) = maybe_width {
                style.spacing.border_top = w;
            }
            if let Some(s) = maybe_style {
                container_style.border_style.top = s;
            }
            if let Some(c) = maybe_color {
                container_style.border_color.top = c;
            }
        }
        ("border-right", _) => {
            let (maybe_width, maybe_style, maybe_color) =
                parse_border_shorthand(name, value, text_flow_style, color_scheme)?;
            if let Some(w) = maybe_width {
                style.spacing.border_right = w;
            }
            if let Some(s) = maybe_style {
                container_style.border_style.right = s;
            }
            if let Some(c) = maybe_color {
                container_style.border_color.right = c;
            }
        }
        ("border-bottom", _) => {
            let (maybe_width, maybe_style, maybe_color) =
                parse_border_shorthand(name, value, text_flow_style, color_scheme)?;
            if let Some(w) = maybe_width {
                style.spacing.border_bottom = w;
            }
            if let Some(s) = maybe_style {
                container_style.border_style.bottom = s;
            }
            if let Some(c) = maybe_color {
                container_style.border_color.bottom = c;
            }
        }
        ("border-left", _) => {
            let (maybe_width, maybe_style, maybe_color) =
                parse_border_shorthand(name, value, text_flow_style, color_scheme)?;
            if let Some(w) = maybe_width {
                style.spacing.border_left = w;
            }
            if let Some(s) = maybe_style {
                container_style.border_style.left = s;
            }
            if let Some(c) = maybe_color {
                container_style.border_color.left = c;
            }
        }

        ("border-radius", v) => {
            let (tl, tr, br, bl) = parse_border_radius_shorthand(name, v, text_flow_style)?;
            container_style.border_radius = BorderRadius {
                top_left: tl,
                top_right: tr,
                bottom_right: br,
                bottom_left: bl,
            };
        }
        ("border-top-left-radius", v) => {
            container_style.border_radius.top_left = parse_corner_radius(name, v, text_flow_style)?;
        }
        ("border-top-right-radius", v) => {
            container_style.border_radius.top_right =
                parse_corner_radius(name, v, text_flow_style)?;
        }
        ("border-bottom-right-radius", v) => {
            container_style.border_radius.bottom_right =
                parse_corner_radius(name, v, text_flow_style)?;
        }
        ("border-bottom-left-radius", v) => {
            container_style.border_radius.bottom_left =
                parse_corner_radius(name, v, text_flow_style)?;
        }

        ("border-style", v) => {
            expand_box(
                name,
                v,
                text_flow_style,
                &|_, cv, _| {
                    let style = match cv {
                        CssValue::Keyword(v) => match v.as_str() {
                            "none" => BorderStyle::None,
                            "solid" => BorderStyle::Solid,
                            "dashed" => BorderStyle::Dashed,
                            "dotted" => BorderStyle::Dotted,
                            _ => return None,
                        },
                        _ => return None,
                    };

                    Some(style)
                },
                |t, r, b, l| {
                    container_style.border_style.top = t;
                    container_style.border_style.right = r;
                    container_style.border_style.bottom = b;
                    container_style.border_style.left = l;
                },
            )?;
        }

        ("border-color", v) => {
            expand_box(
                name,
                v,
                text_flow_style,
                &|_, cv, _| resolve_css_color(name, cv, color_scheme),
                |t, r, b, l| {
                    container_style.border_color.top = t;
                    container_style.border_color.right = r;
                    container_style.border_color.bottom = b;
                    container_style.border_color.left = l;
                },
            )?;
        }
        ("border-top-color", _) => {
            container_style.border_color.top = resolve_css_color(name, value, color_scheme)?;
        }
        ("border-right-color", _) => {
            container_style.border_color.right = resolve_css_color(name, value, color_scheme)?;
        }
        ("border-bottom-color", _) => {
            container_style.border_color.bottom = resolve_css_color(name, value, color_scheme)?;
        }
        ("border-left-color", _) => {
            container_style.border_color.left = resolve_css_color(name, value, color_scheme)?;
        }

        ("border-width", v) => {
            expand_box(
                name,
                v,
                text_flow_style,
                &|_, cv, ts| match cv {
                    CssValue::Keyword(s) => match s.as_str() {
                        "thin" => Some(Length::Px(1.0)),
                        "medium" => Some(Length::Px(3.0)),
                        "thick" => Some(Length::Px(5.0)),
                        _ => resolve_css_len(name, cv, ts),
                    },
                    _ => resolve_css_len(name, cv, ts),
                },
                |t, r, b, l| {
                    style.spacing.border_top = t;
                    style.spacing.border_right = r;
                    style.spacing.border_bottom = b;
                    style.spacing.border_left = l;
                },
            )?;
        }
        ("border-top-width", _) => {
            style.spacing.border_top = match value {
                CssValue::Keyword(s) => match s.as_str() {
                    "thin" => Length::Px(1.0),
                    "medium" => Length::Px(3.0),
                    "thick" => Length::Px(5.0),
                    _ => resolve_css_len(name, value, text_flow_style)?,
                },
                _ => resolve_css_len(name, value, text_flow_style)?,
            };
        }
        ("border-right-width", _) => {
            style.spacing.border_right = match value {
                CssValue::Keyword(s) => match s.as_str() {
                    "thin" => Length::Px(1.0),
                    "medium" => Length::Px(3.0),
                    "thick" => Length::Px(5.0),
                    _ => resolve_css_len(name, value, text_flow_style)?,
                },
                _ => resolve_css_len(name, value, text_flow_style)?,
            };
        }
        ("border-bottom-width", _) => {
            style.spacing.border_bottom = match value {
                CssValue::Keyword(s) => match s.as_str() {
                    "thin" => Length::Px(1.0),
                    "medium" => Length::Px(3.0),
                    "thick" => Length::Px(5.0),
                    _ => resolve_css_len(name, value, text_flow_style)?,
                },
                _ => resolve_css_len(name, value, text_flow_style)?,
            };
        }
        ("border-left-width", _) => {
            style.spacing.border_left = match value {
                CssValue::Keyword(s) => match s.as_str() {
                    "thin" => Length::Px(1.0),
                    "medium" => Length::Px(3.0),
                    "thick" => Length::Px(5.0),
                    _ => resolve_css_len(name, value, text_flow_style)?,
                },
                _ => resolve_css_len(name, value, text_flow_style)?,
            };
        }

        ("padding", v) => {
            expand_box(
                name,
                v,
                text_flow_style,
                &|_, v, ts| resolve_css_len(name, v, ts),
                |t, r, b, l| {
                    style.spacing.padding_top = t;
                    style.spacing.padding_right = r;
                    style.spacing.padding_bottom = b;
                    style.spacing.padding_left = l;
                },
            )?;
        }
        ("padding-top", _) => {
            style.spacing.padding_top = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-right", _) => {
            style.spacing.padding_right = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-bottom", _) => {
            style.spacing.padding_bottom = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-left", _) => {
            style.spacing.padding_left = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-inline", _) => {
            let values = one_or_two_values(value)?;
            style.spacing.padding_left = resolve_css_len(name, values.0, text_flow_style)?;
            style.spacing.padding_right = resolve_css_len(name, values.1, text_flow_style)?;
        }
        ("padding-inline-start", _) => {
            style.spacing.padding_left = resolve_css_len(name, value, text_flow_style)?;
        }
        ("padding-inline-end", _) => {
            style.spacing.padding_right = resolve_css_len(name, value, text_flow_style)?;
        }

        /* ======================
         * Size
         * ====================== */
        ("width", _) => {
            style.size.width = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("height", _) => {
            style.size.height = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("min-width", _) => {
            style.size.min_width = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("min-height", _) => {
            style.size.min_height = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("max-width", _) => {
            style.size.max_width = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("max-height", _) => {
            style.size.max_height = resolve_css_len_auto(name, value, text_flow_style)?;
        }

        /* ======================
         * Position
         * ====================== */
        ("position", _) => {
            style.position.kind = match value {
                CssValue::Keyword(v) => match v.to_ascii_lowercase().as_str() {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    "sticky" => Position::Sticky,
                    _ => return None,
                },
                _ => return None,
            };
        }
        ("top", _) => {
            style.position.top = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("right", _) => {
            style.position.right = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("bottom", _) => {
            style.position.bottom = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("left", _) => {
            style.position.left = resolve_css_len_auto(name, value, text_flow_style)?;
        }
        ("inset", v) => {
            expand_box(
                name,
                v,
                text_flow_style,
                &|_, cv, ts| match cv {
                    CssValue::Keyword(s) if s.eq_ignore_ascii_case("auto") => {
                        Some(ui_layout::LengthOrAuto::Auto)
                    }
                    _ => resolve_css_len(name, cv, ts).map(ui_layout::LengthOrAuto::Length),
                },
                |t, r, b, l| {
                    style.position.top = t;
                    style.position.right = r;
                    style.position.bottom = b;
                    style.position.left = l;
                },
            )?;
        }

        /* ======================
         * Overflow
         * ====================== */
        ("overflow", _) => {
            let (x, y) = overflow_flags(value)?;
            overflow.x = x;
            overflow.y = y;
        }
        ("overflow-x", CssValue::Keyword(k)) => {
            overflow.x = overflow_scrollable(k)?;
        }
        ("overflow-y", CssValue::Keyword(k)) => {
            overflow.y = overflow_scrollable(k)?;
        }

        /* ======================
         * Flex
         * ====================== */
        ("flex-direction", _) => {
            apply_property!(
                flex_direction,
                style,
                parent_style,
                DEFAULT_STYLE,
                value,
                CssValue::Keyword(v),
                flex_direction_keyword(v)?
            );
        }

        ("flex-wrap", _) => {
            apply_property!(
                flex_wrap,
                style,
                parent_style,
                DEFAULT_STYLE,
                value,
                CssValue::Keyword(v),
                flex_wrap_keyword(v)?
            );
        }

        ("flex-flow", _) => {
            let (direction, wrap) = resolve_flex_flow(value)?;
            style.flex_direction = direction;
            style.flex_wrap = wrap;
        }

        ("place-items", value) => {
            let values = match value {
                CssValue::Keyword(_) => std::slice::from_ref(value),
                CssValue::List(values) if (1..=2).contains(&values.len()) => values.as_slice(),
                _ => return None,
            };

            let align_items = match &values[0] {
                CssValue::Keyword(v) => resolve_align_items(v)?,
                _ => return None,
            };

            let justify_items = match values.get(1).unwrap_or(&values[0]) {
                CssValue::Keyword(v) => resolve_justify_items(v)?,
                _ => return None,
            };

            style.align_items = align_items;
            style.justify_items = justify_items;
        }

        ("justify-items", _) => {
            apply_property!(
                justify_items,
                style,
                parent_style,
                DEFAULT_STYLE,
                value,
                CssValue::Keyword(v),
                resolve_justify_items(v)?
            );
        }

        ("justify-content", _) => {
            apply_property!(
                justify_content,
                style,
                parent_style,
                DEFAULT_STYLE,
                value,
                CssValue::Keyword(v),
                match v.as_str() {
                    "flex-start" | "start" => JustifyContent::Start,
                    "center" => JustifyContent::Center,
                    "flex-end" | "end" => JustifyContent::End,
                    "space-between" => JustifyContent::SpaceBetween,
                    "space-around" => JustifyContent::SpaceAround,
                    "space-evenly" => JustifyContent::SpaceEvenly,
                    _ => return None,
                }
            );
        }

        ("align-items", _) => {
            apply_property!(
                align_items,
                style,
                parent_style,
                DEFAULT_STYLE,
                value,
                CssValue::Keyword(v),
                resolve_align_items(v)?
            );
        }

        ("align-content", _) => {
            apply_property!(
                align_content,
                style,
                parent_style,
                DEFAULT_STYLE,
                value,
                CssValue::Keyword(v),
                match v.as_str() {
                    "normal" | "stretch" => AlignContent::Stretch,
                    "flex-start" | "start" => AlignContent::Start,
                    "center" => AlignContent::Center,
                    "flex-end" | "end" => AlignContent::End,
                    "space-between" => AlignContent::SpaceBetween,
                    "space-around" => AlignContent::SpaceAround,
                    "space-evenly" => AlignContent::SpaceEvenly,
                    _ => return None,
                }
            );
        }

        ("gap", _) => match value {
            CssValue::List(l) if l.len() == 2 => {
                let mut l = l.iter();
                let gap = resolve_css_len_auto(name, l.next()?, text_flow_style)?;
                style.row_gap = gap;
                let gap = resolve_css_len_auto(name, l.next()?, text_flow_style)?;
                style.column_gap = gap;
            }
            CssValue::Length(_, _) => {
                let gap = resolve_css_len_auto(name, value, text_flow_style)?;
                style.row_gap = gap.clone();
                style.column_gap = gap;
            }
            _ => {}
        },

        ("flex", _) => {
            let (grow, shrink, basis) = resolve_flex_shorthand(value, text_flow_style)?;
            style.item_style.flex_grow = grow;
            style.item_style.flex_shrink = shrink;
            style.item_style.flex_basis = basis;
        }

        ("flex-grow", CssValue::Number(v)) => {
            if *v < 0.0 {
                return None;
            }
            style.item_style.flex_grow = *v;
        }

        ("flex-basis", _) => {
            style.item_style.flex_basis = resolve_css_len_auto(name, value, text_flow_style)?;
        }

        ("flex-shrink", CssValue::Number(v)) => {
            if *v < 0.0 {
                return None;
            }
            style.item_style.flex_shrink = *v;
        }

        ("place-self", value) => {
            let values = match value {
                CssValue::Keyword(_) => std::slice::from_ref(value),
                CssValue::List(values) if (1..=2).contains(&values.len()) => values.as_slice(),
                _ => return None,
            };

            let align_items = match &values[0] {
                CssValue::Keyword(v) => resolve_align_items(v)?,
                _ => return None,
            };

            let justify_items = match values.get(1).unwrap_or(&values[0]) {
                CssValue::Keyword(v) => resolve_justify_items(v)?,
                _ => return None,
            };

            style.item_style.align_self = Some(align_items);
            style.item_style.justify_self = Some(justify_items);
        }

        ("align-self", CssValue::Keyword(v)) => {
            style.item_style.align_self = Some(match v.as_str() {
                "stretch" => AlignItems::Stretch,
                "flex-start" | "start" => AlignItems::Start,
                "center" => AlignItems::Center,
                "flex-end" | "end" => AlignItems::End,
                "auto" => return Some(()),
                _ => return None,
            });
        }

        ("justify-self", CssValue::Keyword(v)) => {
            style.item_style.justify_self = Some(match v.as_str() {
                "stretch" => JustifyItems::Stretch,
                "flex-start" | "start" => JustifyItems::Start,
                "center" => JustifyItems::Center,
                "flex-end" | "end" => JustifyItems::End,
                "auto" => return Some(()),
                _ => return None,
            });
        }

        ("column-gap", _) => {
            style.column_gap = resolve_css_len_auto(name, value, text_flow_style)?;
        }

        ("row-gap", _) => {
            style.row_gap = resolve_css_len_auto(name, value, text_flow_style)?;
        }

        /* ======================
         * Grid
         * ====================== */
        ("grid-template-columns", _) => {
            style.grid_template_columns = parse_grid_tracks(name, value, text_flow_style)?;
        }

        ("grid-template-rows", _) => {
            style.grid_template_rows = parse_grid_tracks(name, value, text_flow_style)?;
        }

        ("grid-template-areas", _) => {
            style.grid_template_areas = parse_grid_template_areas(value)?;
        }

        ("grid-area", CssValue::Keyword(area)) => {
            style.grid_area = Some(area.to_string());
        }

        ("grid-column", _) => {
            style.grid_column = parse_grid_placement(value)?;
        }

        ("grid-row", _) => {
            style.grid_row = parse_grid_placement(value)?;
        }

        ("grid-column-start", _) => {
            style.grid_column.start = Some(parse_grid_line(value)?);
        }

        ("grid-column-end", _) => {
            style.grid_column.end = parse_grid_line_end(value)?;
        }

        ("grid-row-start", _) => {
            style.grid_row.start = Some(parse_grid_line(value)?);
        }

        ("grid-row-end", _) => {
            style.grid_row.end = parse_grid_line_end(value)?;
        }

        _ => {
            /*
            if !name.starts_with('-') {
                log::error!("{name}, {value:?}");
            }
            */
            return None;
        }
    }
    Some(())
}
