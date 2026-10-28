//! The PaperWM window-management engine.
//!
//! The engine owns window state and layout. The Swift app links it as a
//! static library and talks to it through the C interface in [`ffi`], whose
//! declarations live in `include/paperwm_engine.h`.

pub mod animator;
pub mod applier;
pub mod backend;
pub mod engine;
pub mod ffi;
pub mod geometry;
pub mod mock;
pub mod placement;
pub mod settle;
pub mod workspace;

#[cfg(target_os = "macos")]
pub mod macos;

pub use backend::{BackendError, WindowBackend, WindowId, WindowInfo};
pub use engine::{Command, Config, Engine};
pub use geometry::Rect;

pub(crate) mod platform {
    use crate::backend::WindowBackend;

    #[cfg(target_os = "macos")]
    pub fn default_backend() -> Box<dyn WindowBackend> {
        Box::new(crate::macos::AxBackend::new())
    }

    /// Other platforms have no windows to manage; this keeps the crate
    /// buildable and testable there.
    #[cfg(not(target_os = "macos"))]
    pub fn default_backend() -> Box<dyn WindowBackend> {
        Box::new(crate::mock::MockBackend::new(Vec::new()))
    }
}
