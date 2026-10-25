//! An in-memory [`WindowBackend`] for tests and non-macOS builds.

use std::cell::{Cell, RefCell};

use crate::backend::{BackendError, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::Rect;

#[derive(Debug, Default)]
pub struct MockBackend {
    windows: RefCell<Vec<WindowInfo>>,
    focused: Cell<Option<WindowId>>,
    set_frame_calls: Cell<usize>,
    trusted: bool,
    /// Like macOS, `focus` returns before focus has actually moved.
    focus_is_async: Cell<bool>,
    /// Apps whose windows `windows` leaves out, as if they were busy.
    unresponsive: RefCell<Vec<i32>>,
}

impl MockBackend {
    pub fn new(windows: Vec<WindowInfo>) -> Self {
        Self { windows: RefCell::new(windows), trusted: true, ..Self::default() }
    }

    pub fn untrusted() -> Self {
        Self::default()
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

    pub fn add(&self, window: WindowInfo) {
        self.windows.borrow_mut().push(window);
    }

    pub fn remove(&self, id: WindowId) {
        self.windows.borrow_mut().retain(|w| w.id != id);
    }

    pub fn update(&self, id: WindowId, f: impl FnOnce(&mut WindowInfo)) {
        if let Some(w) = self.windows.borrow_mut().iter_mut().find(|w| w.id == id) {
            f(w);
        }
    }

    /// Simulates the user focusing a window, e.g. by clicking it.
    pub fn set_focused(&self, id: Option<WindowId>) {
        self.focused.set(id);
    }

    pub fn set_focus_is_async(&self, value: bool) {
        self.focus_is_async.set(value);
    }

    /// Makes `windows` skip the app `pid`'s windows and report it as busy.
    pub fn set_unresponsive(&self, pids: Vec<i32>) {
        *self.unresponsive.borrow_mut() = pids;
    }

    pub fn frame_of(&self, id: WindowId) -> Option<Rect> {
        self.windows.borrow().iter().find(|w| w.id == id).map(|w| w.frame)
    }

    pub fn set_frame_calls(&self) -> usize {
        self.set_frame_calls.get()
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
        let busy = self.unresponsive.borrow();
        Ok(self.windows.borrow().iter().filter(|w| !busy.contains(&w.pid)).cloned().collect())
    }

    fn unresponsive_apps(&self) -> Vec<i32> {
        self.unresponsive.borrow().clone()
    }

    fn set_frame(&self, id: WindowId, frame: Rect) -> Result<()> {
        if !self.trusted {
            return Err(BackendError::NotTrusted);
        }
        self.set_frame_calls.set(self.set_frame_calls.get() + 1);
        let mut windows = self.windows.borrow_mut();
        let window = windows.iter_mut().find(|w| w.id == id).ok_or(BackendError::WindowNotFound(id))?;
        if self.unresponsive.borrow().contains(&window.pid) {
            return Err(BackendError::Platform("app is not responding".into()));
        }
        window.frame = frame;
        Ok(())
    }

    fn set_position(&self, id: WindowId, x: f64, y: f64) -> Result<()> {
        let size = self.frame_of(id).ok_or(BackendError::WindowNotFound(id))?;
        self.set_frame(id, Rect { x, y, ..size })
    }

    fn set_size(&self, id: WindowId, width: f64, height: f64) -> Result<()> {
        let origin = self.frame_of(id).ok_or(BackendError::WindowNotFound(id))?;
        self.set_frame(id, Rect { width, height, ..origin })
    }

    fn focused_window(&self) -> Option<WindowId> {
        self.focused.get()
    }

    fn focus(&self, id: WindowId) -> Result<()> {
        if !self.windows.borrow().iter().any(|w| w.id == id) {
            return Err(BackendError::WindowNotFound(id));
        }
        if !self.focus_is_async.get() {
            self.focused.set(Some(id));
        }
        Ok(())
    }
}
