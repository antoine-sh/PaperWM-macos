# PaperWM for macOS

A port of PaperWM (scrollable tiling window manager) to macOS: a Rust engine
(`engine/`) linked as a static library into a Swift menu bar app (`app/`),
shipped as one `.app`. Accessibility permission only; SIP stays on. See
`README.md` for features, shortcuts and the roadmap.

# Commands

```sh
cargo fmt --all --check                       # CI fails on unformatted code
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked                           # engine tests; run anywhere, Linux included
scripts/build-app.sh [--universal]            # macOS only: engine + Swift app -> build/PaperWM.app
scripts/make-dmg.sh                           # macOS only: build/PaperWM-<version>.dmg
```

CI (`.github/workflows/ci.yml`, macos-15) runs fmt, clippy for both
`aarch64-apple-darwin` and `x86_64-apple-darwin` with `-D warnings`, tests,
then the universal build and dmg. Clippy on Linux skips the `macos/` module,
so code there is only linted and compiled in CI.

Swift cannot be built or checked on Linux. Swift changes are verified only by
CI's `Build app` step, so read them over carefully (types, `@MainActor`,
optionals) before pushing. Nothing has been tested by hand on a real Mac yet.

# Architecture

- **Rust engine owns all window state and layout.** Swift owns everything the
  user sees and touches (hot keys, overlays, settings, gestures, drag
  detection) and drives the engine through the C interface.
- `engine/src/engine.rs`: `Engine`, `Config`, `AppRule`, `Command`, sync,
  commands, drops, JSON state. Most logic and most tests live here.
- `workspace.rs` (strip of columns, scroll), `placement.rs` (column frames),
  `animator.rs` (strip slides), `applier.rs` (writes frames, skips redundant
  AX calls, gives up on windows that refuse a frame), `geometry.rs` (`Rect`).
- `backend.rs`: the `WindowBackend` trait, the only OS boundary.
  `macos/` implements it with Accessibility (`ax.rs`, `observer.rs`,
  `window_list.rs`); `mock.rs` is the in-memory backend for tests and non-macOS
  builds.
- `ffi.rs` + `engine/include/paperwm_engine.h`: the C API. Swift imports it
  via `app/Sources/CPaperWMEngine/module.modulemap`; `app/Sources/PaperWM/Engine.swift`
  wraps it. All calls are main-thread only.
- `app/Sources/PaperWM/WindowManager.swift`: wires the system (app launch/quit,
  screens, AX change callback) to the engine, schedules syncs, runs the frame
  driver for slides, routes `Action`s from hot keys and gestures.

Coordinates everywhere are Accessibility coordinates: top-left origin of the
primary display, y grows downwards. Convert at the AppKit boundary only.

# Keeping the two sides in sync

These have no compiler check across the boundary; change all parts together.

- **New command**: add the `Command` variant and its code in `Command::parse`
  (`engine.rs`), the `PWM_COMMAND_*` value in `paperwm_engine.h` (same number,
  comment if it takes an `arg`), handle it in `Engine::command`, then add a
  `HotKey` in `HotKeys.swift` `defaults` and a README shortcut row if bound.
- **New FFI function**: `ffi.rs` and `paperwm_engine.h` together, then a
  wrapper in `Engine.swift`. Strings returned to Swift are freed with
  `pwm_string_free`; window lists with `pwm_window_list_free`.
- **New setting the engine reads**: field on `Config` (serde, snake_case,
  give it a default so older JSON still parses) and in the `Config` struct in
  `Settings.engineConfigJSON` (encoded with `.convertToSnakeCase`). UI-only
  settings live only in `Settings.Values`; decode new ones with
  `decodeIfPresent ?? defaults` so stored settings keep loading.
- **New state field**: `Engine::state` (JSON) and `EngineState.swift`,
  including its `CodingKeys`.
- `WORKSPACE_COUNT` (engine) and `PWM_WORKSPACE_COUNT` (header) must match.

# Testing

Engine behaviour is tested against `MockBackend` (`mock.rs`), which mimics
macOS quirks such as asynchronous focus and apps that don't answer. Engine
tests use a `Shared(Rc<MockBackend>)` wrapper to keep a handle on the backend.
Add a test for every engine behaviour change or bug fix; prefer extending the
mock over special-casing tests. Swift has no tests.

# Conventions

- Rust formatted by `rustfmt.toml` (`max_width = 120`). Keep Swift lines to
  about the same width.
- Doc comments on public items and non-obvious fields say *why*, in plain
  sentences; match the existing comment density rather than adding more.
- Keep the README (features, shortcut table, layout table) current when
  behaviour or shortcuts change.
- Design docs and plans go in `docs/superpowers/specs/` and
  `docs/superpowers/plans/`.

# Git

- Antoine Jeanselme is the sole author of every commit. Commit with
  `git -c user.name="Antoine Jeanselme" -c user.email="67123340+antoine-sh@users.noreply.github.com" commit ...`
  so both author and committer are Antoine.
- Never add `Co-Authored-By`, `Claude-Session` or any other Claude attribution
  lines to commit messages or PR descriptions.
