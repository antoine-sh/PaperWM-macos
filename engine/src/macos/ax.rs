//! A safe wrapper around `AXUIElementRef`.

use std::ffi::c_void;
use std::ptr;

use accessibility_sys::{
    kAXErrorAPIDisabled, kAXErrorCannotComplete, kAXErrorInvalidUIElement, kAXErrorSuccess, kAXValueTypeCGPoint,
    kAXValueTypeCGSize, AXError, AXUIElementCopyAttributeValue, AXUIElementCopyMultipleAttributeValues,
    AXUIElementCreateApplication, AXUIElementCreateSystemWide, AXUIElementGetPid, AXUIElementGetTypeID,
    AXUIElementIsAttributeSettable, AXUIElementPerformAction, AXUIElementRef, AXUIElementSetAttributeValue,
    AXUIElementSetMessagingTimeout, AXValueCreate, AXValueGetTypeID, AXValueGetValue, AXValueRef, AXValueType,
};
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::string::CFString;
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef};
use core_foundation_sys::number::{kCFBooleanTrue, CFBooleanGetTypeID};
use core_foundation_sys::string::CFStringGetTypeID;

use crate::backend::{BackendError, WindowId};
use crate::geometry::{Point, Size};

extern "C" {
    /// Private but long-stable HIServices call mapping an AX window to its
    /// `CGWindowID`. Used by every macOS tiling window manager.
    fn _AXUIElementGetWindow(element: AXUIElementRef, id: *mut WindowId) -> AXError;
}

/// How long to wait for an unresponsive app before giving up, in seconds.
pub const MESSAGING_TIMEOUT: f32 = 0.5;

/// Sets [`MESSAGING_TIMEOUT`] for every element, window elements included,
/// instead of macOS's default of about six seconds.
pub fn set_global_timeout() {
    let system = AxElement::system_wide();
    unsafe { AXUIElementSetMessagingTimeout(system.as_raw(), MESSAGING_TIMEOUT) };
}

pub struct AxElement(AXUIElementRef);

impl AxElement {
    pub fn application(pid: i32) -> Option<Self> {
        let element = unsafe { AXUIElementCreateApplication(pid) };
        if element.is_null() {
            return None;
        }
        unsafe { AXUIElementSetMessagingTimeout(element, MESSAGING_TIMEOUT) };
        Some(Self(element))
    }

    pub fn system_wide() -> Self {
        Self(unsafe { AXUIElementCreateSystemWide() })
    }

    pub fn as_raw(&self) -> AXUIElementRef {
        self.0
    }

