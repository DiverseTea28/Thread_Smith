//! ThreadSmith core types.
//!
//! Everything here uses one shared space: **Y up, right handed, meters**
//! (the same as glTF). Importers convert into this space, exporters convert out.

pub mod anim;
pub mod material;
pub mod mesh;
pub mod scene;
pub mod skeleton;

pub use anim::{AnimClip, Retargeter};
pub use material::{Material, Texture};
pub use mesh::{Mesh, Skin, MAX_INFLUENCES};
pub use scene::{Bounds, MeshPart, PartRole, Scene};
pub use skeleton::{Joint, Pose, Skeleton, Trs};
