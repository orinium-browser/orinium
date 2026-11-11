//! Parsing helpers for CSS transition/animation values.

use crate::engine::animation::types::{
    AnimationDirection, AnimationFillMode, AnimationPlayState, AnimationSpec, Easing,
    IterationCount, StepPosition, TimeMs, TransitionSpec,
};
use crate::engine::css::values::{CssValue, Unit};

pub fn parse_time_ms(value: &CssValue) -> Option<TimeMs> {
    match value {
        CssValue::Length(v, Unit::S) => Some((*v as f64) * 1000.0),
        CssValue::Length(v, Unit::Ms) => Some(*v as f64),
        CssValue::Number(v) if *v == 0.0 => Some(0.0),
        _ => None,
    }
}

pub fn parse_easing(value: &CssValue) -> Option<Easing> {
    match value {
        CssValue::Keyword(k) => match k.to_ascii_lowercase().as_str() {
            "linear" => Some(Easing::Linear),
            "ease" => Some(Easing::Ease),
            "ease-in" => Some(Easing::EaseIn),
            "ease-out" => Some(Easing::EaseOut),
            "ease-in-out" => Some(Easing::EaseInOut),
            "step-start" => Some(Easing::Steps(1, StepPosition::JumpStart)),
            "step-end" => Some(Easing::Steps(1, StepPosition::JumpEnd)),
            _ => None,
        },
        CssValue::Function(name, args) if name.eq_ignore_ascii_case("cubic-bezier") => {
            let nums: Vec<f32> = args
                .iter()
                .flatten()
                .filter_map(|v| match v {
                    CssValue::Number(n) => Some(*n),
                    _ => None,
                })
                .collect();
            if nums.len() == 4 {
                Some(Easing::CubicBezier(nums[0], nums[1], nums[2], nums[3]))
            } else {
                None
            }
        }
        CssValue::Function(name, args) if name.eq_ignore_ascii_case("steps") => {
            let mut count: Option<u32> = None;
            let mut pos = StepPosition::JumpEnd;
            for arg in args.iter().flatten() {
                match arg {
                    CssValue::Number(n) if *n >= 0.0 => count = Some(*n as u32),
                    CssValue::Keyword(k) => {
                        pos = match k.to_ascii_lowercase().as_str() {
                            "jump-start" | "start" => StepPosition::JumpStart,
                            "jump-end" | "end" => StepPosition::JumpEnd,
                            "jump-none" => StepPosition::JumpNone,
                            "jump-both" => StepPosition::JumpBoth,
                            _ => pos,
                        };
                    }
                    _ => {}
                }
            }
            Some(Easing::Steps(count.unwrap_or(1).max(1), pos))
        }
        _ => None,
    }
}

fn split_comma_list(value: &CssValue) -> Vec<Vec<CssValue>> {
    match value {
        CssValue::List(items) => {
            let mut groups: Vec<Vec<CssValue>> = vec![Vec::new()];
            for item in items {
                if matches!(item, CssValue::Keyword(k) if k == ",") {
                    groups.push(Vec::new());
                } else {
                    groups.last_mut().unwrap().push(item.clone());
                }
            }
            groups
        }
        other => vec![vec![other.clone()]],
    }
}

fn flatten_group(group: &[CssValue]) -> Vec<CssValue> {
    let mut out = Vec::new();
    for v in group {
        match v {
            CssValue::List(inner) => out.extend(inner.clone()),
            _ => out.push(v.clone()),
        }
    }
    out
}

