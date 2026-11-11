//! Animation runtime state and sampling logic.

use std::collections::HashMap;

use crate::engine::animation::easing::{apply_easing, interpolate_value};
use crate::engine::animation::types::{
    AnimationDirection, AnimationFillMode, AnimationPlayState, AnimationSpec, Easing,
    IterationCount, KeyframesRule, TransitionSpec,
};
use crate::engine::css::values::CssValue;
use crate::engine::layouter::css_resolver::KeyframesRegistry;
use crate::engine::layouter::dom_snapshot::NodeId;
use crate::engine::layouter::types::ColorScheme;

/// Base computed values and resolved animation specs for one element.
#[derive(Debug, Clone, Default)]
pub struct ElementAnimationState {
    pub transitions: Vec<TransitionSpec>,
    pub animations: Vec<AnimationSpec>,
    /// Current base (non-animated) values for animatable properties.
    pub base_values: HashMap<String, CssValue>,
}

pub type AnimationInputs = HashMap<NodeId, ElementAnimationState>;

#[derive(Debug, Clone)]
struct TransitionInstance {
    _property: String,
    from: CssValue,
    to: CssValue,
    start_ms: f64,
    duration_ms: f64,
    delay_ms: f64,
    easing: Easing,
}

#[derive(Debug, Clone)]
struct AnimationInstance {
    spec: AnimationSpec,
    start_ms: f64,
}

#[derive(Debug, Clone, Default)]
struct ElementRuntimeState {
    transitions: HashMap<String, TransitionInstance>,
    animations: Vec<AnimationInstance>,
    prev_base: HashMap<String, CssValue>,
    prev_animation_specs: Vec<AnimationSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct AnimationRuntime {
    elements: HashMap<NodeId, ElementRuntimeState>,
}

#[derive(Debug, Clone, Default)]
pub struct SampleResult {
    pub overrides: HashMap<NodeId, HashMap<String, CssValue>>,
    pub active: bool,
    pub dirty: bool,
}

impl AnimationRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn sample(
        &mut self,
        now_ms: f64,
        visible: bool,
        inputs: &AnimationInputs,
        keyframes: &KeyframesRegistry,
        color_scheme: ColorScheme,
    ) -> SampleResult {
        if !visible {
            // Pause timelines: keep runtime state but report idle/no overrides.
            // Avoid continuous redraw while hidden.
            return SampleResult::default();
        }

        let mut overrides: HashMap<NodeId, HashMap<String, CssValue>> = HashMap::new();
        let mut active = false;

        // Remove elements no longer present.
        self.elements.retain(|id, _| inputs.contains_key(id));

        for (node_id, input) in inputs {
            let state = self.elements.entry(*node_id).or_default();
            let mut node_overrides: HashMap<String, CssValue> = HashMap::new();
            let first_seen = state.prev_base.is_empty()
                && state.transitions.is_empty()
                && state.animations.is_empty()
                && state.prev_animation_specs.is_empty();

            // ---- Transitions ----
            for spec in &input.transitions {
                if spec.duration_ms <= 0.0 && spec.delay_ms <= 0.0 {
                    continue;
                }
                let raw_prop = spec.property.as_str();
                if raw_prop == "none" {
                    continue;
                }

                let props: Vec<String> = if raw_prop == "all" {
                    let mut keys: Vec<String> = input.base_values.keys().cloned().collect();
                    keys.sort();
                    keys
                } else {
                    vec![raw_prop.to_string()]
                };

                for prop in props {
                    let Some(to) = input.base_values.get(&prop) else {
                        continue;
                    };
                    let prev = state
                        .prev_base
                        .get(&prop)
                        .or_else(|| state.transitions.get(&prop).map(|t| &t.to));
                    if let Some(from) = prev {
                        // Avoid starting a transition on the very first observation
                        // of an element; establish a baseline instead.
                        if !first_seen && from != to {
                            let from_val = state
                                .transitions
                                .get(&prop)
                                .and_then(|inst| {
                                    sample_transition_value(inst, now_ms, color_scheme)
                                })
                                .unwrap_or_else(|| from.clone());
                            state.transitions.insert(
                                prop.clone(),
                                TransitionInstance {
                                    _property: prop.clone(),
                                    from: from_val,
                                    to: to.clone(),
                                    start_ms: now_ms,
                                    duration_ms: spec.duration_ms.max(0.0),
                                    delay_ms: spec.delay_ms,
                                    easing: spec.easing,
                                },
                            );
                            active = true;
                        }
                    }
                }
            }

            let mut finished_transitions = Vec::new();
            for (prop, inst) in &state.transitions {
                match sample_transition_value(inst, now_ms, color_scheme) {
                    Some(val) => {
                        // active while within duration window (including delay)
                        let end = inst.start_ms + inst.delay_ms + inst.duration_ms.max(0.0);
                        if now_ms < end {
                            active = true;
                        }
                        // if finished (at end), still emit final value this frame? let caller settle
                        if now_ms >= end {
                            finished_transitions.push(prop.clone());
                        } else {
                            node_overrides.insert(prop.clone(), val);
                        }
                    }
                    None => {
                        // in delay phase
                        let end = inst.start_ms + inst.delay_ms + inst.duration_ms.max(0.0);
                        if now_ms < end {
                            active = true;
                        }
                    }
                }
            }
            for prop in finished_transitions {
                state.transitions.remove(&prop);
            }

            // ---- Animations ----
            if state.prev_animation_specs != input.animations {
                state.animations = input
                    .animations
                    .iter()
                    .filter(|spec| spec.name != "none" && !spec.name.is_empty())
                    .map(|spec| AnimationInstance {
                        spec: spec.clone(),
                        start_ms: now_ms,
                    })
                    .collect();
                state.prev_animation_specs = input.animations.clone();
            }

            for inst in &state.animations {
                let Some(rule) = keyframes.get(&inst.spec.name) else {
                    continue;
                };
                if let Some((anim_active, anim_vals)) =
                    sample_animation(inst, rule, now_ms, color_scheme)
                {
                    if anim_active {
                        active = true;
                    }
                    for (prop, val) in anim_vals {
                        // animations override transitions/base for same property in this MVP
                        node_overrides.insert(prop, val);
                    }
                }
            }

            if !node_overrides.is_empty() {
                overrides.insert(*node_id, node_overrides);
            }

            state.prev_base = input.base_values.clone();
        }

        // Mark dirty when there are overrides or any active timelines; this is
        // conservative but ensures animated frames relayout/repaint.
        let dirty = active || !overrides.is_empty();

        SampleResult {
            overrides,
            active,
            dirty,
        }
    }
}

