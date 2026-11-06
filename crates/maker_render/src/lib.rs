//! # maker_render — renderizador de escena (wgpu)

pub mod miniatura;
pub mod renderer;
pub mod texture;

pub use renderer::{FrameStats, RenderOptions, RenderView, SceneRenderer, COLOR_FORMAT};
pub use wgpu;
