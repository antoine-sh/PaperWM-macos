//! When a window that still exists but is no longer listed counts as closed.
//!
//! macOS lists only the current Space's windows, and during sleep, the lock
//! screen and Space animations it lists few or none, so a window missing
//! from a read is usually only away. Apps that close to the background order
//! their window out without destroying it, though, so its element stays
//! valid forever. This decides, from reads and transitions, when such a
//! window has really left.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::backend::WindowId;

/// How long a window must stay away, with nothing to explain it, before it
/// counts as closed. Long enough for windows coming back slowly after wake.
pub const GRACE: Duration = Duration::from_secs(5);

/// What one read showed. `absent` holds windows whose element answered but
/// that were not listed, `unanswered` windows whose busy app could not say;
/// their clock starts over, so a due verdict cannot keep the app re-syncing.
/// Windows in none of these were not looked at and keep their clock; the
/// backend must look at each window [`Absence::is_due`].
pub struct Read<'a> {
    pub listed: &'a [WindowId],
    pub absent: &'a [WindowId],
    pub unanswered: &'a [WindowId],
    /// Every standard window still known, listed or not. A read listing
    /// fewer than half of them is not on their Space: another Space, a
    /// fullscreen Space, the lock screen, a wake. Must include `listed`.
    pub known: &'a [WindowId],
    /// Something since the last read that empties the screen for a while:
    /// a Space change, sleep, wake, lock, a display change, a window going
    /// in or out of fullscreen, or macOS listing a window inconsistently.
    pub transition: bool,
}

#[derive(Debug, Default)]
pub struct Absence {
    /// When each window was last listed.
    listed_at: HashMap<WindowId, Instant>,
    /// When each absent window was first found absent.
    absent_since: HashMap<WindowId, Instant>,
    last_transition: Option<Instant>,
    /// When the current run of home reads began, if this read is home.
    home_since: Option<Instant>,
}

impl Absence {
    /// Records one read, returning the absent windows that now count as
    /// closed: away for [`GRACE`], and either they left while their Space
    /// was shown (no transition since they were last listed), or their Space
    /// has been shown again for [`GRACE`] since the last transition.
    pub fn update(&mut self, read: &Read, now: Instant) -> Vec<WindowId> {
        let home = read.listed.len() * 2 >= read.known.len();
        // Several windows vanishing at once is a blackout; one is just closing.
        let blackout = !home && self.home_since.is_some() && read.known.len() - read.listed.len() >= 2;
        if read.transition || blackout {
            self.last_transition = Some(now);
        }
        self.home_since = match self.home_since {
            Some(since) if home && !read.transition => Some(since),
            _ if home => Some(now),
            _ => None,
        };
        self.listed_at.retain(|id, _| read.known.contains(id));
        self.absent_since
            .retain(|id, _| read.known.contains(id) && !read.listed.contains(id) && !read.unanswered.contains(id));
        for &id in read.listed {
            self.listed_at.insert(id, now);
        }
        let mut closed = Vec::new();
        for &id in read.absent {
            let since = *self.absent_since.entry(id).or_insert(now);
            if self.matures(id, since).is_some_and(|at| at <= now) {
                closed.push(id);
            }
        }
        for id in &closed {
            self.absent_since.remove(id);
            self.listed_at.remove(id);
        }
        closed
    }

    /// Whether `id` is absent and on its way to counting as closed.
    pub fn is_counting(&self, id: WindowId) -> bool {
        self.absent_since.contains_key(&id)
    }

    /// Whether `id` would count as closed if found absent now, so it must be
    /// looked at; counting windows that are not due need not be.
    pub fn is_due(&self, id: WindowId, now: Instant) -> bool {
        self.absent_since.get(&id).and_then(|&since| self.matures(id, since)).is_some_and(|at| at <= now)
    }

    /// How long until the next absent window could count as closed, if any
    /// can without another read first.
    pub fn next_check(&self, now: Instant) -> Option<Duration> {
        let at = self.absent_since.iter().filter_map(|(&id, &since)| self.matures(id, since)).min()?;
        Some(at.saturating_duration_since(now))
    }

