# Step 3: PaperWM feature parity — design

Builds on PR #1 (engine, scrolling strip, ⌃⌥ shortcuts, `.app`/`.dmg` in CI).
Step 4 (onboarding, launch at login, signing, README polish) is out of scope.

## Goal

Someone who uses PaperWM on GNOME can use this app on macOS without missing
the features they rely on daily: workspaces, multiple monitors, the scratch
layer, a window switcher, trackpad navigation, the minimap, and rebinding
keys. Success is the manual test list at the end passing on a real Mac, with
CI green.

Constraints (project rules): engine in Rust, UI in Swift, everything in one
`.app`, multi-window apps handled per window, Accessibility permission only
(SIP stays on).

## Architecture

Unchanged split: the Rust engine owns all window state and layout; Swift owns
everything the user sees and touches (hot keys, overlays, settings, gestures)
and calls the engine through the C interface.

Two FFI additions keep that interface small as features grow:

- `pwm_engine_command(engine, command, arg)` gains a `double arg` for
  commands that need one (workspace index, window id, scroll delta).
- `pwm_engine_state_json(engine)` returns a JSON snapshot (workspaces,
  columns, window ids/titles/apps, focus, which monitor shows which
  workspace, scratch visibility, MRU order) that Swift decodes with
  `Codable` for the indicator, minimap and switcher. Freed with
  `pwm_string_free`.
- `pwm_engine_set_config(engine, json)` receives the settings Swift stores.

JSON over the boundary costs one dependency (`serde`, `serde_json`) and
saves writing and freeing nested C structs by hand.

## Features

### 1. Workspaces (emulated, independent of macOS Spaces)

- The engine holds a fixed list of 9 workspaces, each a scrolling strip with
  a name (default "1"…"9"). Each monitor shows exactly one workspace; a
  workspace is shown on at most one monitor.
