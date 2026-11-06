//! # maker_render — renderizador de escena (wgpu)

pub mod renderer;

pub use renderer::{FrameStats, RenderOptions, RenderView, SceneRenderer, COLOR_FORMAT};
pub use wgpu;
