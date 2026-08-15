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
mod table_layout;
pub mod text_layouter;
pub mod types;

pub use builder::{
    InheritedCss, build_layout_and_info, build_layout_and_info_from_snapshot,
    build_layout_and_info_with_images, constrain_auto_grid_track_items,
    correct_atomic_inline_spacing, correct_atomic_inline_spacing_with_info,
    correct_oversized_auto_horizontal_margins, normalize_whitespace,
    refresh_missing_text_layout_results,
};
pub use dom_snapshot::{DomSnapshot, NodeId, SnapNode};
pub use processor::{LayoutProcessor, LayoutResult, LayoutTask};
pub use table_layout::align_table_columns;
