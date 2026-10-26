//! [`WindowBackend`] implementation for macOS.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use accessibility_sys::{
    kAXErrorAPIDisabled, kAXErrorCannotComplete, kAXFocusedApplicationAttribute, kAXFocusedWindowAttribute,
    kAXFrontmostAttribute, kAXMainAttribute, kAXMinimizedAttribute, kAXPositionAttribute, kAXRaiseAction,
    kAXSizeAttribute, kAXStandardWindowSubrole, kAXSubroleAttribute, kAXTitleAttribute, kAXWindowsAttribute, AXError,
};

use super::ax::{self, as_bool, as_point, as_size, as_string, to_backend_error, AxElement};
use super::observer::{AppObserver, HandlerSlot};
use super::window_list;
use crate::backend::{BackendError, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::{Point, Rect, Size};

/// A failed call that took at least this long ran into the messaging
/// timeout, so the app is busy rather than refusing the request.
const TIMED_OUT: Duration = Duration::from_millis((ax::MESSAGING_TIMEOUT * 800.0) as u64);
/// How long to leave a busy app alone before asking it again.
const BACK_OFF: Duration = Duration::from_secs(2);
/// What `windows()` reads of each window, in one round trip. The subrole and
/// minimized state are not always needed, but asking costs nothing extra.
const WINDOW_ATTRIBUTES: [&str; 5] =
    [kAXPositionAttribute, kAXSizeAttribute, kAXTitleAttribute, kAXSubroleAttribute, kAXMinimizedAttribute];

struct CachedWindow {
    element: AxElement,
    pid: i32,
    /// The subrole and resizability never change, so they are read once per window.
    is_standard: bool,
    is_resizable: bool,
}

#[derive(Default)]
pub struct AxBackend {
    /// AX handles for the windows seen by the last `windows()` call.
    elements: RefCell<HashMap<WindowId, CachedWindow>>,
    /// Apps that timed out, and when to try them again. Calls to them fail
    /// at once until then, so one hung app cannot stall every frame.
    stalled: RefCell<HashMap<i32, Instant>>,
    /// Apps left out of the last `windows()` call, busy or unchanged.
    skipped: RefCell<Vec<i32>>,
    /// Each app's on-screen windows when its windows were last read. An app
    /// whose set is unchanged and that sent no notification is not read again.
    seen: RefCell<HashMap<i32, BTreeSet<WindowId>>>,
    /// Application elements, kept between calls.
    apps: RefCell<HashMap<i32, AxElement>>,
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

    fn app(&self, pid: i32) -> Option<AxElement> {
        if let Some(app) = self.apps.borrow().get(&pid) {
            return Some(app.clone());
        }
        let app = AxElement::application(pid)?;
        self.apps.borrow_mut().insert(pid, app.clone());
        Some(app)
    }

    /// Notes that `pid`'s windows were just written, so the next `windows()`
    /// reads where they really went.
    fn wrote(&self, pid: i32, result: Result<()>) -> Result<()> {
        if result.is_ok() {
            self.handler.mark(pid);
        }
        result
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
        let mut apps: BTreeMap<i32, (&str, BTreeSet<WindowId>)> = BTreeMap::new();
        for (&id, w) in &on_screen {
            if w.pid != own_pid {
                apps.entry(w.pid).or_insert((&w.owner_name, BTreeSet::new())).1.insert(id);
            }
        }

        let present: BTreeSet<i32> = apps.keys().copied().collect();
        // The user drags windows of the front app without it notifying, so
        // it is always read.
        // shortcut: other apps' titles go stale until they notify; observe
        // title changes if the switcher shows outdated titles.
        let front = on_screen.values().filter(|w| w.pid != own_pid).min_by_key(|w| w.z_order).map(|w| w.pid);
        let everything = self.handler.everything.replace(false);
        let changed = std::mem::take(&mut *self.handler.changed.borrow_mut());
        let mut seen = std::mem::take(&mut *self.seen.borrow_mut());
        let previous = std::mem::take(&mut *self.elements.borrow_mut());
        let mut observers = self.observers.borrow_mut();
        let mut elements = HashMap::new();
        let mut windows = Vec::new();
        let mut skipped = Vec::new();
        for (pid, (app_name, ids)) in apps {
            if self.is_stalled(pid) {
                // It may change unnoticed while busy, so it is read once back.
                seen.remove(&pid);
                skipped.push(pid);
                continue;
            }
            // Opening, closing, hiding, minimizing or moving a window to
            // another Space changes the app's on-screen set or notifies.
            if !everything && Some(pid) != front && !changed.contains(&pid) && seen.get(&pid) == Some(&ids) {
                skipped.push(pid);
                continue;
            }
            seen.remove(&pid);
            let Some(app) = self.app(pid) else { continue };
            let app_windows = match self.timed(pid, || app.elements(kAXWindowsAttribute)) {
                Ok(windows) => windows,
                Err(err) if err == kAXErrorAPIDisabled => {
                    // What was taken above is lost, so read everything next time.
                    self.handler.everything.set(true);
                    return Err(BackendError::NotTrusted);
                }
                Err(_) => {
                    if self.is_stalled(pid) {
                        skipped.push(pid);
                    }
                    // Apps without Accessibility support are skipped.
                    continue;
                }
            };
            let mut complete = true;
            for element in app_windows {
                let Some(id) = element.window_id() else { continue };
                let values = match self.timed(pid, || element.values(&WINDOW_ATTRIBUTES)) {
                    Ok(values) => values,
                    Err(_) if self.is_stalled(pid) => {
                        complete = false;
                        break;
                    }
                    // The window closed meanwhile.
                    Err(_) => continue,
                };
                let [position, size, title, subrole, minimized] = &values[..] else { continue };
                // Windows on other Spaces are neither on screen nor minimized.
                let (z_order, is_minimized) = match on_screen.get(&id) {
                    Some(w) => (w.z_order, false),
                    None if minimized.as_ref().and_then(as_bool).unwrap_or(false) => (usize::MAX, true),
                    None => continue,
                };
                let (is_standard, is_resizable) = match previous.get(&id) {
                    Some(cached) => (cached.is_standard, cached.is_resizable),
                    None => (
                        subrole.as_ref().and_then(as_string).as_deref() == Some(kAXStandardWindowSubrole),
                        // Only a definite "no" floats a window; a busy app keeps it tiled.
                        element.is_settable(kAXSizeAttribute).unwrap_or(true),
                    ),
                };
                let origin = position.as_ref().and_then(as_point).unwrap_or_default();
                let size = size.as_ref().and_then(as_size).unwrap_or_default();
                let info = WindowInfo {
                    id,
                    pid,
                    app_name: app_name.to_owned(),
                    title: title.as_ref().and_then(as_string).unwrap_or_default(),
                    frame: Rect::new(origin.x, origin.y, size.width, size.height),
                    is_standard,
                    is_resizable,
                    is_minimized,
                };
                if let Some(observer) = observers.get_mut(&pid) {
                    observer.watch_window(id, &element);
                }
                windows.push((z_order, info));
                elements.insert(id, CachedWindow { element, pid, is_standard, is_resizable });
            }
            if complete {
                seen.insert(pid, ids);
            } else {
                skipped.push(pid);
            }
        }
        // Skipped apps' windows still exist; keep their handles for later.
        for (id, cached) in previous {
            if skipped.contains(&cached.pid) {
                elements.entry(id).or_insert(cached);
            }
        }
        // Forget apps that left the screen, so they are read when they return.
        seen.retain(|pid, _| present.contains(pid));
        for observer in observers.values_mut() {
            observer.retain_windows(|id| elements.contains_key(&id));
        }
        windows.sort_by_key(|(z, w)| (*z, w.id));
        *self.elements.borrow_mut() = elements;
        *self.seen.borrow_mut() = seen;
        *self.skipped.borrow_mut() = skipped;
        Ok(windows.into_iter().map(|(_, w)| w).collect())
    }

    fn skipped_apps(&self) -> Vec<i32> {
        self.skipped.borrow().clone()
    }

    fn invalidate(&self) {
        self.handler.everything.set(true);
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
        self.wrote(pid, result.map_err(|err| self.forget_on_error(err, id)))
    }

    fn set_position(&self, id: WindowId, x: f64, y: f64) -> Result<()> {
        let (element, pid) = self.element(id)?;
        let result = self.timed(pid, || element.set_point(kAXPositionAttribute, Point { x, y }));
        self.wrote(pid, result.map_err(|err| self.forget_on_error(err, id)))
    }

    fn set_size(&self, id: WindowId, width: f64, height: f64) -> Result<()> {
        let (element, pid) = self.element(id)?;
        let result = self.timed(pid, || element.set_size(kAXSizeAttribute, Size { width, height }));
        self.wrote(pid, result.map_err(|err| self.forget_on_error(err, id)))
    }

    fn focused_window(&self) -> Option<WindowId> {
        let app = AxElement::system_wide().element(kAXFocusedApplicationAttribute)?;
        let pid = app.pid()?;
        if self.is_stalled(pid) {
            return None;
        }
        self.timed(pid, || app.element(kAXFocusedWindowAttribute).ok_or(kAXErrorCannotComplete)).ok()?.window_id()
    }

    fn focused_app(&self) -> Option<i32> {
        AxElement::system_wide().element(kAXFocusedApplicationAttribute)?.pid()
    }

    fn focus(&self, id: WindowId) -> Result<()> {
        let (element, pid) = self.element(id)?;
        let app = self.app(pid).ok_or(BackendError::WindowNotFound(id))?;
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
        let Some(app) = self.app(pid) else { return };
        let context: *const HandlerSlot = &*self.handler;
        // SAFETY: `handler` is boxed and outlives `observers` (field order).
        if let Some(observer) = unsafe { AppObserver::new(pid, &app, context) } {
            self.observers.borrow_mut().insert(pid, observer);
            // Windows it opened before it could be observed went unnoticed.
            self.handler.mark(pid);
        }
    }

    fn forget_app(&self, pid: i32) {
        self.observers.borrow_mut().remove(&pid);
        self.stalled.borrow_mut().remove(&pid);
        self.apps.borrow_mut().remove(&pid);
        self.seen.borrow_mut().remove(&pid);
    }

    fn set_change_handler(&self, handler: Box<dyn Fn()>) {
        *self.handler.handler.borrow_mut() = Some(handler);
    }
}
