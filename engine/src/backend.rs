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
    /// False for fixed-size windows such as Calculator, which float like
    /// dialogs because a column would only stretch or squeeze them.
    pub is_resizable: bool,
    pub is_minimized: bool,
    /// In macOS native fullscreen, on a Space of its own. macOS owns its
    /// frame until it leaves, so it keeps its column but is not moved.
    pub is_fullscreen: bool,
    /// The background tabs of this window's native tab group (Finder,
    /// Terminal, Safari). macOS makes each tab a window, but only the shown
    /// one is listed and tiled; the others hide behind it at the same frame.
    pub tabs: Vec<WindowId>,
}

impl WindowInfo {
    /// Whether the window should take part in tiling.
    pub fn is_tileable(&self) -> bool {
        self.is_standard && self.is_resizable && !self.is_minimized
    }
}

/// Why a window reported before has left for good. A window that is merely
/// missing from a read (another Space, a busy app, the lock screen) has not
/// departed and keeps its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Departure {
    /// Its Accessibility element is no longer valid.
    Destroyed,
    AppQuit,
    /// Its app was hidden (⌘H).
    AppHidden,
    /// Minimized while its app had nothing on screen, so it was not listed.
    Minimized,
    /// Its element still answers, but it stayed away with nothing to explain
    /// it, as apps that close to the background leave their windows.
    Closed,
}

impl fmt::Display for Departure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Destroyed => "destroyed",
            Self::AppQuit => "app quit",
            Self::AppHidden => "app hidden",
            Self::Minimized => "minimized",
            Self::Closed => "closed: away with nothing to explain it",
        })
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

    /// Windows of regular apps read now, front to back: on the current
    /// Space, minimized, or in fullscreen. Background tabs are left out and
    /// listed in their shown tab's `tabs`. Windows of apps that were busy or
    /// unchanged may be missing; see [`departures`](Self::departures).
    fn windows(&self) -> Result<Vec<WindowInfo>>;

    /// Windows reported before that have left for good, and why, found since
    /// the last call. Reported windows missing from a read without
    /// departing still exist and keep their place.
    fn departures(&self) -> Vec<(WindowId, Departure)> {
        Vec::new()
    }

    /// Notes something that empties the screen for a while (a Space change,
    /// sleep, wake, lock, unlock, a display change), so windows missing
    /// meanwhile are taken for away rather than closed.
    fn note_transition(&self) {}

    /// How long until a missing window may count as closed without anything
    /// notifying, so the caller syncs again then; `None` if no window is.
    fn next_check(&self) -> Option<std::time::Duration> {
        None
    }

    /// Whether the last [`windows`](Self::windows) call met a new window
    /// that may still become tileable, not yet on screen or without its
    /// subrole. Nothing notifies when that settles, so sync again shortly.
    fn settling(&self) -> bool {
        false
    }

    /// Makes the next [`windows`](Self::windows) call read every app.
    fn invalidate(&self) {}

    fn set_frame(&self, id: WindowId, frame: Rect) -> Result<()>;

    /// Moves `id` without resizing it.
    fn set_position(&self, id: WindowId, x: f64, y: f64) -> Result<()>;

    /// Resizes `id` without moving it.
    fn set_size(&self, id: WindowId, width: f64, height: f64) -> Result<()>;

    /// The window with keyboard focus, if it belongs to a regular app.
    fn focused_window(&self) -> Option<WindowId>;

    /// The pid of the app with keyboard focus.
    fn focused_app(&self) -> Option<i32>;

    /// Gives `id` keyboard focus and raises it.
    fn focus(&self, id: WindowId) -> Result<()>;

    /// Starts watching the app `pid` for window changes.
    fn observe_app(&self, _pid: i32) {}

    fn forget_app(&self, _pid: i32) {}

    /// Called, outside any engine call, when an observed app's windows
    /// change. It should schedule a sync rather than sync directly.
    fn set_change_handler(&self, _handler: Box<dyn Fn()>) {}
}