    pub fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        (unsafe { AXUIElementGetPid(self.0, &mut pid) } == kAXErrorSuccess).then_some(pid)
    }

    /// Wraps a reference the caller does not own, retaining it.
    unsafe fn retained(element: AXUIElementRef) -> Self {
        CFRetain(element as CFTypeRef);
        Self(element)
    }

    fn copy(&self, attribute: &'static str) -> Result<CFType, AXError> {
        let name = CFString::from_static_string(attribute);
        let mut value: CFTypeRef = ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(self.0, name.as_concrete_TypeRef(), &mut value) };
        if err != kAXErrorSuccess {
            return Err(err);
        }
        if value.is_null() {
            return Err(kAXErrorCannotComplete);
        }
        Ok(unsafe { CFType::wrap_under_create_rule(value) })
    }

    /// Several attributes in one round trip to the app, in the order asked
    /// for; attributes the window lacks come back as `None`.
    pub fn values(&self, attributes: &[&'static str]) -> Result<Vec<Option<CFType>>, AXError> {
        let names: Vec<CFString> = attributes.iter().map(|a| CFString::from_static_string(a)).collect();
        let names = CFArray::from_CFTypes(&names);
        let mut values: CFArrayRef = ptr::null();
        // Without StopOnError, a missing attribute fails alone, as an AXValue
        // holding its error, instead of failing the whole call.
        let err =
            unsafe { AXUIElementCopyMultipleAttributeValues(self.0, names.as_concrete_TypeRef(), 0, &mut values) };
        if err != kAXErrorSuccess {
            return Err(err);
        }
        if values.is_null() {
            return Err(kAXErrorCannotComplete);
        }
        let values: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(values) };
        let mut result: Vec<Option<CFType>> = values.iter().map(|v| Some(v.clone())).collect();
        result.resize(attributes.len(), None);
        Ok(result)
    }

    /// An element-valued attribute such as `AXFocusedWindow`.
    pub fn element(&self, attribute: &'static str) -> Option<AxElement> {
        let value = self.copy(attribute).ok()?;
        let raw = value.as_CFTypeRef();
        if unsafe { CFGetTypeID(raw) } != unsafe { AXUIElementGetTypeID() } {
            return None;
        }
        Some(unsafe { AxElement::retained(raw as AXUIElementRef) })
    }

    pub fn is_settable(&self, attribute: &'static str) -> Result<bool, AXError> {
        let name = CFString::from_static_string(attribute);
        let mut settable = 0;
        let err = unsafe { AXUIElementIsAttributeSettable(self.0, name.as_concrete_TypeRef(), &mut settable) };
        if err == kAXErrorSuccess {
            Ok(settable != 0)
        } else {
            Err(err)
        }
    }

    pub fn set_true(&self, attribute: &'static str) -> Result<(), AXError> {
        let name = CFString::from_static_string(attribute);
        let err =
            unsafe { AXUIElementSetAttributeValue(self.0, name.as_concrete_TypeRef(), kCFBooleanTrue as CFTypeRef) };
        if err == kAXErrorSuccess {
            Ok(())
        } else {
            Err(err)
        }
    }

    pub fn perform(&self, action: &'static str) -> Result<(), AXError> {
        let name = CFString::from_static_string(action);
        let err = unsafe { AXUIElementPerformAction(self.0, name.as_concrete_TypeRef()) };
        if err == kAXErrorSuccess {
            Ok(())
        } else {
            Err(err)
        }
    }

    fn set_ax_value<T>(&self, attribute: &'static str, kind: AXValueType, value: &T) -> Result<(), AXError> {
        let name = CFString::from_static_string(attribute);
        let value = unsafe { AXValueCreate(kind, (value as *const T).cast::<c_void>()) };
        if value.is_null() {
            return Err(kAXErrorCannotComplete);
        }
        let err = unsafe { AXUIElementSetAttributeValue(self.0, name.as_concrete_TypeRef(), value as CFTypeRef) };
        unsafe { CFRelease(value as CFTypeRef) };
        if err == kAXErrorSuccess {
            Ok(())
        } else {
            Err(err)
        }
    }

    pub fn set_point(&self, attribute: &'static str, value: Point) -> Result<(), AXError> {
        self.set_ax_value(attribute, kAXValueTypeCGPoint, &value)
    }

    pub fn set_size(&self, attribute: &'static str, value: Size) -> Result<(), AXError> {
        self.set_ax_value(attribute, kAXValueTypeCGSize, &value)
    }

    /// The elements of an array-valued attribute such as `AXWindows`.
    pub fn elements(&self, attribute: &'static str) -> Result<Vec<AxElement>, AXError> {
        let value = self.copy(attribute)?;
        let array = value.as_CFTypeRef() as CFArrayRef;
        if unsafe { CFGetTypeID(array.cast()) } != unsafe { CFArrayGetTypeID() } {
            return Ok(Vec::new());
        }
        let count = unsafe { CFArrayGetCount(array) };
        Ok((0..count)
            .map(|i| unsafe { AxElement::retained(CFArrayGetValueAtIndex(array, i) as AXUIElementRef) })
            .collect())
    }

    pub fn window_id(&self) -> Option<WindowId> {
        let mut id: WindowId = 0;
        let err = unsafe { _AXUIElementGetWindow(self.0, &mut id) };
        (err == kAXErrorSuccess && id != 0).then_some(id)
    }
}

impl Clone for AxElement {
    fn clone(&self) -> Self {
        unsafe { Self::retained(self.0) }
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0 as CFTypeRef) };
    }
}

pub fn as_string(value: &CFType) -> Option<String> {
    if unsafe { CFGetTypeID(value.as_CFTypeRef()) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    let s = unsafe { CFString::wrap_under_get_rule(value.as_CFTypeRef().cast()) };
    Some(s.to_string())
}

pub fn as_bool(value: &CFType) -> Option<bool> {
    if unsafe { CFGetTypeID(value.as_CFTypeRef()) } != unsafe { CFBooleanGetTypeID() } {
        return None;
    }
    Some(value.as_CFTypeRef() == unsafe { kCFBooleanTrue } as CFTypeRef)
}

pub fn as_point(value: &CFType) -> Option<Point> {
    ax_value(value, kAXValueTypeCGPoint)
}

pub fn as_size(value: &CFType) -> Option<Size> {
    ax_value(value, kAXValueTypeCGSize)
}

/// The contents of an `AXValue` of type `kind`. An attribute that failed in
/// [`AxElement::values`] holds an error instead, which reads as `None`.
fn ax_value<T: Default>(value: &CFType, kind: AXValueType) -> Option<T> {
    let raw = value.as_CFTypeRef();
    if unsafe { CFGetTypeID(raw) } != unsafe { AXValueGetTypeID() } {
        return None;
    }
    let mut out = T::default();
    let ok = unsafe { AXValueGetValue(raw as AXValueRef, kind, (&mut out as *mut T).cast::<c_void>()) };
    ok.then_some(out)
}

// The AX error constants are lower-case Apple names.
#[allow(non_upper_case_globals)]
pub fn to_backend_error(err: AXError, id: WindowId) -> BackendError {
    match err {
        kAXErrorAPIDisabled => BackendError::NotTrusted,
        kAXErrorInvalidUIElement => BackendError::WindowNotFound(id),
        other => BackendError::Platform(format!(
            "Accessibility call failed for window {id}: {}",
            accessibility_sys::error_string(other)
        )),
    }
}
