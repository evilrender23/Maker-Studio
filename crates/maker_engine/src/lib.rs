//! # maker_engine — núcleo de Maker Studio
//!
//! ECS (hecs), escena serializable, jerarquía de transforms, física (Rapier),
//! scripting en Rust con recarga en caliente y proyectos.

pub mod animation;
pub mod biblioteca;
pub mod components;
pub mod convexo;
pub mod cursor;
pub mod input;
pub mod log;
pub mod math;
pub mod mesh;
pub mod model;
pub mod physics;
pub mod postproceso;
pub mod project;
pub mod runtime;
pub mod scene;
pub mod scripting;

pub use components::*;
pub use cursor::{CursorCapture, CursorMode};
pub use glam;
pub use hecs;
pub use input::{Input, Key};
pub use project::Project;
pub use runtime::Runtime;
pub use scene::{Environment, Hierarchy, Scene};
pub use scripting::ScriptLib;
