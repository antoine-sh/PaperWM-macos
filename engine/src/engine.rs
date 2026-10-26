//! The engine object the app talks to: keeps one scrolling workspace per
//! monitor in step with the real windows.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::animator::Animator;
use crate::applier::FrameApplier;
use crate::backend::{BackendError, Result, WindowBackend, WindowId, WindowInfo};
use crate::geometry::{Point, Rect};
use crate::placement;
use crate::workspace::{CenterFocus, Pin, Workspace};

/// Number of PaperWM workspaces.
pub const WORKSPACE_COUNT: usize = 9;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Space between columns, between stacked windows and around the edges.
    pub gap: f64,
    /// Column widths `CycleWidth` steps through, as fractions of the screen.
    pub width_presets: Vec<f64>,
    /// Share of the screen `WidenColumn` and `NarrowColumn` add or take away.
    pub width_step: f64,
    /// One name per workspace.
    pub workspace_names: Vec<String>,
    /// Seconds the strip takes to slide to a new scroll position; 0 jumps.
    pub animation_duration: f64,
    /// When the focused column is kept centred.
    pub center_focused: CenterFocus,
    /// How windows of particular apps are placed; the first rule naming an
    /// app wins.
    pub app_rules: Vec<AppRule>,
}

/// Placement for every window of one app.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AppRule {
    /// The app's name as macOS shows it, e.g. "System Settings".
    pub app: String,
    pub action: AppAction,
    /// For `Slot`: the workspace index the app's windows open on.
    #[serde(default)]
    pub workspace: usize,
    /// For `Slot`: the column index they open at, or the end of the strip
    /// if it has fewer columns.
    #[serde(default)]
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppAction {
    /// Left out of tiling, like dialogs: never moved, on every workspace.
    Float,
    /// Tiled at a fixed column of a fixed workspace when it opens.
    Slot,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gap: 8.0,
            width_presets: vec![1.0 / 3.0, 0.5, 2.0 / 3.0],
            width_step: 0.1,
            workspace_names: (1..=WORKSPACE_COUNT).map(|i| i.to_string()).collect(),
            animation_duration: 0.25,
            center_focused: CenterFocus::Never,
            app_rules: Vec::new(),
        }
    }
}

impl Config {
    /// The rule for windows of the app named `app`.
    pub fn rule_for(&self, app: &str) -> Option<&AppRule> {
        self.app_rules.iter().find(|r| r.app == app)
    }

    /// Parses the app's settings; missing or empty fields, and a width step
    /// that is not positive, take their defaults. Returns `None` for
    /// malformed JSON or a negative gap or animation duration. Rules naming a
    /// workspace that does not exist are dropped.
    pub fn from_json(json: &str) -> Option<Self> {
        let mut config: Config = serde_json::from_str(json).ok()?;
        if !(config.gap >= 0.0 && config.animation_duration >= 0.0) {
            return None;
        }
        let defaults = Config::default();
        if config.width_presets.is_empty() {
            config.width_presets = defaults.width_presets;
        }
        if config.width_step <= 0.0 {
            config.width_step = defaults.width_step;
        }
        config.workspace_names.truncate(WORKSPACE_COUNT);
        let have = config.workspace_names.len();
        config.workspace_names.extend(defaults.workspace_names.into_iter().skip(have));
        config.app_rules.retain(|r| r.workspace < WORKSPACE_COUNT);
        Some(config)
    }
}

/// Keyboard and gesture actions. The numbers are the `PWM_COMMAND_*`
/// values of the C interface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
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
    /// Pull the next column's window into the focused column.
    Consume,
    /// Push the focused window out into its own column.
    Expel,
    SwitchWorkspace(usize),
    MoveToWorkspace(usize),
    NextWorkspace,
    PreviousWorkspace,
    MoveToNextWorkspace,
    MoveToPreviousWorkspace,
    /// The most recently shown other workspace.
    LastWorkspace,
    FocusMonitorLeft,
    FocusMonitorRight,
    MoveToMonitorLeft,
    MoveToMonitorRight,
    ToggleScratch,
    /// Moves the focused window into the scratch layer, or back out.
    MoveToScratch,
    FocusWindow(WindowId),
    /// Pans the active strip by this many points without changing focus.
    ScrollBy(f64),
    /// Ends a pan: focuses the column nearest the middle of the screen.
    ScrollEnd,
    FocusFirst,
    FocusLast,
    /// Widens the focused column by `Config::width_step` of the screen.
    WidenColumn,
    NarrowColumn,
    /// Switches the focused column between stacked and tabbed.
    ToggleTabbed,
    /// Pins the focused column to the left edge, or unpins it.
    TogglePinLeft,
    /// Pins the focused column to the right edge, or unpins it.
    TogglePinRight,
}

impl Command {
    /// Decodes a `PWM_COMMAND_*` value and its argument.
    pub fn parse(code: u32, arg: f64) -> Option<Self> {
        use Command::*;
        let index = || (arg >= 0.0 && arg.fract() == 0.0 && arg <= u32::MAX as f64).then_some(arg as u32);
        Some(match code {
            0 => FocusLeft,
            1 => FocusRight,
            2 => FocusUp,
            3 => FocusDown,
            4 => MoveLeft,
            5 => MoveRight,
            6 => MoveUp,
            7 => MoveDown,
            8 => CycleWidth,
            9 => ToggleFullWidth,
            10 => Center,
            11 => Consume,
            12 => Expel,
            13 => SwitchWorkspace(index()? as usize),
            14 => MoveToWorkspace(index()? as usize),
            15 => NextWorkspace,
            16 => PreviousWorkspace,
            17 => MoveToNextWorkspace,
            18 => MoveToPreviousWorkspace,
            19 => LastWorkspace,
            20 => FocusMonitorLeft,
            21 => FocusMonitorRight,
            22 => MoveToMonitorLeft,
            23 => MoveToMonitorRight,
            24 => ToggleScratch,
            25 => MoveToScratch,
            26 => FocusWindow(index()?),
            27 if arg.is_finite() => ScrollBy(arg),
            28 => ScrollEnd,
            29 => FocusFirst,
            30 => FocusLast,
            31 => WidenColumn,
            32 => NarrowColumn,
            33 => ToggleTabbed,
            34 => TogglePinLeft,
            35 => TogglePinRight,
            _ => return None,
        })
    }
}