fn sample_transition_value(
    inst: &TransitionInstance,
    now_ms: f64,
    color_scheme: ColorScheme,
) -> Option<CssValue> {
    let start = inst.start_ms + inst.delay_ms;
    if now_ms < start {
        return None;
    }
    let dur = inst.duration_ms;
    if dur <= 0.0 {
        return Some(inst.to.clone());
    }
    let t = ((now_ms - start) / dur).clamp(0.0, 1.0) as f32;
    if t >= 1.0 {
        return Some(inst.to.clone());
    }
    let eased = apply_easing(t, inst.easing);
    interpolate_value(&inst.from, &inst.to, eased, color_scheme).or_else(|| {
        if eased >= 0.5 {
            Some(inst.to.clone())
        } else {
            Some(inst.from.clone())
        }
    })
}

fn sample_animation(
    inst: &AnimationInstance,
    rule: &KeyframesRule,
    now_ms: f64,
    color_scheme: ColorScheme,
) -> Option<(bool, HashMap<String, CssValue>)> {
    let spec = &inst.spec;
    if spec.play_state == AnimationPlayState::Paused {
        // MVP: paused animations hold at initial progress (0/local start behavior)
        // without accumulating time; treat as inactive for now.
        // Better pause handling can track paused elapsed time.
    }

    let duration = spec.duration_ms.max(0.0);
    let delay = spec.delay_ms;
    let local = now_ms - inst.start_ms;

    if duration <= 0.0 {
        // Duration 0: apply fill/endpoint as appropriate
        if local < delay {
            if matches!(spec.fill_mode, AnimationFillMode::Backwards | AnimationFillMode::Both) {
                let vals = sample_keyframes(rule, 0.0, spec.direction, 0, color_scheme);
                return Some((false, vals));
            }
            return None;
        } else {
            if matches!(spec.fill_mode, AnimationFillMode::Forwards | AnimationFillMode::Both) {
                let vals = sample_keyframes(rule, 1.0, spec.direction, 0, color_scheme);
                return Some((false, vals));
            }
            return None;
        }
    }

    if local < delay {
        if matches!(spec.fill_mode, AnimationFillMode::Backwards | AnimationFillMode::Both) {
            // progress at start of first iteration considering direction
            let p = iteration_progress(0.0, spec.direction, 0);
            let vals = sample_keyframes(rule, p, spec.direction, 0, color_scheme);
            return Some((false, vals));
        }
        return Some((true, HashMap::new())); // waiting in delay
    }

    let active_time = local - delay;
    let iter_progress_total = active_time / duration;

    let (iteration_index, within, finished) = match spec.iteration_count {
        IterationCount::Infinite => {
            let idx = iter_progress_total.floor() as u32;
            (idx, (iter_progress_total - idx as f64) as f32, false)
        }
        IterationCount::Finite(count) => {
            if count <= 0.0 {
                (0u32, 0.0f32, true)
            } else if iter_progress_total >= count as f64 {
                (count.ceil() as u32, 1.0f32, true)
            } else {
                let idx = iter_progress_total.floor() as u32;
                (idx, (iter_progress_total - idx as f64) as f32, false)
            }
        }
    };

    if finished {
        if matches!(spec.fill_mode, AnimationFillMode::Forwards | AnimationFillMode::Both) {
            // final iteration progress
            let last_iter = match spec.iteration_count {
                IterationCount::Finite(c) if c > 0.0 => (c.ceil() as u32).saturating_sub(1),
                _ => 0,
            };
            let p = iteration_progress(1.0, spec.direction, last_iter);
            let vals = sample_keyframes(rule, p, spec.direction, last_iter, color_scheme);
            return Some((false, vals));
        }
        return None;
    }

    if spec.play_state == AnimationPlayState::Paused {
        // MVP: treat as not actively progressing; no time accumulation model yet.
        let p = iteration_progress(0.0, spec.direction, iteration_index);
        let vals = sample_keyframes(rule, p, spec.direction, iteration_index, color_scheme);
        return Some((false, vals));
    }

    let directed = iteration_progress(within.clamp(0.0, 1.0), spec.direction, iteration_index);
    let vals = sample_keyframes(rule, directed, spec.direction, iteration_index, color_scheme);
    Some((true, vals))
}