- Windows of hidden workspaces are parked at the bottom-right corner of the
  main monitor with 1pt on screen (macOS won't move windows fully off
  screen; the same trick as column parking in PR #1).
- Switching to a workspace shown on another monitor swaps the two, as in
  PaperWM.
- New windows join the workspace of the monitor they appear on.
- Multi-window apps: if focus lands on a window of a hidden workspace (⌘Tab,
  Dock click, an app opening a window in an old one), the engine shows that
  workspace on the active monitor and scrolls to the window.
- Everything stays on the current macOS Space. Windows on other Spaces and
  native full-screen windows are ignored, as today.

Commands: switch to workspace N, move window to workspace N, next/previous
workspace, previous (most recently used) workspace.

### 2. Monitors

- Focus monitor left/right (by screen position): activates that monitor's
  workspace and focuses its focused window.
- Move window to monitor left/right: the window leaves its workspace and is
  inserted next to the focused column of the target monitor's workspace,
  keeping focus.

### 3. Scratch layer

- A global set of floating windows, as in PaperWM. A window moved to the
  scratch layer leaves its strip, keeps its own frame (centered at its
  current size), and floats above tiled windows.
- Toggle the layer: hides (parks) all scratch windows or brings them back to
  their saved frames, raised. Moving a scratch window back to tiling
  inserts it next to the focused column.

### 4. Window switcher with previews

- ⌃⌥Tab (PaperWM's Super+Tab): hold ⌃⌥, press Tab to step through windows in
  most-recently-used order across all workspaces (⇧ steps back), release ⌃⌥
  to focus the selection. Esc cancels.
- A centered panel shows a thumbnail per window with app icon and title.
  Thumbnails come from ScreenCaptureKit (`SCScreenshotManager`, macOS 14+),
  which needs Screen Recording permission. Without it, the panel shows app
  icons only and offers to grant access once; nothing else needs it.
- Focusing the choice goes through the engine, so a window on a hidden
  workspace brings its workspace along.

### 5. Trackpad gestures

PaperWM uses three-finger swipes. macOS gives apps no public access to
three-finger swipes (the system keeps them for Spaces and Mission Control),
so the default is a public-API equivalent:

- ⌃⌥ + two-finger horizontal scroll pans the strip freely; when the gesture
  (including momentum) ends, focus moves to the column nearest the
  viewport centre.
- ⌃⌥ + two-finger vertical scroll past a threshold switches to the
  next/previous workspace.

Implemented with a `CGEventTap` on scroll events (covered by the
Accessibility permission). The tap only swallows events while ⌃⌥ is held.

Open choice for the owner: real three-finger swipes are possible with the
private `MultitouchSupport` framework (used by Jitouch and similar apps), but
it can break across macOS releases and the user has to turn off the
matching system gestures. Recommendation: ship the public version now, add
three-finger swipes later if wanted.

### 6. Swift UI

- **Workspace indicator**: the menu bar item shows the active workspace's
  name next to the icon; the menu lists workspaces (click to switch).
- **Minimap**: a translucent overlay on the active monitor, like PaperWM's,
  drawn from the state snapshot: one rectangle per column (stacked windows
  split vertically), proportional widths, the viewport outlined, focused
  window highlighted, app names inside. Shown after any layout/navigation
  command and on workspace switches (with the workspace name); hidden when
  ⌃⌥ is released or after 1s.
- **Settings window** (SwiftUI, opened from the menu): gap, width presets,
  workspace names, and a shortcut list where each action can be re-recorded
  or reset. Stored in `UserDefaults`; changes apply immediately (config
  pushed to the engine, hot keys re-registered).

### 7. Configurable hot keys

The `HotKey` table moves from a hard-coded list to defaults merged with
user overrides. Defaults (⌃⌥ stands in for Super; ⌃ itself is taken, so
PaperWM's Super+Ctrl maps to ⌃⌥⌘):

| Shortcut | Action |
| --- | --- |
| existing ⌃⌥ arrows, R, F, C, I, O | unchanged |
| ⌃⌥ 1…9 | switch to workspace N |
| ⌃⌥⇧ 1…9 | move window to workspace N |
| ⌃⌥ PageDown / PageUp | next / previous workspace |
| ⌃⌥⇧ PageDown / PageUp | move window to next / previous workspace |
| ⌃⌥ \` | previous workspace (PaperWM's Super+Above_Tab) |
| ⌃⌥⌘ ← / → | focus monitor left / right |
| ⌃⌥⌘⇧ ← / → | move window to monitor left / right |
| ⌃⌥ Esc | toggle scratch layer |
| ⌃⌥⇧ Esc | move window to / from scratch layer |
| ⌃⌥ Tab / ⌃⌥⇧ Tab | window switcher |
| ⌃⌥ Home / End | focus first / last column |

## Error handling

Same rules as PR #1: Accessibility failures on single windows are ignored
and fixed on the next sync, missing permission is surfaced in the menu.
Hot keys that fail to register (taken by another app) are flagged in
Settings. Invalid config JSON is rejected with `InvalidArgument` and the
engine keeps its previous config.

## Testing

- Engine: unit tests against `MockBackend` for each command (workspace
  switch/swap/move, hidden-workspace parking, focus following into hidden
  workspaces, monitor focus/move, scratch toggle/move, MRU order, scroll and
  snap, config parsing, state JSON shape).
- Swift: compiled in CI (macOS 15 runner); no UI tests.
- Manual checklist for the owner's Mac, posted with the PR: workspaces
  1–9, a multi-window app across two workspaces with ⌘Tab, two monitors,
  scratch toggle, switcher with and without Screen Recording, ⌃⌥ +
  trackpad scroll, minimap, rebinding a shortcut.

## Delivery

One PR on top of PR #1's branch, built in this order, each a commit with
green CI: engine workspaces + monitors + FFI; scratch layer; hot key config
+ settings + indicator; minimap; switcher; gestures; README.