pub fn parse_transition_property_list(value: &CssValue) -> Vec<String> {
    split_comma_list(value)
        .into_iter()
        .map(|g| {
            flatten_group(&g)
                .into_iter()
                .find_map(|v| match v {
                    CssValue::Keyword(k) => Some(k.to_string()),
                    CssValue::String(s) => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| "all".to_string())
        })
        .collect()
}

pub fn parse_time_list(value: &CssValue) -> Vec<TimeMs> {
    split_comma_list(value)
        .into_iter()
        .map(|g| {
            flatten_group(&g)
                .into_iter()
                .find_map(|v| parse_time_ms(&v))
                .unwrap_or(0.0)
        })
        .collect()
}

pub fn parse_easing_list(value: &CssValue) -> Vec<Easing> {
    split_comma_list(value)
        .into_iter()
        .map(|g| {
            flatten_group(&g)
                .into_iter()
                .find_map(|v| parse_easing(&v))
                .unwrap_or(Easing::Ease)
        })
        .collect()
}

pub fn parse_iteration_count(value: &CssValue) -> Option<IterationCount> {
    match value {
        CssValue::Keyword(k) if k.eq_ignore_ascii_case("infinite") => Some(IterationCount::Infinite),
        CssValue::Number(n) if *n >= 0.0 => Some(IterationCount::Finite(*n)),
        _ => None,
    }
}

pub fn parse_iteration_count_list(value: &CssValue) -> Vec<IterationCount> {
    split_comma_list(value)
        .into_iter()
        .map(|g| {
            flatten_group(&g)
                .into_iter()
                .find_map(|v| parse_iteration_count(&v))
                .unwrap_or(IterationCount::Finite(1.0))
        })
        .collect()
}

pub fn parse_direction(value: &CssValue) -> Option<AnimationDirection> {
    match value {
        CssValue::Keyword(k) => match k.to_ascii_lowercase().as_str() {
            "normal" => Some(AnimationDirection::Normal),
            "reverse" => Some(AnimationDirection::Reverse),
            "alternate" => Some(AnimationDirection::Alternate),
            "alternate-reverse" => Some(AnimationDirection::AlternateReverse),
            _ => None,
        },
        _ => None,
    }
}

pub fn parse_direction_list(value: &CssValue) -> Vec<AnimationDirection> {
    split_comma_list(value)
        .into_iter()
        .map(|g| flatten_group(&g).into_iter().find_map(|v| parse_direction(&v)).unwrap_or_default())
        .collect()
}

pub fn parse_fill_mode(value: &CssValue) -> Option<AnimationFillMode> {
    match value {
        CssValue::Keyword(k) => match k.to_ascii_lowercase().as_str() {
            "none" => Some(AnimationFillMode::None),
            "forwards" => Some(AnimationFillMode::Forwards),
            "backwards" => Some(AnimationFillMode::Backwards),
            "both" => Some(AnimationFillMode::Both),
            _ => None,
        },
        _ => None,
    }
}

pub fn parse_fill_mode_list(value: &CssValue) -> Vec<AnimationFillMode> {
    split_comma_list(value)
        .into_iter()
        .map(|g| flatten_group(&g).into_iter().find_map(|v| parse_fill_mode(&v)).unwrap_or_default())
        .collect()
}

pub fn parse_play_state(value: &CssValue) -> Option<AnimationPlayState> {
    match value {
        CssValue::Keyword(k) => match k.to_ascii_lowercase().as_str() {
            "running" => Some(AnimationPlayState::Running),
            "paused" => Some(AnimationPlayState::Paused),
            _ => None,
        },
        _ => None,
    }
}

pub fn parse_play_state_list(value: &CssValue) -> Vec<AnimationPlayState> {
    split_comma_list(value)
        .into_iter()
        .map(|g| flatten_group(&g).into_iter().find_map(|v| parse_play_state(&v)).unwrap_or_default())
        .collect()
}

pub fn parse_animation_name_list(value: &CssValue) -> Vec<String> {
    split_comma_list(value)
        .into_iter()
        .map(|g| {
            flatten_group(&g)
                .into_iter()
                .find_map(|v| match v {
                    CssValue::Keyword(k) => Some(k.to_string()),
                    CssValue::String(s) => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| "none".to_string())
        })
        .collect()
}

fn list_nth<T: Clone>(list: &[T], i: usize, default: T) -> T {
    if list.is_empty() {
        default
    } else {
        list.get(i % list.len()).cloned().unwrap_or(default)
    }
}

/// Build transition specs from resolved longhand/shorthand declaration values.
/// Callers should pass the winning values for each transition-* property.
#[derive(Debug, Clone, Default)]
pub struct TransitionInputs {
    pub property: Option<CssValue>,
    pub duration: Option<CssValue>,
    pub delay: Option<CssValue>,
    pub timing_function: Option<CssValue>,
}

pub fn build_transition_specs(input: &TransitionInputs) -> Vec<TransitionSpec> {
    let properties = input
        .property
        .as_ref()
        .map(parse_transition_property_list)
        .unwrap_or_else(|| vec!["all".to_string()]);
    let durations = input
        .duration
        .as_ref()
        .map(parse_time_list)
        .unwrap_or_else(|| vec![0.0]);
    let delays = input
        .delay
        .as_ref()
        .map(parse_time_list)
        .unwrap_or_else(|| vec![0.0]);
    let easings = input
        .timing_function
        .as_ref()
        .map(parse_easing_list)
        .unwrap_or_else(|| vec![Easing::Ease]);

    let count = properties.len().max(1);
    (0..count)
        .map(|i| TransitionSpec {
            property: list_nth(&properties, i, "all".to_string()),
            duration_ms: list_nth(&durations, i, 0.0),
            delay_ms: list_nth(&delays, i, 0.0),
            easing: list_nth(&easings, i, Easing::Ease),
        })
        .collect()
}

/// Build animation specs from resolved animation-* declaration values.
#[derive(Debug, Clone, Default)]
pub struct AnimationInputs {
    pub name: Option<CssValue>,
    pub duration: Option<CssValue>,
    pub delay: Option<CssValue>,
    pub timing_function: Option<CssValue>,
    pub iteration_count: Option<CssValue>,
    pub direction: Option<CssValue>,
    pub fill_mode: Option<CssValue>,
    pub play_state: Option<CssValue>,
}

pub fn build_animation_specs(input: &AnimationInputs) -> Vec<AnimationSpec> {
    let names = input
        .name
        .as_ref()
        .map(parse_animation_name_list)
        .unwrap_or_else(|| vec!["none".to_string()]);
    let durations = input.duration.as_ref().map(parse_time_list).unwrap_or_else(|| vec![0.0]);
    let delays = input.delay.as_ref().map(parse_time_list).unwrap_or_else(|| vec![0.0]);
    let easings = input.timing_function.as_ref().map(parse_easing_list).unwrap_or_else(|| vec![Easing::Ease]);
    let iterations = input.iteration_count.as_ref().map(parse_iteration_count_list).unwrap_or_else(|| vec![IterationCount::Finite(1.0)]);
    let directions = input.direction.as_ref().map(parse_direction_list).unwrap_or_else(|| vec![AnimationDirection::Normal]);
    let fills = input.fill_mode.as_ref().map(parse_fill_mode_list).unwrap_or_else(|| vec![AnimationFillMode::None]);
    let states = input.play_state.as_ref().map(parse_play_state_list).unwrap_or_else(|| vec![AnimationPlayState::Running]);

    let count = names.len().max(1);
    (0..count)
        .map(|i| AnimationSpec {
            name: list_nth(&names, i, "none".to_string()),
            duration_ms: list_nth(&durations, i, 0.0),
            delay_ms: list_nth(&delays, i, 0.0),
            easing: list_nth(&easings, i, Easing::Ease),
            iteration_count: list_nth(&iterations, i, IterationCount::Finite(1.0)),
            direction: list_nth(&directions, i, AnimationDirection::Normal),
            fill_mode: list_nth(&fills, i, AnimationFillMode::None),
            play_state: list_nth(&states, i, AnimationPlayState::Running),
        })
        .collect()
}
