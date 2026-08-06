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
pub mod dom_snapshot;
pub mod processor;
pub mod text_layouter;
pub mod types;

pub use builder::{
    InheritedCss, build_layout_and_info, build_layout_and_info_from_snapshot,
    build_layout_and_info_with_images,
};
pub use dom_snapshot::{DomSnapshot, NodeId, SnapNode};
pub use processor::{LayoutProcessor, LayoutResult, LayoutTask};
