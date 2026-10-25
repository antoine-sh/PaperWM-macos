//! Accessibility notifications telling the engine an app's windows changed.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr;

use accessibility_sys::{
    kAXApplicationHiddenNotification, kAXApplicationShownNotification, kAXErrorSuccess,
    kAXFocusedWindowChangedNotification, kAXMainWindowChangedNotification, kAXUIElementDestroyedNotification,
    kAXWindowCreatedNotification, kAXWindowDeminiaturizedNotification, kAXWindowMiniaturizedNotification,
    AXObserverAddNotification, AXObserverCreate, AXObserverGetRunLoopSource, AXObserverRef, AXUIElementGetPid,
    AXUIElementRef,
};
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::base::{CFRelease, CFTypeRef};
use core_foundation_sys::runloop::{
    kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetMain, CFRunLoopRemoveSource,
};
use core_foundation_sys::string::CFStringRef;

use super::ax::AxElement;
use crate::backend::WindowId;

/// Where notifications are forwarded, and which apps they came from. Boxed
/// so its address, passed to macOS as the callback's context, never moves.
#[derive(Default)]
pub struct HandlerSlot {
    pub handler: RefCell<Option<Box<dyn Fn()>>>,
    /// Apps whose windows may have changed since they were last read.
    pub changed: RefCell<HashSet<i32>>,
    /// Whether every app's windows must be read again.
    pub everything: Cell<bool>,
}

impl HandlerSlot {
    pub fn mark(&self, pid: i32) {
        self.changed.borrow_mut().insert(pid);
    }
}

const APP_NOTIFICATIONS: [&str; 7] = [
    kAXWindowCreatedNotification,
    kAXFocusedWindowChangedNotification,
    kAXMainWindowChangedNotification,
    kAXWindowMiniaturizedNotification,
    kAXWindowDeminiaturizedNotification,
    kAXApplicationHiddenNotification,
    kAXApplicationShownNotification,
];

/// Watches one app. Window destruction is only reported to observers of the
/// window itself, so each window is registered as it is discovered.
pub struct AppObserver {
    observer: AXObserverRef,
    context: *const HandlerSlot,
    windows: HashSet<WindowId>,
}

impl AppObserver {
    /// # Safety
    /// `context` must outlive the returned observer.
    pub unsafe fn new(pid: i32, app: &AxElement, context: *const HandlerSlot) -> Option<Self> {
        let mut observer: AXObserverRef = ptr::null_mut();
        if AXObserverCreate(pid, on_notification, &mut observer) != kAXErrorSuccess || observer.is_null() {
            return None;
        }
        let this = Self { observer, context, windows: HashSet::new() };
        // Apps that are still launching refuse registration; returning None
        // lets the caller retry later.
        // Dropping `this` releases the observer.
        let registered = APP_NOTIFICATIONS.into_iter().filter(|n| this.add(app.as_raw(), n)).count();
        if registered == 0 {
            return None;
        }
        CFRunLoopAddSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), kCFRunLoopDefaultMode);
        Some(this)
    }

    pub fn watch_window(&mut self, id: WindowId, window: &AxElement) {
        if self.windows.insert(id) {
            let _ = self.add(window.as_raw(), kAXUIElementDestroyedNotification);
        }
    }

    /// Stops tracking windows that no longer exist.
    pub fn retain_windows(&mut self, alive: impl Fn(WindowId) -> bool) {
        self.windows.retain(|&id| alive(id));
    }

    fn add(&self, element: AXUIElementRef, notification: &'static str) -> bool {
        let name = CFString::from_static_string(notification);
        let err = unsafe {
            AXObserverAddNotification(self.observer, element, name.as_concrete_TypeRef(), self.context as *mut c_void)
        };
        err == kAXErrorSuccess
    }
}

impl Drop for AppObserver {
    fn drop(&mut self) {
        unsafe {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(self.observer), kCFRunLoopDefaultMode);
            CFRelease(self.observer as CFTypeRef);
        }
    }
}

unsafe extern "C" fn on_notification(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    _notification: CFStringRef,
    context: *mut c_void,
) {
    let slot = &*(context as *const HandlerSlot);
    let mut pid = 0;
    if AXUIElementGetPid(element, &mut pid) == kAXErrorSuccess {
        slot.mark(pid);
    } else {
        slot.everything.set(true);
    }
    // A busy slot means the handler is being replaced; dropping one
    // notification is harmless since the replacement triggers a sync.
    if let Ok(handler) = slot.handler.try_borrow() {
        if let Some(handler) = handler.as_ref() {
            handler();
        }
    }
}
