//! Layout builder
//!
//! Converts DOM + resolved CSS into layout and render-info trees.
//!
//! Responsibilities:
//! - Style inheritance and cascade
//! - Text measurement
//! - Incremental (diff-based) update of layout/info trees
//!
//! Out of scope:
//! - Rendering
//! - Draw command generation
//! - GPU / platform concerns

mod builder;
pub mod css_resolver;
mod diff;
pub mod types;

pub use builder::build_layout_and_info;