/// Snapshot of the layout for the app's UI, sent as JSON.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct State {
    pub active_monitor: usize,
    pub gap: f64,
    pub monitors: Vec<MonitorState>,
    pub workspaces: Vec<WorkspaceState>,
    pub windows: Vec<WindowState>,
    /// Window ids, most recently focused first.
    pub mru: Vec<WindowId>,
    pub scratch: Vec<WindowId>,
    pub scratch_visible: bool,
    /// The window with keyboard focus, managed or not, as far as the engine
    /// knows: what macOS last reported, or what the engine just focused.
    pub focused_window: Option<WindowId>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MonitorState {
    pub area: Rect,
    pub workspace: usize,
    /// Width of the strip's viewport: the area without the outer gaps.
    pub viewport: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceState {
    pub name: String,
    pub scroll: f64,
    pub focused_column: usize,
    pub columns: Vec<ColumnState>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColumnState {
    /// Strip x of the column's left edge.
    pub x: f64,
    pub width: f64,
    pub windows: Vec<WindowId>,
    pub focused_row: usize,
    /// Only the focused window shows; the UI draws a tab bar for the rest.
    pub tabbed: bool,
    /// The edge the column is pinned to, if any; its `x` is where it is shown.
    pub pinned: Option<Pin>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowState {
    pub id: WindowId,
    pub pid: i32,
    pub app_name: String,
    pub title: String,
    /// Where the engine put the window, or where it was last seen.
    pub frame: Rect,
}

#[derive(Debug, Clone, PartialEq)]
struct Monitor {
    /// Usable area (without menu bar and Dock), in Accessibility coordinates.
    area: Rect,
    /// Index of the workspace shown on this monitor.
    workspace: usize,
}

pub struct Engine {
    backend: Box<dyn WindowBackend>,
    config: Config,
    /// Always `WORKSPACE_COUNT` long. Each is shown on at most one monitor.
    workspaces: Vec<Workspace>,
    monitors: Vec<Monitor>,
    /// Index of the monitor keyboard commands act on.
    active: usize,
    /// The workspace shown before the latest switch, for `LastWorkspace`.
    last_workspace: Option<usize>,
    /// Floating windows above all workspaces, with the frame each returns to.
    scratch: Vec<(WindowId, Rect)>,
    scratch_visible: bool,
    /// Windows the user focused, most recent first.
    mru: Vec<WindowId>,
    /// System focus seen just before the engine last moved focus. macOS
    /// applies focus changes asynchronously, so until the reported focus
    /// differs from this, it is old news and must not be followed.
    stale_focus: Option<WindowId>,
    /// The focused window for the UI; see `State::focused_window`.
    system_focus: Option<WindowId>,
    applier: FrameApplier,
    /// What the last sync saw of each managed window, for the UI.
    info: HashMap<WindowId, WindowInfo>,
    /// The scroll on screen for each workspace, sliding toward its target.
    animator: Animator,
}

impl Engine {
    pub fn new(backend: Box<dyn WindowBackend>) -> Self {
        Self {
            backend,
            config: Config::default(),
            workspaces: vec![Workspace::default(); WORKSPACE_COUNT],
            monitors: Vec::new(),
            active: 0,
            last_workspace: None,
            scratch: Vec::new(),
            scratch_visible: true,
            mru: Vec::new(),
            stale_focus: None,
            system_focus: None,
            applier: FrameApplier::default(),
            info: HashMap::new(),
            animator: Animator::new(WORKSPACE_COUNT),
        }
    }

    /// An engine driving the current platform's windows.
    pub fn with_platform_backend() -> Self {
        Self::new(crate::platform::default_backend())
    }

    pub fn backend(&self) -> &dyn WindowBackend {
        self.backend.as_ref()
    }

    /// Every window, read afresh.
    pub fn windows(&self) -> Result<Vec<WindowInfo>> {
        self.backend.invalidate();
        let windows = self.backend.windows();
        // The next sync must see what this call read.
        self.backend.invalidate();
        windows
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Replaces the settings, laying windows out again only when a setting
    /// that affects the layout changed.
    pub fn set_config(&mut self, config: Config) -> Result<()> {
        let relayout = config.gap != self.config.gap || config.center_focused != self.config.center_focused;
        let resnap = relayout || config.animation_duration != self.config.animation_duration;
        let rules_changed = config.app_rules != self.config.app_rules;
        self.config = config;
        if relayout {
            self.ensure_visible_all();
        }
        if resnap {
            self.snap_all();
        }
        if rules_changed {
            // Floating windows are not tracked, so every app is read again
            // to tile the ones that no longer float, or release new floaters.
            // The sync lays everything out, so it comes last.
            self.backend.invalidate();
            return self.sync();
        }
        if relayout {
            self.apply()?;
        }
        Ok(())
    }

    pub fn workspace(&self, index: usize) -> Option<&Workspace> {
        self.workspaces.get(index)
    }

    pub fn state(&self) -> State {
        let workspaces = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, ws)| {
                let monitor = self.monitors.iter().position(|m| m.workspace == i).unwrap_or(self.active);
                WorkspaceState {
                    name: self.config.workspace_names.get(i).cloned().unwrap_or_default(),
                    scroll: ws.scroll(),
                    focused_column: ws.focused_column(),
                    columns: ws
                        .columns()
                        .iter()
                        .zip(ws.column_xs(self.viewport(monitor), self.config.gap))
                        .map(|(c, x)| ColumnState {
                            x,
                            width: c.width(),
                            windows: c.windows().to_vec(),
                            focused_row: c.focused_row(),
                            tabbed: c.is_tabbed(),
                            pinned: c.pinned(),
                        })
                        .collect(),
                }
            })
            .collect();
        let windows: Vec<WindowState> = self
            .workspaces
            .iter()
            .flat_map(Workspace::windows)
            .chain(self.scratch.iter().map(|&(id, _)| id))
            .filter_map(|id| self.info.get(&id))
            .map(|w| WindowState {
                id: w.id,
                pid: w.pid,
                app_name: w.app_name.clone(),
                title: w.title.clone(),
                frame: self.applier.get(w.id).unwrap_or(w.frame),
            })
            .collect();
        // Windows never focused yet follow the ones that were, in layout order.
        let focused: HashSet<WindowId> = self.mru.iter().copied().collect();
        let mut mru = self.mru.clone();
        mru.extend(windows.iter().map(|w| w.id).filter(|id| !focused.contains(id)));
        State {
            active_monitor: self.active,
            gap: self.config.gap,
            monitors: self
                .monitors
                .iter()
                .enumerate()
                .map(|(i, m)| MonitorState { area: m.area, workspace: m.workspace, viewport: self.viewport(i) })
                .collect(),
            workspaces,
            windows,
            mru,
            scratch: self.scratch.iter().map(|&(id, _)| id).collect(),
            scratch_visible: self.scratch_visible,
            focused_window: self.system_focus,
        }
    }

    /// Sets the usable area of each monitor, the main one first. Monitors
    /// keep the workspace they showed by index; new ones get the first
    /// workspace not shown anywhere. Workspaces of removed monitors are
    /// hidden with their windows.
    pub fn set_monitors(&mut self, areas: &[Rect]) -> Result<()> {
        if areas.is_empty() {
            return Ok(());
        }
        let mut monitors: Vec<Monitor> = Vec::with_capacity(areas.len());
        for (i, &area) in areas.iter().enumerate() {
            let kept = self.monitors.get(i).map(|m| m.workspace);
            let workspace = kept
                .filter(|w| monitors.iter().all(|m| m.workspace != *w))
                .or_else(|| (0..WORKSPACE_COUNT).find(|w| monitors.iter().all(|m| m.workspace != *w)))
                .unwrap_or(0);
            monitors.push(Monitor { area, workspace });
        }
        self.monitors = monitors;
        self.active = self.active.min(self.monitors.len() - 1);
        // macOS moves windows itself when displays change.
        self.applier.clear();
        self.backend.invalidate();
        self.ensure_visible_all();
        self.snap_all();
        self.apply()
    }

    /// Reconciles the layout with the windows that exist now: adds new ones
    /// next to the focused column, drops closed or minimized ones, puts back
    /// tiled windows that were moved, and follows the system's keyboard
    /// focus, bringing in its workspace. Windows of apps the backend skipped,
    /// busy or unchanged, are kept as they were.
    pub fn sync(&mut self) -> Result<()> {
        self.sync_dropping(None)
    }

    /// Syncs after the user let go of the mouse at `x`, `y` at the end of a
    /// drag. A tiled window dragged there by its title bar moves into the
    /// place of the column under the pointer, on whichever monitor that is;
    /// the strip rearranges around it.
    pub fn drop_at(&mut self, x: f64, y: f64) -> Result<()> {
        self.sync_dropping(Some(Point { x, y }))
    }

    fn sync_dropping(&mut self, drop: Option<Point>) -> Result<()> {
        if self.monitors.is_empty() {
            return Ok(());
        }
        let mut current: HashMap<WindowId, WindowInfo> =
            self.backend.windows()?.into_iter().filter(WindowInfo::is_tileable).map(|w| (w.id, w)).collect();
        // Windows whose frame was just read, as opposed to kept from before.
        let fresh: HashSet<WindowId> = current.keys().copied().collect();
        let skipped = self.backend.skipped_apps();
        if !skipped.is_empty() {
            for (&id, info) in &self.info {
                if skipped.contains(&info.pid) {
                    current.entry(id).or_insert_with(|| info.clone());
                }
            }
        }
        // Windows of floating apps leave the layout; ones it held go back on
        // screen, since they may be parked or hidden: centred on the monitor
        // showing them, or else the active one.
        let floating: Vec<WindowId> = current
            .values()
            .filter(|w| self.config.rule_for(&w.app_name).is_some_and(|r| r.action == AppAction::Float))
            .map(|w| w.id)
            .collect();
        let mut released = Vec::new();
        for id in floating {
            let info = current.remove(&id).expect("listed above");
            if self.info.contains_key(&id) {
                let workspace = self.workspaces.iter().position(|w| w.contains(id));
                let monitor = self.monitors.iter().find(|m| Some(m.workspace) == workspace);
                let area = monitor.unwrap_or(&self.monitors[self.active]).area;
                released.push((id, centred(self.applier.get(id).unwrap_or(info.frame), area)));
                self.applier.forget(id);
            }
        }
        let previous = std::mem::replace(&mut self.info, current);
        self.follow_tabs(&previous);

        self.scratch.retain(|(id, _)| self.info.contains_key(id));
        self.mru.retain(|id| self.info.contains_key(id));
        if self.scratch_visible {
            // Keep wherever the user dragged them.
            for (id, frame) in &mut self.scratch {
                *frame = self.info[id].frame;
            }
        }
        let mut managed: HashSet<WindowId> = self.scratch.iter().map(|&(id, _)| id).collect();
        for workspace in &mut self.workspaces {
            let gone: Vec<WindowId> = workspace.windows().filter(|id| !self.info.contains_key(id)).collect();
            for id in gone {
                workspace.remove_window(id);
                self.applier.forget(id);
            }
            managed.extend(workspace.windows());
        }

        let mut new: Vec<(WindowId, Rect)> =
            self.info.values().filter(|w| !managed.contains(&w.id)).map(|w| (w.id, w.frame)).collect();
        new.sort_by(|a, b| a.1.x.total_cmp(&b.1.x).then(a.0.cmp(&b.0)));
        // Only the frontmost app's windows take focus; a window popped up by
        // an app in the background opens beside the focused column, or in
        // its slot, without moving focus or the strip.
        let front = if new.is_empty() { None } else { self.backend.focused_app() };
        // The window each touched workspace keeps focused.
        let mut keep: HashMap<usize, Option<WindowId>> = HashMap::new();
        // Windows of one slotted app placed so far, so several arriving at
        // once keep their left-to-right order.
        let mut slotted: HashMap<String, usize> = HashMap::new();
        for (id, frame) in new {
            let app = &self.info[&id].app_name;
            let slot = self.config.rule_for(app).filter(|r| r.action == AppAction::Slot);
            let (index, monitor) = match slot {
                Some(rule) => {
                    let monitor = self.monitors.iter().position(|m| m.workspace == rule.workspace);
                    (rule.workspace, monitor.unwrap_or(self.active))
                }
                None => {
                    let monitor = self.monitor_at(frame).unwrap_or(self.active);
                    (self.monitors[monitor].workspace, monitor)
                }
            };
            let viewport = self.viewport(monitor);
            let ws = &mut self.workspaces[index];
            let kept = keep.entry(index).or_insert(ws.focused_window());
            match slot {
                Some(rule) => {
                    let placed = slotted.entry(app.clone()).or_default();
                    ws.insert_window_at(id, rule.column + *placed, frame.width, viewport);
                    *placed += 1;
                }
                None => ws.insert_window(id, frame.width, viewport),
            }
            if front == Some(self.info[&id].pid) {
                *kept = Some(id);
            }
        }
        for (index, id) in keep {
            if let Some(id) = id {
                self.workspaces[index].focus_window(id);
            }
        }

        if let Some(point) = drop {
            let focused = self.backend.focused_window();
            self.drop_window(point, focused, &fresh);
        }
        for (id, frame) in released {
            match self.backend.set_frame(id, frame) {
                Ok(()) | Err(BackendError::WindowNotFound(_)) => {}
                Err(err) => return Err(err),
            }
        }

        // Tiled windows on screen that the user or their app moved.
        for monitor in &self.monitors {
            if self.animator.is_moving(monitor.workspace) {
                continue;
            }
            for id in self.workspaces[monitor.workspace].windows().filter(|id| fresh.contains(id)) {
                self.applier.observe(id, self.info[&id].frame);
            }
        }

        let focused = self.backend.focused_window();
        if focused.is_some() && focused == self.stale_focus {
            // Our own focus request has not landed yet.
        } else {
            self.stale_focus = None;
            self.system_focus = focused;
            if let Some(id) = focused {
                self.follow_focus(id);
                self.touch(id);
            }
        }

        self.ensure_visible_all();
        self.apply()
    }

    /// Gives the place of each managed window that went away to the tab that
    /// came forward in its native tab group, one it hid or one hiding it, so
    /// opening, switching and closing tabs keep the group's column.
    fn follow_tabs(&mut self, previous: &HashMap<WindowId, WindowInfo>) {
        for (&old, info) in previous {
            if self.info.contains_key(&old) {
                continue;
            }
            let managed =
                |id| self.scratch.iter().any(|&(w, _)| w == id) || self.workspaces.iter().any(|w| w.contains(id));
            let Some(new) = self
                .info
                .values()
                .find(|w| w.pid == info.pid && (w.tabs.contains(&old) || info.tabs.contains(&w.id)) && !managed(w.id))
                .map(|w| w.id)
            else {
                continue;
            };
            for workspace in &mut self.workspaces {
                workspace.replace_window(old, new);
            }
            for id in self.scratch.iter_mut().map(|(id, _)| id).chain(&mut self.mru) {
                if *id == old {
                    *id = new;
                }
            }
            self.applier.forget(old);
        }
    }

    /// Puts every window back in its place, including ones that seemed to
    /// refuse it, and syncs.
    pub fn retile(&mut self) -> Result<()> {
        self.applier.clear();
        self.backend.invalidate();
        self.sync()
    }

    /// Runs a keyboard command on the active monitor. Returns whether it
    /// changed anything.
    pub fn command(&mut self, command: Command) -> Result<bool> {
        if self.monitors.is_empty() {
            return Ok(false);
        }
        let gap = self.config.gap;
        let viewport = self.viewport(self.active);
        let current = self.monitors[self.active].workspace;
        let before = self.focused_window();
        // Set by commands that decide focus themselves.
        let mut raise = Vec::new();
        let ws = &mut self.workspaces[current];
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
            Command::Consume => {
                let changed = ws.consume();
                // The pulled-in window may have come up above the active tab.
                if changed && ws.columns()[ws.focused_column()].is_tabbed() {
                    raise.extend(ws.focused_window());
                }
                changed
            }
            Command::Expel => {
                let changed = ws.expel();
                let source = &ws.columns()[ws.focused_column().saturating_sub(1)];
                // The expelled window was the tab in front; bring up the one now active.
                if changed && source.is_tabbed() {
                    raise.push(source.focused_window());
                    raise.extend(ws.focused_window());
                }
                changed
            }
            Command::Center => {
                ws.center_focused(viewport, gap);
                true
            }
            Command::SwitchWorkspace(target) => self.show_workspace(target),
            Command::NextWorkspace => self.show_workspace((current + 1) % WORKSPACE_COUNT),
            Command::PreviousWorkspace => self.show_workspace((current + WORKSPACE_COUNT - 1) % WORKSPACE_COUNT),
            Command::LastWorkspace => self.last_workspace.is_some_and(|w| self.show_workspace(w)),
            Command::MoveToWorkspace(target) => self.move_to_workspace(target),
            Command::MoveToNextWorkspace => self.move_to_workspace((current + 1) % WORKSPACE_COUNT),
            Command::MoveToPreviousWorkspace => {
                self.move_to_workspace((current + WORKSPACE_COUNT - 1) % WORKSPACE_COUNT)
            }
            Command::FocusMonitorLeft => self.focus_monitor(false),
            Command::FocusMonitorRight => self.focus_monitor(true),
            Command::MoveToMonitorLeft => self.move_to_monitor(false),
            Command::MoveToMonitorRight => self.move_to_monitor(true),
            Command::ToggleScratch => self.toggle_scratch(&mut raise),
            Command::MoveToScratch => self.move_to_scratch(&mut raise),
            Command::FocusWindow(id) => self.focus_window(id, &mut raise),
            Command::ScrollBy(dx) => {
                // Pan from what is on screen, even mid-slide.
                ws.scroll_by(dx + self.animator.shown(current) - ws.scroll(), viewport, gap);
                true
            }
            Command::ScrollEnd => {
                ws.focus_nearest_centre(viewport, gap);
                true
            }
            Command::FocusFirst => ws.focus_edge(false),
            Command::FocusLast => ws.focus_edge(true),
            Command::WidenColumn => ws.nudge_width(self.config.width_step * viewport, viewport),
            Command::NarrowColumn => ws.nudge_width(-self.config.width_step * viewport, viewport),
            Command::ToggleTabbed => {
                // Brings the active tab in front of the windows now behind it.
                raise.extend(ws.focused_window());
                ws.toggle_tabbed()
            }
            Command::TogglePinLeft => ws.toggle_pin(Pin::Left, viewport, gap),
            Command::TogglePinRight => ws.toggle_pin(Pin::Right, viewport, gap),
        };
        if !matches!(command, Command::Center | Command::ScrollBy(_)) {
            self.ensure_visible_all();
        }
        if matches!(command, Command::ScrollBy(_)) {
            // The strip follows the fingers directly.
            self.animator.snap(current, self.workspaces[current].scroll());
        }

        self.apply()?;
        let after = self.focused_window();
        if raise.is_empty() {
            raise.extend(after.filter(|&id| Some(id) != before));
        }
        // A pan never hides or leaves a window, and arrives every frame.
        if after.is_none() && raise.is_empty() && !matches!(command, Command::ScrollBy(_)) {
            // Nothing to focus here, so macOS keeps focus on a window we just
            // hid or left; following it on the next sync would undo this.
            self.stale_focus = self.backend.focused_window();
        }
        if !raise.is_empty() {
            // Asked once: macOS moves focus asynchronously, so it would not
            // have changed between raises anyway.
            let previous = self.backend.focused_window();
            for id in raise {
                self.focus(id, previous)?;
            }
        }
        Ok(changed)
    }

    /// Advances strip slides by `seconds` and moves the windows. Returns
    /// whether any strip is still sliding.
    pub fn tick(&mut self, seconds: f64) -> Result<bool> {
        if self.animator.advance(seconds, self.config.animation_duration) {
            self.write_frames()?;
        }
        Ok(self.is_animating())
    }

    pub fn is_animating(&self) -> bool {
        self.animator.is_animating()
    }

    fn snap_all(&mut self) {
        for (index, ws) in self.workspaces.iter().enumerate() {
            self.animator.snap(index, ws.scroll());
        }
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

    /// The focused window of the active monitor's workspace.
    fn focused_window(&self) -> Option<WindowId> {
        self.monitors.get(self.active).and_then(|m| self.workspaces[m.workspace].focused_window())
    }

    /// Focuses `id`; `previous` is the system focus before the engine moved it.
    fn focus(&mut self, id: WindowId, previous: Option<WindowId>) -> Result<()> {
        match self.backend.focus(id) {
            Ok(()) => {
                self.stale_focus = previous.filter(|&p| p != id);
                self.system_focus = Some(id);
                self.touch(id);
                Ok(())
            }
            Err(BackendError::WindowNotFound(_)) => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Moves `id` to the front of the recently used list.
    fn touch(&mut self, id: WindowId) {
        if !self.info.contains_key(&id) {
            return;
        }
        self.mru.retain(|&w| w != id);
        self.mru.insert(0, id);
    }

    /// Focuses any managed window: tiled ones bring in their workspace,
    /// scratch ones show the scratch layer.
    fn focus_window(&mut self, id: WindowId, raise: &mut Vec<WindowId>) -> bool {
        if self.scratch.iter().any(|&(w, _)| w == id) {
            self.scratch_visible = true;
            raise.push(id);
            return true;
        }
        if !self.workspaces.iter().any(|w| w.contains(id)) {
            return false;
        }
        self.follow_focus(id);
        raise.push(id);
        true
    }

    /// Makes the workspace holding `id` current and focuses `id` in it.
    fn follow_focus(&mut self, id: WindowId) {
        let Some(workspace) = self.workspaces.iter().position(|w| w.contains(id)) else { return };
        match self.monitors.iter().position(|m| m.workspace == workspace) {
            Some(monitor) => self.active = monitor,
            None => {
                self.show_workspace(workspace);
            }
        }
        self.workspaces[workspace].focus_window(id);
    }

    /// Shows workspace `target` on the active monitor, swapping with the
    /// monitor that showed it, if any.
    fn show_workspace(&mut self, target: usize) -> bool {
        let current = self.monitors[self.active].workspace;
        if target >= WORKSPACE_COUNT || target == current {
            return false;
        }
        if let Some(other) = self.monitors.iter_mut().find(|m| m.workspace == target) {
            other.workspace = current;
        }
        self.monitors[self.active].workspace = target;
        self.last_workspace = Some(current);
        true
    }

    /// Moves the focused window next to workspace `target`'s focused column.
    fn move_to_workspace(&mut self, target: usize) -> bool {
        let current = self.monitors[self.active].workspace;
        let Some(id) = self.focused_window() else { return false };
        if target >= WORKSPACE_COUNT || target == current {
            return false;
        }
        let width = self.workspaces[current].columns()[self.workspaces[current].focused_column()].width();
        self.workspaces[current].remove_window(id);
        let monitor = self.monitors.iter().position(|m| m.workspace == target).unwrap_or(self.active);
        let viewport = self.viewport(monitor);
        self.workspaces[target].insert_window(id, width, viewport);
        true
    }

    /// Shows or hides the scratch layer. Shown windows are raised; hiding
    /// returns focus to the active workspace.
    fn toggle_scratch(&mut self, raise: &mut Vec<WindowId>) -> bool {
        if self.scratch.is_empty() {
            return false;
        }
        self.scratch_visible = !self.scratch_visible;
        if self.scratch_visible {
            raise.extend(self.scratch.iter().map(|&(id, _)| id));
        } else {
            raise.extend(self.focused_window());
        }
        true
    }

    /// Floats the focused tiled window in the scratch layer, centred on the
    /// active monitor, or tiles the focused scratch window again.
    fn move_to_scratch(&mut self, raise: &mut Vec<WindowId>) -> bool {
        let current = self.monitors[self.active].workspace;
        let system_focus = self.backend.focused_window();
        if let Some(index) = self.scratch.iter().position(|&(id, _)| Some(id) == system_focus) {
            let (id, frame) = self.scratch.remove(index);
            let viewport = self.viewport(self.active);
            self.workspaces[current].insert_window(id, frame.width, viewport);
            return true;
        }
        let Some(id) = self.focused_window() else { return false };
        self.workspaces[current].remove_window(id);
        let frame = centred(self.last_frame(id), self.monitors[self.active].area);
        self.scratch.push((id, frame));
        self.scratch_visible = true;
        raise.push(id);
        true
    }

    /// The nearest monitor whose centre is right (or left) of the active one's.
    fn monitor_toward(&self, right: bool) -> Option<usize> {
        let centre = |m: &Monitor| m.area.x + m.area.width / 2.0;
        let from = centre(&self.monitors[self.active]);
        self.monitors
            .iter()
            .enumerate()
            .map(|(i, m)| (i, centre(m) - from))
            .filter(|&(_, d)| if right { d > 0.0 } else { d < 0.0 })
            .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, _)| i)
    }

    fn focus_monitor(&mut self, right: bool) -> bool {
        let Some(monitor) = self.monitor_toward(right) else { return false };
        self.active = monitor;
        true
    }

    fn move_to_monitor(&mut self, right: bool) -> bool {
        let Some(monitor) = self.monitor_toward(right) else { return false };
        if !self.move_to_workspace(self.monitors[monitor].workspace) {
            return false;
        }
        self.active = monitor;
        true
    }

    /// Moves the tiled window that was dragged to `point`, if any, into the
    /// place of the column under it. Returns whether one was.
    fn drop_window(&mut self, point: Point, focused: Option<WindowId>, fresh: &HashSet<WindowId>) -> bool {
        let Some(monitor) = self.monitors.iter().position(|m| m.area.contains(point)) else { return false };
        let target = self.monitors[monitor].workspace;
        // Clicking the title bar focused the dragged window, which moved
        // without resizing and is still under the pointer. Sliding strips
        // only have windows in motion.
        let Some(id) = focused.filter(|id| fresh.contains(id)) else { return false };
        let Some(source) = self.workspaces.iter().position(|w| w.contains(id)) else { return false };
        let frame = self.info[&id].frame;
        if !self.monitors.iter().any(|m| m.workspace == source)
            || self.animator.is_moving(source)
            || self.animator.is_moving(target)
            || !frame.contains(point)
            || !self.applier.moved(id, frame)
        {
            return false;
        }
        let gap = self.config.gap;
        let strip_x = point.x - self.monitors[monitor].area.x - gap + self.animator.shown(target);
        let column = self.workspaces[target].column_at(strip_x, self.viewport(monitor), gap).unwrap_or(0);
        let moved = if source == target {
            self.workspaces[target].move_window_to(id, column)
        } else {
            self.workspaces[source].remove_window(id);
            let viewport = self.viewport(monitor);
            self.workspaces[target].insert_window_at(id, column, self.info[&id].frame.width, viewport);
            self.active = monitor;
            true
        };
        // Wherever it lands, it goes there rather than back.
        self.applier.forget(id);
        moved
    }

    fn viewport(&self, monitor: usize) -> f64 {
        self.monitors.get(monitor).map_or(0.0, |m| (m.area.width - 2.0 * self.config.gap).max(1.0))
    }

    fn monitor_at(&self, frame: Rect) -> Option<usize> {
        let centre = Point { x: frame.x + frame.width / 2.0, y: frame.y + frame.height / 2.0 };
        self.monitors.iter().position(|m| m.area.contains(centre))
    }

    fn ensure_visible_all(&mut self) {
        for i in 0..self.monitors.len() {
            let viewport = self.viewport(i);
            let ws = &mut self.workspaces[self.monitors[i].workspace];
            ws.follow_focus(self.config.center_focused, viewport, self.config.gap);
        }
    }

    /// Where `id` was put last, or where it was last seen.
    fn last_frame(&self, id: WindowId) -> Rect {
        self.applier.get(id).or(self.info.get(&id).map(|w| w.frame)).unwrap_or_default()
    }

    /// Points each shown strip's slide at its workspace's scroll, then moves
    /// every window into place.
    fn apply(&mut self) -> Result<()> {
        let animate = self.config.animation_duration > 0.0;
        for (index, ws) in self.workspaces.iter().enumerate() {
            if animate && self.monitors.iter().any(|m| m.workspace == index) {
                self.animator.retarget(index, ws.scroll());
            } else {
                self.animator.snap(index, ws.scroll());
            }
        }
        self.write_frames()
    }

    /// Moves every managed window to its place: shown workspaces in their
    /// strips as they are on screen now, everything else into the hiding
    /// corner.
    fn write_frames(&mut self) -> Result<()> {
        let areas: Vec<Rect> = self.monitors.iter().map(|m| m.area).collect();
        let mut frames = Vec::new();
        for (i, monitor) in self.monitors.iter().enumerate() {
            let others: Vec<Rect> = areas.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, &a)| a).collect();
            let workspace = &self.workspaces[monitor.workspace];
            let scroll = self.animator.shown(monitor.workspace);
            frames.extend(placement::frames(workspace, scroll, monitor.area, self.config.gap, &others));
        }
        for (index, workspace) in self.workspaces.iter().enumerate() {
            if self.monitors.iter().any(|m| m.workspace == index) {
                continue;
            }
            for id in workspace.windows() {
                frames.push((id, placement::hidden_frame(self.last_frame(id), &areas)));
            }
        }
        for &(id, frame) in &self.scratch {
            frames.push((id, if self.scratch_visible { frame } else { placement::hidden_frame(frame, &areas) }));
        }
        self.applier.write(self.backend.as_ref(), frames)
    }
}

