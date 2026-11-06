//! # maker_sculpt — esculpido digital sin interfaz
//!
//! Malla esculpible (estructura de arrays con mapa a los vértices de render), BVH con hojas
//! de vértices contiguos, trazo (espaciado, cuerda, línea recta, filtrado del punto), caída,
//! pinceles, simetría, máscara, deshacer por trazo y escritura a `.glb`.
//!
//! El editor solo pasa la entrada del puntero a un [`Controller`], llama a
//! [`Sculpt::flush`] una vez por fotograma y sube a la GPU los rangos de
//! [`Sculpt::take_gpu_ranges`].

pub mod brush;
pub mod bvh;
pub mod falloff;
pub mod glb;
pub mod mesh;
pub mod primitives;
pub mod sculpt;
pub mod stroke;
pub mod symmetry;
pub mod undo;

pub use brush::{Brush, BrushSettings};
pub use mesh::SculptMesh;
pub use sculpt::{Action, Dab, DabOptions, Hit, Sculpt};
pub use stroke::{Controller, CursorInfo, PointerState, ToolSettings, ViewParams};
pub use symmetry::{Axis, Symmetry};
