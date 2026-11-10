//! [`WindowBackend`] implementation for macOS.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use accessibility_sys::{
    kAXErrorAPIDisabled, kAXErrorCannotComplete, kAXErrorInvalidUIElement, kAXFocusedApplicationAttribute,
    kAXFocusedWindowAttribute, kAXFrontmostAttribute, kAXHiddenAttribute, kAXMainAttribute, kAXMinimizedAttribute,
    kAXPositionAttribute, kAXRaiseAction, kAXSizeAttribute, kAXStandardWindowSubrole, kAXSubroleAttribute,
    kAXTitleAttribute, kAXWindowsAttribute, AXError,
};

use super::ax::{self, as_bool, as_point, as_size, as_string, to_backend_error, AxElement};
use super::observer::{AppObserver, HandlerSlot};
use super::window_list;
use crate::absence::{Absence, Read};
use crate::backend::{BackendError, Departure, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::{Point, Rect, Size};
use crate::log;
use crate::settle::Settling;

/// A failed call that took at least this long ran into the messaging
/// timeout, so the app is busy rather than refusing the request.
const TIMED_OUT: Duration = Duration::from_millis((ax::MESSAGING_TIMEOUT * 800.0) as u64);
/// How long to leave a busy app alone before asking it again.
const BACK_OFF: Duration = Duration::from_secs(2);
/// Not among `accessibility_sys`'s constants.
const FULL_SCREEN_ATTRIBUTE: &str = "AXFullScreen";
/// What `windows()` reads of each window, in one round trip. The minimized
/// state and fullscreen flag are not always needed, but asking costs nothing extra.
const WINDOW_ATTRIBUTES: [&str; 6] = [
    kAXPositionAttribute,
    kAXSizeAttribute,
    kAXTitleAttribute,
    kAXSubroleAttribute,
    kAXMinimizedAttribute,
    FULL_SCREEN_ATTRIBUTE,
];

struct CachedWindow {
    element: AxElement,
    pid: i32,
    /// Latched: once a read finds the window standard, it stays standard.
    /// A window read as it is created may not have its subrole yet, and a
    /// read mid-close or mid-transition must not take it out of its column.
    is_standard: bool,
    /// Resizability never changes, so it is read once per window.
    /// `None` until the app answers, so a busy app is asked again later.
    is_resizable: Option<bool>,
    app_name: String,
    /// Its app is read even with nothing on screen, so a window on its own
    /// fullscreen Space is still reported.
    is_fullscreen: bool,
    /// Whether the last read that looked at it reported it.
    was_reported: bool,
}

#[derive(Default)]
pub struct AxBackend {
    /// AX handles for every window reported and not departed since, kept
    /// while it is away (another Space, the lock screen) to ask it directly.
    elements: RefCell<HashMap<WindowId, CachedWindow>>,
    /// Apps that timed out, and when to try them again. Calls to them fail
    /// at once until then, so one hung app cannot stall every frame.
    stalled: RefCell<HashMap<i32, Instant>>,
    /// Windows found gone since `departures()` was last called.
    departed: RefCell<Vec<(WindowId, Departure)>>,
    /// Decides when a window that still exists but stays away has closed.
    absence: RefCell<Absence>,
    /// Whether a transition was noted since the last read.
    transition: Cell<bool>,
    /// Each app's on-screen windows when its windows were last read. An app
    /// whose set is unchanged and that sent no notification is not read again.
    seen: RefCell<HashMap<i32, BTreeSet<WindowId>>>,
    /// New windows read before they were on screen or had their subrole.
    settling: RefCell<Settling>,
    /// Whether the last `windows()` call met one.
    is_settling: Cell<bool>,
    /// The last read's summary line, logged again only when it changes.
    last_summary: RefCell<String>,
    /// Each app's lagging windows last logged, so a lasting lag logs once.
    last_lagging: RefCell<HashMap<i32, Vec<WindowId>>>,
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
            if let Some(cached) = self.elements.borrow_mut().remove(&id) {
                self.depart(id, &cached, Departure::Destroyed);
            }
        }
        err
    }

    fn depart(&self, id: WindowId, cached: &CachedWindow, why: Departure) {
        log::line(format!("backend: {id} {} departed: {why}", cached.app_name));
        self.departed.borrow_mut().push((id, why));
    }

    /// Whether app `pid` is hidden (⌘H), asked once per read.
    fn is_hidden(&self, pid: i32, cache: &mut HashMap<i32, bool>) -> bool {
        *cache.entry(pid).or_insert_with(|| {
            let Some(app) = self.app(pid) else { return false };
            let values = self.timed(pid, || app.values(&[kAXHiddenAttribute]));
            values.ok().and_then(|v| v.into_iter().next().flatten()).as_ref().and_then(as_bool).unwrap_or(false)
        })
    }
}

