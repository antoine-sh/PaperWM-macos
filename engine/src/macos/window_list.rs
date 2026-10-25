//! The window server's view of on-screen windows, via `CGWindowListCopyWindowInfo`.

use std::collections::HashMap;
use std::ffi::c_void;

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{CFGetTypeID, CFRelease, CFTypeRef};
use core_foundation_sys::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
use core_foundation_sys::number::{kCFNumberSInt64Type, CFNumberGetTypeID, CFNumberGetValue, CFNumberRef};
use core_foundation_sys::string::{CFStringGetTypeID, CFStringRef};

use crate::backend::WindowId;

const ON_SCREEN_ONLY: u32 = 1 << 0;
const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const NULL_WINDOW_ID: u32 = 0;
/// Layer of regular app windows; menus, the Dock and overlays sit above it.
const NORMAL_LAYER: i64 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    static kCGWindowNumber: CFStringRef;
    static kCGWindowOwnerPID: CFStringRef;
    static kCGWindowOwnerName: CFStringRef;
    static kCGWindowLayer: CFStringRef;
}

pub struct ScreenWindow {
    /// Position in the front-to-back stacking order.
    pub z_order: usize,
    pub pid: i32,
    pub owner_name: String,
}

/// Regular app windows currently on screen, keyed by window id.
pub fn on_screen_windows() -> HashMap<WindowId, ScreenWindow> {
    let mut result = HashMap::new();
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP_ELEMENTS, NULL_WINDOW_ID);
        if list.is_null() {
            return result;
        }
        for i in 0..CFArrayGetCount(list) {
            let info = CFArrayGetValueAtIndex(list, i) as CFDictionaryRef;
            let (Some(id), Some(pid), Some(layer)) =
                (number(info, kCGWindowNumber), number(info, kCGWindowOwnerPID), number(info, kCGWindowLayer))
            else {
                continue;
            };
            if layer != NORMAL_LAYER {
                continue;
            }
            let owner_name = string(info, kCGWindowOwnerName).unwrap_or_default();
            result.insert(id as WindowId, ScreenWindow { z_order: i as usize, pid: pid as i32, owner_name });
        }
        CFRelease(list as CFTypeRef);
    }
    result
}

unsafe fn number(dict: CFDictionaryRef, key: CFStringRef) -> Option<i64> {
    let value = CFDictionaryGetValue(dict, key as *const c_void);
    if value.is_null() || CFGetTypeID(value) != CFNumberGetTypeID() {
        return None;
    }
    let mut out: i64 = 0;
    CFNumberGetValue(value as CFNumberRef, kCFNumberSInt64Type, (&mut out as *mut i64).cast()).then_some(out)
}

unsafe fn string(dict: CFDictionaryRef, key: CFStringRef) -> Option<String> {
    let value = CFDictionaryGetValue(dict, key as *const c_void);
    if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
        return None;
    }
    Some(CFString::wrap_under_get_rule(value as CFStringRef).to_string())
}
