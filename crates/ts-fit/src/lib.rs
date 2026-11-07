//! ThreadSmith garment fitting and auto skinning.
//!
//! - [`bvh`]: fast "closest point on a mesh" search.
//! - [`fit`]: line a garment up with a body and fit it to the body's surface.
//! - [`skin`]: copy skin weights from the body onto the garment.
//! - [`sculpt`]: brushes (grab, relax, inflate) and ray picking.
//! - [`graph`]: vertex neighbours, with split vertices (UV seams) joined.

pub mod bvh;
pub mod fit;
pub mod graph;
pub mod sculpt;
pub mod skin;

pub use bvh::{ClosestHit, TriBvh};
pub use fit::{
    align_to_body, fit_to_body, grow_to_clear, inside_fraction, push_out, shrink_wrap, FitSettings,
};
pub use graph::WeldGraph;
pub use skin::{transfer_skin, SkinReport, SkinSettings};