impl WindowBackend for AxBackend {
    fn is_trusted(&self) -> bool {
        super::is_trusted(false)
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        // Without the list every window would look away.
        let on_screen = window_list::on_screen_windows()
            .ok_or_else(|| BackendError::Platform("the window server did not list windows".into()))?;
        let own_pid = std::process::id() as i32;
        let previous = std::mem::take(&mut *self.elements.borrow_mut());
        // Apps with nothing on screen are not read: their windows are
        // minimized, hidden or on other Spaces, and keep their place until
        // they are found gone below. Apps with a window in fullscreen are
        // read anyway, as that window keeps its column.
        let mut apps: BTreeMap<i32, (String, BTreeSet<WindowId>)> = BTreeMap::new();
        for (&id, w) in &on_screen {
            if w.pid != own_pid {
                apps.entry(w.pid).or_insert((w.owner_name.clone(), BTreeSet::new())).1.insert(id);
            }
        }
        for cached in previous.values().filter(|c| c.is_fullscreen) {
            apps.entry(cached.pid).or_insert((cached.app_name.clone(), BTreeSet::new()));
        }

        let present_apps: BTreeSet<i32> = apps.keys().copied().collect();
        // The user drags windows of the front app without it notifying, so
        // it is always read.
        // shortcut: other apps' titles go stale until they notify; observe
        // title changes if the switcher shows outdated titles.
        let front = on_screen.values().filter(|w| w.pid != own_pid).min_by_key(|w| w.z_order).map(|w| w.pid);
        let everything = self.handler.everything.replace(false);
        let changed = std::mem::take(&mut *self.handler.changed.borrow_mut());
        let mut transition = self.transition.replace(false);
        let mut seen = std::mem::take(&mut *self.seen.borrow_mut());
        let mut observers = self.observers.borrow_mut();
        let mut elements = HashMap::new();
        let mut windows = Vec::new();
        let mut read_apps = BTreeSet::new();
        // Known windows whose app failed to answer for them: kept as they are.
        let mut busy = BTreeSet::new();
        // Each window listed, with its app and whether it may still be being created.
        let mut listed = Vec::new();
        let mut app_errors: HashMap<i32, String> = HashMap::new();
        for (pid, (app_name, ids)) in apps {
            if self.is_stalled(pid) {
                // It may change unnoticed while busy, so it is read once back.
                seen.remove(&pid);
                continue;
            }
            // Opening, closing, hiding, minimizing or moving a window to
            // another Space changes the app's on-screen set or notifies.
            if !everything && Some(pid) != front && !changed.contains(&pid) && seen.get(&pid) == Some(&ids) {
                continue;
            }
            seen.remove(&pid);
            let Some(app) = self.app(pid) else {
                app_errors.insert(pid, "no Accessibility element for its app".into());
                continue;
            };
            let app_windows = match self.timed(pid, || app.elements(kAXWindowsAttribute)) {
                Ok(windows) => windows,
                Err(err) if err == kAXErrorAPIDisabled => {
                    // Nothing was read: keep every handle, and read everything next time.
                    drop(observers);
                    *self.elements.borrow_mut() = previous;
                    self.transition.set(transition);
                    self.handler.everything.set(true);
                    return Err(BackendError::NotTrusted);
                }
                Err(err) => {
                    // Busy, or without Accessibility support: its windows stay as they are.
                    app_errors.insert(pid, format!("its app failed to list windows (AX error {err})"));
                    continue;
                }
            };
            read_apps.insert(pid);
            let mut complete = true;
            let mut in_list = BTreeSet::new();
            // Off-screen standard windows, which hide behind the shown tab of
            // their native tab group at the same frame.
            let mut hidden: Vec<(WindowId, Rect)> = Vec::new();
            let first = windows.len();
            for element in app_windows {
                let Some(id) = element.window_id() else { continue };
                in_list.insert(id);
                let cached = previous.get(&id);
                let values = match self.timed(pid, || element.values(&WINDOW_ATTRIBUTES)) {
                    Ok(values) => values,
                    Err(_) if self.is_stalled(pid) => {
                        complete = false;
                        break;
                    }
                    // Closed meanwhile; found gone below if it was known.
                    Err(err) if err == kAXErrorInvalidUIElement => continue,
                    Err(_) => {
                        complete = false;
                        busy.insert(id);
                        continue;
                    }
                };
                let [position, size, title, subrole, minimized, fullscreen] = &values[..] else { continue };
                let is_fullscreen = fullscreen.as_ref().and_then(as_bool).unwrap_or(false);
                if cached.is_some_and(|c| c.is_fullscreen != is_fullscreen) {
                    // Entering or leaving fullscreen moves every other window off screen for a while.
                    log::line(format!(
                        "backend: {id} {app_name} fullscreen {}",
                        if is_fullscreen { "on" } else { "off" }
                    ));
                    transition = true;
                }
                let is_standard = cached.is_some_and(|cached| cached.is_standard)
                    || subrole.as_ref().and_then(as_string).as_deref() == Some(kAXStandardWindowSubrole);
                let origin = position.as_ref().and_then(as_point).unwrap_or_default();
                let size = size.as_ref().and_then(as_size).unwrap_or_default();
                let frame = Rect::new(origin.x, origin.y, size.width, size.height);
                let mut info = WindowInfo {
                    id,
                    pid,
                    app_name: app_name.to_owned(),
                    title: title.as_ref().and_then(as_string).unwrap_or_default(),
                    frame,
                    is_standard,
                    // Only a definite "no" floats a window; a busy app keeps it tiled.
                    is_resizable: cached.and_then(|cached| cached.is_resizable).unwrap_or(true),
                    is_minimized: false,
                    is_fullscreen,
                    tabs: Vec::new(),
                };
                let is_minimized = minimized.as_ref().and_then(as_bool).unwrap_or(false);
                // Off screen, or on screen with no subrole yet; a definite
                // other subrole is a dialog or popup, not a window being made.
                let unsettled = match on_screen.get(&id) {
                    Some(_) => !is_standard && subrole.as_ref().and_then(as_string).is_none_or(|s| s.is_empty()),
                    None => !is_minimized && !is_fullscreen,
                };
                listed.push((id, pid, unsettled));
                // Background tabs and windows on other Spaces are neither on
                // screen nor minimized, unless on their own fullscreen Space.
                let z_order = match on_screen.get(&id) {
                    Some(w) => w.z_order,
                    None if is_minimized => {
                        info.is_minimized = true;
                        usize::MAX
                    }
                    None if is_fullscreen => usize::MAX,
                    None => {
                        if is_standard {
                            hidden.push((id, frame));
                        }
                        continue;
                    }
                };
                // macOS owns a fullscreen window's size, so it is asked once it leaves.
                let is_resizable = match cached.and_then(|cached| cached.is_resizable) {
                    Some(resizable) => Some(resizable),
                    None if is_fullscreen => None,
                    None => self.timed(pid, || element.is_settable(kAXSizeAttribute)).ok(),
                };
                info.is_resizable = is_resizable.unwrap_or(true);
                if let Some(observer) = observers.get_mut(&pid) {
                    observer.watch_window(id, &element);
                }
                windows.push((z_order, info));
                elements.insert(
                    id,
                    CachedWindow {
                        element,
                        pid,
                        is_standard,
                        is_resizable,
                        app_name: app_name.to_owned(),
                        is_fullscreen,
                        was_reported: true,
                    },
                );
            }
            // shortcut: a window on another Space at exactly a shown window's
            // frame passes for its tab; read the tab group if that misleads.
            for (_, info) in windows[first..].iter_mut().filter(|(_, w)| !w.is_minimized) {
                info.tabs = hidden.iter().filter(|&&(_, frame)| frame == info.frame).map(|&(id, _)| id).collect();
            }
            // A known window on screen that its app does not list: Accessibility
            // already shows another Space while the window server does not, as
            // during a Space or fullscreen animation. Read the app again next time.
            // One missing while others are listed is that window closing.
            let known_on_screen: Vec<WindowId> = ids.iter().copied().filter(|id| previous.contains_key(id)).collect();
            let lagging = !known_on_screen.is_empty() && known_on_screen.iter().all(|id| !in_list.contains(id));
            let mut last_lagging = self.last_lagging.borrow_mut();
            if lagging {
                if last_lagging.get(&pid) != Some(&known_on_screen) {
                    log::line(format!("backend: {app_name} does not list on-screen windows {known_on_screen:?}"));
                    last_lagging.insert(pid, known_on_screen);
                }
                transition = true;
                complete = false;
            } else {
                last_lagging.remove(&pid);
            }
            if complete {
                seen.insert(pid, ids);
            }
        }

        // Known windows not reported now: background tabs, departed, or away.
        let tabs: BTreeSet<WindowId> = windows.iter().flat_map(|(_, w)| w.tabs.iter().copied()).collect();
        let mut hidden_apps = HashMap::new();
        let mut absent = Vec::new();
        let mut unanswered = Vec::new();
        // Windows in fullscreen on another Space: not listed there, but keeping their column.
        let mut fullscreen_away = BTreeSet::new();
        let now = Instant::now();
        let mut absence = self.absence.borrow_mut();
        for (id, mut cached) in previous {
            if elements.contains_key(&id) {
                continue;
            }
            if tabs.contains(&id) {
                // The engine hands its place to the shown tab.
                continue;
            }
            let pid = cached.pid;
            // Windows on their way to closing are only asked again once due,
            // unless their app notified; others when their app was read or
            // left the screen (closed to the background, say).
            let wanted = if absence.is_counting(id) {
                absence.is_due(id, now)
            } else {
                read_apps.contains(&pid) || (seen.contains_key(&pid) && !present_apps.contains(&pid))
            };
            let look = !busy.contains(&id) && !self.is_stalled(pid) && (wanted || everything || changed.contains(&pid));
            if busy.contains(&id) {
                unanswered.push(id);
            }
            if look {
                let probe = [kAXSubroleAttribute, kAXMinimizedAttribute, FULL_SCREEN_ATTRIBUTE];
                match self.timed(pid, || cached.element.values(&probe)) {
                    Err(err) if err == kAXErrorInvalidUIElement => {
                        self.depart(id, &cached, Departure::Destroyed);
                        continue;
                    }
                    Ok(values) if values.get(1).and_then(Option::as_ref).and_then(as_bool) == Some(true) => {
                        self.depart(id, &cached, Departure::Minimized);
                        continue;
                    }
                    Ok(_) if self.is_hidden(pid, &mut hidden_apps) => {
                        self.depart(id, &cached, Departure::AppHidden);
                        continue;
                    }
                    Ok(values) if values.get(2).and_then(Option::as_ref).and_then(as_bool) == Some(true) => {
                        cached.is_fullscreen = true;
                        fullscreen_away.insert(id);
                    }
                    // The engine never manages other windows; one ordered out is just forgotten.
                    Ok(_) if !cached.is_standard => continue,
                    Ok(_) => {
                        if cached.was_reported {
                            let why = if read_apps.contains(&pid) {
                                "its app does not show it"
                            } else if app_errors.contains_key(&pid) {
                                "its app failed to answer"
                            } else {
                                "its app has nothing on screen"
                            };
                            log::line(format!("backend: {id} {} away: {why}", cached.app_name));
                        }
                        cached.was_reported = false;
                        absent.push(id);
                    }
                    // Busy: it stays as it is.
                    Err(_) => unanswered.push(id),
                }
            }
            elements.insert(id, cached);
        }
        let known: Vec<WindowId> = elements.iter().filter(|(_, c)| c.is_standard).map(|(&id, _)| id).collect();
        // Windows of apps not read are there if the window server shows them.
        let reported: BTreeSet<WindowId> = windows.iter().map(|(_, w)| w.id).collect();
        let present: Vec<WindowId> = known
            .iter()
            .copied()
            .filter(|id| {
                reported.contains(id)
                    || fullscreen_away.contains(id)
                    || (on_screen.contains_key(id) && !absent.contains(id))
            })
            .collect();
        let read = Read { listed: &present, absent: &absent, unanswered: &unanswered, known: &known, transition };
        for id in absence.update(&read, now) {
            if let Some(cached) = elements.remove(&id) {
                self.depart(id, &cached, Departure::Closed);
            }
        }
        drop(absence);

        // Apps with a window still being created are read again next time.
        let unread: BTreeSet<i32> = elements.values().map(|c| c.pid).filter(|pid| !read_apps.contains(pid)).collect();
        let unread: Vec<i32> = unread.into_iter().collect();
        let settling = self.settling.borrow_mut().update(&listed, &unread, Instant::now());
        self.is_settling.set(!settling.is_empty());
        // Forget apps that left the screen, so they are read when they return.
        seen.retain(|pid, _| present_apps.contains(pid) && !settling.contains(pid));
        for observer in observers.values_mut() {
            observer.retain_windows(|id| elements.contains_key(&id));
        }
        let summary = format!(
            "backend: {} windows on screen; reported {}, away {}, known {}{}",
            on_screen.len(),
            windows.len(),
            absent.len(),
            known.len(),
            if transition { " (transition)" } else { "" },
        );
        if *self.last_summary.borrow() != summary {
            log::line(summary.clone());
            *self.last_summary.borrow_mut() = summary;
        }
        windows.sort_by_key(|(z, w)| (*z, w.id));
        *self.elements.borrow_mut() = elements;
        *self.seen.borrow_mut() = seen;
        Ok(windows.into_iter().map(|(_, w)| w).collect())
    }

    fn departures(&self) -> Vec<(WindowId, Departure)> {
        std::mem::take(&mut *self.departed.borrow_mut())
    }

    fn note_transition(&self) {
        self.transition.set(true);
    }

    fn next_check(&self) -> Option<Duration> {
        self.absence.borrow().next_check(Instant::now())
    }

    fn settling(&self) -> bool {
        self.is_settling.get()
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
        let mut elements = self.elements.borrow_mut();
        let gone: Vec<WindowId> = elements.iter().filter(|(_, c)| c.pid == pid).map(|(&id, _)| id).collect();
        for id in gone {
            let cached = elements.remove(&id).expect("listed above");
            self.depart(id, &cached, Departure::AppQuit);
        }
        drop(elements);
        self.observers.borrow_mut().remove(&pid);
        self.stalled.borrow_mut().remove(&pid);
        self.apps.borrow_mut().remove(&pid);
        self.seen.borrow_mut().remove(&pid);
    }

    fn set_change_handler(&self, handler: Box<dyn Fn()>) {
        *self.handler.handler.borrow_mut() = Some(handler);
    }
}
