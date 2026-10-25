//! [`WindowBackend`] implementation for macOS.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use accessibility_sys::{
    kAXErrorAPIDisabled, kAXErrorCannotComplete, kAXFocusedApplicationAttribute, kAXFocusedWindowAttribute,
    kAXFrontmostAttribute, kAXMainAttribute, kAXMinimizedAttribute, kAXPositionAttribute, kAXRaiseAction,
    kAXSizeAttribute, kAXStandardWindowSubrole, kAXSubroleAttribute, kAXTitleAttribute, kAXWindowsAttribute, AXError,
};

use super::ax::{self, to_backend_error, AxElement};
use super::observer::{AppObserver, HandlerSlot};
use super::window_list;
use crate::backend::{BackendError, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::{Point, Rect, Size};

/// A failed call that took at least this long ran into the messaging
/// timeout, so the app is busy rather than refusing the request.
const TIMED_OUT: Duration = Duration::from_millis((ax::MESSAGING_TIMEOUT * 800.0) as u64);
/// How long to leave a busy app alone before asking it again.
const BACK_OFF: Duration = Duration::from_secs(2);

struct CachedWindow {
    element: AxElement,
    pid: i32,
    /// The subrole never changes, so it is read once per window.
    is_standard: bool,
}

#[derive(Default)]
pub struct AxBackend {
    /// AX handles for the windows seen by the last `windows()` call.
    elements: RefCell<HashMap<WindowId, CachedWindow>>,
    /// Apps that timed out, and when to try them again. Calls to them fail
    /// at once until then, so one hung app cannot stall every frame.
    stalled: RefCell<HashMap<i32, Instant>>,
    /// Apps left out of the last `windows()` call because they were busy.
    unresponsive: RefCell<Vec<i32>>,
    /// Declared before `handler` so observers are torn down first.
    observers: RefCell<HashMap<i32, AppObserver>>,
    handler: Box<HandlerSlot>,
}

impl AxBackend {
    pub fn new() -> Self {
        ax::set_global_timeout();
        Self::default()
    }

    /// The cached handle and app of `id`. Windows are only found by
    /// `windows()`; a window it has not seen yet is reported as missing and
    /// picked up by the next sync.
    fn element(&self, id: WindowId) -> Result<(AxElement, i32)> {
        let elements = self.elements.borrow();
        let window = elements.get(&id).ok_or(BackendError::WindowNotFound(id))?;
        if self.is_stalled(window.pid) {
            return Err(BackendError::Platform(format!("app {} is not responding", window.pid)));
        }
        Ok((window.element.clone(), window.pid))
    }

    fn is_stalled(&self, pid: i32) -> bool {
        let mut stalled = self.stalled.borrow_mut();
        match stalled.get(&pid) {
            Some(&until) if Instant::now() < until => true,
            Some(_) => {
                stalled.remove(&pid);
                false
            }
            None => false,
        }
    }

    /// Runs an Accessibility call to app `pid`, backing off from the app if
    /// the call timed out.
    fn timed<T>(
        &self,
        pid: i32,
        call: impl FnOnce() -> std::result::Result<T, AXError>,
    ) -> std::result::Result<T, AXError> {
        let start = Instant::now();
        let result = call();
        if result.as_ref().is_err_and(|&err| err == kAXErrorCannotComplete) && start.elapsed() >= TIMED_OUT {
            self.stalled.borrow_mut().insert(pid, Instant::now() + BACK_OFF);
        }
        result
    }

    /// Maps an AX error, dropping the cached handle of a window that is gone.
    fn forget_on_error(&self, err: AXError, id: WindowId) -> BackendError {
        let err = to_backend_error(err, id);
        if err == BackendError::WindowNotFound(id) {
            self.elements.borrow_mut().remove(&id);
        }
        err
    }
}

impl WindowBackend for AxBackend {
    fn is_trusted(&self) -> bool {
        super::is_trusted(false)
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        let on_screen = window_list::on_screen_windows();
        let own_pid = std::process::id() as i32;
        // Apps without on-screen windows only have minimized windows or
        // windows on other Spaces, none of which are tiled.
        let mut names: HashMap<i32, &str> = HashMap::new();
        for w in on_screen.values() {
            names.entry(w.pid).or_insert(&w.owner_name);
        }
        let pids: BTreeSet<i32> = names.keys().copied().filter(|&pid| pid != own_pid).collect();

        let previous = std::mem::take(&mut *self.elements.borrow_mut());
        let mut observers = self.observers.borrow_mut();
        let mut elements = HashMap::new();
        let mut windows = Vec::new();
        let mut unresponsive = Vec::new();
        for pid in pids {
            if self.is_stalled(pid) {
                unresponsive.push(pid);
                continue;
            }
            let Some(app) = AxElement::application(pid) else { continue };
            let app_windows = match self.timed(pid, || app.elements(kAXWindowsAttribute)) {
                Ok(windows) => windows,
                Err(err) if err == kAXErrorAPIDisabled => return Err(BackendError::NotTrusted),
                Err(_) => {
                    if self.is_stalled(pid) {
                        unresponsive.push(pid);
                    }
                    // Apps without Accessibility support are skipped.
                    continue;
                }
            };
            let app_name = names.get(&pid).copied().unwrap_or_default();
            for element in app_windows {
                let Some(id) = element.window_id() else { continue };
                // Windows on other Spaces are neither on screen nor minimized.
                let (z_order, is_minimized) = match on_screen.get(&id) {
                    Some(w) => (w.z_order, false),
                    None if element.bool(kAXMinimizedAttribute).unwrap_or(false) => (usize::MAX, true),
                    None => continue,
                };
                let is_standard = match previous.get(&id) {
                    Some(cached) => cached.is_standard,
                    None => element.string(kAXSubroleAttribute).as_deref() == Some(kAXStandardWindowSubrole),
                };
                let origin = element.point(kAXPositionAttribute).unwrap_or_default();
                let size = element.size(kAXSizeAttribute).unwrap_or_default();
                let info = WindowInfo {
                    id,
                    pid,
                    app_name: app_name.to_owned(),
                    title: element.string(kAXTitleAttribute).unwrap_or_default(),
                    frame: Rect::new(origin.x, origin.y, size.width, size.height),
                    is_standard,
                    is_minimized,
                };
                if let Some(observer) = observers.get_mut(&pid) {
                    observer.watch_window(id, &element);
                }
                windows.push((z_order, info));
                elements.insert(id, CachedWindow { element, pid, is_standard });
            }
        }
        // Busy apps' windows still exist; keep their handles for later.
        for (id, cached) in previous {
            if unresponsive.contains(&cached.pid) {
                elements.entry(id).or_insert(cached);
            }
        }
        for observer in observers.values_mut() {
            observer.retain_windows(|id| elements.contains_key(&id));
        }
        windows.sort_by_key(|(z, w)| (*z, w.id));
        *self.elements.borrow_mut() = elements;
        *self.unresponsive.borrow_mut() = unresponsive;
        Ok(windows.into_iter().map(|(_, w)| w).collect())
    }

    fn unresponsive_apps(&self) -> Vec<i32> {
        self.unresponsive.borrow().clone()
    }

    fn set_frame(&self, id: WindowId, frame: Rect) -> Result<()> {
        let (element, pid) = self.element(id)?;
        let size = Size { width: frame.width, height: frame.height };
        let origin = Point { x: frame.x, y: frame.y };
        // Resizing first lets the move succeed when the old size would not fit
        // at the new position; resizing again fixes the size when the move
        // crossed onto a display that clamped it.
        let result = self.timed(pid, || {
            element
                .set_size(kAXSizeAttribute, size)
                .and_then(|()| element.set_point(kAXPositionAttribute, origin))
                .and_then(|()| element.set_size(kAXSizeAttribute, size))
        });
        result.map_err(|err| self.forget_on_error(err, id))
    }

    fn set_position(&self, id: WindowId, x: f64, y: f64) -> Result<()> {
        let (element, pid) = self.element(id)?;
        self.timed(pid, || element.set_point(kAXPositionAttribute, Point { x, y }))
            .map_err(|err| self.forget_on_error(err, id))
    }

    fn set_size(&self, id: WindowId, width: f64, height: f64) -> Result<()> {
        let (element, pid) = self.element(id)?;
        self.timed(pid, || element.set_size(kAXSizeAttribute, Size { width, height }))
            .map_err(|err| self.forget_on_error(err, id))
    }

    fn focused_window(&self) -> Option<WindowId> {
        let app = AxElement::system_wide().element(kAXFocusedApplicationAttribute)?;
        let pid = app.pid()?;
        if self.is_stalled(pid) {
            return None;
        }
        self.timed(pid, || app.element(kAXFocusedWindowAttribute).ok_or(kAXErrorCannotComplete)).ok()?.window_id()
    }

    fn focus(&self, id: WindowId) -> Result<()> {
        let (element, pid) = self.element(id)?;
        let app = AxElement::application(pid).ok_or(BackendError::WindowNotFound(id))?;
        // Making the window main before activating the app keeps macOS from
        // bringing a different window of the same app forward.
        self.timed(pid, || {
            let _ = element.set_true(kAXMainAttribute);
            let _ = app.set_true(kAXFrontmostAttribute);
            element.perform(kAXRaiseAction)
        })
        .map_err(|err| to_backend_error(err, id))
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
        self.stalled.borrow_mut().remove(&pid);
    }

    fn set_change_handler(&self, handler: Box<dyn Fn()>) {
        *self.handler.borrow_mut() = Some(handler);
    }
}
