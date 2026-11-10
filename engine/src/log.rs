//! Diagnostic lines for the app's log: why windows joined or left the layout,
//! what each read saw, how displays mapped to workspaces. Swift drains them
//! with `pwm_take_log` after each engine call. Lines name windows by id and
//! app, never by title, which may be private.

use std::cell::RefCell;

/// Lines kept when nobody drains them, as in tests.
const LIMIT: usize = 1000;

thread_local! {
    static LINES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn line(text: String) {
    LINES.with(|lines| {
        let mut lines = lines.borrow_mut();
        if lines.len() >= LIMIT {
            lines.remove(0);
        }
        lines.push(text);
    });
}

/// Every line logged since the last call, oldest first.
pub fn take() -> Vec<String> {
    LINES.with(|lines| std::mem::take(&mut *lines.borrow_mut()))
}
