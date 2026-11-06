//! # maker_render — renderizador de escena (wgpu)

pub mod miniatura;
pub mod postproceso;
pub mod renderer;
pub mod texture;
pub mod ui;

pub use renderer::{FrameStats, RenderOptions, RenderView, SceneRenderer, COLOR_FORMAT};
pub use ui::{ObjetivoUi, UiRenderer};
pub use wgpu;
