# Step 3 Feature Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Workspaces, monitor moves, scratch layer, window switcher, ⌃⌥ + trackpad scroll, minimap, workspace indicator and a settings window with rebindable shortcuts.

**Architecture:** The Rust engine gains workspaces, scratch, MRU and scrolling; commands with an argument become enum variants with data, parsed at the FFI from `(u32, double)`. Swift reads a JSON state snapshot for all UI and pushes JSON config. Each Swift feature is one file.

**Tech Stack:** Rust 2021 (`serde`, `serde_json` added), Swift 5.9 / AppKit + SwiftUI, Carbon hot keys, ScreenCaptureKit, CGEventTap. macOS 14+.

**Spec:** `docs/superpowers/specs/2026-10-25-feature-parity-design.md`

## Global Constraints

- Engine in Rust, UI in Swift, one `.app`; Accessibility permission only (Screen Recording optional, for thumbnails).
- 9 workspaces, default names "1"…"9".
- Hidden windows keep 1pt on screen (`placement::PEEK`).
- Default shortcuts exactly as the spec's table; ⌃⌥ = Super, ⌃⌥⌘ = Super+Ctrl.
- Minimap hides when ⌃⌥ is released or after 1s.
- CI (`cargo fmt --check`, clippy `-D warnings` on both macOS targets, `cargo test`, app build, dmg) stays green after every task.
- `include/paperwm_engine.h` mirrors `ffi.rs` and `Command` numbering.

## Review Focus

1. A monitor showing a workspace is unplugged → that workspace becomes hidden (parked), nothing lost, active monitor clamped. Test in Task 2.
2. A window on a hidden workspace or in the scratch layer is closed → it disappears from the model. Tests in Tasks 2 and 4.
3. Switching to the workspace already shown on the active monitor → no change, returns false. Test in Task 2.
4. Config JSON missing fields, with fewer than 9 names, or with empty `width_presets` → defaults fill in; malformed JSON → `InvalidArgument`, old config kept. Test in Task 1.
5. Moving the last window off a workspace, then switching to the empty one → no panic, focus `None`. Test in Task 2.

---

### Task 1: Commands with arguments, config and state over FFI

**Files:**
- Modify: `engine/Cargo.toml`, `engine/src/engine.rs`, `engine/src/ffi.rs`, `engine/include/paperwm_engine.h`, `engine/src/geometry.rs`

**Interfaces:**
- Produces:
  - `enum Command` keeps variants 0–12; adds `SwitchWorkspace(usize)=13, MoveToWorkspace(usize)=14, NextWorkspace=15, PreviousWorkspace=16, MoveToNextWorkspace=17, MoveToPreviousWorkspace=18, LastWorkspace=19, FocusMonitorLeft=20, FocusMonitorRight=21, MoveToMonitorLeft=22, MoveToMonitorRight=23, ToggleScratch=24, MoveToScratch=25, FocusWindow(WindowId)=26, ScrollBy(f64)=27, ScrollEnd=28, FocusFirst=29, FocusLast=30`. `Command::parse(code: u32, arg: f64) -> Option<Command>` (replaces `TryFrom<u32>`).
  - `Config { gap, width_presets, workspace_names: Vec<String> }`, `#[serde(default)]`; `Config::from_json(&str) -> Option<Config>` (empty presets → default presets; names padded to 9 with defaults).
  - `Engine::set_config(&mut self, Config) -> Result<()>` (re-applies layout).
  - `Engine::state(&self) -> State` (`Serialize`); shape: `{active_monitor, gap, monitors:[{area, workspace}], workspaces:[{name, scroll, focused_column, columns:[{width, windows, focused_row}]}], windows:[{id, pid, app_name, title}], mru:[id], scratch:[id], scratch_visible}`.
  - C: `PwmStatus pwm_engine_command(PwmEngine*, uint32_t command, double arg)`, `PwmStatus pwm_engine_set_config(PwmEngine*, const char *json)`, `char *pwm_engine_state_json(const PwmEngine*)`, `void pwm_string_free(char*)`.

