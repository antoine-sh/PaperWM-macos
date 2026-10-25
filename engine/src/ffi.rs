//! C interface used by the Swift app. Keep in sync with
//! `include/paperwm_engine.h`.
//!
//! All functions must be called from the main thread.

use std::ffi::{c_char, CString};
use std::ptr;

use crate::backend::{BackendError, WindowInfo};
use crate::engine::Engine;
use crate::geometry::Rect;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwmStatus {
    Ok = 0,
    NotTrusted = 1,
    WindowNotFound = 2,
    PlatformError = 3,
    NullPointer = 4,
}

impl From<&BackendError> for PwmStatus {
    fn from(err: &BackendError) -> Self {
        match err {
            BackendError::NotTrusted => Self::NotTrusted,
            BackendError::WindowNotFound(_) => Self::WindowNotFound,
            BackendError::Platform(_) => Self::PlatformError,
        }
    }
}

#[repr(C)]
pub struct PwmWindow {
    pub id: u32,
    pub pid: i32,
    pub frame: Rect,
    /// Owned by the list; freed by `pwm_window_list_free`.
    pub app_name: *mut c_char,
    pub title: *mut c_char,
    pub is_standard: bool,
    pub is_minimized: bool,
}

#[repr(C)]
pub struct PwmWindowList {
    pub windows: *mut PwmWindow,
    pub len: usize,
    pub status: PwmStatus,
}

impl PwmWindowList {
    fn empty(status: PwmStatus) -> Self {
        Self { windows: ptr::null_mut(), len: 0, status }
    }
}

fn c_string(s: &str) -> *mut c_char {
    // Interior NULs cannot be represented in a C string; drop them.
    CString::new(s.replace('\0', "")).unwrap_or_default().into_raw()
}

fn to_ffi(w: &WindowInfo) -> PwmWindow {
    PwmWindow {
        id: w.id,
        pid: w.pid,
        frame: w.frame,
        app_name: c_string(&w.app_name),
        title: c_string(&w.title),
        is_standard: w.is_standard,
        is_minimized: w.is_minimized,
    }
}

#[no_mangle]
pub extern "C" fn pwm_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// Whether the app has Accessibility permission. With `prompt`, macOS shows
/// its "grant access" dialog when it does not.
#[no_mangle]
pub extern "C" fn pwm_accessibility_is_trusted(prompt: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::macos::is_trusted(prompt)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = prompt;
        true
    }
}

#[no_mangle]
pub extern "C" fn pwm_engine_new() -> *mut Engine {
    Box::into_raw(Box::new(Engine::with_platform_backend()))
}

/// # Safety
/// `engine` must come from `pwm_engine_new` and not have been freed.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_free(engine: *mut Engine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
    }
}

/// Lists the windows on the current Space. Free the result with
/// `pwm_window_list_free`.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_list_windows(engine: *const Engine) -> PwmWindowList {
    let Some(engine) = engine.as_ref() else {
        return PwmWindowList::empty(PwmStatus::NullPointer);
    };
    match engine.windows() {
        Ok(windows) => {
            let boxed: Box<[PwmWindow]> = windows.iter().map(to_ffi).collect();
            let len = boxed.len();
            PwmWindowList { windows: Box::into_raw(boxed).cast(), len, status: PwmStatus::Ok }
        }
        Err(err) => PwmWindowList::empty((&err).into()),
    }
}

/// # Safety
/// `list` must come from `pwm_engine_list_windows` and be freed only once.
#[no_mangle]
pub unsafe extern "C" fn pwm_window_list_free(list: PwmWindowList) {
    if list.windows.is_null() {
        return;
    }
    let windows = Box::from_raw(ptr::slice_from_raw_parts_mut(list.windows, list.len));
    for w in windows.iter() {
        drop(CString::from_raw(w.app_name));
        drop(CString::from_raw(w.title));
    }
}

/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_set_window_frame(engine: *const Engine, id: u32, frame: Rect) -> PwmStatus {
    let Some(engine) = engine.as_ref() else {
        return PwmStatus::NullPointer;
    };
    match engine.backend().set_frame(id, frame) {
        Ok(()) => PwmStatus::Ok,
        Err(err) => (&err).into(),
    }
}

/// Tiles the windows on `area` into equal columns. On success, writes the
/// number of tiled windows to `tiled` when it is not null.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null. `tiled`
/// must be null or valid for writes.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_tile_columns(
    engine: *const Engine,
    area: Rect,
    gap: f64,
    tiled: *mut usize,
) -> PwmStatus {
    let Some(engine) = engine.as_ref() else {
        return PwmStatus::NullPointer;
    };
    match engine.tile_columns(area, gap) {
        Ok(count) => {
            if !tiled.is_null() {
                *tiled = count;
            }
            PwmStatus::Ok
        }
        Err(err) => (&err).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockBackend;
    use std::ffi::CStr;

    fn raw_engine(windows: Vec<WindowInfo>) -> *mut Engine {
        Box::into_raw(Box::new(Engine::new(Box::new(MockBackend::new(windows)))))
    }

    #[test]
    fn version_is_a_c_string() {
        let v = unsafe { CStr::from_ptr(pwm_version()) };
        assert_eq!(v.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn list_round_trips_window_data() {
        let mut w = MockBackend::window(7, Rect::new(1.0, 2.0, 3.0, 4.0));
        w.title = "a\0b".into();
        let engine = raw_engine(vec![w]);
        unsafe {
            let list = pwm_engine_list_windows(engine);
            assert_eq!(list.status, PwmStatus::Ok);
            assert_eq!(list.len, 1);
            let first = &*list.windows;
            assert_eq!(first.id, 7);
            assert_eq!(first.frame, Rect::new(1.0, 2.0, 3.0, 4.0));
            assert_eq!(CStr::from_ptr(first.title).to_str().unwrap(), "ab");
            assert_eq!(CStr::from_ptr(first.app_name).to_str().unwrap(), "App 7");
            pwm_window_list_free(list);
            pwm_engine_free(engine);
        }
    }

    #[test]
    fn null_engine_is_reported() {
        unsafe {
            assert_eq!(pwm_engine_list_windows(ptr::null()).status, PwmStatus::NullPointer);
            assert_eq!(pwm_engine_set_window_frame(ptr::null(), 1, Rect::default()), PwmStatus::NullPointer);
            assert_eq!(
                pwm_engine_tile_columns(ptr::null(), Rect::default(), 0.0, ptr::null_mut()),
                PwmStatus::NullPointer
            );
            pwm_engine_free(ptr::null_mut());
        }
    }

    #[test]
    fn tile_and_move_report_status() {
        let engine = raw_engine(vec![MockBackend::window(1, Rect::new(10.0, 10.0, 10.0, 10.0))]);
        unsafe {
            let mut tiled = 0;
            let status = pwm_engine_tile_columns(engine, Rect::new(0.0, 0.0, 100.0, 100.0), 0.0, &mut tiled);
            assert_eq!((status, tiled), (PwmStatus::Ok, 1));
            assert_eq!(pwm_engine_set_window_frame(engine, 9, Rect::default()), PwmStatus::WindowNotFound);
            pwm_engine_free(engine);
        }
    }
}
