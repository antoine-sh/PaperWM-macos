//! [`WindowBackend`] implementation for macOS.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use accessibility_sys::{
    kAXFocusedApplicationAttribute, kAXFocusedWindowAttribute, kAXFrontmostAttribute, kAXMainAttribute,
    kAXMinimizedAttribute, kAXPositionAttribute, kAXRaiseAction, kAXSizeAttribute, kAXStandardWindowSubrole,
    kAXSubroleAttribute, kAXTitleAttribute, kAXWindowsAttribute,
};

use super::ax::{to_backend_error, AxElement};
use super::observer::{AppObserver, HandlerSlot};
use super::window_list;
use crate::backend::{BackendError, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::{Point, Rect, Size};

#[derive(Default)]
pub struct AxBackend {
    /// AX handles for the windows seen by the last `windows()` call.
    elements: RefCell<HashMap<WindowId, AxElement>>,
    /// Declared before `handler` so observers are torn down first.
    observers: RefCell<HashMap<i32, AppObserver>>,
    handler: Box<HandlerSlot>,
}

impl AxBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn element(&self, id: WindowId) -> Result<AxElement> {
        if let Some(element) = self.elements.borrow().get(&id) {
            return Ok(element.clone());
        }
        self.windows()?;
        self.elements.borrow().get(&id).cloned().ok_or(BackendError::WindowNotFound(id))
    }
}

impl WindowBackend for AxBackend {
    fn is_trusted(&self) -> bool {
        super::is_trusted(false)
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        if !self.is_trusted() {
            return Err(BackendError::NotTrusted);
        }
        let on_screen = window_list::on_screen_windows();
        let own_pid = std::process::id() as i32;
        let mut observers = self.observers.borrow_mut();
        let pids: BTreeSet<i32> =
            on_screen.values().map(|w| w.pid).chain(observers.keys().copied()).filter(|&pid| pid != own_pid).collect();

        let mut elements = HashMap::new();
        let mut windows = Vec::new();
        for pid in pids {
            let Some(app) = AxElement::application(pid) else { continue };
            // Apps that do not answer (or have no AX support) are skipped.
            let Ok(app_windows) = app.elements(kAXWindowsAttribute) else { continue };
            let app_name = on_screen.values().find(|w| w.pid == pid).map(|w| w.owner_name.clone()).unwrap_or_default();
            for element in app_windows {
                let Some(id) = element.window_id() else { continue };
                let is_minimized = element.bool(kAXMinimizedAttribute).unwrap_or(false);
                // Windows on other Spaces are neither on screen nor minimized.
                let z_order = match on_screen.get(&id) {
                    Some(w) => w.z_order,
                    None if is_minimized => usize::MAX,
                    None => continue,
                };
                let origin = element.point(kAXPositionAttribute).unwrap_or_default();
                let size = element.size(kAXSizeAttribute).unwrap_or_default();
                let info = WindowInfo {
                    id,
                    pid,
                    app_name: app_name.clone(),
                    title: element.string(kAXTitleAttribute).unwrap_or_default(),
                    frame: Rect::new(origin.x, origin.y, size.width, size.height),
                    is_standard: element.string(kAXSubroleAttribute).as_deref() == Some(kAXStandardWindowSubrole),
                    is_minimized,
                };
                if let Some(observer) = observers.get_mut(&pid) {
                    observer.watch_window(id, &element);
                }
                windows.push((z_order, info));
                elements.insert(id, element);
            }
        }
        for observer in observers.values_mut() {
            observer.retain_windows(|id| elements.contains_key(&id));
        }
        windows.sort_by_key(|(z, w)| (*z, w.id));
        *self.elements.borrow_mut() = elements;
        Ok(windows.into_iter().map(|(_, w)| w).collect())
    }

    fn set_frame(&self, id: WindowId, frame: Rect) -> Result<()> {
        let element = self.element(id)?;
        let size = Size { width: frame.width, height: frame.height };
        let origin = Point { x: frame.x, y: frame.y };
        // Resizing first lets the move succeed when the old size would not fit
        // at the new position; resizing again fixes the size when the move
        // crossed onto a display that clamped it.
        let result = element
            .set_size(kAXSizeAttribute, size)
            .and_then(|()| element.set_point(kAXPositionAttribute, origin))
            .and_then(|()| element.set_size(kAXSizeAttribute, size));
        result.map_err(|err| {
            let err = to_backend_error(err, id);
            if err == BackendError::WindowNotFound(id) {
                self.elements.borrow_mut().remove(&id);
            }
            err
        })
    }

    fn focused_window(&self) -> Option<WindowId> {
        AxElement::system_wide()
            .element(kAXFocusedApplicationAttribute)?
            .element(kAXFocusedWindowAttribute)?
            .window_id()
    }

    fn focus(&self, id: WindowId) -> Result<()> {
        let element = self.element(id)?;
        let pid = element.pid().ok_or(BackendError::WindowNotFound(id))?;
        let app = AxElement::application(pid).ok_or(BackendError::WindowNotFound(id))?;
        // Making the window main before activating the app keeps macOS from
        // bringing a different window of the same app forward.
        let _ = element.set_true(kAXMainAttribute);
        let _ = app.set_true(kAXFrontmostAttribute);
        element.perform(kAXRaiseAction).map_err(|err| to_backend_error(err, id))
    }

    fn observe_app(&self, pid: i32) {
        if pid == std::process::id() as i32 || self.observers.borrow().contains_key(&pid) {
            return;
        }
        let Some(app) = AxElement::application(pid) else { return };
        let context: *const HandlerSlot = &*self.handler;
        // SAFETY: `handler` is boxed and outlives `observers` (field order).
        if let Some(observer) = unsafe { AppObserver::new(pid, &app, context) } {
            self.observers.borrow_mut().insert(pid, observer);
        }
    }

    fn forget_app(&self, pid: i32) {
        self.observers.borrow_mut().remove(&pid);
    }

    fn set_change_handler(&self, handler: Box<dyn Fn()>) {
        *self.handler.borrow_mut() = Some(handler);
    }
}
