//! Writes window frames through the backend, skipping windows that are
//! already where they belong.

use std::collections::HashMap;

use crate::backend::{BackendError, Result, WindowBackend, WindowId};
use crate::geometry::Rect;

#[derive(Debug, Default)]
pub struct FrameApplier {
    /// Last frame set on each window, to skip redundant Accessibility calls.
    applied: HashMap<WindowId, Rect>,
    /// Windows that ended up somewhere other than where they were put, with
    /// the frame they were given and where they went. Seeing the same pair
    /// again means the window refuses that frame, so it is left alone.
    refused: HashMap<WindowId, (Rect, Rect)>,
}

impl FrameApplier {
    /// The frame last set on `id`.
    pub fn get(&self, id: WindowId) -> Option<Rect> {
        self.applied.get(&id).copied()
    }

    /// Forgets a window that no longer exists.
    pub fn forget(&mut self, id: WindowId) {
        self.applied.remove(&id);
        self.refused.remove(&id);
    }

    /// Makes the next write put every window in place again.
    pub fn clear(&mut self) {
        self.applied.clear();
        self.refused.clear();
    }

    /// Notes where `id` really is. A window that was moved or resized since
    /// it was placed, by the user or by its app, is placed again on the next
    /// write, unless it already went back to the same wrong frame once.
    pub fn observe(&mut self, id: WindowId, actual: Rect) {
        let Some(applied) = self.get(id) else { return };
        if within(applied, actual, DRIFT) {
            self.refused.remove(&id);
            return;
        }
        if self.refused.get(&id).is_some_and(|&(a, b)| within(a, applied, SAME) && within(b, actual, DRIFT)) {
            return;
        }
        self.refused.insert(id, (applied, actual));
        self.applied.remove(&id);
    }

    /// Whether `id` was moved since it was placed without being resized, as
    /// when dragged by its title bar. A window that went back to the same
    /// wrong place it refused before was not moved.
    pub fn moved(&self, id: WindowId, actual: Rect) -> bool {
        let Some(applied) = self.get(id) else { return false };
        if self.refused.get(&id).is_some_and(|&(a, b)| within(a, applied, SAME) && within(b, actual, DRIFT)) {
            return false;
        }
        !within(applied, actual, DRIFT) && within(applied, Rect { x: applied.x, y: applied.y, ..actual }, DRIFT)
    }

    /// Moves each window to its frame, with as few Accessibility calls as
    /// possible. Windows that fail are retried on the next write.
    pub fn write(&mut self, backend: &dyn WindowBackend, frames: Vec<(WindowId, Rect)>) -> Result<()> {
        for (id, frame) in frames {
            let previous = self.get(id);
            if previous.is_some_and(|f| within(f, frame, SAME)) {
                continue;
            }
            let same_size = previous.is_some_and(|f| within(f, Rect { x: f.x, y: f.y, ..frame }, SAME));
            let same_origin =
                previous.is_some_and(|f| within(f, Rect { width: f.width, height: f.height, ..frame }, SAME));
            // Slides only move windows and width changes of the leftmost
            // column only resize them; either needs a single call.
            let result = if same_size {
                backend.set_position(id, frame.x, frame.y)
            } else if same_origin {
                backend.set_size(id, frame.width, frame.height)
            } else {
                backend.set_frame(id, frame)
            };
            match result {
                Ok(()) => {
                    self.applied.insert(id, frame);
                }
                Err(BackendError::NotTrusted) => return Err(BackendError::NotTrusted),
                // Closed windows are dropped on the next sync, busy apps are
                // retried, and windows that refuse a size keep their own.
                Err(_) => {}
            }
        }
        Ok(())
    }
}

/// Tolerance for frames that are the same.
const SAME: f64 = 0.5;
/// How far a window may be from its frame before it counts as moved.
const DRIFT: f64 = 1.0;

fn within(a: Rect, b: Rect, tolerance: f64) -> bool {
    (a.x - b.x).abs() < tolerance
        && (a.y - b.y).abs() < tolerance
        && (a.width - b.width).abs() < tolerance
        && (a.height - b.height).abs() < tolerance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockBackend;

    const FRAME: Rect = Rect::new(10.0, 10.0, 300.0, 200.0);

    fn placed() -> (FrameApplier, MockBackend) {
        let backend = MockBackend::new(vec![MockBackend::window(1, Rect::default())]);
        let mut applier = FrameApplier::default();
        applier.write(&backend, vec![(1, FRAME)]).unwrap();
        (applier, backend)
    }

    #[test]
    fn skips_windows_already_in_place() {
        let (mut applier, backend) = placed();
        let calls = backend.set_frame_calls();
        applier.write(&backend, vec![(1, FRAME)]).unwrap();
        assert_eq!(backend.set_frame_calls(), calls);
    }

    #[test]
    fn moved_window_is_put_back() {
        let (mut applier, backend) = placed();
        backend.update(1, |w| w.frame.x = 500.0);
        applier.observe(1, backend.frame_of(1).unwrap());
        applier.write(&backend, vec![(1, FRAME)]).unwrap();
        assert_eq!(backend.frame_of(1), Some(FRAME));
    }

    #[test]
    fn window_refusing_its_frame_is_left_alone() {
        let (mut applier, backend) = placed();
        let refused = Rect { width: 400.0, ..FRAME };
        backend.update(1, |w| w.frame = refused);
        applier.observe(1, refused);
        applier.write(&backend, vec![(1, FRAME)]).unwrap();
        // The app keeps its minimum width.
        backend.update(1, |w| w.frame = refused);
        applier.observe(1, refused);
        let calls = backend.set_frame_calls();
        applier.write(&backend, vec![(1, FRAME)]).unwrap();
        assert_eq!(backend.set_frame_calls(), calls);
    }

    #[test]
    fn uses_one_call_for_moves_and_resizes_in_place() {
        let (mut applier, backend) = placed();
        let calls = backend.set_frame_calls();
        applier.write(&backend, vec![(1, Rect { x: 50.0, ..FRAME })]).unwrap();
        applier.write(&backend, vec![(1, Rect { x: 50.0, width: 100.0, ..FRAME })]).unwrap();
        assert_eq!(backend.set_frame_calls(), calls + 2);
        assert_eq!(backend.frame_of(1), Some(Rect { x: 50.0, width: 100.0, ..FRAME }));
    }
}
