use super::args::{ArgResolver, CssGroupCursor, ParseError};
use crate::engine::css::values::{CssValue, Unit};
use crate::engine::layouter::types::TextFlowStyle;
use ui_layout::{
    GridPlacement, GridPlacementEnd, GridRepeat, GridTrack, Length, LengthOrAuto, RoundingStrategy,
};

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
        Length::Round {
            strategy,
            value,
            step,
        } => {
            let value_px = resolve_font_size_px(value.as_ref(), inherited_size)?;
            let step_px = resolve_font_size_px(step.as_ref(), inherited_size)?;
            if step_px == 0.0 {
                None
            } else {
                Some(round_to_step(*strategy, value_px, step_px))
            }
        }
    }
}

/// Snaps `value` to a multiple of `step` following the CSS `round()` definition.
///
/// The two multiples of `abs(step)` that bracket `value` are computed directly
/// rather than rounding `value / step`. The two are not equivalent for a
/// negative step: `nearest` ties resolve to the numerically larger multiple, so
/// `round(15px, -10px)` is `20px` and not the `10px` that rounding the quotient
/// would produce.
///
/// This mirrors `ui_layout`'s own resolution of [`Length::Round`] so that
/// values resolved eagerly (font sizes, gradient stop fractions) stay
/// consistent with values the layout pass resolves later.
pub(super) fn round_to_step(strategy: RoundingStrategy, value: f32, step: f32) -> f32 {
    let magnitude = step.abs();
    let lower = (value / magnitude).floor() * magnitude;

    // A value that is already an exact multiple of the step resolves to itself
    // under every strategy, so `up` and `to-zero` must not nudge it off.
    if value == lower {
        return value;
    }

    let upper = lower + magnitude;
    match strategy {
        RoundingStrategy::Nearest => {
            if value - lower < upper - value {
                lower
            } else {
                upper
            }
        }
        RoundingStrategy::Up => upper,
        RoundingStrategy::Down => lower,
        RoundingStrategy::ToZero => {
            if value > 0.0 {
                lower
            } else {
                upper
            }
        }
    }
}

/// calc() の評価結果。型 (number / length) を保持する。
#[derive(Debug, Clone, PartialEq)]
pub enum CalcValue {
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

