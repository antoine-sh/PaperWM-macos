//! The engine object the app talks to: keeps one scrolling workspace per
//! monitor in step with the real windows.

use std::collections::{HashMap, HashSet};

use crate::backend::{BackendError, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::Rect;
use crate::placement;
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Space between columns, between stacked windows and around the edges.
    pub gap: f64,
    /// Column widths `CycleWidth` steps through, as fractions of the screen.
    pub width_presets: Vec<f64>,
}

impl Default for Config {
    fn default() -> Self {
        Self { gap: 8.0, width_presets: vec![1.0 / 3.0, 0.5, 2.0 / 3.0] }
    }
}

/// Keyboard actions, matching PaperWM's defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Command {
    FocusLeft = 0,
    FocusRight = 1,
    FocusUp = 2,
    FocusDown = 3,
    MoveLeft = 4,
    MoveRight = 5,
    MoveUp = 6,
    MoveDown = 7,
    CycleWidth = 8,
    ToggleFullWidth = 9,
    Center = 10,
    /// Pull the next column's window into the focused column.
    Consume = 11,
    /// Push the focused window out into its own column.
    Expel = 12,
}

impl TryFrom<u32> for Command {
    type Error = u32;

    fn try_from(value: u32) -> std::result::Result<Self, u32> {
        use Command::*;
        const ALL: [Command; 13] = [
            FocusLeft,
            FocusRight,
            FocusUp,
            FocusDown,
            MoveLeft,
            MoveRight,
            MoveUp,
            MoveDown,
            CycleWidth,
            ToggleFullWidth,
            Center,
            Consume,
            Expel,
        ];
        ALL.get(value as usize).copied().ok_or(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Monitor {
    /// Usable area (without menu bar and Dock), in Accessibility coordinates.
    area: Rect,
    workspace: Workspace,
}

pub struct Engine {
    backend: Box<dyn WindowBackend>,
    config: Config,
    monitors: Vec<Monitor>,
    active: usize,
    /// Last frame set on each window, to skip redundant Accessibility calls.
    applied: HashMap<WindowId, Rect>,
}

impl Engine {
    pub fn new(backend: Box<dyn WindowBackend>) -> Self {
        Self { backend, config: Config::default(), monitors: Vec::new(), active: 0, applied: HashMap::new() }
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

    pub fn workspace(&self, monitor: usize) -> Option<&Workspace> {
        self.monitors.get(monitor).map(|m| &m.workspace)
    }

    /// Sets the usable area of each monitor, the main one first. Workspaces
    /// stay with their monitor index; windows of removed monitors move to
    /// the first one.
    pub fn set_monitors(&mut self, areas: &[Rect]) -> Result<()> {
        if areas.is_empty() {
            return Ok(());
        }
        let mut old = std::mem::take(&mut self.monitors).into_iter();
        self.monitors = areas
            .iter()
            .map(|&area| Monitor { area, workspace: old.next().map(|m| m.workspace).unwrap_or_default() })
            .collect();
        let viewport = self.viewport(0);
        for orphan in old.flat_map(|m| m.workspace.windows().collect::<Vec<_>>()) {
            self.monitors[0].workspace.insert_window(orphan, viewport / 2.0, viewport);
        }
        self.active = self.active.min(self.monitors.len() - 1);
        self.applied.clear();
        self.ensure_visible_all();
        self.apply()
    }

    /// Reconciles the layout with the windows that exist now: adds new ones
    /// next to the focused column, drops closed or minimized ones, and
    /// follows the system's keyboard focus.
    pub fn sync(&mut self) -> Result<()> {
        if self.monitors.is_empty() {
            return Ok(());
        }
        let windows = self.backend.windows()?;
        let tileable: HashMap<WindowId, &WindowInfo> =
            windows.iter().filter(|w| w.is_tileable()).map(|w| (w.id, w)).collect();

        let mut managed = HashSet::new();
        for monitor in &mut self.monitors {
            let gone: Vec<WindowId> = monitor.workspace.windows().filter(|id| !tileable.contains_key(id)).collect();
            for id in gone {
                monitor.workspace.remove_window(id);
                self.applied.remove(&id);
            }
            managed.extend(monitor.workspace.windows());
        }

        let mut new: Vec<&WindowInfo> = tileable.values().copied().filter(|w| !managed.contains(&w.id)).collect();
        new.sort_by(|a, b| a.frame.x.total_cmp(&b.frame.x).then(a.id.cmp(&b.id)));
        for window in new {
            let index = self.monitor_at(window.frame).unwrap_or(self.active);
            let viewport = self.viewport(index);
            self.monitors[index].workspace.insert_window(window.id, window.frame.width, viewport);
        }

        if let Some(focused) = self.backend.focused_window() {
            if let Some(index) = self.monitors.iter().position(|m| m.workspace.contains(focused)) {
                self.monitors[index].workspace.focus_window(focused);
                self.active = index;
            }
        }

        self.ensure_visible_all();
        self.apply()
    }

    /// Runs a keyboard command on the active monitor. Returns whether it
    /// changed anything.
    pub fn command(&mut self, command: Command) -> Result<bool> {
        let gap = self.config.gap;
        let viewport = self.viewport(self.active);
        let Some(monitor) = self.monitors.get_mut(self.active) else { return Ok(false) };
        let ws = &mut monitor.workspace;
        let before = ws.focused_window();
        let changed = match command {
            Command::FocusLeft => ws.focus_column(-1),
            Command::FocusRight => ws.focus_column(1),
            Command::FocusUp => ws.focus_row(-1),
            Command::FocusDown => ws.focus_row(1),
            Command::MoveLeft => ws.move_column(-1),
            Command::MoveRight => ws.move_column(1),
            Command::MoveUp => ws.move_row(-1),
            Command::MoveDown => ws.move_row(1),
            Command::CycleWidth => ws.cycle_width(&self.config.width_presets, viewport, gap),
            Command::ToggleFullWidth => ws.toggle_full_width(viewport),
            Command::Consume => ws.consume(),
            Command::Expel => ws.expel(),
            Command::Center => {
                ws.center_focused(viewport, gap);
                true
            }
        };
        if command != Command::Center {
            ws.ensure_focused_visible(viewport, gap);
        }
        let after = ws.focused_window();

        self.apply()?;
        if let Some(id) = after.filter(|&id| Some(id) != before) {
            match self.backend.focus(id) {
                Ok(()) | Err(BackendError::WindowNotFound(_)) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(changed)
    }

    pub fn observe_app(&self, pid: i32) {
        self.backend.observe_app(pid);
    }

    pub fn forget_app(&self, pid: i32) {
        self.backend.forget_app(pid);
    }

    pub fn set_change_handler(&self, handler: Box<dyn Fn()>) {
        self.backend.set_change_handler(handler);
    }

    fn viewport(&self, monitor: usize) -> f64 {
        self.monitors.get(monitor).map_or(0.0, |m| (m.area.width - 2.0 * self.config.gap).max(1.0))
    }

    fn monitor_at(&self, frame: Rect) -> Option<usize> {
        let (cx, cy) = (frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);
        self.monitors
            .iter()
            .position(|m| cx >= m.area.x && cx < m.area.max_x() && cy >= m.area.y && cy < m.area.max_y())
    }

    fn ensure_visible_all(&mut self) {
        for i in 0..self.monitors.len() {
            let viewport = self.viewport(i);
            self.monitors[i].workspace.ensure_focused_visible(viewport, self.config.gap);
        }
    }

    /// Moves every managed window to its place in the layout.
    fn apply(&mut self) -> Result<()> {
        let areas: Vec<Rect> = self.monitors.iter().map(|m| m.area).collect();
        for (i, monitor) in self.monitors.iter().enumerate() {
            let others: Vec<Rect> = areas.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, &a)| a).collect();
            for (id, frame) in placement::frames(&monitor.workspace, monitor.area, self.config.gap, &others) {
                if self.applied.get(&id).is_some_and(|f| approx_eq(*f, frame)) {
                    continue;
                }
                match self.backend.set_frame(id, frame) {
                    Ok(()) => {
                        self.applied.insert(id, frame);
                    }
                    Err(BackendError::NotTrusted) => return Err(BackendError::NotTrusted),
                    // Closed windows are dropped on the next sync; windows
                    // that refuse a size keep their own.
                    Err(_) => {}
                }
            }
        }
        Ok(())
    }
}

fn approx_eq(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < 0.5
        && (a.y - b.y).abs() < 0.5
        && (a.width - b.width).abs() < 0.5
        && (a.height - b.height).abs() < 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
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
        fn set_frame(&self, id: WindowId, frame: Rect) -> Result<()> {
            self.0.set_frame(id, frame)
        }
        fn focused_window(&self) -> Option<WindowId> {
            self.0.focused_window()
        }
        fn focus(&self, id: WindowId) -> Result<()> {
            self.0.focus(id)
        }
    }

    // Viewport is 1000 wide with the default 8pt gap.
    const SCREEN: Rect = Rect::new(0.0, 25.0, 1016.0, 600.0);

    fn engine_with(windows: Vec<WindowInfo>) -> (Engine, Rc<MockBackend>) {
        let backend = Rc::new(MockBackend::new(windows));
        let mut engine = Engine::new(Box::new(Shared(backend.clone())));
        engine.set_monitors(&[SCREEN]).unwrap();
        (engine, backend)
    }

    fn window(id: WindowId, x: f64, width: f64) -> WindowInfo {
        MockBackend::window(id, Rect::new(x, 100.0, width, 300.0))
    }

    fn columns(engine: &Engine) -> Vec<Vec<WindowId>> {
        engine.workspace(0).unwrap().columns().iter().map(|c| c.windows().to_vec()).collect()
    }

    #[test]
    fn first_sync_adopts_windows_left_to_right() {
        let (mut engine, backend) = engine_with(vec![window(1, 500.0, 300.0), window(2, 0.0, 400.0)]);
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![2], vec![1]]);
        assert_eq!(backend.frame_of(2), Some(Rect::new(8.0, 33.0, 400.0, 584.0)));
        assert_eq!(backend.frame_of(1), Some(Rect::new(416.0, 33.0, 300.0, 584.0)));
    }

    #[test]
    fn ignores_dialogs_and_minimized_windows() {
        let mut dialog = window(2, 0.0, 300.0);
        dialog.is_standard = false;
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), dialog]);
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1]]);
        assert_eq!(backend.frame_of(2), Some(Rect::new(0.0, 100.0, 300.0, 300.0)));

        backend.update(1, |w| w.is_minimized = true);
        engine.sync().unwrap();
        assert!(columns(&engine).is_empty());
    }

    #[test]
    fn new_window_opens_right_of_focus_and_closed_window_leaves() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.sync().unwrap();

        backend.add(window(3, 900.0, 300.0));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![3], vec![2]]);

        backend.remove(3);
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2]]);
        assert_eq!(backend.frame_of(2).unwrap().x, 316.0);
    }

    #[test]
    fn follows_system_focus_and_scrolls_to_it() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 600.0), window(2, 100.0, 600.0), window(3, 200.0, 600.0)]);
        engine.sync().unwrap();
        assert_eq!(engine.workspace(0).unwrap().focused_window(), Some(3));

        backend.set_focused(Some(1));
        engine.sync().unwrap();
        assert_eq!(engine.workspace(0).unwrap().scroll(), 0.0);
        assert_eq!(backend.frame_of(1).unwrap().x, 8.0);
    }

    #[test]
    fn focus_command_focuses_the_window_and_scrolls() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 600.0), window(2, 100.0, 600.0), window(3, 200.0, 600.0)]);
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::FocusLeft), Ok(true));
        assert_eq!(backend.focused_window(), Some(2));
        assert_eq!(backend.frame_of(2).unwrap().x, 8.0);
        assert_eq!(engine.command(Command::FocusRight), Ok(true));
        assert_eq!(engine.command(Command::FocusRight), Ok(false));
    }

    #[test]
    fn move_and_stack_commands_rearrange_columns() {
        let (mut engine, _) =
            engine_with(vec![window(1, 0.0, 300.0), window(2, 100.0, 300.0), window(3, 200.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::MoveLeft).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![3], vec![2]]);
        engine.command(Command::Consume).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![3, 2]]);
        engine.command(Command::FocusDown).unwrap();
        engine.command(Command::Expel).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![3], vec![2]]);
    }

    #[test]
    fn unchanged_layout_does_not_move_windows_again() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 100.0, 300.0)]);
        engine.sync().unwrap();
        let calls = backend.set_frame_calls();
        engine.sync().unwrap();
        assert_eq!(backend.set_frame_calls(), calls);
    }

    #[test]
    fn windows_go_to_the_monitor_they_are_on() {
        let (mut engine, _) = engine_with(vec![window(1, 0.0, 300.0), window(2, 1200.0, 300.0)]);
        engine.set_monitors(&[SCREEN, Rect::new(1016.0, 0.0, 800.0, 600.0)]).unwrap();
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1]]);
        assert_eq!(engine.workspace(1).unwrap().columns()[0].windows(), &[2]);

        engine.set_monitors(&[SCREEN]).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2]]);
    }

    #[test]
    fn command_parsing_rejects_unknown_values() {
        assert_eq!(Command::try_from(0), Ok(Command::FocusLeft));
        assert_eq!(Command::try_from(12), Ok(Command::Expel));
        assert_eq!(Command::try_from(13), Err(13));
    }

    #[test]
    fn reports_missing_permission() {
        let mut engine = Engine::new(Box::new(MockBackend::untrusted()));
        engine.set_monitors(&[SCREEN]).unwrap();
        assert_eq!(engine.sync(), Err(BackendError::NotTrusted));
    }
}
