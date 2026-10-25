//! macOS backend built on the Accessibility (AX) API.
//!
//! Only public APIs plus `_AXUIElementGetWindow` are used, so nothing here
//! requires System Integrity Protection to be disabled.

mod ax;
mod backend;
mod observer;
mod window_list;

pub use backend::AxBackend;

use accessibility_sys::{kAXTrustedCheckOptionPrompt, AXIsProcessTrustedWithOptions};
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

/// Whether this process has Accessibility permission. With `prompt`, macOS
/// shows its dialog pointing the user to System Settings when it does not.
pub fn is_trusted(prompt: bool) -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::from(prompt))]);
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
    }
}
