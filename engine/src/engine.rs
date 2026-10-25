//! The engine object the app talks to.

use crate::backend::{Result, WindowBackend, WindowInfo};
use crate::geometry::Rect;
use crate::layout;

pub struct Engine {
    backend: Box<dyn WindowBackend>,
}

impl Engine {
    pub fn new(backend: Box<dyn WindowBackend>) -> Self {
        Self { backend }
    }

    /// An engine driving the current platform's windows.
    pub fn with_platform_backend() -> Self {
        Self::new(crate::platform::default_backend())
    }

    pub fn backend(&self) -> &dyn WindowBackend {
        self.backend.as_ref()
    }

    pub fn windows(&self) -> Result<Vec<WindowInfo>> {
        self.backend.windows()
    }

    /// Arranges the tileable windows whose centre lies inside `area` into
    /// equal columns, ordered left to right by their current position.
    ///
    /// This is a proof of concept for driving windows from the engine; the
    /// scrollable PaperWM layout replaces it.
    pub fn tile_columns(&self, area: Rect, gap: f64) -> Result<usize> {
        let mut windows: Vec<WindowInfo> =
            self.backend.windows()?.into_iter().filter(|w| w.is_tileable() && contains_centre(area, w.frame)).collect();
        windows.sort_by(|a, b| a.frame.x.total_cmp(&b.frame.x).then(a.id.cmp(&b.id)));

        let frames = layout::columns(area, windows.len(), gap);
        for (window, frame) in windows.iter().zip(frames) {
            self.backend.set_frame(window.id, frame)?;
        }
        Ok(windows.len())
    }
}

fn contains_centre(area: Rect, frame: Rect) -> bool {
    let cx = frame.x + frame.width / 2.0;
    let cy = frame.y + frame.height / 2.0;
    cx >= area.x && cx < area.max_x() && cy >= area.y && cy < area.max_y()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendError;
    use crate::mock::MockBackend;
    use std::rc::Rc;

    /// Lets a test keep a handle on the backend after handing it to the engine.
    struct Shared(Rc<MockBackend>);

    impl WindowBackend for Shared {
        fn is_trusted(&self) -> bool {
            self.0.is_trusted()
        }
        fn windows(&self) -> Result<Vec<WindowInfo>> {
            self.0.windows()
        }
        fn set_frame(&self, id: u32, frame: Rect) -> Result<()> {
            self.0.set_frame(id, frame)
        }
    }

    fn engine_with(windows: Vec<WindowInfo>) -> (Engine, Rc<MockBackend>) {
        let backend = Rc::new(MockBackend::new(windows));
        (Engine::new(Box::new(Shared(backend.clone()))), backend)
    }

    const SCREEN: Rect = Rect::new(0.0, 0.0, 1030.0, 500.0);

    #[test]
    fn tiles_windows_in_left_to_right_order() {
        let (engine, backend) = engine_with(vec![
            MockBackend::window(1, Rect::new(600.0, 50.0, 300.0, 300.0)),
            MockBackend::window(2, Rect::new(10.0, 50.0, 300.0, 300.0)),
        ]);
        assert_eq!(engine.tile_columns(SCREEN, 10.0), Ok(2));
        assert_eq!(backend.frame_of(2), Some(Rect::new(10.0, 10.0, 500.0, 480.0)));
        assert_eq!(backend.frame_of(1), Some(Rect::new(520.0, 10.0, 500.0, 480.0)));
    }

    #[test]
    fn skips_minimized_non_standard_and_offscreen_windows() {
        let mut minimized = MockBackend::window(2, Rect::new(100.0, 100.0, 100.0, 100.0));
        minimized.is_minimized = true;
        let mut dialog = MockBackend::window(3, Rect::new(100.0, 100.0, 100.0, 100.0));
        dialog.is_standard = false;
        let offscreen = MockBackend::window(4, Rect::new(2000.0, 100.0, 100.0, 100.0));
        let (engine, backend) = engine_with(vec![
            MockBackend::window(1, Rect::new(100.0, 100.0, 100.0, 100.0)),
            minimized,
            dialog,
            offscreen,
        ]);

        assert_eq!(engine.tile_columns(SCREEN, 10.0), Ok(1));
        assert_eq!(backend.frame_of(1), Some(Rect::new(10.0, 10.0, 1010.0, 480.0)));
        for id in 2..=4 {
            assert_eq!(backend.frame_of(id).map(|f| f.width), Some(100.0));
        }
    }

    #[test]
    fn reports_missing_permission() {
        let engine = Engine::new(Box::new(MockBackend::untrusted()));
        assert_eq!(engine.tile_columns(SCREEN, 10.0), Err(BackendError::NotTrusted));
    }
}
