//! Windows read while they are still being created.
//!
//! macOS reports a new window before it is on screen or has its subrole, and
//! sends nothing once it does, so a backend that skipped it must read its app
//! again shortly. This tracks which apps have such a window.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::backend::WindowId;

/// How long a new window may stay untileable before it is taken as it is:
/// a background tab, a window on another Space.
pub const SETTLE_TIME: Duration = Duration::from_secs(2);

#[derive(Debug, Default)]
pub struct Settling {
    /// Every window listed by the last read, with its app, so only windows
    /// never listed before count as new.
    known: HashMap<WindowId, i32>,
    /// New windows not yet tileable, and when they were first listed.
    since: HashMap<WindowId, Instant>,
}

impl Settling {
    /// Records one read: each listed window with its app and whether it
    /// looks unsettled. Apps in `skipped` were not read and keep what was
    /// known of them. Returns the apps with a window still settling.
    pub fn update(&mut self, listed: &[(WindowId, i32, bool)], skipped: &[i32], now: Instant) -> Vec<i32> {
        let mut known = HashMap::new();
        let mut since = HashMap::new();
        let mut apps = Vec::new();
        for &(id, pid, unsettled) in listed {
            known.insert(id, pid);
            let first = match self.since.get(&id) {
                Some(&first) => first,
                None if !self.known.contains_key(&id) => now,
                None => continue,
            };
            if unsettled && now.duration_since(first) < SETTLE_TIME {
                since.insert(id, first);
                if !apps.contains(&pid) {
                    apps.push(pid);
                }
            }
        }
        for (&id, &pid) in self.known.iter().filter(|(_, pid)| skipped.contains(pid)) {
            known.entry(id).or_insert(pid);
            if let Some(&first) = self.since.get(&id) {
                since.entry(id).or_insert(first);
            }
        }
        self.known = known;
        self.since = since;
        apps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATER: Duration = Duration::from_millis(200);

    #[test]
    fn new_window_settles_until_tileable() {
        let mut settling = Settling::default();
        let t = Instant::now();
        assert!(settling.update(&[(1, 10, false)], &[], t).is_empty());
        assert_eq!(settling.update(&[(1, 10, false), (2, 10, true)], &[], t + LATER), vec![10]);
        assert_eq!(settling.update(&[(1, 10, false), (2, 10, true)], &[], t + 2 * LATER), vec![10]);
        assert!(settling.update(&[(1, 10, false), (2, 10, false)], &[], t + 3 * LATER).is_empty());
    }

    #[test]
    fn known_window_turning_untileable_does_not_settle() {
        // A window going to another Space, or a tab going to the background.
        let mut settling = Settling::default();
        let t = Instant::now();
        settling.update(&[(1, 10, false)], &[], t);
        assert!(settling.update(&[(1, 10, true)], &[], t + LATER).is_empty());
    }

    #[test]
    fn gives_up_after_settle_time_for_good() {
        let mut settling = Settling::default();
        let t = Instant::now();
        assert_eq!(settling.update(&[(1, 10, true)], &[], t), vec![10]);
        assert!(settling.update(&[(1, 10, true)], &[], t + SETTLE_TIME).is_empty());
        assert!(settling.update(&[(1, 10, true)], &[], t + SETTLE_TIME + LATER).is_empty());
    }

    #[test]
    fn skipped_apps_keep_their_windows_and_clock() {
        let mut settling = Settling::default();
        let t = Instant::now();
        assert_eq!(settling.update(&[(1, 10, true), (2, 20, false)], &[], t), vec![10]);
        // App 10 was busy: nothing of it was listed, but its window is not
        // new when it is read again, and its clock kept running.
        settling.update(&[(2, 20, false)], &[10], t + LATER);
        assert!(settling.update(&[(1, 10, true), (2, 20, false)], &[], t + SETTLE_TIME).is_empty());
    }
}