- [ ] **Step 1: Tests**: `command_parsing_takes_arguments` (`parse(13, 2.0) == Some(SwitchWorkspace(2))`, `parse(26, 7.0) == Some(FocusWindow(7))`, `parse(31, 0.0) == None`, `parse(13, -1.0) == None`); `config_fills_defaults` (`from_json("{\"gap\":4}")` → gap 4, 9 names "1".."9", default presets; `{"width_presets":[]}` → default presets; `"nope"` → None); `state_json_lists_columns` (after sync of two windows, `state().workspaces[0].columns.len() == 2`, `serde_json::to_value` has key `"mru"`); FFI test `set_config_rejects_bad_json` (`InvalidArgument`) and `state_json_round_trips` (parses with serde_json, freed with `pwm_string_free`).
- [ ] **Step 2: Run** `cargo test` → new tests fail to compile.
- [ ] **Step 3: Implement.** Add `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`. Derive `Serialize` on `Rect`. Workspace-dependent variants return `Ok(false)` until their task lands.
- [ ] **Step 4: Run** `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` → pass.
- [ ] **Step 5: Commit** "Pass command arguments, config and state across the FFI".

### Task 2: Emulated workspaces

**Files:**
- Modify: `engine/src/engine.rs`, `engine/src/placement.rs`

**Interfaces:**
- Consumes: Task 1 `Command`, `Config.workspace_names`.
- Produces: `Engine { workspaces: Vec<Workspace> /* 9 */, monitors: Vec<Monitor { area, workspace: usize }>, last_workspace: Option<usize> }`; `Engine::workspace(i)` now indexes workspaces (not monitors); `Engine::shown_workspace(monitor) -> Option<usize>`; `placement::hidden_frame(frame: Rect, monitors: &[Rect]) -> Rect`: keep size, place at bottom-right of the monitor with the largest `max_x + max_y`, minus `PEEK`.

- [ ] **Step 1: Tests** (engine.rs): `monitors_show_distinct_workspaces` (2 monitors → shown 0 and 1); `switch_workspace_hides_old_windows` (window 1 on ws0; `SwitchWorkspace(1)` → frame of 1 == hidden_frame; back to 0 → tiled again); `switching_to_shown_workspace_is_noop` (returns `Ok(false)`); `switch_to_workspace_on_other_monitor_swaps`; `move_to_workspace_hides_window_and_keeps_focus_on_neighbour`; `empty_workspace_has_no_focus` (move only window away, switch to empty → `focused_window()==None`, no panic); `focus_on_hidden_workspace_window_brings_it_in` (backend focus set to a window on ws3 → after sync, active monitor shows ws3); `closed_window_on_hidden_workspace_is_forgotten`; `unplugged_monitor_hides_its_workspace` (2→1 monitors: ws1 windows parked, still in ws1, `active==0`); `next_previous_and_last_workspace` (wraps 8→0; `LastWorkspace` returns to previous). Update existing tests that used `workspace(monitor)` index (identical for monitor 0/1 at start).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.** `set_monitors` keeps each monitor's workspace index where possible, gives new monitors the lowest unshown workspace; windows are no longer moved between workspaces on unplug. `apply` places shown workspaces as before and every window of unshown workspaces at `hidden_frame`. `sync` scans all workspaces for gone windows; if the focused window belongs to an unshown workspace, show it on the active monitor (swap rules as `SwitchWorkspace`). Remember `last_workspace` on every switch.
- [ ] **Step 4: Run** full check → pass.
- [ ] **Step 5: Commit** "Add emulated workspaces".

### Task 3: Monitor focus and moves

**Files:** Modify `engine/src/engine.rs`

**Interfaces:** Produces `fn monitor_toward(&self, from: usize, right: bool) -> Option<usize>`: nearest monitor whose area centre x is strictly right/left of `from`'s.