/// `frame`'s size, centred in `area`.
fn centred(frame: Rect, area: Rect) -> Rect {
    Rect::new(
        area.x + (area.width - frame.width) / 2.0,
        area.y + (area.height - frame.height) / 2.0,
        frame.width,
        frame.height,
    )
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
        fn set_position(&self, id: WindowId, x: f64, y: f64) -> Result<()> {
            self.0.set_position(id, x, y)
        }
        fn set_size(&self, id: WindowId, width: f64, height: f64) -> Result<()> {
            self.0.set_size(id, width, height)
        }
        fn skipped_apps(&self) -> Vec<i32> {
            self.0.skipped_apps()
        }
        fn focused_window(&self) -> Option<WindowId> {
            self.0.focused_window()
        }
        fn focused_app(&self) -> Option<i32> {
            self.0.focused_app()
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
        engine.set_config(Config { animation_duration: 0.0, ..Config::default() }).unwrap();
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

    fn widths(engine: &Engine) -> Vec<f64> {
        engine.workspace(0).unwrap().columns().iter().map(|c| c.width()).collect()
    }

    #[test]
    fn tab_group_keeps_one_column_that_follows_the_shown_tab() {
        let windows = vec![window(1, 0.0, 300.0), window(2, 400.0, 500.0), window(4, 950.0, 200.0)];
        let (mut engine, backend) = engine_with(windows);
        engine.sync().unwrap();
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        let column = backend.frame_of(2).unwrap();

        backend.add_tab(2, 3);
        backend.set_focused(Some(3));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![3], vec![4]]);
        assert_eq!(widths(&engine), vec![300.0, 500.0, 200.0]);

        backend.select_tab(2);
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2], vec![4]]);

        // Closing the shown tab brings the next one forward in its place.
        backend.remove(2);
        backend.set_focused(Some(3));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![3], vec![4]]);
        assert_eq!(backend.frame_of(3), Some(column));
        assert_eq!(engine.state().focused_window, Some(3));
    }

    #[test]
    fn closing_a_background_tab_changes_nothing() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        backend.add_tab(1, 2);
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![2]]);
        let calls = backend.set_frame_calls();

        backend.remove(1);
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![2]]);
        assert_eq!(backend.set_frame_calls(), calls);
    }

    #[test]
    fn detached_tab_opens_its_own_column() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        backend.add_tab(1, 2);
        backend.add_tab(2, 3);
        backend.set_focused(Some(3));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![3]]);

        backend.detach_tab(1, Rect::new(500.0, 100.0, 400.0, 300.0));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![3], vec![1]]);

        // The shown tab keeps its column; the tab behind it opens beside it.
        backend.detach_tab(3, Rect::new(600.0, 100.0, 400.0, 300.0));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![3], vec![2], vec![1]]);
    }

    #[test]
    fn background_app_window_keeps_focus_and_scroll() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 600.0), window(2, 100.0, 600.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.sync().unwrap();
        // Focus on a dialog, which does not pin the strip's focus.
        backend.add(WindowInfo { pid: 101, is_standard: false, ..window(9, 300.0, 200.0) });
        backend.set_focused(Some(9));
        engine.sync().unwrap();
        let scroll = engine.workspace(0).unwrap().scroll();
        let frame = backend.frame_of(1);

        // App 103 is not frontmost.
        backend.add(window(3, 900.0, 600.0));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![3], vec![2]]);
        assert_eq!(engine.workspace(0).unwrap().focused_window(), Some(1));
        assert_eq!(engine.workspace(0).unwrap().scroll(), scroll);
        assert_eq!(backend.frame_of(1), frame);
        assert_eq!(backend.focused_window(), Some(9));

        // Two at once keep their left-to-right order.
        backend.add(window(4, 300.0, 300.0));
        backend.add(window(5, 200.0, 300.0));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![5], vec![4], vec![3], vec![2]]);
        assert_eq!(engine.workspace(0).unwrap().focused_window(), Some(1));
    }

    #[test]
    fn frontmost_app_window_takes_focus() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 600.0), window(2, 100.0, 600.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.sync().unwrap();
        backend.add(WindowInfo { pid: 101, is_standard: false, ..window(9, 300.0, 200.0) });
        backend.set_focused(Some(9));
        engine.sync().unwrap();

        // Same app as the focused dialog.
        backend.add(WindowInfo { pid: 101, ..window(3, 900.0, 600.0) });
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![3], vec![2]]);
        assert_eq!(engine.workspace(0).unwrap().focused_window(), Some(3));
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
    fn widen_column_resizes_and_scrolls_it_into_view() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 400.0), window(2, 500.0, 500.0)]);
        engine.sync().unwrap();
        engine.command(Command::FocusRight).unwrap();
        assert_eq!(engine.command(Command::WidenColumn), Ok(true));
        assert_eq!(engine.workspace(0).unwrap().scroll(), 8.0);
        assert_eq!(backend.frame_of(2), Some(Rect::new(408.0, 33.0, 600.0, 584.0)));
        assert_eq!(engine.command(Command::NarrowColumn), Ok(true));
        assert_eq!(engine.workspace(0).unwrap().scroll(), 0.0);
        assert_eq!(backend.frame_of(2).unwrap().width, 500.0);
    }

    #[test]
    fn width_step_sets_how_far_widen_and_narrow_go() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.set_config(Config { width_step: 0.25, animation_duration: 0.0, ..Config::default() }).unwrap();
        engine.sync().unwrap();
        engine.command(Command::WidenColumn).unwrap();
        assert_eq!(backend.frame_of(1).unwrap().width, 550.0);
        engine.command(Command::NarrowColumn).unwrap();
        assert_eq!(backend.frame_of(1).unwrap().width, 300.0);
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
    fn tabbed_column_shows_one_window_at_full_height_and_focus_switches_tabs() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 100.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::FocusLeft).unwrap();
        engine.command(Command::Consume).unwrap();
        engine.command(Command::FocusDown).unwrap();
        backend.set_focused(Some(1));
        assert_eq!(engine.command(Command::ToggleTabbed), Ok(true));
        let full = Rect::new(8.0, 33.0, 300.0, 584.0);
        assert_eq!(backend.frame_of(1), Some(full));
        assert_eq!(backend.frame_of(2), Some(full));
        // Toggling raises the active tab above the others.
        assert_eq!(backend.focused_window(), Some(2));
        assert!(engine.state().workspaces[0].columns[0].tabbed);

        assert_eq!(engine.command(Command::FocusUp), Ok(true));
        assert_eq!(backend.focused_window(), Some(1));
        assert_eq!(engine.command(Command::FocusUp), Ok(false));

        engine.command(Command::ToggleTabbed).unwrap();
        assert!(!engine.state().workspaces[0].columns[0].tabbed);
        assert_eq!(backend.frame_of(1).unwrap().height, 288.0);
    }

    #[test]
    fn tabbed_column_keeps_its_active_tab_in_front_through_consume_and_expel() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 300.0), window(2, 100.0, 300.0), window(3, 200.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::FocusFirst).unwrap();
        engine.command(Command::ToggleTabbed).unwrap();
        let before = backend.raised().len();
        engine.command(Command::Consume).unwrap();
        // The pulled-in window must not stay above the active tab.
        assert_eq!(backend.raised()[before..], [1]);
        engine.command(Command::Consume).unwrap();
        engine.command(Command::FocusDown).unwrap();
        engine.command(Command::Expel).unwrap();
        assert_eq!(columns(&engine), vec![vec![1, 3], vec![2]]);
        // The column's new active tab comes to the front, then the expelled window takes focus.
        assert_eq!(backend.raised()[backend.raised().len() - 2..], [3, 2]);
        assert_eq!(backend.focused_window(), Some(2));
    }

    #[test]
    fn unchanged_layout_does_not_move_windows_again() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 100.0, 300.0)]);
        engine.sync().unwrap();
        let calls = backend.set_frame_calls();
        engine.sync().unwrap();
        assert_eq!(backend.set_frame_calls(), calls);
    }

    const RIGHT: Rect = Rect::new(1016.0, 0.0, 800.0, 600.0);

    fn shown(engine: &Engine) -> Vec<usize> {
        engine.state().monitors.iter().map(|m| m.workspace).collect()
    }

    fn is_hidden(frame: Rect) -> bool {
        frame.x == SCREEN.max_x() - placement::PEEK && frame.y == SCREEN.max_y() - placement::PEEK
    }

    #[test]
    fn windows_go_to_the_monitor_they_are_on() {
        let (mut engine, _) = engine_with(vec![window(1, 0.0, 300.0), window(2, 1200.0, 300.0)]);
        engine.set_monitors(&[SCREEN, RIGHT]).unwrap();
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1]]);
        assert_eq!(engine.workspace(1).unwrap().columns()[0].windows(), &[2]);
    }

    #[test]
    fn monitors_show_distinct_workspaces() {
        let (mut engine, _) = engine_with(vec![]);
        engine.set_monitors(&[SCREEN, RIGHT]).unwrap();
        assert_eq!(shown(&engine), vec![0, 1]);
    }

    #[test]
    fn unplugged_monitor_hides_its_workspace() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 1200.0, 300.0)]);
        engine.set_monitors(&[SCREEN, RIGHT]).unwrap();
        engine.sync().unwrap();
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        assert_eq!(engine.state().active_monitor, 1);

        engine.set_monitors(&[SCREEN]).unwrap();
        assert_eq!(engine.state().active_monitor, 0);
        assert_eq!(columns(&engine), vec![vec![1]]);
        assert_eq!(engine.workspace(1).unwrap().columns()[0].windows(), &[2]);
        assert!(is_hidden(backend.frame_of(2).unwrap()));
    }

    #[test]
    fn switch_workspace_hides_old_windows() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::SwitchWorkspace(1)), Ok(true));
        assert_eq!(shown(&engine), vec![1]);
        assert!(is_hidden(backend.frame_of(1).unwrap()));
        assert_eq!(backend.frame_of(1).unwrap().width, 300.0);

        assert_eq!(engine.command(Command::SwitchWorkspace(0)), Ok(true));
        assert_eq!(backend.frame_of(1).unwrap().x, 8.0);
        assert_eq!(backend.focused_window(), Some(1));
    }

    #[test]
    fn switching_to_shown_or_unknown_workspace_is_noop() {
        let (mut engine, _) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::SwitchWorkspace(0)), Ok(false));
        assert_eq!(engine.command(Command::SwitchWorkspace(WORKSPACE_COUNT)), Ok(false));
        assert_eq!(shown(&engine), vec![0]);
    }

    #[test]
    fn switch_to_workspace_on_other_monitor_swaps() {
        let (mut engine, _) = engine_with(vec![]);
        engine.set_monitors(&[SCREEN, RIGHT]).unwrap();
        assert_eq!(engine.command(Command::SwitchWorkspace(1)), Ok(true));
        assert_eq!(shown(&engine), vec![1, 0]);
    }

    #[test]
    fn move_to_workspace_hides_window_and_focuses_neighbour() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::MoveToWorkspace(3)), Ok(true));
        assert_eq!(columns(&engine), vec![vec![1]]);
        assert_eq!(engine.workspace(3).unwrap().focused_window(), Some(2));
        assert!(is_hidden(backend.frame_of(2).unwrap()));
        assert_eq!(backend.focused_window(), Some(1));

        assert_eq!(engine.command(Command::MoveToWorkspace(0)), Ok(false));
        assert_eq!(engine.command(Command::MoveToNextWorkspace), Ok(true));
        assert_eq!(engine.workspace(1).unwrap().focused_window(), Some(1));
    }

    #[test]
    fn empty_workspace_has_no_focus() {
        let (mut engine, _) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::MoveToWorkspace(1)).unwrap();
        assert!(engine.workspace(0).unwrap().is_empty());
        assert_eq!(engine.command(Command::SwitchWorkspace(2)), Ok(true));
        assert_eq!(engine.command(Command::FocusLeft), Ok(false));
        assert_eq!(engine.command(Command::MoveToWorkspace(1)), Ok(false));
        assert_eq!(engine.workspace(2).unwrap().focused_window(), None);
    }

    #[test]
    fn focus_on_hidden_workspace_window_brings_it_in() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::MoveToWorkspace(3)).unwrap();
        // The user ⌘Tabs to window 2.
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        assert_eq!(shown(&engine), vec![3]);
        assert_eq!(backend.frame_of(2).unwrap().x, 8.0);
        assert!(is_hidden(backend.frame_of(1).unwrap()));
    }

    #[test]
    fn stale_system_focus_after_a_command_is_ignored() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(2));
        backend.set_focus_is_async(true);
        engine.command(Command::MoveToWorkspace(3)).unwrap();
        // macOS has not applied the focus change yet when the next sync runs.
        engine.sync().unwrap();
        assert_eq!(shown(&engine), vec![0]);
        // Once focus really changes, it is followed again.
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        assert_eq!(shown(&engine), vec![0]);
        backend.set_focused(Some(1));
        engine.sync().unwrap();
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        assert_eq!(shown(&engine), vec![3]);
    }

    #[test]
    fn closed_window_on_hidden_workspace_is_forgotten() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::MoveToWorkspace(3)).unwrap();
        backend.remove(2);
        engine.sync().unwrap();
        assert!(engine.workspace(3).unwrap().is_empty());
        assert!(engine.state().windows.iter().all(|w| w.id != 2));
    }

    #[test]
    fn next_previous_and_last_workspace() {
        let (mut engine, _) = engine_with(vec![]);
        engine.command(Command::SwitchWorkspace(8)).unwrap();
        engine.command(Command::NextWorkspace).unwrap();
        assert_eq!(shown(&engine), vec![0]);
        engine.command(Command::PreviousWorkspace).unwrap();
        assert_eq!(shown(&engine), vec![8]);
        engine.command(Command::SwitchWorkspace(2)).unwrap();
        assert_eq!(engine.command(Command::LastWorkspace), Ok(true));
        assert_eq!(shown(&engine), vec![8]);
        assert_eq!(engine.command(Command::LastWorkspace), Ok(true));
        assert_eq!(shown(&engine), vec![2]);
    }

    #[test]
    fn focus_monitor_right_activates_and_focuses() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 1200.0, 300.0)]);
        engine.set_monitors(&[SCREEN, RIGHT]).unwrap();
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::FocusMonitorRight), Ok(true));
        assert_eq!(engine.state().active_monitor, 1);
        assert_eq!(backend.focused_window(), Some(2));
        assert_eq!(engine.command(Command::FocusMonitorLeft), Ok(true));
        assert_eq!(backend.focused_window(), Some(1));
    }

    #[test]
    fn no_monitor_in_direction_returns_false() {
        let (mut engine, _) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::FocusMonitorRight), Ok(false));
        assert_eq!(engine.command(Command::MoveToMonitorLeft), Ok(false));
        assert_eq!(columns(&engine), vec![vec![1]]);
    }

    #[test]
    fn move_to_monitor_inserts_next_to_target_focus() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 300.0), window(2, 1100.0, 300.0), window(3, 1500.0, 300.0)]);
        engine.set_monitors(&[SCREEN, RIGHT]).unwrap();
        engine.sync().unwrap();
        // Window 2 is focused on the right monitor; focus window 1 on the left.
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::MoveToMonitorRight), Ok(true));
        assert!(engine.workspace(0).unwrap().is_empty());
        let right: Vec<Vec<WindowId>> =
            engine.workspace(1).unwrap().columns().iter().map(|c| c.windows().to_vec()).collect();
        assert_eq!(right, vec![vec![2], vec![1], vec![3]]);
        assert_eq!(engine.state().active_monitor, 1);
        assert_eq!(backend.focused_window(), Some(1));
        assert!(backend.frame_of(1).unwrap().x >= RIGHT.x);
    }

    /// Engine with windows 1 and 2 tiled and 2 moved to the scratch layer.
    fn engine_with_scratch() -> (Engine, Rc<MockBackend>) {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::MoveToScratch), Ok(true));
        (engine, backend)
    }

    #[test]
    fn pans_and_raises_ask_for_focus_sparingly() {
        let (mut engine, backend) = engine_with(vec![]);
        engine.sync().unwrap();
        let asked = backend.focus_queries();
        for _ in 0..5 {
            engine.command(Command::ScrollBy(5.0)).unwrap();
        }
        assert_eq!(backend.focus_queries(), asked);

        let (mut engine, backend) = engine_with_scratch();
        backend.add(window(3, 800.0, 300.0));
        backend.set_focused(Some(3));
        engine.sync().unwrap();
        engine.command(Command::MoveToScratch).unwrap();
        engine.command(Command::ToggleScratch).unwrap();
        let asked = backend.focus_queries();
        // Shows and raises both scratch windows.
        assert_eq!(engine.command(Command::ToggleScratch), Ok(true));
        assert_eq!(backend.focus_queries(), asked + 1);
    }

    #[test]
    fn tab_group_keeps_its_scratch_place() {
        let (mut engine, backend) = engine_with_scratch();
        backend.add_tab(2, 3);
        backend.set_focused(Some(3));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1]]);
        assert_eq!(engine.state().scratch, vec![3]);
    }

    #[test]
    fn move_to_scratch_floats_window_centered() {
        let (engine, backend) = engine_with_scratch();
        assert_eq!(columns(&engine), vec![vec![1]]);
        assert_eq!(engine.state().scratch, vec![2]);
        // 300x584 centred on the 1016x600 screen at y 25.
        assert_eq!(backend.frame_of(2), Some(Rect::new(358.0, 33.0, 300.0, 584.0)));
        assert_eq!(backend.focused_window(), Some(2));
    }

    #[test]
    fn toggle_scratch_parks_and_restores() {
        let (mut engine, backend) = engine_with_scratch();
        backend.update(2, |w| w.frame = Rect::new(100.0, 100.0, 400.0, 300.0));
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        assert_eq!(engine.command(Command::ToggleScratch), Ok(true));
        assert!(!engine.state().scratch_visible);
        assert!(is_hidden(backend.frame_of(2).unwrap()));
        assert_eq!(engine.command(Command::ToggleScratch), Ok(true));
        assert_eq!(backend.frame_of(2), Some(Rect::new(100.0, 100.0, 400.0, 300.0)));
        assert_eq!(backend.focused_window(), Some(2));
    }

    #[test]
    fn move_to_scratch_again_returns_to_tiling() {
        let (mut engine, backend) = engine_with_scratch();
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1]]);
        backend.set_focused(Some(2));
        assert_eq!(engine.command(Command::MoveToScratch), Ok(true));
        assert_eq!(columns(&engine), vec![vec![1], vec![2]]);
        assert!(engine.state().scratch.is_empty());
        assert_eq!(backend.frame_of(2).unwrap().x, 316.0);
    }

    #[test]
    fn closed_scratch_window_is_forgotten() {
        let (mut engine, backend) = engine_with_scratch();
        backend.remove(2);
        engine.sync().unwrap();
        assert!(engine.state().scratch.is_empty());
    }

    #[test]
    fn mru_tracks_focus_changes() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 300.0), window(2, 100.0, 300.0), window(3, 200.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.sync().unwrap();
        backend.set_focused(Some(2));
        engine.sync().unwrap();
        engine.command(Command::FocusLeft).unwrap();
        assert_eq!(engine.state().mru, vec![1, 2, 3]);
        backend.set_focused(Some(3));
        engine.sync().unwrap();
        assert_eq!(engine.state().mru, vec![3, 1, 2]);
        backend.remove(1);
        engine.sync().unwrap();
        assert_eq!(engine.state().mru, vec![3, 2]);
    }

    #[test]
    fn focus_window_switches_workspace() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::MoveToWorkspace(3)).unwrap();
        assert_eq!(engine.command(Command::FocusWindow(2)), Ok(true));
        assert_eq!(shown(&engine), vec![3]);
        assert_eq!(backend.focused_window(), Some(2));
        assert_eq!(engine.command(Command::FocusWindow(99)), Ok(false));
    }

    #[test]
    fn focus_window_shows_hidden_scratch_window() {
        let (mut engine, backend) = engine_with_scratch();
        engine.command(Command::ToggleScratch).unwrap();
        assert_eq!(engine.command(Command::FocusWindow(2)), Ok(true));
        assert!(engine.state().scratch_visible);
        assert_eq!(backend.focused_window(), Some(2));
    }

    #[test]
    fn scroll_by_pans_and_scroll_end_focuses_centre_column() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 600.0), window(2, 100.0, 600.0), window(3, 200.0, 600.0)]);
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::ScrollBy(-10000.0)), Ok(true));
        assert_eq!(backend.frame_of(1).unwrap().x, 8.0 + 500.0);
        assert_eq!(engine.workspace(0).unwrap().focused_window(), Some(3));
        assert_eq!(engine.command(Command::ScrollEnd), Ok(true));
        assert_eq!(backend.focused_window(), Some(1));
        assert_eq!(backend.frame_of(1).unwrap().x, 8.0);
    }

    #[test]
    fn focus_first_and_last() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 300.0), window(2, 100.0, 300.0), window(3, 200.0, 300.0)]);
        engine.sync().unwrap();
        assert_eq!(engine.command(Command::FocusFirst), Ok(true));
        assert_eq!(backend.focused_window(), Some(1));
        assert_eq!(engine.command(Command::FocusLast), Ok(true));
        assert_eq!(backend.focused_window(), Some(3));
    }

    #[test]
    fn switch_to_empty_workspace_survives_sync() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.command(Command::SwitchWorkspace(4)).unwrap();
        // macOS still reports the now hidden window as focused.
        engine.sync().unwrap();
        assert_eq!(shown(&engine), vec![4]);
    }

    #[test]
    fn moving_last_window_away_survives_sync() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.command(Command::MoveToWorkspace(4)).unwrap();
        engine.sync().unwrap();
        assert_eq!(shown(&engine), vec![0]);
    }

    #[test]
    fn focusing_monitor_with_empty_workspace_survives_sync() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.set_monitors(&[SCREEN, RIGHT]).unwrap();
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.command(Command::FocusMonitorRight).unwrap();
        engine.sync().unwrap();
        assert_eq!(engine.state().active_monitor, 1);
    }

    #[test]
    fn command_parsing_rejects_unknown_values() {
        assert_eq!(Command::parse(0, 0.0), Some(Command::FocusLeft));
        assert_eq!(Command::parse(12, 0.0), Some(Command::Expel));
        assert_eq!(Command::parse(u32::MAX, 0.0), None);
    }

    #[test]
    fn command_parsing_takes_arguments() {
        assert_eq!(Command::parse(13, 2.0), Some(Command::SwitchWorkspace(2)));
        assert_eq!(Command::parse(14, 8.0), Some(Command::MoveToWorkspace(8)));
        assert_eq!(Command::parse(26, 7.0), Some(Command::FocusWindow(7)));
        assert_eq!(Command::parse(27, -12.5), Some(Command::ScrollBy(-12.5)));
        assert_eq!(Command::parse(30, 0.0), Some(Command::FocusLast));
        assert_eq!(Command::parse(13, -1.0), None);
        assert_eq!(Command::parse(13, 1.5), None);
    }

    #[test]
    fn config_fills_defaults() {
        let config = Config::from_json(r#"{"gap":4}"#).unwrap();
        assert_eq!(config.gap, 4.0);
        assert_eq!(config.width_presets, Config::default().width_presets);
        assert_eq!(config.width_step, 0.1);
        assert_eq!(config.workspace_names, (1..=9).map(|i| i.to_string()).collect::<Vec<_>>());

        let config = Config::from_json(r#"{"width_presets":[],"workspace_names":["Web"]}"#).unwrap();
        assert_eq!(config.width_presets, Config::default().width_presets);
        assert_eq!(config.workspace_names[0], "Web");
        assert_eq!(config.workspace_names[1], "2");
        assert_eq!(config.workspace_names.len(), WORKSPACE_COUNT);

        assert_eq!(Config::from_json("nope"), None);
        assert_eq!(Config::from_json(r#"{"gap":-3}"#), None);
        assert_eq!(Config::from_json(r#"{"animation_duration":-1}"#), None);
        assert_eq!(Config::from_json(r#"{"width_step":0}"#).unwrap().width_step, 0.1);
        assert_eq!(
            Config::from_json(r#"{"center_focused":"on_overflow"}"#).unwrap().center_focused,
            CenterFocus::OnOverflow
        );
        assert_eq!(Config::from_json(r#"{"center_focused":"sometimes"}"#), None);
    }

    #[test]
    fn config_reads_app_rules() {
        let config = Config::from_json(
            r#"{"app_rules":[{"app":"Calculator","action":"float"},
                {"app":"Mail","action":"slot","workspace":2,"column":1},
                {"app":"Notes","action":"slot","workspace":9}]}"#,
        )
        .unwrap();
        assert_eq!(config.app_rules.len(), 2, "the rule for a missing workspace is dropped");
        assert_eq!(config.rule_for("Calculator").map(|r| r.action), Some(AppAction::Float));
        let mail = config.rule_for("Mail").unwrap();
        assert_eq!((mail.action, mail.workspace, mail.column), (AppAction::Slot, 2, 1));
        assert_eq!(config.rule_for("Notes"), None);
        assert_eq!(Config::from_json(r#"{"app_rules":[{"app":"Mail","action":"tile"}]}"#), None);
    }

    fn rules(rules: &[(&str, AppAction, usize, usize)]) -> Config {
        let app_rules = rules
            .iter()
            .map(|&(app, action, workspace, column)| AppRule { app: app.into(), action, workspace, column })
            .collect();
        Config { animation_duration: 0.0, app_rules, ..Config::default() }
    }

    #[test]
    fn floating_apps_are_left_alone() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 500.0, 300.0)]);
        engine.set_config(rules(&[("App 2", AppAction::Float, 0, 0)])).unwrap();
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1]]);
        assert_eq!(backend.frame_of(2), Some(Rect::new(500.0, 100.0, 300.0, 300.0)));

        backend.set_focused(Some(2));
        engine.sync().unwrap();
        assert_eq!(engine.state().focused_window, Some(2));
        assert!(engine.state().windows.iter().all(|w| w.id != 2));
    }

    #[test]
    fn floating_a_tiled_app_brings_its_window_back_on_screen() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::SwitchWorkspace(1)).unwrap();
        assert!(is_hidden(backend.frame_of(1).unwrap()));

        engine.set_config(rules(&[("App 1", AppAction::Float, 0, 0)])).unwrap();
        assert_eq!(columns(&engine), Vec::<Vec<WindowId>>::new());
        let frame = backend.frame_of(1).unwrap();
        assert_eq!(frame.width, 300.0);
        assert_eq!(frame.x, SCREEN.x + (SCREEN.width - 300.0) / 2.0);

        // Without the rule it tiles again, on the workspace in view.
        engine.set_config(rules(&[])).unwrap();
        assert_eq!(engine.workspace(1).unwrap().windows().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn slotted_apps_open_at_their_column_and_workspace() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.set_config(rules(&[("App 3", AppAction::Slot, 0, 0), ("App 4", AppAction::Slot, 2, 5)])).unwrap();
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2]]);

        backend.add(window(3, 800.0, 300.0));
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![3], vec![1], vec![2]]);

        backend.add(window(4, 800.0, 300.0));
        engine.sync().unwrap();
        assert_eq!(engine.workspace(2).unwrap().windows().collect::<Vec<_>>(), vec![4]);
        assert!(is_hidden(backend.frame_of(4).unwrap()));

        // Several at once keep their left-to-right order.
        let mut a = window(5, 0.0, 300.0);
        let mut b = window(6, 400.0, 300.0);
        a.app_name = "App 3".into();
        b.app_name = "App 3".into();
        backend.add(b);
        backend.add(a);
        engine.sync().unwrap();
        assert_eq!(columns(&engine), vec![vec![5], vec![6], vec![3], vec![1], vec![2]]);

        // Focusing it brings its workspace in, as for any hidden window.
        backend.set_focused(Some(4));
        engine.sync().unwrap();
        assert_eq!(shown(&engine), vec![2]);
    }

    #[test]
    fn set_config_relayouts_with_the_new_gap() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        engine.set_config(Config { gap: 20.0, ..Config::default() }).unwrap();
        assert_eq!(backend.frame_of(1).unwrap().x, 20.0);
    }

    #[test]
    fn state_lists_columns_and_windows() {
        let (mut engine, _) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        let state = engine.state();
        assert_eq!(state.workspaces[0].columns.len(), 2);
        assert_eq!(state.workspaces[0].name, "1");
        assert_eq!(state.monitors[0].workspace, 0);
        assert_eq!(state.windows.iter().map(|w| w.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(state.windows[0].app_name, "App 1");
        let json = serde_json::to_value(&state).unwrap();
        for key in [
            "active_monitor",
            "gap",
            "monitors",
            "workspaces",
            "windows",
            "mru",
            "scratch",
            "scratch_visible",
            "focused_window",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
    }

    #[test]
    fn reports_missing_permission() {
        let mut engine = Engine::new(Box::new(MockBackend::untrusted()));
        engine.set_monitors(&[SCREEN]).unwrap();
        assert_eq!(engine.sync(), Err(BackendError::NotTrusted));
    }

    fn sliding_engine() -> (Engine, Rc<MockBackend>) {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 600.0), window(2, 100.0, 600.0), window(3, 200.0, 600.0)]);
        engine.sync().unwrap();
        engine.set_config(Config { animation_duration: 0.2, ..Config::default() }).unwrap();
        assert_eq!(engine.workspace(0).unwrap().scroll(), 816.0);
        (engine, backend)
    }

    #[test]
    fn strip_slides_to_the_new_scroll() {
        let (mut engine, backend) = sliding_engine();
        let parked = backend.frame_of(1).unwrap();
        engine.command(Command::FocusFirst).unwrap();
        assert_eq!(backend.frame_of(1), Some(parked));
        assert!(engine.is_animating());

        assert_eq!(engine.tick(0.1), Ok(true));
        // Halfway in time, eased out: 816 * (1 - 0.875) = 102 left to go.
        assert_eq!(backend.frame_of(1).unwrap().x, 8.0 - 102.0);
        assert_eq!(backend.frame_of(1).unwrap().width, 600.0);
        assert_eq!(engine.tick(0.1), Ok(false));
        assert_eq!(backend.frame_of(1).unwrap().x, 8.0);
    }

    #[test]
    fn new_target_mid_slide_starts_from_where_the_strip_is() {
        let (mut engine, backend) = sliding_engine();
        engine.command(Command::FocusFirst).unwrap();
        engine.tick(0.1).unwrap();
        engine.command(Command::FocusLast).unwrap();
        // Nothing jumps: the strip turns around from scroll 102.
        assert_eq!(backend.frame_of(1).unwrap().x, 8.0 - 102.0);
        engine.tick(1.0).unwrap();
        assert_eq!(backend.frame_of(3).unwrap().x, 1216.0 + 8.0 - 816.0);
    }

    #[test]
    fn dragged_tiled_window_is_put_back_on_sync() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        let placed = backend.frame_of(2).unwrap();
        backend.update(2, |w| w.frame = Rect::new(600.0, 200.0, 350.0, 250.0));
        engine.sync().unwrap();
        assert_eq!(backend.frame_of(2), Some(placed));
    }

    /// Drags window `id` by `dx` without resizing it.
    fn drag(backend: &MockBackend, id: WindowId, dx: f64) {
        backend.update(id, |w| w.frame.x += dx);
    }

    #[test]
    fn window_dropped_on_another_column_takes_its_place() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0), window(3, 800.0, 300.0)]);
        engine.sync().unwrap();
        let third = backend.frame_of(3).unwrap();
        drag(&backend, 1, 600.0);
        backend.set_focused(Some(1));
        engine.drop_at(700.0, 50.0).unwrap();
        assert_eq!(columns(&engine), vec![vec![2], vec![3], vec![1]]);
        assert_eq!(backend.frame_of(1), Some(third));
        assert_eq!(backend.frame_of(2).unwrap().x, 8.0);
    }

    #[test]
    fn window_dropped_on_its_own_column_goes_back() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        let placed = backend.frame_of(1).unwrap();
        drag(&backend, 1, 100.0);
        backend.set_focused(Some(1));
        engine.drop_at(200.0, 50.0).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2]]);
        assert_eq!(backend.frame_of(1), Some(placed));
    }

    #[test]
    fn resizing_or_clicking_rearranges_nothing() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        // Dragging the left edge moves and resizes.
        backend.update(1, |w| {
            w.frame.x += 400.0;
            w.frame.width -= 100.0;
        });
        engine.drop_at(450.0, 50.0).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2]]);
        engine.drop_at(450.0, 50.0).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2]]);
    }

    #[test]
    fn only_the_focused_window_counts_as_dragged() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        // Window 1 strayed on its own, under where the user drags window 2.
        drag(&backend, 1, 300.0);
        drag(&backend, 2, 10.0);
        backend.set_focused(Some(2));
        engine.drop_at(350.0, 50.0).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2]]);
    }

    #[test]
    fn drop_on_a_sliding_strip_rearranges_nothing() {
        let (mut engine, backend) = sliding_engine();
        engine.command(Command::FocusFirst).unwrap();
        engine.tick(0.05).unwrap();
        // Window 1 dragged over window 2's place.
        let over = backend.frame_of(2).unwrap().x - backend.frame_of(1).unwrap().x;
        drag(&backend, 1, over);
        backend.set_focused(Some(1));
        engine.drop_at(backend.frame_of(2).unwrap().x + 10.0, 50.0).unwrap();
        assert_eq!(columns(&engine), vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn window_dropped_on_another_monitor_joins_its_strip() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 1200.0, 300.0)]);
        engine.set_monitors(&[SCREEN, RIGHT]).unwrap();
        engine.sync().unwrap();
        drag(&backend, 1, 1050.0);
        backend.set_focused(Some(1));
        engine.drop_at(1100.0, 100.0).unwrap();
        assert!(columns(&engine).is_empty());
        let right: Vec<Vec<WindowId>> =
            engine.workspace(1).unwrap().columns().iter().map(|c| c.windows().to_vec()).collect();
        assert_eq!(right, vec![vec![1], vec![2]]);
        assert_eq!(engine.state().active_monitor, 1);
        assert_eq!(backend.frame_of(1).unwrap().x, 1024.0);
    }

    #[test]
    fn window_refusing_its_size_is_not_fought_every_sync() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0)]);
        engine.sync().unwrap();
        let refused = Rect { width: 500.0, ..backend.frame_of(1).unwrap() };
        backend.update(1, |w| w.frame = refused);
        engine.sync().unwrap();
        // The app clamps to its minimum width again.
        backend.update(1, |w| w.frame = refused);
        engine.sync().unwrap();
        let calls = backend.set_frame_calls();
        engine.sync().unwrap();
        assert_eq!(backend.set_frame_calls(), calls);

        engine.retile().unwrap();
        assert_eq!(backend.set_frame_calls(), calls + 1);
    }

    #[test]
    fn busy_app_keeps_its_windows_in_place() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 300.0), window(2, 100.0, 300.0), window(3, 200.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::FocusFirst).unwrap();
        let before = columns(&engine);
        // Window 2's app stops answering.
        backend.set_unresponsive(vec![102]);
        engine.sync().unwrap();
        assert_eq!(columns(&engine), before);
        assert!(engine.state().windows.iter().any(|w| w.id == 2));

        backend.set_unresponsive(vec![]);
        engine.sync().unwrap();
        assert_eq!(columns(&engine), before);
    }

    #[test]
    fn renaming_a_workspace_moves_no_windows() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        let calls = backend.set_frame_calls();
        let mut config = engine.config().clone();
        config.workspace_names[0] = "Mail".into();
        engine.set_config(config).unwrap();
        assert_eq!(backend.set_frame_calls(), calls);
        assert_eq!(engine.state().workspaces[0].name, "Mail");
    }

    #[test]
    fn always_centring_keeps_the_focused_column_in_the_middle() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.set_config(Config { center_focused: CenterFocus::Always, ..engine.config().clone() }).unwrap();
        engine.sync().unwrap();
        let middle = |id| backend.frame_of(id).map(|f| f.x + f.width / 2.0);
        let screen_middle = engine.monitors[0].area.x + engine.monitors[0].area.width / 2.0;
        let focused = engine.focused_window().unwrap();
        assert_eq!(middle(focused), Some(screen_middle));
        engine.command(Command::FocusLeft).unwrap();
        let focused = engine.focused_window().unwrap();
        assert_eq!(middle(focused), Some(screen_middle));
    }

    #[test]
    fn centred_column_stays_centred_after_sync() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.sync().unwrap();
        engine.command(Command::Center).unwrap();
        let centred = backend.frame_of(1).unwrap();
        assert!(centred.x > 8.0);
        engine.sync().unwrap();
        assert_eq!(backend.frame_of(1), Some(centred));
    }

    #[test]
    fn state_reports_focus_frames_and_column_positions() {
        let (mut engine, backend) = engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0)]);
        engine.sync().unwrap();
        backend.set_focused(Some(1));
        engine.sync().unwrap();
        let state = engine.state();
        assert_eq!(state.focused_window, Some(1));
        assert_eq!(state.monitors[0].viewport, 1000.0);
        let xs: Vec<f64> = state.workspaces[0].columns.iter().map(|c| c.x).collect();
        assert_eq!(xs, vec![0.0, 308.0]);
        assert_eq!(state.windows[0].frame, backend.frame_of(1).unwrap());

        engine.command(Command::FocusRight).unwrap();
        assert_eq!(engine.state().focused_window, Some(2));
    }

    #[test]
    fn pin_commands_keep_a_column_at_the_screen_edge() {
        let (mut engine, backend) =
            engine_with(vec![window(1, 0.0, 300.0), window(2, 400.0, 300.0), window(3, 800.0, 300.0)]);
        engine.sync().unwrap();
        engine.command(Command::FocusFirst).unwrap();
        assert_eq!(engine.command(Command::TogglePinRight), Ok(true));
        assert_eq!(columns(&engine), vec![vec![2], vec![3], vec![1]]);
        assert_eq!(backend.frame_of(1).unwrap().x, SCREEN.max_x() - 8.0 - 300.0);
        let state = engine.state();
        let pinned = &state.workspaces[0].columns[2];
        assert_eq!((pinned.pinned, pinned.x), (Some(Pin::Right), 700.0));
        assert!(serde_json::to_string(&state).unwrap().contains(r#""pinned":"right""#));

        assert_eq!(engine.command(Command::FocusLeft), Ok(true));
        assert_eq!(backend.focused_window(), Some(3));
        engine.command(Command::FocusRight).unwrap();
        assert_eq!(engine.command(Command::TogglePinRight), Ok(true));
        assert_eq!(engine.state().workspaces[0].columns[2].pinned, None);
    }

    #[test]
    fn pans_follow_the_fingers_without_sliding() {
        let (mut engine, backend) = sliding_engine();
        engine.command(Command::FocusFirst).unwrap();
        engine.tick(0.1).unwrap();
        engine.command(Command::ScrollBy(-50.0)).unwrap();
        assert!(!engine.is_animating());
        assert_eq!(engine.workspace(0).unwrap().scroll(), 52.0);
        assert_eq!(backend.frame_of(1).unwrap().x, 8.0 - 52.0);
    }
}
