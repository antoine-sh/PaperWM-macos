//! C interface used by the Swift app. Keep in sync with
//! `include/paperwm_engine.h`.
//!
//! All functions must be called from the main thread.

use std::cell::RefCell;
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

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::default();
}

fn status(result: crate::backend::Result<impl Sized>) -> PwmStatus {
    match result {
        Ok(_) => PwmStatus::Ok,
        Err(err) => {
            LAST_ERROR
                .with(|last| *last.borrow_mut() = CString::new(err.to_string().replace('\0', "")).unwrap_or_default());
            (&err).into()
        }
    }
}

/// What went wrong in the latest call that failed, for logs. Valid until the
/// next failing call.
#[no_mangle]
pub extern "C" fn pwm_last_error() -> *const c_char {
    LAST_ERROR.with(|last| last.borrow().as_ptr())
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

/// Puts every window back in its place and syncs.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_retile(engine: *mut Engine) -> PwmStatus {
    let Some(engine) = engine.as_mut() else {
        return PwmStatus::NullPointer;
    };
    status(engine.retile())
}

/// Runs one of the `PWM_COMMAND_*` actions. `arg` is the workspace index,
/// window id or scroll distance for commands that take one, else ignored.
/// `changed`, if not null, is set to whether the command changed anything.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
/// `changed` must be valid for writes, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_command(
    engine: *mut Engine,
    command: u32,
    arg: f64,
    changed: *mut bool,
) -> PwmStatus {
    if let Some(changed) = changed.as_mut() {
        *changed = false;
    }
    let Some(engine) = engine.as_mut() else {
        return PwmStatus::NullPointer;
    };
    let Some(command) = Command::parse(command, arg) else {
        return PwmStatus::InvalidArgument;
    };
    let result = engine.command(command);
    if let (Ok(true), Some(changed)) = (&result, changed.as_mut()) {
        *changed = true;
    }
    status(result)
}

/// Advances strip slides by `seconds` and moves the windows. Returns
/// whether a strip is still sliding; call again on the next frame if so.
/// `status`, if not null, receives the outcome; a strip whose windows could
/// not be moved stops sliding.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
/// `status` must be valid for writes, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_tick(engine: *mut Engine, seconds: f64, status_out: *mut PwmStatus) -> bool {
    let Some(engine) = engine.as_mut() else {
        if let Some(out) = status_out.as_mut() {
            *out = PwmStatus::NullPointer;
        }
        return false;
    };
    let result = engine.tick(seconds);
    let sliding = *result.as_ref().unwrap_or(&false);
    if let Some(out) = status_out.as_mut() {
        *out = status(result);
    }
    sliding
}