- [ ] **Step 1: Tests**: `focus_monitor_right_activates_and_focuses` (focus goes to that workspace's focused window via backend); `move_to_monitor_inserts_next_to_target_focus` (window leaves ws0, lands right of ws1's focused column, still focused, active monitor follows); `no_monitor_in_direction_returns_false`.
- [ ] **Step 2: Run** → fail. **Step 3: Implement.** **Step 4: Run** → pass.
- [ ] **Step 5: Commit** "Focus and move windows across monitors".

### Task 4: Scratch layer

**Files:** Modify `engine/src/engine.rs`

**Interfaces:** Produces `Engine { scratch: Vec<(WindowId, Rect)>, scratch_visible: bool }`.

- [ ] **Step 1: Tests**: `move_to_scratch_floats_window_centered` (leaves strip; frame centred on active monitor at current size; `state().scratch == [id]`); `toggle_scratch_parks_and_restores` (hidden → `hidden_frame`; shown again → saved frame, focused); `move_to_scratch_again_returns_to_tiling` (inserted right of focused column); `closed_scratch_window_is_forgotten`; `new_windows_do_not_join_scratch`.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.** Sync must skip scratch windows when adopting and drop closed ones; while visible, refresh each saved frame from the backend so the user's drags stick.
- [ ] **Step 4: Run** → pass. **Step 5: Commit** "Add the scratch layer".

### Task 5: MRU, focus window, first/last, scrolling

**Files:** Modify `engine/src/workspace.rs`, `engine/src/engine.rs`

**Interfaces:** Produces `Workspace::scroll_by(&mut self, dx, viewport, gap)` (clamped to `[-viewport/2, strip_width - viewport/2]`), `Workspace::focus_nearest_centre(&mut self, viewport, gap) -> bool`, `Workspace::focus_edge(&mut self, last: bool) -> bool`; `Engine.mru: Vec<WindowId>` (most recent first, closed windows removed).

- [ ] **Step 1: Tests**: workspace `scroll_by_clamps`, `focus_nearest_centre_picks_column_under_middle`, `focus_edge`; engine `mru_tracks_focus_changes` (sync focus 1, 2, then command focus → order), `focus_window_switches_workspace` (`FocusWindow(id)` on hidden ws → shown + focused), `scroll_end_focuses_centre_column`.
- [ ] **Step 2: Run** → fail. **Step 3: Implement.** `ScrollBy` must not call `ensure_focused_visible`. **Step 4: Run** → pass.
- [ ] **Step 5: Commit** "Track recently used windows and scroll the strip freely".

### Task 6: Configurable shortcuts, settings, workspace indicator (Swift)

**Files:**
- Modify: `app/Sources/PaperWM/Engine.swift`, `HotKeys.swift`, `WindowManager.swift`, `AppDelegate.swift`
- Create: `app/Sources/PaperWM/EngineState.swift`, `Settings.swift`, `SettingsView.swift`

**Interfaces:**
- Consumes: Task 1 C API.
- Produces: `Engine.run(_ command: PwmCommand, arg: Double = 0)`, `Engine.state() -> EngineState?` (Codable mirror of `State`, snake_case decoding), `Engine.setConfig(_ settings: Settings)`; `enum Action { case engine(PwmCommand, Double); case switcher(forward: Bool) }`; `struct HotKey { id: String; keyCode; modifiers; action; title }` with `static let defaults: [HotKey]` (spec table, ids like `"focus-left"`, `"switch-workspace-3"`); `final class Settings: ObservableObject` (`gap`, `widthPresets`, `workspaceNames`, `shortcuts: [String: KeyCombo]` overrides, JSON in `UserDefaults` key `"settings"`), `Settings.hotKeys -> [HotKey]`; `HotKeyCenter.reregister(_:)`, `HotKeyCenter.failed: Set<String>`; `WindowManager.onStateChange: ((EngineState) -> Void)?` called after every sync/command.

