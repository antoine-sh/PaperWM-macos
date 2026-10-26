//! An in-memory [`WindowBackend`] for tests and non-macOS builds.

use std::cell::{Cell, RefCell};

use crate::backend::{BackendError, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::Rect;

#[derive(Debug, Default)]
pub struct MockBackend {
    windows: RefCell<Vec<WindowInfo>>,
    focused: Cell<Option<WindowId>>,
    set_frame_calls: Cell<usize>,
    focus_queries: Cell<usize>,
    trusted: bool,
    /// Like macOS, `focus` returns before focus has actually moved.
    focus_is_async: Cell<bool>,
    /// Apps whose windows `windows` leaves out, as if they were busy.
    unresponsive: RefCell<Vec<i32>>,
    /// Background tabs. Like macOS, a tab group is the windows of one app
    /// sharing a frame, of which only the shown one is on screen.
    hidden_tabs: RefCell<Vec<WindowId>>,
    /// Every window `focus` raised, oldest first; the last is frontmost.
    raised: RefCell<Vec<WindowId>>,
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
            tabs: Vec::new(),
        }
    }

    pub fn add(&self, window: WindowInfo) {
        self.windows.borrow_mut().push(window);
    }

    pub fn remove(&self, id: WindowId) {
        self.leave_tab_group(id);
        self.windows.borrow_mut().retain(|w| w.id != id);
    }

    /// Opens tab `id` in front of `shown` in its tab group.
    pub fn add_tab(&self, shown: WindowId, id: WindowId) {
        let Some(mut tab) = self.windows.borrow().iter().find(|w| w.id == shown).cloned() else { return };
        tab.id = id;
        tab.title = format!("Window {id}");
        self.hidden_tabs.borrow_mut().push(shown);
        self.add(tab);
    }

    /// Brings the background tab `id` to the front of its group.
    pub fn select_tab(&self, id: WindowId) {
        let mut hidden = self.hidden_tabs.borrow_mut();
        let shown: Vec<WindowId> = self.tab_group(id).into_iter().filter(|t| !hidden.contains(t)).collect();
        hidden.extend(shown);
        hidden.retain(|&t| t != id);
    }

    /// Drags tab `id` out of its group into a window of its own at `frame`.
    pub fn detach_tab(&self, id: WindowId, frame: Rect) {
        self.leave_tab_group(id);
        self.update(id, |w| w.frame = frame);
    }

    /// The other tabs of `id`'s group: the app's other windows at its frame.
    fn tab_group(&self, id: WindowId) -> Vec<WindowId> {
        let windows = self.windows.borrow();
        let Some(tab) = windows.iter().find(|w| w.id == id) else { return Vec::new() };
        windows.iter().filter(|w| w.id != id && w.pid == tab.pid && w.frame == tab.frame).map(|w| w.id).collect()
    }

    /// Takes `id` out of its group; if it was shown, the next tab comes forward.
    fn leave_tab_group(&self, id: WindowId) {
        let was_shown = !self.hidden_tabs.borrow().contains(&id);
        let next = self.tab_group(id).first().copied();
        self.hidden_tabs.borrow_mut().retain(|&t| t != id && !(was_shown && Some(t) == next));
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

    pub fn raised(&self) -> Vec<WindowId> {
        self.raised.borrow().clone()
    }

    /// How often `focused_window` was asked, each two round trips on macOS.
    pub fn focus_queries(&self) -> usize {
        self.focus_queries.get()
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
        let hidden = self.hidden_tabs.borrow();
        let windows = self.windows.borrow();
        let shown = windows.iter().filter(|w| !busy.contains(&w.pid) && !hidden.contains(&w.id));
        let tabs = |id| self.tab_group(id).into_iter().filter(|t| hidden.contains(t)).collect();
        Ok(shown.map(|w| WindowInfo { tabs: tabs(w.id), ..w.clone() }).collect())
    }

    fn skipped_apps(&self) -> Vec<i32> {
        self.unresponsive.borrow().clone()
    }

    fn set_frame(&self, id: WindowId, frame: Rect) -> Result<()> {
        if !self.trusted {
            return Err(BackendError::NotTrusted);
        }
        self.set_frame_calls.set(self.set_frame_calls.get() + 1);
        let mut windows = self.windows.borrow_mut();
        let window = windows.iter().find(|w| w.id == id).ok_or(BackendError::WindowNotFound(id))?;
        if self.unresponsive.borrow().contains(&window.pid) {
            return Err(BackendError::Platform("app is not responding".into()));
        }
        // Background tabs move with the shown one.
        let (pid, old) = (window.pid, window.frame);
        let hidden = self.hidden_tabs.borrow();
        for w in windows.iter_mut().filter(|w| w.id == id || (w.pid == pid && w.frame == old && hidden.contains(&w.id)))
        {
            w.frame = frame;
        }
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
        self.focus_queries.set(self.focus_queries.get() + 1);
        self.focused.get()
    }

    fn focused_app(&self) -> Option<i32> {
        let id = self.focused.get()?;
        self.windows.borrow().iter().find(|w| w.id == id).map(|w| w.pid)
    }

    fn focus(&self, id: WindowId) -> Result<()> {
        if !self.windows.borrow().iter().any(|w| w.id == id) {
            return Err(BackendError::WindowNotFound(id));
        }
        self.raised.borrow_mut().push(id);
        if !self.focus_is_async.get() {
            self.focused.set(Some(id));
        }
        Ok(())
    }
}
