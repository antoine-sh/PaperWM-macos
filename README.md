# PaperWM for macOS

A port of [PaperWM](https://github.com/paperwm/PaperWM), the scrollable tiling window manager, to macOS.

The window-management engine is written in Rust and the user interface in Swift, bundled together as a single `.app`. It only needs Accessibility permission; System Integrity Protection stays on.

## Status

Covers PaperWM's everyday features. Built and unit-tested in CI, but not yet tested by hand on a real Mac.


- **Scrolling strip**: every window opens as a new column to the right of the focused one, focusing a column slides the strip to bring it into view, and columns can be rearranged, resized and stacked from the keyboard. Dialogs, sheets and other non-standard windows float. Columns that scroll off screen are parked at the screen edge, since macOS doesn't allow hiding windows entirely off screen.
- **Drag to reorder**: drag a window by its title bar onto another column and let go: it takes that column's place and the strip rearranges around it. A window dragged out of a stack gets a column of its own, and dropping onto another monitor moves it into that monitor's strip.
- **Native tabs**: a window with tabs (Finder, Terminal, Safari) is one column; opening, switching or closing tabs keeps it in place, and a tab dragged out into its own window gets a column of its own.
- **Workspaces**: nine PaperWM workspaces, independent of macOS Spaces (keep everything on one Space). Each monitor shows one; the others' windows are tucked into a screen corner. Focusing a window that lives on a hidden workspace (⌘Tab, Dock, a click on a notification) brings its workspace in.
- **Monitors**: focus or move windows to the monitor on the left or right.
- **Scratch layer**: floating windows above the workspaces that you can show or hide together.
- **Window switcher**: ⌃⌥Tab through recently used windows across all workspaces, with live previews if you grant Screen Recording permission (app icons otherwise).
- **Trackpad**: hold ⌃⌥ and scroll with two fingers sideways to pan the strip (focus lands on the column in the middle when you let go), or up/down to switch workspace. macOS reserves three-finger swipes, so this stands in for PaperWM's.
- **Minimap**: an overview of the strip appears briefly while you navigate. In Settings, pick the strip view or a small toolbar of app icons along the bottom of the screen (one per column, focused one highlighted).
- **Focus highlight**: optionally draw a colored border around the focused window and dim the windows behind it. Both are off by default; turn them on from the menu, Settings or their shortcuts. With Screen Recording permission the border matches each window's own rounded corners; otherwise it uses the corner radius set in Settings.
- **App rules**: in Settings › Apps, pick an app from the running ones (or from the Applications folder) and either float it, so its windows are never tiled or moved and stay on every workspace like dialogs, or give it a fixed slot: its windows open at a set column of a set workspace. Apps are matched by name.
- **Menu bar**: shows the current workspace's name; the menu switches workspaces, toggles the focus border and dimming, and opens Settings.
- **Settings** (⌘, from the menu): gaps, column width presets, the widen / narrow step, scroll animation length (0 turns it off), whether to keep the focused column centred (never, when the columns overflow the screen, or always), workspace names, app rules, minimap style, border color and width, dim strength, and every shortcut can be re-recorded.

### Keyboard shortcuts

⌃⌥ stands in for PaperWM's Super key and ⌃⌥⌘ for Super+Ctrl. All of these can be changed in Settings.

| Shortcut | Action |
| --- | --- |
| ⌃⌥ ← / → | Focus the column on the left / right |
| ⌃⌥ ↑ / ↓ | Focus the window above / below in the column |
| ⌃⌥ Home / End | Focus the first / last column |
| ⌃⌥⇧ ← / → | Move the column left / right |
| ⌃⌥⇧ ↑ / ↓ | Move the window up / down in its column |
| ⌃⌥ R | Cycle column width (⅓, ½, ⅔ of the screen) |
| ⌃⌥ = / - | Widen / narrow the column (by 10% of the screen; set the step in Settings) |
| ⌃⌥ F | Toggle full width |
| ⌃⌥ C | Center the column |
| ⌃⌥ I | Pull the next column's window into this column |
| ⌃⌥ O | Push the focused window out into its own column |
| ⌃⌥⇧ = / - | Make the window taller / shorter in its stacked column |
| ⌃⌥ 0 | Give every window in the column the same height |
| ⌃⌥ Tab / ⌃⌥⇧ Tab | Switch windows (release ⌃⌥ to pick, ⌃⌥ Esc to cancel) |
| ⌃⌥ 1…9 | Switch to workspace 1…9 |
| ⌃⌥⇧ 1…9 | Move the window to workspace 1…9 |
| ⌃⌥ PageDown / PageUp | Next / previous workspace |
| ⌃⌥⇧ PageDown / PageUp | Move the window to the next / previous workspace |
| ⌃⌥ \` | Back to the last used workspace |
| ⌃⌥⌘ ← / → | Focus the monitor on the left / right |
| ⌃⌥⌘⇧ ← / → | Move the window to the monitor on the left / right |
| ⌃⌥ Esc | Show or hide the scratch layer |
| ⌃⌥⇧ Esc | Move the window to or from the scratch layer |
| ⌃⌥ B | Show or hide the focus border |
| ⌃⌥ D | Dim background windows on or off |

### Permissions

- **Accessibility** (required): to move, resize and focus windows, and to read the trackpad while ⌃⌥ is held.
- **Screen Recording** (optional): for window previews in the switcher, and to match the focus border to each window's rounded corners. macOS asks the first time you open the switcher.

## Building

Requirements: macOS 14 or later, Xcode 15 or later (for Swift), and Rust via [rustup](https://rustup.rs).

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/build-app.sh               # this Mac's architecture
scripts/build-app.sh --universal   # Apple Silicon and Intel
open build/PaperWM.app
scripts/make-dmg.sh                # optional: package as build/PaperWM-<version>.dmg
```

CI builds a universal disk image on every push and pull request; download it from the run's **PaperWM-dmg** artifact.

On first launch macOS asks you to grant Accessibility access in System Settings › Privacy & Security › Accessibility. Builds are signed ad hoc, so after rebuilding you may need to remove PaperWM from that list and add it again.

Engine tests run anywhere, including Linux:

```sh
cargo test
```

## Layout

| Path | Contents |
| --- | --- |
| `engine/` | Rust engine: scrolling layout (`workspace.rs`, `placement.rs`), workspaces, monitors, scratch layer, window sync and commands (`engine.rs`), macOS Accessibility backend (`macos/`), C interface (`ffi.rs`, `include/paperwm_engine.h`) |
| `app/` | Swift menu bar app (SwiftPM package): hot keys, settings, minimap, switcher, trackpad gestures, window drops, app and monitor tracking; links the engine as a static library |
| `scripts/build-app.sh` | Builds both and assembles `build/PaperWM.app` |
| `scripts/make-dmg.sh` | Packages the app as a `.dmg` |
