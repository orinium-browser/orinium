//! CSS transition and animation runtime.
//!
//! This module is engine-internal and clock-injected: callers provide the
//! current time and visibility state, and the runtime returns animated style
//! overrides plus activity/dirty flags. It performs no I/O.

pub mod easing;
pub mod parse;
pub mod runtime;
pub mod types;

pub use runtime::{AnimationInputs, AnimationRuntime, ElementAnimationState, SampleResult};

use std::collections::HashMap;

use crate::engine::css::values::CssValue;
use crate::engine::layouter::dom_snapshot::NodeId;

/// Animated property values to overlay on an element's effective style.
pub type AnimatedOverrides = HashMap<NodeId, HashMap<String, CssValue>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::layouter::css_resolver::KeyframesRegistry;
    use crate::engine::layouter::types::ColorScheme;

    #[test]
    fn sample_defaults_to_idle() {
        let mut runtime = AnimationRuntime::new();
        let inputs = AnimationInputs::new();
        let kf = KeyframesRegistry::new();
        let result = runtime.sample(0.0, true, &inputs, &kf, ColorScheme::Light);
        assert!(!result.active);
        assert!(!result.dirty);
        assert!(result.overrides.is_empty());
    }
}