    /// When `id`, absent since `since`, counts as closed if nothing changes.
    fn matures(&self, id: WindowId, since: Instant) -> Option<Instant> {
        let left_in_view = self.listed_at.get(&id).is_some_and(|&seen| self.last_transition.is_none_or(|t| t < seen));
        if left_in_view {
            return Some(since + GRACE);
        }
        self.home_since.map(|home| since.max(home) + GRACE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &[WindowId] = &[1, 2, 3];
    const SECOND: Duration = Duration::from_secs(1);

    fn read<'a>(listed: &'a [WindowId], absent: &'a [WindowId], transition: bool) -> Read<'a> {
        Read { listed, absent, unanswered: &[], known: KNOWN, transition }
    }

    #[test]
    fn window_gone_from_its_shown_space_closes_after_the_grace() {
        let mut absence = Absence::default();
        let t = Instant::now();
        absence.update(&read(&[1, 2, 3], &[], false), t);
        assert!(absence.update(&read(&[1, 2], &[3], false), t + SECOND).is_empty());
        assert_eq!(absence.next_check(t + SECOND), Some(GRACE));
        assert!(absence.update(&read(&[1, 2], &[3], false), t + GRACE).is_empty());
        assert_eq!(absence.update(&read(&[1, 2], &[3], false), t + SECOND + GRACE), vec![3]);
    }

    #[test]
    fn coming_back_resets_the_clock() {
        let mut absence = Absence::default();
        let t = Instant::now();
        absence.update(&read(&[1, 2, 3], &[], false), t);
        absence.update(&read(&[1, 2], &[3], false), t + SECOND);
        absence.update(&read(&[1, 2, 3], &[], false), t + 2 * SECOND);
        assert!(absence.update(&read(&[1, 2], &[3], false), t + GRACE + SECOND).is_empty());
    }

    #[test]
    fn windows_away_after_a_transition_stay_while_their_space_is_not_shown() {
        let mut absence = Absence::default();
        let t = Instant::now();
        absence.update(&read(&[1, 2, 3], &[], false), t);
        // A fullscreen window on its own Space: the others are away.
        assert!(absence.update(&read(&[1], &[2, 3], true), t + SECOND).is_empty());
        assert_eq!(absence.next_check(t + SECOND), None);
        assert!(absence.update(&read(&[1], &[2, 3], false), t + 60 * GRACE).is_empty());
    }

    #[test]
    fn a_window_still_away_once_its_space_is_back_closes_after_the_grace() {
        let mut absence = Absence::default();
        let t = Instant::now();
        absence.update(&read(&[1, 2, 3], &[], false), t);
        absence.update(&read(&[1], &[2, 3], true), t + SECOND);
        // Back on the desktop; window 3 was closed to the background meanwhile.
        let back = t + 10 * GRACE;
        assert!(absence.update(&read(&[1, 2], &[3], true), back).is_empty());
        assert!(absence.update(&read(&[1, 2], &[3], false), back + GRACE - SECOND).is_empty());
        assert_eq!(absence.update(&read(&[1, 2], &[3], false), back + GRACE), vec![3]);
    }

    #[test]
    fn a_read_listing_less_than_half_counts_as_a_transition() {
        let mut absence = Absence::default();
        let t = Instant::now();
        absence.update(&read(&[1, 2, 3], &[], false), t);
        // A blackout nobody announced, then window 3 lags behind the others.
        absence.update(&read(&[], &[1, 2, 3], false), t + SECOND);
        assert!(absence.update(&read(&[1, 2], &[3], false), t + 2 * SECOND).is_empty());
        assert!(absence.update(&read(&[1, 2], &[3], false), t + SECOND + GRACE).is_empty());
        assert_eq!(absence.update(&read(&[1, 2], &[3], false), t + 2 * SECOND + GRACE), vec![3]);
    }

    #[test]
    fn windows_not_looked_at_keep_their_clock() {
        let mut absence = Absence::default();
        let t = Instant::now();
        absence.update(&read(&[1, 2, 3], &[], false), t);
        absence.update(&read(&[1, 2], &[3], false), t + SECOND);
        assert!(!absence.is_due(3, t + 2 * SECOND));
        assert!(absence.update(&read(&[1, 2], &[], false), t + 2 * SECOND).is_empty());
        assert_eq!(absence.update(&read(&[1, 2], &[3], false), t + SECOND + GRACE), vec![3]);
    }

    #[test]
    fn a_lone_window_closed_to_the_background_still_closes() {
        let mut absence = Absence::default();
        let t = Instant::now();
        let known = &[1];
        absence.update(&Read { listed: &[1], absent: &[], unanswered: &[], known, transition: false }, t);
        let gone = Read { listed: &[], absent: &[1], unanswered: &[], known, transition: false };
        assert!(absence.update(&gone, t + SECOND).is_empty());
        assert_eq!(absence.update(&gone, t + SECOND + GRACE), vec![1]);
    }

    #[test]
    fn a_window_its_busy_app_cannot_answer_for_starts_over() {
        let mut absence = Absence::default();
        let t = Instant::now();
        absence.update(&read(&[1, 2, 3], &[], false), t);
        absence.update(&read(&[1, 2], &[3], false), t + SECOND);
        assert!(absence.is_counting(3));
        assert!(absence.is_due(3, t + SECOND + GRACE));
        // Its app was busy this time.
        let busy = Read { unanswered: &[3], ..read(&[1, 2], &[], false) };
        assert!(absence.update(&busy, t + 2 * GRACE).is_empty());
        assert!(!absence.is_counting(3));
        assert_eq!(absence.next_check(t + 2 * GRACE), None);
        assert!(absence.update(&read(&[1, 2], &[3], false), t + 2 * GRACE + SECOND).is_empty());
        assert_eq!(absence.update(&read(&[1, 2], &[3], false), t + 3 * GRACE + SECOND), vec![3]);
    }
}