        // CSS 2.2 §6.4: 1px = 1/96in, so physical units map to fixed pixel values.
        Unit::In => Some(Length::Px(v * 96.0)),
        Unit::Cm => Some(Length::Px(v * 96.0 / 2.54)),
        Unit::Mm => Some(Length::Px(v * 96.0 / 25.4)),
        Unit::Pt => Some(Length::Px(v * 96.0 / 72.0)),
        Unit::Pc => Some(Length::Px(v * 96.0 / 6.0)),

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
        Unit::Unknown => {
            log::warn!(
                target: "Layouter",
                "Unknown CSS unit for `{}`, treating as unsupported", name
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

/// Resolve one comma-separated argument group as an arithmetic expression.
fn resolve_arg_group(
    name: &str,
    group: &[CssValue],
    text_flow_style: &TextFlowStyle,
) -> Option<CalcValue> {
    resolve_calc_value(name, &CssValue::List(group.to_vec()), text_flow_style)
}

impl ArgResolver for CalcValue {
    fn resolve(name: &str, text_flow_style: &TextFlowStyle, group: &[CssValue]) -> Option<Self> {
        resolve_arg_group(name, group, text_flow_style)
    }
}

impl ArgResolver for RoundingStrategy {
    fn resolve(_name: &str, _text_flow_style: &TextFlowStyle, group: &[CssValue]) -> Option<Self> {
        match group {
            [CssValue::Keyword(keyword)] => parse_rounding_strategy(keyword),
            _ => None,
        }
    }
}

/// 数値オペランド群の型統一結果。
///
/// 全オペランドが `<number>` なら [`OperandMode::Numbers`] に、それ以外は
/// [`OperandMode::Lengths`] に統一される (bare `0` は `<length>` として扱う)。
/// number/length の混在や非0の数値はエラーになる。
enum OperandMode {
    Numbers(Vec<f32>),
    Lengths(Vec<Length>),
}

/// Resolve a list of [`CalcValue`]s into a uniform operand mode, applying the
/// "bare `0` counts as a `<length>`" coercion shared by `min()`/`max()`/
/// `clamp()`/`round()` and logging on any other number/length mismatch.
fn unify_operands(name: &str, fn_name: &str, values: Vec<CalcValue>) -> Option<OperandMode> {
    if values.iter().all(|v| matches!(v, CalcValue::Number(_))) {
        return Some(OperandMode::Numbers(
            values
                .into_iter()
                .map(|v| match v {
                    CalcValue::Number(n) => n,
                    _ => unreachable!(),
                })
                .collect(),
        ));
    }

    let mut lengths = Vec::with_capacity(values.len());
    for value in values {
        match value {
            CalcValue::Length(l) => lengths.push(l),
            // A bare `0` is a valid <length>, as for min()/clamp().
            CalcValue::Number(0.0) => lengths.push(Length::Px(0.0)),
            CalcValue::Number(n) => {
                log::error!(
                    target: "Layouter",
                    "Invalid operand for {fn_name}() in `{}` (expected length): {}",
                    name,
                    n
                );
                return None;
            }
        }
    }
    Some(OperandMode::Lengths(lengths))
}

/// Bridge a [`ParseError`] from `parse_args!` to the CSS-layer error log.
///
/// Only arity failures are reported; resolver/operand failures are silent.
fn report_parse_error(err: &ParseError, fn_name: &str, name: &str) {
    match err {
        ParseError::MissingArgument => log::error!(
            target: "Layouter",
            "Not enough arguments for {}() in `{}`",
            fn_name,
            name
        ),
        ParseError::UnexpectedArgument => log::error!(
            target: "Layouter",
            "Too many arguments for {}() in `{}`",
            fn_name,
            name
        ),
        _ => {}
    }
}

/// CssValue を number / length の型情報付きで評価する。
pub fn resolve_calc_value(
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
        CssValue::Function(fn_name, args) if fn_name == "calc" => parse_args! {
            fn calc(name: &str, args: &[Vec<CssValue>], text_flow_style: &TextFlowStyle)
            -> Result<CalcValue, ParseError>
            {
                expr: CalcValue
            }
            => { Some(expr) }
        }
        .inspect_err(|e| report_parse_error(e, fn_name, name))
        .ok(),
        CssValue::Function(fn_name, args) if (fn_name == "min" || fn_name == "max") => {
            parse_args! {
                fn min_max(
                    name: &str,
                    args: &[Vec<CssValue>],
                    text_flow_style: &TextFlowStyle,
                    fn_name: &str
                )
                as fn_name
                -> Result<CalcValue, ParseError>
                {
                    groups: Vec<CalcValue>
                }
                number {
                    groups
                        .into_iter()
                        .reduce(if fn_name == "min" { f32::min } else { f32::max })
                        .map(CalcValue::Number)
                }
                length {
                    // Fold right so the last operand becomes the innermost
                    // argument: min(a, b, c) => Min(a, Min(b, c)).
                    groups
                        .into_iter()
                        .rev()
                        .reduce(|acc, arg| {
                            if fn_name == "min" {
                                Length::Min(Box::new(arg), Box::new(acc))
                            } else {
                                Length::Max(Box::new(arg), Box::new(acc))
                            }
                        })
                        .map(CalcValue::Length)
                }
            }
            .inspect_err(|e| report_parse_error(e, fn_name, name))
            .ok()
        }
        CssValue::Function(fn_name, args) if fn_name == "clamp" => parse_args! {
            fn clamp(
                name: &str,
                args: &[Vec<CssValue>],
                text_flow_style: &TextFlowStyle,
                fn_name: &str
            )
            as fn_name
            -> Result<CalcValue, ParseError>
            {
                min: CalcValue, value: CalcValue, max: CalcValue
            }
            number {
                Some(CalcValue::Number(value.clamp(min, max)))
            }
            length {
                Some(CalcValue::Length(Length::Clamp {
                    min: Box::new(min),
                    val: Box::new(value),
                    max: Box::new(max),
                }))
            }
        }
        .inspect_err(|e| report_parse_error(e, fn_name, name))
        .ok(),
        CssValue::Function(fn_name, args) if fn_name == "round" => parse_args! {
            fn round(
                name: &str,
                args: &[Vec<CssValue>],
                text_flow_style: &TextFlowStyle,
                fn_name: &str
            )
            as fn_name
            -> Result<CalcValue, ParseError>
            {
                strategy: Option<RoundingStrategy>,
                value: CalcValue,
                step: CalcValue = CalcValue::Number(1.0)
            }
            number {
                if step == 0.0 {
                    log::error!(
                        target: "Layouter",
                        "Zero step in round() for `{}`",
                        name
                    );
                    None
                } else {
                    Some(CalcValue::Number(round_to_step(
                        strategy.unwrap_or(RoundingStrategy::Nearest),
                        value,
                        step,
                    )))
                }
            }
            length {
                round_length(
                    name,
                    strategy.unwrap_or(RoundingStrategy::Nearest),
                    value,
                    step,
                )
            }
        }
        .inspect_err(|e| report_parse_error(e, fn_name, name))
        .ok(),
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

/// Parse a `<rounding-strategy>` keyword (ASCII case-insensitive) from
/// `round()`. `line-width` and other keywords are not supported.
fn parse_rounding_strategy(keyword: &str) -> Option<RoundingStrategy> {
    if keyword.eq_ignore_ascii_case("nearest") {
        Some(RoundingStrategy::Nearest)
    } else if keyword.eq_ignore_ascii_case("up") {
        Some(RoundingStrategy::Up)
    } else if keyword.eq_ignore_ascii_case("down") {
        Some(RoundingStrategy::Down)
    } else if keyword.eq_ignore_ascii_case("to-zero") {
        Some(RoundingStrategy::ToZero)
    } else {
        None
    }
}

/// Build a [`Length::Round`], rejecting a step that is already known to be
/// zero. Steps that only evaluate to zero after resolution (e.g. inside a
/// `calc()`) are rejected by the consumers of the resolved length instead.
fn round_length(
    name: &str,
    strategy: RoundingStrategy,
    value: Length,
    step: Length,
) -> Option<CalcValue> {
    if matches!(step, Length::Px(0.0) | Length::Percent(0.0)) {
        log::error!(
            target: "Layouter",
            "Zero step in round() for `{}`",
            name
        );
        return None;
    }
    Some(CalcValue::Length(Length::Round {
        strategy,
        value: Box::new(value),
        step: Box::new(step),
    }))
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
        Some(CssValue::Function(_, _)) => calc_value_to_length(
            name,
            resolve_calc_value(name, components.first()?, text_flow_style)?,
        ),
        Some(CssValue::Number(_)) => None,
        Some(CssValue::Keyword(_)) => None,
        Some(CssValue::Color(_)) => None,
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
    parse_grid_track_list(name, &values, text_flow_style)
}

fn parse_grid_track_list(
    name: &str,
    values: &[&CssValue],
    text_flow_style: &TextFlowStyle,
) -> Option<Vec<GridTrack>> {
    // A lone `none` is valid for both track-list longhands, but a token slice
    // can no longer recognise it as the whole value the way `parse_grid_tracks`
    // does.
    if matches!(values, [CssValue::Keyword(keyword)] if keyword == "none") {
        return Some(Vec::new());
    }
    values
        .iter()
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
    let start = parse_positive_grid_line(start_values);
    if end_values.is_empty() {
        let span = parse_span(start_values).unwrap_or(1);
        return Some(GridPlacement {
            start,
            end: GridPlacementEnd::Span(span),
        });
    }
    if let Some(span) = parse_span(end_values) {
        return Some(GridPlacement {
            start,
            end: GridPlacementEnd::Span(span),
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
            start,
            end: GridPlacementEnd::NegativeLine((-*end) as usize),
        });
    }
    let end = parse_positive_grid_line(end_values)?;
    let start = start?;
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

fn parse_span(values: &[&CssValue]) -> Option<usize> {
    let [CssValue::Keyword(keyword), CssValue::Number(span)] = values else {
        return None;
    };
    (keyword == "span" && *span >= 1.0 && span.fract().abs() < f32::EPSILON)
        .then_some(*span as usize)
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
        // TODO: Support content-based sizing; fall back to `auto` for now.
        CssValue::Keyword(keyword) if matches!(keyword.as_str(), "min-content" | "max-content") => {
            Some(GridTrack::Breadth(LengthOrAuto::Auto))
        }
        // TODO: Support `fit-content()` sizing; fall back to `auto` for now.
        CssValue::Function(function, _) if function == "fit-content" => {
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

/// The three longhands a `grid-template` shorthand declaration covers.
pub struct GridTemplate {
    pub rows: Vec<GridTrack>,
    pub columns: Vec<GridTrack>,
    pub areas: Vec<Vec<String>>,
}

/// Parse the `grid-template` shorthand (CSS Grid Layout 1 §7.4):
///
/// ```text
/// none
///  | <'grid-template-rows'> / <'grid-template-columns'>
///  | [ <line-names>? <string> <track-size>? <line-names>? ]+
///      [ / <explicit-track-list> ]?
/// ```
///
/// In the third form the area strings and the row sizes that follow them are
/// interleaved, so the tokens before the `/` are not a plain track list.
///
/// All three longhands are always set, since a shorthand resets every longhand
/// it covers.
pub fn parse_grid_template(
    value: &CssValue,
    text_flow_style: &TextFlowStyle,
) -> Option<GridTemplate> {
    if matches!(value, CssValue::Keyword(keyword) if keyword == "none") {
        return Some(GridTemplate {
            rows: Vec::new(),
            columns: Vec::new(),
            areas: Vec::new(),
        });
    }

    let values: Vec<&CssValue> = match value {
        CssValue::List(values) => values.iter().collect(),
        value => vec![value],
    };

    let slash = values
        .iter()
        .position(|value| matches!(value, CssValue::Keyword(keyword) if keyword == "/"));
    let (head, columns) = match slash {
        Some(slash) => (&values[..slash], Some(&values[slash + 1..])),
        None => (values.as_slice(), None),
    };

    // Named grid lines are not modelled by `Style`, so drop them up front
    // rather than mistake them for track sizes.
    let head = without_grid_line_names(head)?;
    let columns = match columns {
        Some(columns) => Some(without_grid_line_names(columns)?),
        None => None,
    };

    // Area strings only appear in the third form.
    if head
        .iter()
        .any(|value| matches!(value, CssValue::String(_)))
    {
        let (rows, areas) = parse_grid_template_row_areas(&head, text_flow_style)?;
        // Omitting `/ <explicit-track-list>` leaves grid-template-columns none.
        let columns = match columns {
            Some(columns) => {
                parse_grid_track_list("grid-template-columns", &columns, text_flow_style)?
            }
            None => Vec::new(),
        };
        return Some(GridTemplate {
            rows,
            columns,
            areas,
        });
    }

    // Without a `/` a bare track list matches none of the grammar's forms.
    let rows = parse_grid_track_list("grid-template-rows", &head, text_flow_style)?;
    let columns = parse_grid_track_list("grid-template-columns", &columns?, text_flow_style)?;

    // `<rows> / <columns>` sets grid-template-areas to none.
    Some(GridTemplate {
        rows,
        columns,
        areas: Vec::new(),
    })
}

/// Parse `[ <line-names>? <string> <track-size>? <line-names>? ]+` into the
/// `grid-template-rows` tracks and the `grid-template-areas` rows. A string
/// with no `<track-size>` after it sizes its row `auto`.
fn parse_grid_template_row_areas(
    values: &[&CssValue],
    text_flow_style: &TextFlowStyle,
) -> Option<(Vec<GridTrack>, Vec<Vec<String>>)> {
    let mut rows = Vec::new();
    let mut strings = Vec::new();
    let mut index = 0;
    while let Some(value) = values.get(index) {
        // Each group starts with an area string.
        let CssValue::String(string) = value else {
            return None;
        };
        strings.push(string.as_str());
        match values.get(index + 1) {
            Some(next) if !matches!(next, CssValue::String(_)) => {
                rows.push(parse_grid_track(
                    "grid-template-rows",
                    next,
                    text_flow_style,
                )?);
                index += 2;
            }
            _ => {
                rows.push(GridTrack::default());
                index += 1;
            }
        }
    }
    Some((rows, resolve_grid_template_area_rows(&strings)?))
}

/// Drop `[ <custom-ident>* ]` groups from a token slice. `None` means a group
/// was left unterminated.
fn without_grid_line_names<'a>(values: &[&'a CssValue]) -> Option<Vec<&'a CssValue>> {
    let mut tracks = Vec::with_capacity(values.len());
    let mut index = 0;
    while index < values.len() {
        index = skip_grid_line_names(values, index)?;
        if let Some(value) = values.get(index) {
            tracks.push(*value);
            index += 1;
        }
    }
    Some(tracks)
}

/// A `[ <custom-ident>* ]` group is tokenized as separate `[`, ident and `]`
/// keywords. Returns the index just past the group starting at `index`, or
/// `index` itself when no group starts there.
fn skip_grid_line_names(values: &[&CssValue], index: usize) -> Option<usize> {
    if !matches!(values.get(index), Some(CssValue::Keyword(keyword)) if keyword == "[") {
        return Some(index);
    }
    let mut index = index + 1;
    while !matches!(values.get(index), Some(CssValue::Keyword(keyword)) if keyword == "]") {
        if !matches!(values.get(index), Some(CssValue::Keyword(_))) {
            return None;
        }
        index += 1;
    }
    Some(index + 1)
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
    resolve_grid_template_area_rows(&rows)
}

/// Split one template string per row into area names, rejecting rows of
/// differing widths and names that do not form a rectangle.
fn resolve_grid_template_area_rows(rows: &[&str]) -> Option<Vec<Vec<String>>> {
    let areas: Vec<Vec<String>> = rows
        .iter()
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
    match value {
        CssValue::List(values) => {
            let refs: Vec<&CssValue> = values.iter().collect();
            if let Some(span) = parse_span(&refs) {
                return Some(GridPlacementEnd::Span(span));
            }
            let [CssValue::Number(n)] = refs.as_slice() else {
                return None;
            };
            resolve_grid_line_end_number(*n)
        }
        value => {
            let CssValue::Number(n) = value else {
                return None;
            };
            resolve_grid_line_end_number(*n)
        }
    }
}

fn resolve_grid_line_end_number(n: f32) -> Option<GridPlacementEnd> {
    if n.fract().abs() >= f32::EPSILON {
        return None;
    }

    match n {
        n if n >= 1.0 => Some(GridPlacementEnd::Line(n as usize)),
        n if n <= -1.0 => Some(GridPlacementEnd::NegativeLine(-n as usize)),
        _ => None,
    }
}
