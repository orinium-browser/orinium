use crate::engine::css::values::{CssValue, Unit};
use crate::engine::layouter::types::TextFlowStyle;
use ui_layout::{GridPlacement, GridPlacementEnd, GridRepeat, GridTrack, Length, LengthOrAuto};

/// Extract font family names from a `font-family` CSS value.
///
/// Accepts a single keyword/string or a comma-separated list.
pub fn extract_font_families(value: &CssValue) -> Vec<String> {
    let items: Vec<&CssValue> = match value {
        CssValue::List(list) => list.iter().collect(),
        other => vec![other],
    };

    let mut families = Vec::new();
    for item in items {
        let name = match item {
            CssValue::Keyword(k) => k.to_string(),
            CssValue::String(s) => s.clone(),
            _ => continue,
        };
        if !name.is_empty() {
            families.push(name);
        }
    }
    families
}

/// Resolve CssValue to LengthOrAuto.
pub fn resolve_css_len_auto(
    name: &str,
    css_len: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<LengthOrAuto> {
    match &css_len {
        CssValue::Keyword(s) if s == "auto" => Some(LengthOrAuto::Auto),
        _ => {
            resolve_css_len(name, std::slice::from_ref(css_len), text_flow_style).map(|l| l.into())
        }
    }
}

pub fn one_or_two_values(value: &CssValue) -> Option<(&CssValue, &CssValue)> {
    match value {
        CssValue::List(values) if values.len() == 2 => Some((&values[0], &values[1])),
        CssValue::List(values) if values.len() == 1 => Some((&values[0], &values[0])),
        CssValue::List(_) => None,
        value => Some((value, value)),
    }
}

pub fn resolve_font_size_px(length: &Length, inherited_size: f32) -> Option<f32> {
    match length {
        Length::Px(value) => Some(*value),
        Length::Percent(value) => Some(*value * inherited_size / 100.0),
        Length::Clamp { min, val, max } => {
            let minimum = resolve_font_size_px(min, inherited_size)?;
            let maximum = resolve_font_size_px(max, inherited_size)?;
            let preferred = resolve_font_size_px(val, inherited_size).unwrap_or(maximum);
            Some(preferred.clamp(minimum, maximum))
        }
        Length::Min(left, right) => Some(
            resolve_font_size_px(left, inherited_size)?
                .min(resolve_font_size_px(right, inherited_size)?),
        ),
        Length::Max(left, right) => Some(
            resolve_font_size_px(left, inherited_size)?
                .max(resolve_font_size_px(right, inherited_size)?),
        ),
        Length::Add(left, right) => Some(
            resolve_font_size_px(left, inherited_size)?
                + resolve_font_size_px(right, inherited_size)?,
        ),
        Length::Sub(left, right) => Some(
            resolve_font_size_px(left, inherited_size)?
                - resolve_font_size_px(right, inherited_size)?,
        ),
        Length::Mul(value, factor) => Some(resolve_font_size_px(value, inherited_size)? * factor),
        Length::Div(value, factor) if *factor != 0.0 => {
            Some(resolve_font_size_px(value, inherited_size)? / factor)
        }
        Length::Vw(_) | Length::Vh(_) | Length::Div(_, _) => None,
    }
}

/// calc() の評価結果。型 (number / length) を保持する。
#[derive(Debug, Clone, PartialEq)]
enum CalcValue {
    Number(f32),
    Length(Length),
}

fn resolve_length(
    name: &str,
    v: f32,
    unit: &Unit,
    text_flow_style: &TextFlowStyle,
) -> Option<Length> {
    match unit {
        Unit::Px => Some(Length::Px(v)),
        Unit::Em => Some(Length::Px(text_flow_style.font_size * v)),
        Unit::Rem => Some(Length::Px(16.0 * v)), // Stub
        Unit::Percent => Some(Length::Percent(v)),
        Unit::Vw => Some(Length::Vw(v)),
        Unit::Vh => Some(Length::Vh(v)),

        // TODO: Resolve physical units using the device DPI.
        Unit::Cm | Unit::Mm | Unit::In | Unit::Pt | Unit::Pc => {
            log::warn!(
                target: "Layouter",
                "TODO: Resolve physical length unit: {}", unit.as_str());
            None
        }

        // TODO: Resolve against the viewport dimensions.
        Unit::Vmin | Unit::Vmax => {
            log::warn!(
                target: "Layouter",
                "TODO: Resolve viewport length unit: {}", unit.as_str());
            None
        }

        Unit::Deg => {
            log::error!(
                target: "Layouter",
                "Unexpected deg unit for `{}` (expected length)",
                name
            );
            None
        }
        Unit::Fr => {
            log::error!(
                target: "Layouter",
                "Unexpected fr unit for `{}` (expected length)",
                name
            );
            None
        }
    }
}

fn calc_combine(name: &str, op: &CssValue, left: CalcValue, right: CalcValue) -> Option<CalcValue> {
    match op {
        CssValue::Keyword(o) if o == "+" => match (left, right) {
            (CalcValue::Number(x), CalcValue::Number(y)) => Some(CalcValue::Number(x + y)),
            (CalcValue::Length(x), CalcValue::Length(y)) => {
                Some(CalcValue::Length(Length::Add(Box::new(x), Box::new(y))))
            }
            _ => {
                log::error!(
                    target: "Layouter",
                    "Cannot add number and length in calc() for `{}`",
                    name
                );
                None
            }
        },
        CssValue::Keyword(o) if o == "-" => match (left, right) {
            (CalcValue::Number(x), CalcValue::Number(y)) => Some(CalcValue::Number(x - y)),
            (CalcValue::Length(x), CalcValue::Length(y)) => {
                Some(CalcValue::Length(Length::Sub(Box::new(x), Box::new(y))))
            }
            _ => {
                log::error!(
                    target: "Layouter",
                    "Cannot subtract number and length in calc() for `{}`",
                    name
                );
                None
            }
        },
        CssValue::Keyword(o) if o == "*" => match (left, right) {
            (CalcValue::Number(x), CalcValue::Number(y)) => Some(CalcValue::Number(x * y)),
            (CalcValue::Number(x), CalcValue::Length(y)) => {
                Some(CalcValue::Length(Length::Mul(Box::new(y), x)))
            }
            (CalcValue::Length(x), CalcValue::Number(y)) => {
                Some(CalcValue::Length(Length::Mul(Box::new(x), y)))
            }
            _ => {
                log::error!(
                    target: "Layouter",
                    "Cannot multiply two lengths in calc() for `{}`",
                    name
                );
                None
            }
        },
        CssValue::Keyword(o) if o == "/" => match (left, right) {
            (CalcValue::Number(_), CalcValue::Number(0.0))
            | (CalcValue::Length(_), CalcValue::Number(0.0)) => {
                log::error!(
                    target: "Layouter",
                    "Division by zero in calc() for `{}`",
                    name
                );
                None
            }
            (CalcValue::Number(x), CalcValue::Number(y)) => Some(CalcValue::Number(x / y)),
            (CalcValue::Length(x), CalcValue::Number(y)) => {
                Some(CalcValue::Length(Length::Div(Box::new(x), y)))
            }
            _ => {
                log::error!(
                    target: "Layouter",
                    "Cannot divide by length in calc() for `{}`",
                    name
                );
                None
            }
        },
        _ => {
            log::error!(target: "Layouter", "Unknown operator in calc() for `{}`: {:?}{:?}{:?}", name, left, op, right);
            None
        }
    }
}

/// CssValue を number / length の型情報付きで評価する。
fn resolve_calc_value(
    name: &str,
    value: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<CalcValue> {
    match value {
        CssValue::List(v) => {
            if v.len() == 1 {
                resolve_calc_value(name, &v[0], text_flow_style)
            } else {
                resolve_calc_value_slice(name, v, text_flow_style)
            }
        }
        CssValue::Length(v, unit) => {
            resolve_length(name, *v, unit, text_flow_style).map(CalcValue::Length)
        }
        CssValue::Number(n) => Some(CalcValue::Number(*n)),
        CssValue::Function(fn_name, args) if fn_name == "calc" => {
            if args.len() == 1 {
                let list = args[0].clone();
                resolve_calc_value(name, &CssValue::List(list), text_flow_style)
            } else {
                // Syntax error
                None
            }
        }
        CssValue::Function(fn_name, args)
            if (fn_name == "min" || fn_name == "max") && args.len() >= 2 =>
        {
            let mut resolved: Vec<Length> = Vec::with_capacity(args.len());
            for arg in args.iter().flatten() {
                resolved.push(match resolve_calc_value(name, arg, text_flow_style)? {
                    CalcValue::Length(l) => l,
                    CalcValue::Number(0.0) => Length::Px(0.0),
                    CalcValue::Number(n) => {
                        log::error!(
                            target: "Layouter",
                            "Invalid operand for {fn_name}() in `{}` (expected length): {}",
                            name,
                            n
                        );
                        return None;
                    }
                });
            }
            let mut result = resolved.remove(resolved.len() - 1);
            for arg in resolved.into_iter().rev() {
                result = if fn_name == "min" {
                    Length::Min(Box::new(arg), Box::new(result))
                } else {
                    Length::Max(Box::new(arg), Box::new(result))
                };
            }
            Some(CalcValue::Length(result))
        }
        CssValue::Function(fn_name, args) if fn_name == "clamp" && args.len() == 3 => {
            let min = resolve_css_len(
                name,
                std::slice::from_ref(args[0].first()?),
                text_flow_style,
            )?;
            let val = resolve_css_len(
                name,
                std::slice::from_ref(args[1].first()?),
                text_flow_style,
            )?;
            let max = resolve_css_len(
                name,
                std::slice::from_ref(args[2].first()?),
                text_flow_style,
            )?;
            Some(CalcValue::Length(Length::Clamp {
                min: Box::new(min),
                val: Box::new(val),
                max: Box::new(max),
            }))
        }
        _ => {
            log::error!(
                target: "Layouter",
                "Invalid lenth value for `{}`: {:?}",
                name,
                value
            );
            None
        }
    }
}

/// Resolve a slice of CSS component values to a Length.
///
/// Accepts a component list and supports several forms:
/// - A single primitive (`&[10px]`)
/// - A single length-valued function (`&[calc(...)]`, `&[min(...)]`, ...)
/// - A flat arithmetic expression (`&[a, +, b]`)
///
/// Arithmetic over multiple components is resolved using the same type-checked
/// rules as `calc()`.
pub fn resolve_css_len(
    name: &str,
    components: &[CssValue],
    text_flow_style: &TextFlowStyle,
) -> Option<Length> {
    if components.len() > 1 {
        return calc_value_to_length(
            name,
            resolve_calc_value_slice(name, components, text_flow_style)?,
        );
    }

    match components.first() {
        Some(CssValue::List(v)) => {
            if v.len() > 1 {
                calc_value_to_length(name, resolve_calc_value_slice(name, v, text_flow_style)?)
            } else {
                resolve_css_len(name, v, text_flow_style)
            }
        }
        Some(CssValue::Length(v, unit)) => resolve_length(name, *v, unit, text_flow_style),
        Some(CssValue::Number(0.0)) => Some(Length::Px(0.0)),
        Some(CssValue::Keyword(_)) => None,
        Some(CssValue::Color(_)) => None,
        Some(CssValue::Function(_, _)) => calc_value_to_length(
            name,
            resolve_calc_value(name, components.first()?, text_flow_style)?,
        ),
        None => None,
        _ => {
            log::error!(
                target: "Layouter",
                "Unknown CSS Length type for `{}`: {:?}",
                name,
                components
            );
            None
        }
    }
}

/// Convert a resolved `CalcValue` into a `Length`, rejecting bare numbers.
fn calc_value_to_length(name: &str, value: CalcValue) -> Option<Length> {
    match value {
        CalcValue::Length(l) => Some(l),
        CalcValue::Number(0.0) => Some(Length::Px(0.0)),
        CalcValue::Number(n) => {
            log::error!(
                target: "Layouter",
                "calc() resolved to a number ({}) for `{}` (expected length)",
                n,
                name
            );
            None
        }
    }
}

/// Resolve a sequence of `[value, operator, value, ...]` into a `CalcValue`,
/// applying the same type-checked arithmetic as `calc()`.
fn resolve_calc_value_slice(
    name: &str,
    components: &[CssValue],
    text_flow_style: &TextFlowStyle,
) -> Option<CalcValue> {
    fn resolve_product<'a>(
        name: &str,
        iter: &mut std::iter::Peekable<impl Iterator<Item = &'a CssValue>>,
        text_flow_style: &TextFlowStyle,
    ) -> Option<CalcValue> {
        let first = iter.next()?;
        let mut result = resolve_calc_value(name, first, text_flow_style)?;

        loop {
            let op = match iter.peek() {
                Some(CssValue::Keyword(value)) if value == "*" || value == "/" => iter.next()?,
                _ => break,
            };

            let value = iter.next()?;
            let rhs = resolve_calc_value(name, value, text_flow_style)?;

            result = calc_combine(name, op, result, rhs)?;
        }

        Some(result)
    }

    let mut iter = components.iter().peekable();
    let mut result = resolve_product(name, &mut iter, text_flow_style)?;

    loop {
        let op = match iter.peek() {
            Some(CssValue::Keyword(value)) if value == "+" || value == "-" => iter.next()?,
            _ => break,
        };

        let rhs = resolve_product(name, &mut iter, text_flow_style)?;
        result = calc_combine(name, op, result, rhs)?;
    }

    Some(result)
}

pub fn parse_grid_tracks(
    name: &str,
    value: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<Vec<GridTrack>> {
    if matches!(value, CssValue::Keyword(keyword) if keyword == "none") {
        return Some(Vec::new());
    }
    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        value => vec![value],
    };
    values
        .into_iter()
        .map(|value| parse_grid_track(name, value, text_flow_style))
        .collect()
}

pub fn parse_grid_placement(value: &CssValue) -> Option<GridPlacement> {
    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        value => vec![value],
    };
    if matches!(values.as_slice(), [CssValue::Keyword(keyword)] if keyword == "auto") {
        return Some(GridPlacement::default());
    }

    let slash = values
        .iter()
        .position(|value| matches!(value, CssValue::Keyword(keyword) if keyword == "/"));
    let (start_values, end_values) = slash.map_or((values.as_slice(), &[][..]), |slash| {
        (&values[..slash], &values[slash + 1..])
    });
    let start = parse_positive_grid_line(start_values)?;
    if end_values.is_empty() {
        return Some(GridPlacement {
            start: Some(start),
            end: GridPlacementEnd::Span(1),
        });
    }
    if matches!(end_values, [CssValue::Number(end)] if *end < 0.0) {
        let [CssValue::Number(end)] = end_values else {
            unreachable!();
        };
        if end.fract().abs() >= f32::EPSILON {
            return None;
        }

        return Some(GridPlacement {
            start: Some(start),
            end: GridPlacementEnd::NegativeLine((-*end) as usize),
        });
    }
    let end = parse_positive_grid_line(end_values)?;
    (end > start).then_some(GridPlacement {
        start: Some(start),
        end: GridPlacementEnd::Line(end),
    })
}

