//! An in-memory [`WindowBackend`] for tests and non-macOS builds.

use std::cell::RefCell;

use crate::backend::{BackendError, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::Rect;

#[derive(Debug, Default)]
pub struct MockBackend {
    windows: RefCell<Vec<WindowInfo>>,
    trusted: bool,
}

impl MockBackend {
    pub fn new(windows: Vec<WindowInfo>) -> Self {
        Self { windows: RefCell::new(windows), trusted: true }
    }

    pub fn untrusted() -> Self {
        Self { windows: RefCell::default(), trusted: false }
    }

    /// A standard, visible window with the given id and frame.
    pub fn window(id: WindowId, frame: Rect) -> WindowInfo {
        WindowInfo {
            id,
            pid: 100 + id as i32,
            app_name: format!("App {id}"),
            title: format!("Window {id}"),
            frame,
            is_standard: true,
            is_minimized: false,
        }
    }

    pub fn frame_of(&self, id: WindowId) -> Option<Rect> {
        self.windows.borrow().iter().find(|w| w.id == id).map(|w| w.frame)
    }
}

impl WindowBackend for MockBackend {
    fn is_trusted(&self) -> bool {
        self.trusted
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        if !self.trusted {
            return Err(BackendError::NotTrusted);
        }
        Ok(self.windows.borrow().clone())
    }

    fn set_frame(&self, id: WindowId, frame: Rect) -> Result<()> {
        if !self.trusted {
            return Err(BackendError::NotTrusted);
        }
        let mut windows = self.windows.borrow_mut();
        let window = windows.iter_mut().find(|w| w.id == id).ok_or(BackendError::WindowNotFound(id))?;
        window.frame = frame;
        Ok(())
    }
}
