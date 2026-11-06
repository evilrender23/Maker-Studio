//! # maker_render — renderizador de escena (wgpu)

pub mod miniatura;
pub mod postproceso;
pub mod renderer;
pub mod texture;
pub mod ui;

#[cfg(test)]
mod estilo_visual_tests;

pub use renderer::{FrameStats, NormalInfo, RenderOptions, RenderView, SceneRenderer, COLOR_FORMAT};
pub use ui::{ObjetivoUi, UiRenderer};
pub use wgpu;
