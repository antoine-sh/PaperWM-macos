//! C interface used by the Swift app. Keep in sync with
//! `include/paperwm_engine.h`.
//!
//! All functions must be called from the main thread.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;

use crate::backend::{BackendError, WindowInfo};
use crate::engine::{Command, Config, Engine};
use crate::geometry::Rect;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwmStatus {
    Ok = 0,
    NotTrusted = 1,
    WindowNotFound = 2,
    PlatformError = 3,
    NullPointer = 4,
    InvalidArgument = 5,
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

fn status(result: crate::backend::Result<impl Sized>) -> PwmStatus {
    match result {
        Ok(_) => PwmStatus::Ok,
        Err(err) => (&err).into(),
    }
}

/// Sets the usable area of each monitor, main monitor first.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null. `areas`
/// must point to `count` rects.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_set_monitors(engine: *mut Engine, areas: *const Rect, count: usize) -> PwmStatus {
    let Some(engine) = engine.as_mut() else {
        return PwmStatus::NullPointer;
    };
    if areas.is_null() || count == 0 {
        return PwmStatus::InvalidArgument;
    }
    status(engine.set_monitors(std::slice::from_raw_parts(areas, count)))
}

/// Brings the layout up to date with the current windows and focus.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_sync(engine: *mut Engine) -> PwmStatus {
    let Some(engine) = engine.as_mut() else {
        return PwmStatus::NullPointer;
    };
    status(engine.sync())
}

/// Runs one of the `PWM_COMMAND_*` actions. `arg` is the workspace index,
/// window id or scroll distance for commands that take one, else ignored.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_command(engine: *mut Engine, command: u32, arg: f64) -> PwmStatus {
    let Some(engine) = engine.as_mut() else {
        return PwmStatus::NullPointer;
    };
    let Some(command) = Command::parse(command, arg) else {
        return PwmStatus::InvalidArgument;
    };
    status(engine.command(command))
}

/// Advances strip slides by `seconds` and moves the windows. Returns
/// whether a strip is still sliding; call again on the next frame if so.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_tick(engine: *mut Engine, seconds: f64) -> bool {
    engine.as_mut().is_some_and(|engine| engine.tick(seconds).unwrap_or(false))
}

/// Applies settings given as JSON (see `engine::Config`). Malformed JSON
/// leaves the current settings in place.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null. `json`
/// must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_set_config(engine: *mut Engine, json: *const c_char) -> PwmStatus {
    let Some(engine) = engine.as_mut() else {
        return PwmStatus::NullPointer;
    };
    if json.is_null() {
        return PwmStatus::InvalidArgument;
    }
    let Some(config) = CStr::from_ptr(json).to_str().ok().and_then(Config::from_json) else {
        return PwmStatus::InvalidArgument;
    };
    status(engine.set_config(config))
}

/// The layout as JSON (see `engine::State`), or null for a null engine.
/// Free with `pwm_string_free`.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_state_json(engine: *const Engine) -> *mut c_char {
    let Some(engine) = engine.as_ref() else {
        return ptr::null_mut();
    };
    c_string(&serde_json::to_string(&engine.state()).unwrap_or_default())
}

/// # Safety
/// `string` must come from this library and be freed only once, or be null.
#[no_mangle]
pub unsafe extern "C" fn pwm_string_free(string: *mut c_char) {
    if !string.is_null() {
        drop(CString::from_raw(string));
    }
}

/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_observe_app(engine: *const Engine, pid: i32) {
    if let Some(engine) = engine.as_ref() {
        engine.observe_app(pid);
    }
}

/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_forget_app(engine: *const Engine, pid: i32) {
    if let Some(engine) = engine.as_ref() {
        engine.forget_app(pid);
    }
}

/// Registers `handler`, called with `context` on the main run loop when an
/// observed app's windows change. It must not call back into the engine
/// directly; schedule `pwm_engine_sync` instead.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null. `context`
/// must stay valid for as long as the engine lives.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_set_change_handler(
    engine: *const Engine,
    handler: Option<extern "C" fn(*mut c_void)>,
    context: *mut c_void,
) {
    let (Some(engine), Some(handler)) = (engine.as_ref(), handler) else { return };
    let context = context as usize;
    engine.set_change_handler(Box::new(move || handler(context as *mut c_void)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockBackend;

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
            assert_eq!(pwm_engine_sync(ptr::null_mut()), PwmStatus::NullPointer);
            assert_eq!(pwm_engine_command(ptr::null_mut(), 0, 0.0), PwmStatus::NullPointer);
            pwm_engine_free(ptr::null_mut());
        }
    }

    #[test]
    fn set_config_rejects_bad_json() {
        let engine = raw_engine(vec![]);
        unsafe {
            assert_eq!(pwm_engine_set_config(engine, c"{\"gap\":12}".as_ptr()), PwmStatus::Ok);
            assert_eq!(pwm_engine_set_config(engine, c"{".as_ptr()), PwmStatus::InvalidArgument);
            assert_eq!(pwm_engine_set_config(engine, ptr::null()), PwmStatus::InvalidArgument);
            assert_eq!(pwm_engine_set_config(ptr::null_mut(), c"{}".as_ptr()), PwmStatus::NullPointer);
            assert_eq!((*engine).config().gap, 12.0);
            pwm_engine_free(engine);
        }
    }

    #[test]
    fn state_json_round_trips() {
        let engine = raw_engine(vec![MockBackend::window(1, Rect::new(10.0, 10.0, 10.0, 10.0))]);
        unsafe {
            let areas = [Rect::new(0.0, 0.0, 800.0, 600.0)];
            pwm_engine_set_monitors(engine, areas.as_ptr(), 1);
            pwm_engine_sync(engine);
            let json = pwm_engine_state_json(engine);
            let value: serde_json::Value = serde_json::from_str(CStr::from_ptr(json).to_str().unwrap()).unwrap();
            assert_eq!(value["windows"][0]["id"], 1);
            pwm_string_free(json);
            assert!(pwm_engine_state_json(ptr::null()).is_null());
            pwm_string_free(ptr::null_mut());
            pwm_engine_free(engine);
        }
    }

    #[test]
    fn commands_and_monitors_report_status() {
        let engine = raw_engine(vec![MockBackend::window(1, Rect::new(10.0, 10.0, 10.0, 10.0))]);
        unsafe {
            assert_eq!(pwm_engine_sync(engine), PwmStatus::Ok);
            assert_eq!(pwm_engine_set_monitors(engine, ptr::null(), 0), PwmStatus::InvalidArgument);
            let areas = [Rect::new(0.0, 0.0, 800.0, 600.0)];
            assert_eq!(pwm_engine_set_monitors(engine, areas.as_ptr(), 1), PwmStatus::Ok);
            assert_eq!(pwm_engine_sync(engine), PwmStatus::Ok);
            assert_eq!((*engine).workspace(0).unwrap().focused_window(), Some(1));
            assert_eq!(pwm_engine_command(engine, 0, 0.0), PwmStatus::Ok);
            assert_eq!(pwm_engine_command(engine, 99, 0.0), PwmStatus::InvalidArgument);
            assert_eq!(pwm_engine_set_window_frame(engine, 9, Rect::default()), PwmStatus::WindowNotFound);
            pwm_engine_free(engine);
        }
    }
}
