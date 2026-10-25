//! The boundary between the engine and the operating system.
//!
//! Everything platform specific lives behind [`WindowBackend`], so the layout
//! logic can be tested on any machine with [`crate::mock::MockBackend`].

use std::fmt;

use crate::geometry::Rect;

/// The window server's id for a window (`CGWindowID` on macOS).
pub type WindowId = u32;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub pid: i32,
    pub app_name: String,
    pub title: String,
    pub frame: Rect,
    /// A regular document window, as opposed to a dialog, sheet, panel or popup.
    pub is_standard: bool,
    pub is_minimized: bool,
}

impl WindowInfo {
    /// Whether the window should take part in tiling.
    pub fn is_tileable(&self) -> bool {
        self.is_standard && !self.is_minimized
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The app has not been granted Accessibility permission.
    NotTrusted,
    WindowNotFound(WindowId),
    Platform(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTrusted => write!(f, "Accessibility permission has not been granted"),
            Self::WindowNotFound(id) => write!(f, "window {id} not found"),
            Self::Platform(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for BackendError {}

pub type Result<T> = std::result::Result<T, BackendError>;

pub trait WindowBackend {
    /// Whether the process may control other apps' windows.
    fn is_trusted(&self) -> bool;

    /// All windows of regular apps on the current Space, front to back.
    fn windows(&self) -> Result<Vec<WindowInfo>>;

    /// Apps that did not answer during the last [`windows`](Self::windows)
    /// call. Their windows are missing from it but still exist.
    fn unresponsive_apps(&self) -> Vec<i32> {
        Vec::new()
    }

    fn set_frame(&self, id: WindowId, frame: Rect) -> Result<()>;

    /// Moves `id` without resizing it.
    fn set_position(&self, id: WindowId, x: f64, y: f64) -> Result<()>;

    /// Resizes `id` without moving it.
    fn set_size(&self, id: WindowId, width: f64, height: f64) -> Result<()>;

    /// The window with keyboard focus, if it belongs to a regular app.
    fn focused_window(&self) -> Option<WindowId>;

    /// Gives `id` keyboard focus and raises it.
    fn focus(&self, id: WindowId) -> Result<()>;

    /// Starts watching the app `pid` for window changes.
    fn observe_app(&self, _pid: i32) {}

    fn forget_app(&self, _pid: i32) {}

    /// Called, outside any engine call, when an observed app's windows
    /// change. It should schedule a sync rather than sync directly.
    fn set_change_handler(&self, _handler: Box<dyn Fn()>) {}
}