fn parse_positive_grid_line(values: &[&CssValue]) -> Option<usize> {
    let [CssValue::Number(line)] = values else {
        return None;
    };
    (*line >= 1.0 && line.fract().abs() < f32::EPSILON).then_some(*line as usize)
}

fn parse_grid_track(
    name: &str,
    value: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<GridTrack> {
    match value {
        CssValue::Keyword(keyword) if keyword == "auto" => {
            Some(GridTrack::Breadth(LengthOrAuto::Auto))
        }
        CssValue::Length(factor, Unit::Fr) if *factor >= 0.0 => Some(GridTrack::Flex(*factor)),
        CssValue::Function(function, args) if function == "minmax" => {
            let [minimum, maximum] = args.as_slice() else {
                return None;
            };
            let minimum = minimum.first()?;
            let maximum = maximum.first()?;
            Some(GridTrack::MinMax(
                Box::new(parse_grid_track(name, minimum, text_flow_style)?),
                Box::new(parse_grid_track(name, maximum, text_flow_style)?),
            ))
        }
        CssValue::Function(function, args) if function == "repeat" => {
            let (repeat, pattern) = args.split_first()?;
            let repeat = match repeat.first() {
                Some(CssValue::Number(count))
                    if *count >= 1.0 && count.fract().abs() < f32::EPSILON =>
                {
                    GridRepeat::Count(*count as usize)
                }
                Some(CssValue::Keyword(keyword)) if keyword == "auto-fit" => GridRepeat::AutoFit,
                Some(CssValue::Keyword(keyword)) if keyword == "auto-fill" => GridRepeat::AutoFill,
                _ => return None,
            };
            let pattern = pattern
                .iter()
                .flatten()
                .map(|value| parse_grid_track(name, value, text_flow_style))
                .collect::<Option<Vec<_>>>()?;
            (!pattern.is_empty()).then_some(GridTrack::Repeat(repeat, pattern))
        }
        _ => resolve_css_len(name, std::slice::from_ref(value), text_flow_style)
            .map(LengthOrAuto::Length)
            .map(GridTrack::Breadth),
    }
}

pub fn parse_grid_template_areas(value: &CssValue) -> Option<Vec<Vec<String>>> {
    if matches!(value, CssValue::Keyword(keyword) if keyword == "none") {
        return Some(Vec::new());
    }
    let rows: Vec<&str> = match value {
        CssValue::String(row) => vec![row],
        CssValue::List(values) => values
            .iter()
            .map(|value| match value {
                CssValue::String(row) => Some(row.as_str()),
                _ => None,
            })
            .collect::<Option<_>>()?,
        _ => return None,
    };
    let areas: Vec<Vec<String>> = rows
        .into_iter()
        .map(|row| {
            row.split_whitespace()
                .map(|name| {
                    if name.chars().all(|character| character == '.') {
                        ".".to_string()
                    } else {
                        name.to_string()
                    }
                })
                .collect()
        })
        .collect();
    let width = areas.first()?.len();
    if width == 0 || areas.iter().any(|row| row.len() != width) {
        return None;
    }
    for name in areas.iter().flatten().filter(|name| name.as_str() != ".") {
        let cells: Vec<_> = areas
            .iter()
            .enumerate()
            .flat_map(|(row, areas)| {
                areas
                    .iter()
                    .enumerate()
                    .filter(|(_, area)| *area == name)
                    .map(move |(column, _)| (row, column))
            })
            .collect();
        let min_row = cells.iter().map(|(row, _)| *row).min()?;
        let max_row = cells.iter().map(|(row, _)| *row).max()?;
        let min_column = cells.iter().map(|(_, column)| *column).min()?;
        let max_column = cells.iter().map(|(_, column)| *column).max()?;
        if (min_row..=max_row)
            .any(|row| (min_column..=max_column).any(|column| areas[row][column] != *name))
        {
            return None;
        }
    }
    Some(areas)
}

pub fn parse_grid_line(value: &CssValue) -> Option<usize> {
    match value {
        CssValue::Number(n) if *n >= 1.0 && n.fract() == 0.0 => Some(*n as usize),
        _ => None,
    }
}

pub fn parse_grid_line_end(value: &CssValue) -> Option<GridPlacementEnd> {
    let CssValue::Number(n) = value else {
        return None;
    };

    if n.fract().abs() >= f32::EPSILON {
        return None;
    }

    match *n {
        n if n >= 1.0 => Some(GridPlacementEnd::Line(n as usize)),
        n if n <= -1.0 => Some(GridPlacementEnd::NegativeLine(-n as usize)),
        _ => None,
    }
}