- [ ] **Step 1:** Engine wrapper + `EngineState`; `WindowManager` forwards `Action`; menu bar title shows the active monitor's workspace name; menu lists workspaces (click → `SwitchWorkspace`) and "Settings…" (⌘,).
- [ ] **Step 2:** `SettingsView` (SwiftUI `Form` in an `NSWindow`): gap stepper, presets text field (comma-separated percentages), 9 name fields, shortcut list with a recorder button (captures next keyDown via local `NSEvent` monitor, Esc cancels) and Reset; rows in `failed` show a warning.
- [ ] **Step 3: Verify** CI app build passes (push and check run). **Step 4: Commit** "Add settings, configurable shortcuts and workspace indicator".

### Task 7: Minimap (Swift)

**Files:** Create `app/Sources/PaperWM/Minimap.swift`, `ModifierWatcher.swift`; modify `WindowManager.swift`.

**Interfaces:** `ModifierWatcher(onRelease: () -> Void)` using global + local `.flagsChanged` monitors, fires when ⌃ or ⌥ is lifted; `Minimap.show(state: EngineState, monitor: Int)`, `Minimap.hide()`; non-activating borderless `NSPanel` at `.statusBar` level, ignores mouse.

- [ ] **Step 1:** Draw with SwiftUI: workspace name, columns scaled to fit 60% of screen width, stacked rows, viewport outline, focused window highlighted, app names. Show after engine actions (not on plain syncs), hide on release or 1s.
- [ ] **Step 2: Verify** CI build. **Step 3: Commit** "Show the minimap while navigating".

### Task 8: Window switcher (Swift)

**Files:** Create `app/Sources/PaperWM/Switcher.swift`, `Thumbnails.swift`; modify `WindowManager.swift`.

**Interfaces:** `Switcher.step(forward: Bool, state: EngineState)` opens on first call with MRU order, selection starting at index 1; `Switcher.commit() -> UInt32?`, `Switcher.cancel()`; `Thumbnails.capture(ids: [UInt32]) async -> [UInt32: CGImage]` via `SCShareableContent` + `SCScreenshotManager`, empty when `CGPreflightScreenCaptureAccess()` is false. `ModifierWatcher.onRelease` → `commit()` → `.engine(FOCUS_WINDOW, id)`; ⌃⌥Esc while open cancels instead of toggling scratch.

- [ ] **Step 1:** Panel with a row of cards (thumbnail or large app icon, title), selected one highlighted; one "Show previews…" button calling `CGRequestScreenCaptureAccess()` when access is missing (asked at most once, flag in `UserDefaults`).
- [ ] **Step 2: Verify** CI build. **Step 3: Commit** "Add the window switcher with previews".

### Task 9: Trackpad gestures (Swift)

**Files:** Create `app/Sources/PaperWM/Gestures.swift`; modify `WindowManager.swift`.

**Interfaces:** `Gestures(perform: (Action) -> Void).start()`: `CGEvent.tapCreate(.cgSessionEventTap, .headInsertEventTap, .defaultTap, scrollWheel mask)`; passes events through unless flags contain ⌃ and ⌥.

- [ ] **Step 1:** Horizontal point delta (`scrollWheelEventPointDeltaAxis2`) → `ScrollBy(-dx)`; vertical accumulated past 60pt → `NextWorkspace`/`PreviousWorkspace` once per gesture; `ScrollEnd` 150ms after the last event (covers momentum). Re-enable the tap on `tapDisabledByTimeout`.
- [ ] **Step 2: Verify** CI build. **Step 3: Commit** "Navigate with ⌃⌥ and two-finger scroll".

### Task 10: Docs and PR

**Files:** Modify `README.md`.

- [ ] **Step 1:** Update Status, shortcut table, gestures, Settings, permissions (Screen Recording optional).
- [ ] **Step 2: Verify** CI green on the branch. **Step 3: Commit**, open PR against `claude/project-thread-yrc917` with the manual test checklist from the spec.
