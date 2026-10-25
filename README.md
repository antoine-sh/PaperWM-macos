# PaperWM for macOS

A port of [PaperWM](https://github.com/paperwm/PaperWM), the scrollable tiling window manager, to macOS.

The window-management engine is written in Rust and the user interface in Swift, bundled together as a single `.app`. It only needs Accessibility permission; System Integrity Protection stays on.

## Status

Early proof of concept. The menu bar app can list your windows and tile the ones on the current screen into equal columns. The scrollable PaperWM layout, keyboard control and the rest of the feature set are on the roadmap below.

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
| `engine/` | Rust engine: layout, window model, macOS Accessibility backend, C interface (`src/ffi.rs`, `include/paperwm_engine.h`) |
| `app/` | Swift menu bar app (SwiftPM package) that links the engine as a static library |
| `scripts/build-app.sh` | Builds both and assembles `build/PaperWM.app` |
| `scripts/make-dmg.sh` | Packages the app as a `.dmg` |

## Roadmap

1. **Skeleton and proof of concept**: Rust engine, Swift shell, `.app` bundling, CI.
2. **Core scrolling layout**: a horizontal strip of columns per workspace and monitor, keyboard focus and movement, resizing, one tile per window of multi-window apps, floating dialogs.
3. **Feature parity**: hotkeys, workspaces, scratch layer, window switcher, trackpad gestures, workspace indicator, minimap, settings.
4. **Packaging and polish**: Accessibility onboarding, launch at login, config file, signed and notarized releases.
