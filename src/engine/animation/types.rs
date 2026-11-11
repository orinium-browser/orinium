//! Shared types for CSS transitions and animations.

use crate::engine::css::values::CssValue;

/// CSS time value in milliseconds.
pub type TimeMs = f64;

/// Supported easing functions for MVP.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    CubicBezier(f32, f32, f32, f32),
    Steps(u32, StepPosition),
}

impl Default for Easing {
    fn default() -> Self {
        Self::Ease
    }
}

/// Step position for `steps()` timing functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepPosition {
    JumpStart,
    JumpEnd,
    JumpNone,
    JumpBoth,
}

impl Default for StepPosition {
    fn default() -> Self {
        Self::JumpEnd
    }
}

/// Animation iteration count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IterationCount {
    Finite(f32),
    Infinite,
}

impl Default for IterationCount {
    fn default() -> Self {
        Self::Finite(1.0)
    }
}

/// Animation direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationDirection {
    #[default]
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// Animation fill mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationFillMode {
    #[default]
    None,
    Forwards,
    Backwards,
    Both,
}

/// Animation play state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationPlayState {
    #[default]
    Running,
    Paused,
}

/// A single keyframe stop.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframeStop {
    /// Offset in the range [0.0, 1.0].
    pub offset: f32,
    pub declarations: Vec<(String, CssValue)>,
}

/// A parsed `@keyframes` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframesRule {
    pub name: String,
    pub stops: Vec<KeyframeStop>,
}


/// A resolved CSS transition specification for one property slot.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionSpec {
    pub property: String,
    pub duration_ms: TimeMs,
    pub delay_ms: TimeMs,
    pub easing: Easing,
}

/// A resolved CSS animation specification for one animation slot.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationSpec {
    pub name: String,
    pub duration_ms: TimeMs,
    pub delay_ms: TimeMs,
    pub easing: Easing,
    pub iteration_count: IterationCount,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

impl Default for TransitionSpec {
    fn default() -> Self {
        Self {
            property: "all".to_string(),
            duration_ms: 0.0,
            delay_ms: 0.0,
            easing: Easing::Ease,
        }
    }
}

impl Default for AnimationSpec {
    fn default() -> Self {
        Self {
            name: "none".to_string(),
            duration_ms: 0.0,
            delay_ms: 0.0,
            easing: Easing::Ease,
            iteration_count: IterationCount::Finite(1.0),
            direction: AnimationDirection::Normal,
            fill_mode: AnimationFillMode::None,
            play_state: AnimationPlayState::Running,
        }
    }
}