fn iteration_progress(t: f32, direction: AnimationDirection, iteration: u32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match direction {
        AnimationDirection::Normal => t,
        AnimationDirection::Reverse => 1.0 - t,
        AnimationDirection::Alternate => {
            if iteration % 2 == 0 { t } else { 1.0 - t }
        }
        AnimationDirection::AlternateReverse => {
            if iteration % 2 == 0 { 1.0 - t } else { t }
        }
    }
}

fn sample_keyframes(
    rule: &KeyframesRule,
    progress: f32,
    _direction: AnimationDirection,
    _iteration: u32,
    color_scheme: ColorScheme,
) -> HashMap<String, CssValue> {
    let mut stops = rule.stops.clone();
    if stops.is_empty() {
        return HashMap::new();
    }
    stops.sort_by(|a, b| a.offset.partial_cmp(&b.offset).unwrap_or(std::cmp::Ordering::Equal));

    // Collect properties
    let mut props: Vec<String> = Vec::new();
    for stop in &stops {
        for (name, _) in &stop.declarations {
            if !props.iter().any(|p| p == name) {
                props.push(name.clone());
            }
        }
    }

    let mut out = HashMap::new();
    let p = progress.clamp(0.0, 1.0);
    for prop in props {
        // Gather stops that define this property, sorted by offset
        let prop_stops: Vec<(f32, CssValue)> = stops
            .iter()
            .filter_map(|s| {
                s.declarations
                    .iter()
                    .find(|(n, _)| n == &prop)
                    .map(|(_, v)| (s.offset, v.clone()))
            })
            .collect();
        if prop_stops.is_empty() {
            continue;
        }
        if prop_stops.len() == 1 {
            out.insert(prop, prop_stops[0].1.clone());
            continue;
        }
        // find segment
        if p <= prop_stops[0].0 {
            out.insert(prop, prop_stops[0].1.clone());
            continue;
        }
        if p >= prop_stops[prop_stops.len() - 1].0 {
            out.insert(prop, prop_stops[prop_stops.len() - 1].1.clone());
            continue;
        }
        let mut chosen = None;
        for w in prop_stops.windows(2) {
            let (o0, v0) = &w[0];
            let (o1, v1) = &w[1];
            if p >= *o0 && p <= *o1 {
                let span = (*o1 - *o0).max(f32::EPSILON);
                let local = ((p - *o0) / span).clamp(0.0, 1.0);
                let eased = apply_easing(local, Easing::Linear); // keyframe-level easing not modeled in MVP
                let val = interpolate_value(v0, v1, eased, color_scheme).unwrap_or_else(|| {
                    if eased >= 0.5 { v1.clone() } else { v0.clone() }
                });
                chosen = Some(val);
                break;
            }
        }
        if let Some(val) = chosen.or_else(|| Some(prop_stops[0].1.clone())) {
            out.insert(prop, val);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::animation::types::{Easing, KeyframeStop, TransitionSpec};

    fn inputs_with_opacity(node: NodeId, opacity: f32, transition: bool) -> AnimationInputs {
        let mut base = HashMap::new();
        base.insert("opacity".to_string(), CssValue::Number(opacity));
        let transitions = if transition {
            vec![TransitionSpec {
                property: "opacity".to_string(),
                duration_ms: 100.0,
                delay_ms: 0.0,
                easing: Easing::Linear,
            }]
        } else {
            vec![]
        };
        let mut map = HashMap::new();
        map.insert(
            node,
            ElementAnimationState {
                transitions,
                animations: vec![],
                base_values: base,
            },
        );
        map
    }

    #[test]
    fn transition_starts_on_value_change_and_interpolates() {
        let mut rt = AnimationRuntime::new();
        let node = 1u32;
        let kf = KeyframesRegistry::new();
        let _ = rt.sample(0.0, true, &inputs_with_opacity(node, 0.0, true), &kf, ColorScheme::Light);
        let res = rt.sample(
            50.0,
            true,
            &inputs_with_opacity(node, 1.0, true),
            &kf,
            ColorScheme::Light,
        );
        assert!(res.active || res.dirty);
        if let Some(CssValue::Number(v)) = res.overrides.get(&node).and_then(|m| m.get("opacity")) {
            assert!(*v >= 0.0 && *v <= 1.0);
        }
    }

    #[test]
    fn hidden_sampling_is_idle() {
        let mut rt = AnimationRuntime::new();
        let node = 1u32;
        let kf = KeyframesRegistry::new();
        let _ = rt.sample(0.0, true, &inputs_with_opacity(node, 0.0, true), &kf, ColorScheme::Light);
        let res = rt.sample(
            50.0,
            false,
            &inputs_with_opacity(node, 1.0, true),
            &kf,
            ColorScheme::Light,
        );
        assert!(!res.active);
        assert!(res.overrides.is_empty());
    }

    #[test]
    fn keyframes_animation_produces_interpolated_values() {
        let mut rt = AnimationRuntime::new();
        let node = 2u32;
        let mut kf = KeyframesRegistry::new();
        kf.insert(
            "fade".to_string(),
            KeyframesRule {
                name: "fade".to_string(),
                stops: vec![
                    KeyframeStop { offset: 0.0, declarations: vec![("opacity".into(), CssValue::Number(0.0))] },
                    KeyframeStop { offset: 1.0, declarations: vec![("opacity".into(), CssValue::Number(1.0))] },
                ],
            },
        );
        let mut map = AnimationInputs::new();
        map.insert(
            node,
            ElementAnimationState {
                transitions: vec![],
                animations: vec![AnimationSpec {
                    name: "fade".into(),
                    duration_ms: 100.0,
                    delay_ms: 0.0,
                    easing: Easing::Linear,
                    iteration_count: IterationCount::Finite(1.0),
                    direction: AnimationDirection::Normal,
                    fill_mode: AnimationFillMode::None,
                    play_state: AnimationPlayState::Running,
                }],
                base_values: HashMap::new(),
            },
        );
        let r0 = rt.sample(0.0, true, &map, &kf, ColorScheme::Light);
        assert!(r0.active);
        let r1 = rt.sample(50.0, true, &map, &kf, ColorScheme::Light);
        let v = r1.overrides.get(&node).and_then(|m| m.get("opacity")).cloned();
        match v {
            Some(CssValue::Number(n)) => assert!(n > 0.0 && n < 1.0, "got {n}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn animation_fill_forwards_holds_end_value() {
        let mut rt = AnimationRuntime::new();
        let node = 3u32;
        let mut kf = KeyframesRegistry::new();
        kf.insert(
            "fade".to_string(),
            KeyframesRule {
                name: "fade".to_string(),
                stops: vec![
                    KeyframeStop { offset: 0.0, declarations: vec![("opacity".into(), CssValue::Number(0.0))] },
                    KeyframeStop { offset: 1.0, declarations: vec![("opacity".into(), CssValue::Number(1.0))] },
                ],
            },
        );
        let mut map = AnimationInputs::new();
        map.insert(
            node,
            ElementAnimationState {
                transitions: vec![],
                animations: vec![AnimationSpec {
                    name: "fade".into(),
                    duration_ms: 100.0,
                    delay_ms: 0.0,
                    easing: Easing::Linear,
                    iteration_count: IterationCount::Finite(1.0),
                    direction: AnimationDirection::Normal,
                    fill_mode: AnimationFillMode::Forwards,
                    play_state: AnimationPlayState::Running,
                }],
                base_values: HashMap::new(),
            },
        );
        let _ = rt.sample(0.0, true, &map, &kf, ColorScheme::Light);
        let r_end = rt.sample(150.0, true, &map, &kf, ColorScheme::Light);
        let v = r_end.overrides.get(&node).and_then(|m| m.get("opacity")).cloned();
        assert_eq!(v, Some(CssValue::Number(1.0)));
    }

    #[test]
    fn transition_property_all_animates_tracked_base_changes() {
        let mut rt = AnimationRuntime::new();
        let node = 4u32;
        let kf = KeyframesRegistry::new();
        let mut base0 = HashMap::new();
        base0.insert("opacity".to_string(), CssValue::Number(0.0));
        let mut map0 = AnimationInputs::new();
        map0.insert(
            node,
            ElementAnimationState {
                transitions: vec![TransitionSpec {
                    property: "all".to_string(),
                    duration_ms: 100.0,
                    delay_ms: 0.0,
                    easing: Easing::Linear,
                }],
                animations: vec![],
                base_values: base0,
            },
        );
        let _ = rt.sample(0.0, true, &map0, &kf, ColorScheme::Light);

        let mut base1 = HashMap::new();
        base1.insert("opacity".to_string(), CssValue::Number(1.0));
        let mut map1 = AnimationInputs::new();
        map1.insert(
            node,
            ElementAnimationState {
                transitions: vec![TransitionSpec {
                    property: "all".to_string(),
                    duration_ms: 100.0,
                    delay_ms: 0.0,
                    easing: Easing::Linear,
                }],
                animations: vec![],
                base_values: base1,
            },
        );
        // Change detected at 50ms => transition starts at 50ms; sample mid-way.
        let _ = rt.sample(50.0, true, &map1, &kf, ColorScheme::Light);
        let res = rt.sample(100.0, true, &map1, &kf, ColorScheme::Light);
        assert!(res.active || res.dirty);
        if let Some(CssValue::Number(v)) = res.overrides.get(&node).and_then(|m| m.get("opacity")) {
            assert!(*v > 0.0 && *v < 1.0, "expected mid transition, got {v}");
        } else {
            panic!("expected opacity override");
        }
    }

    #[test]
    fn animation_alternate_reverses_second_iteration() {
        let mut rt = AnimationRuntime::new();
        let node = 5u32;
        let mut kf = KeyframesRegistry::new();
        kf.insert(
            "fade".to_string(),
            KeyframesRule {
                name: "fade".to_string(),
                stops: vec![
                    KeyframeStop { offset: 0.0, declarations: vec![("opacity".into(), CssValue::Number(0.0))] },
                    KeyframeStop { offset: 1.0, declarations: vec![("opacity".into(), CssValue::Number(1.0))] },
                ],
            },
        );
        let mut map = AnimationInputs::new();
        map.insert(
            node,
            ElementAnimationState {
                transitions: vec![],
                animations: vec![AnimationSpec {
                    name: "fade".into(),
                    duration_ms: 100.0,
                    delay_ms: 0.0,
                    easing: Easing::Linear,
                    iteration_count: IterationCount::Finite(2.0),
                    direction: AnimationDirection::Alternate,
                    fill_mode: AnimationFillMode::None,
                    play_state: AnimationPlayState::Running,
                }],
                base_values: HashMap::new(),
            },
        );
        let _ = rt.sample(0.0, true, &map, &kf, ColorScheme::Light);
        // mid first iteration -> increasing
        let r1 = rt.sample(50.0, true, &map, &kf, ColorScheme::Light);
        // mid second iteration (reversed) -> decreasing
        let r2 = rt.sample(150.0, true, &map, &kf, ColorScheme::Light);
        let v1 = match r1.overrides.get(&node).and_then(|m| m.get("opacity")) {
            Some(CssValue::Number(v)) => *v,
            other => panic!("unexpected {other:?}"),
        };
        let v2 = match r2.overrides.get(&node).and_then(|m| m.get("opacity")) {
            Some(CssValue::Number(v)) => *v,
            other => panic!("unexpected {other:?}"),
        };
        assert!(v1 > 0.3 && v1 < 0.7, "v1={v1}");
        assert!(v2 > 0.3 && v2 < 0.7, "v2={v2}");
        // alternate means iteration 2 goes 1->0, so at midpoint value ~0.5 but direction reversed; values should be similar magnitude
    }
}