/// Whether a strip is sliding, so `pwm_engine_tick` should be called on
/// every frame.
///
/// # Safety
/// `engine` must be a live pointer from `pwm_engine_new`, or null.
#[no_mangle]
pub unsafe extern "C" fn pwm_engine_is_animating(engine: *const Engine) -> bool {
    engine.as_ref().is_some_and(Engine::is_animating)
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
            assert_eq!(pwm_engine_sync(ptr::null_mut()), PwmStatus::NullPointer);
            assert_eq!(pwm_engine_retile(ptr::null_mut()), PwmStatus::NullPointer);
            assert_eq!(pwm_engine_command(ptr::null_mut(), 0, 0.0, ptr::null_mut()), PwmStatus::NullPointer);
            let mut status = PwmStatus::Ok;
            assert!(!pwm_engine_tick(ptr::null_mut(), 0.1, &mut status));
            assert_eq!(status, PwmStatus::NullPointer);
            assert!(!pwm_engine_is_animating(ptr::null()));
            pwm_engine_free(ptr::null_mut());
        }
    }

    #[test]
    fn last_error_describes_the_failure() {
        let engine = Box::into_raw(Box::new(Engine::new(Box::new(MockBackend::untrusted()))));
        unsafe {
            let areas = [Rect::new(0.0, 0.0, 800.0, 600.0)];
            pwm_engine_set_monitors(engine, areas.as_ptr(), 1);
            assert_eq!(pwm_engine_sync(engine), PwmStatus::NotTrusted);
            let message = CStr::from_ptr(pwm_last_error()).to_str().unwrap();
            assert!(message.contains("Accessibility"), "{message}");
            pwm_engine_free(engine);
        }
    }

    /// The header is written by hand; this keeps its numbers in step with
    /// the Rust side.
    #[test]
    fn header_matches_the_engine() {
        let header = include_str!("../include/paperwm_engine.h");
        let value = |name: &str| -> u32 {
            let line = header.lines().map(str::trim).find_map(|l| {
                l.strip_prefix(&format!("#define {name} ")).or_else(|| l.strip_prefix(&format!("{name} = ")))
            });
            let rest = line.unwrap_or_else(|| panic!("{name} missing from the header"));
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().unwrap_or_else(|_| panic!("no value for {name}"))
        };
        assert_eq!(value("PWM_WORKSPACE_COUNT") as usize, crate::engine::WORKSPACE_COUNT);
        let commands = [
            ("FOCUS_LEFT", Command::FocusLeft),
            ("FOCUS_RIGHT", Command::FocusRight),
            ("FOCUS_UP", Command::FocusUp),
            ("FOCUS_DOWN", Command::FocusDown),
            ("MOVE_LEFT", Command::MoveLeft),
            ("MOVE_RIGHT", Command::MoveRight),
            ("MOVE_UP", Command::MoveUp),
            ("MOVE_DOWN", Command::MoveDown),
            ("CYCLE_WIDTH", Command::CycleWidth),
            ("TOGGLE_FULL_WIDTH", Command::ToggleFullWidth),
            ("CENTER", Command::Center),
            ("CONSUME", Command::Consume),
            ("EXPEL", Command::Expel),
            ("SWITCH_WORKSPACE", Command::SwitchWorkspace(1)),
            ("MOVE_TO_WORKSPACE", Command::MoveToWorkspace(1)),
            ("NEXT_WORKSPACE", Command::NextWorkspace),
            ("PREVIOUS_WORKSPACE", Command::PreviousWorkspace),
            ("MOVE_TO_NEXT_WORKSPACE", Command::MoveToNextWorkspace),
            ("MOVE_TO_PREVIOUS_WORKSPACE", Command::MoveToPreviousWorkspace),
            ("LAST_WORKSPACE", Command::LastWorkspace),
            ("FOCUS_MONITOR_LEFT", Command::FocusMonitorLeft),
            ("FOCUS_MONITOR_RIGHT", Command::FocusMonitorRight),
            ("MOVE_TO_MONITOR_LEFT", Command::MoveToMonitorLeft),
            ("MOVE_TO_MONITOR_RIGHT", Command::MoveToMonitorRight),
            ("TOGGLE_SCRATCH", Command::ToggleScratch),
            ("MOVE_TO_SCRATCH", Command::MoveToScratch),
            ("FOCUS_WINDOW", Command::FocusWindow(1)),
            ("SCROLL_BY", Command::ScrollBy(1.0)),
            ("SCROLL_END", Command::ScrollEnd),
            ("FOCUS_FIRST", Command::FocusFirst),
            ("FOCUS_LAST", Command::FocusLast),
        ];
        for (name, command) in commands {
            assert_eq!(Command::parse(value(&format!("PWM_COMMAND_{name}")), 1.0), Some(command), "{name}");
        }
        assert_eq!(Command::parse(commands.len() as u32, 1.0), None, "a command is missing from the header");
        let statuses = [
            ("OK", PwmStatus::Ok),
            ("NOT_TRUSTED", PwmStatus::NotTrusted),
            ("WINDOW_NOT_FOUND", PwmStatus::WindowNotFound),
            ("PLATFORM_ERROR", PwmStatus::PlatformError),
            ("NULL_POINTER", PwmStatus::NullPointer),
            ("INVALID_ARGUMENT", PwmStatus::InvalidArgument),
        ];
        for (name, status) in statuses {
            assert_eq!(value(&format!("PWM_STATUS_{name}")), status as u32, "{name}");
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
            let mut changed = true;
            assert_eq!(pwm_engine_command(engine, 0, 0.0, &mut changed), PwmStatus::Ok);
            assert!(!changed);
            assert_eq!(pwm_engine_command(engine, 10, 0.0, &mut changed), PwmStatus::Ok);
            assert!(changed);
            assert_eq!(pwm_engine_command(engine, 99, 0.0, &mut changed), PwmStatus::InvalidArgument);
            assert!(!changed);
            assert_eq!(pwm_engine_retile(engine), PwmStatus::Ok);
            pwm_engine_free(engine);
        }
    }
}
