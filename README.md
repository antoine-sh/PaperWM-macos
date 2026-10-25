# PaperWM for macOS

A port of [PaperWM](https://github.com/paperwm/PaperWM), the scrollable tiling window manager, to macOS.

The window-management engine is written in Rust and the user interface in Swift, bundled together as a single `.app`. It only needs Accessibility permission; System Integrity Protection stays on.

## Status

Early but usable. Each monitor gets a PaperWM-style scrolling strip: every window opens as a new column to the right of the focused one, focusing a column scrolls it into view, and columns can be rearranged, resized and stacked from the keyboard. Dialogs, sheets and other non-standard windows float. Columns that scroll off screen are parked at the screen edge, since macOS doesn't allow hiding windows entirely off screen.

Not yet: workspaces (switching macOS Spaces drops the windows of the old Space from the layout), moving windows between monitors, the scratch layer, gestures and configuration. See the roadmap below.

### Keyboard shortcuts

⌃⌥ stands in for PaperWM's Super key.

| Shortcut | Action |
| --- | --- |
| ⌃⌥ ← / → | Focus the column on the left / right |
| ⌃⌥ ↑ / ↓ | Focus the window above / below in the column |
| ⌃⌥⇧ ← / → | Move the column left / right |
| ⌃⌥⇧ ↑ / ↓ | Move the window up / down in its column |
| ⌃⌥ R | Cycle column width (⅓, ½, ⅔ of the screen) |
| ⌃⌥ F | Toggle full width |
| ⌃⌥ C | Center the column |
| ⌃⌥ I | Pull the next column's window into this column |
| ⌃⌥ O | Push the focused window out into its own column |

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
| `engine/` | Rust engine: scrolling layout (`workspace.rs`, `placement.rs`), window sync and commands (`engine.rs`), macOS Accessibility backend (`macos/`), C interface (`ffi.rs`, `include/paperwm_engine.h`) |
| `app/` | Swift menu bar app (SwiftPM package): hot keys, app and monitor tracking; links the engine as a static library |
| `scripts/build-app.sh` | Builds both and assembles `build/PaperWM.app` |
| `scripts/make-dmg.sh` | Packages the app as a `.dmg` |

## Roadmap

1. **Skeleton and proof of concept**: Rust engine, Swift shell, `.app` bundling, CI.
2. **Core scrolling layout**: a horizontal strip of columns per workspace and monitor, keyboard focus and movement, resizing, one tile per window of multi-window apps, floating dialogs.
3. **Feature parity**: hotkeys, workspaces, scratch layer, window switcher, trackpad gestures, workspace indicator, minimap, settings.
4. **Packaging and polish**: Accessibility onboarding, launch at login, config file, signed and notarized releases.
