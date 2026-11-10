# Windows leave the layout only on evidence — design

Status: implemented after one diagnostic log from a real Mac (below).

## What the first log showed (Zen into fullscreen and back)

- `kAXWindowsAttribute` follows the active Space and switches at a different
  moment than CGWindowList. Entering fullscreen, Zen's AX list dropped its
  window while CG still showed it on screen; leaving, six on-screen apps were
  read and reported nothing, because AX still showed the fullscreen Space.
  So the AX list does **not** list other Spaces' windows, and during an
  animation the two sources disagree.
- After that empty read, `seen` recorded each app's on-screen set as read,
  so the apps were skipped as unchanged and their windows never came back
  until each app notified. This is why every window stayed forgotten.

Refinements made in the implementation:

- A known window that CG shows on screen but its app's AX list leaves out
  counts as a transition, and the app is not recorded in `seen`, so it is
  read again next time.
- A known window counts as present for the home test when it is reported or
  CG shows it on screen; windows of apps skipped as unchanged would
  otherwise make every read look like a blackout.
- The implicit blackout needs at least two windows to vanish in one read, so
  a lone window closed to the background still closes.
- A window whose busy app cannot answer for it loses its absence clock, so
  a due verdict never makes the app re-sync in a tight loop; the app also
  waits at least 0.5 s between rechecks.
- `Departure::Minimized` covers a window minimized while its app has nothing
  else on screen, which is never listed.
- Not done: the `kill(pid, 0)` check for a quit missed during sleep;
  NSWorkspace delivers the termination on wake.

## Problem

After waking from sleep and after leaving a fullscreen app, every window drops
out of the layout and comes back as a new window, losing its column, pins and
tabs. Five targeted fixes (f1b850f, d224f75, 4831525, 4e2fa4e, and the
STILL_BUSY grace inside 4e2fa4e) each guarded one moment, and the user reports
the behaviour got worse.

## Root cause (verified in the code)

The engine treats "not in this read" as "closed".

- `Engine::sync_dropping` (`engine.rs`) builds `current` from what
  `backend.windows()` returned, plus previous info of apps in
  `skipped_apps()`. Every managed window in neither is removed from its
  workspace (`gone` loop), the scratch layer and the MRU, and the applier
  forgets it. Nothing else counts as a reason.
- `AxBackend::windows()` (`macos/backend.rs`) only reads apps that own a window
  in `CGWindowListCopyWindowInfo(ON_SCREEN_ONLY)`, plus apps with a cached
  fullscreen window. An app with nothing on screen is neither read nor
  skipped, so all its windows are dropped.
- In apps it does read, a known standard window that is not on screen and not
  minimized or fullscreen goes into `away`, which is reported only if a
  fullscreen window is on screen in this same read (f1b850f). Otherwise it is
  dropped.

So one read taken while the on-screen list is empty or partial wipes the
layout. That happens:

- **Fullscreen exit.** The window stops reporting AXFullScreen before the
  desktop Space's windows are back on screen. f1b850f's guard needs a
  fullscreen window on screen, which is no longer true, so everything else
  drops.
- **Wake and unlock.** `WindowManager.resume()` holds syncs for 0.5 s, then
  calls `updateMonitors()` at 0.6, 2, 5 and 10 s. `set_monitors` calls
  `backend.invalidate()`, so each of those is a full re-read of every app at
  the moment windows are still coming back. The held syncs make the forced
  0.6 s read the *first* read after unlock, which is the likeliest one to
  catch a partial list. This plausibly explains "worse than before": the wake
  fixes moved the first read onto the worst moment and added three more
  chances to hit it. (Not confirmable without the logs planned below.)
- **Busy apps.** 4e2fa4e keeps a busy app's windows for 15 s
  (`STILL_BUSY`), then drops them even though nothing closed. Apps can stay
  slow longer than that after wake.

The second suspect, display ids (2efab51, 33dd7b1), does not drop windows,
but can hide them: `set_monitors` gives a display whose id it has never seen
"the first workspace not shown anywhere". With one display whose id changes
across wake, that is the first workspace ("1" in the menu bar), whatever the user had on
screen; their windows are intact on a hidden workspace, parked in the corner.
To the user that looks the same as "gone". See "Display ids" below.

## Goal

A managed window leaves the layout only on positive evidence that it left:
it was destroyed, its app quit, its app was hidden, it was minimized, it
stopped being tileable, or it stayed away in a way no transition explains.
Being absent from one read is never enough.

Non-goals: supporting windows spread over several macOS Spaces (the README
says "keep everything on one Space"; behaviour there stays as today or
better), remembering a hidden or minimized window's slot (open question 3).

## macOS facts the design depends on

| Fact | What we believe | Confidence | Design depends on it? |
|---|---|---|---|
| A cached `AXUIElement` of a closed (released) window answers `kAXErrorInvalidUIElement` | Yes; this is how every AX window manager detects closed windows | High | Yes: main evidence of "destroyed" |
| The same element still answers while its window is on another Space, on a fullscreen Space, or the screen is locked | Yes, the element belongs to the app, not the Space | High; confirm with the log | Yes |
| `kAXWindowsAttribute` lists windows on other Spaces | Mostly **no**: it lists the current Space's windows and minimized ones; some apps keep listing windows already seen. f1b850f's comment assumes the other way | Medium; **confirm on a Mac** | No: the design probes cached elements instead of relying on the list |
| Windows parked on hidden workspaces stay in the ON_SCREEN_ONLY list | Yes: `hidden_frame` leaves 1 pt (`PEEK`) inside the corner display, and "on screen" means ordered in on the active Space anyway | High; confirm with the log | No longer (they are kept either way), but a "no" would make every workspace switch look like a blackout |
| CGWindowList during the lock screen | Unknown. Either the session's windows stay listed under the shield, or the list is empty / only loginwindow's. With fast user switching the list is the other session's | **Confirm on a Mac** | No: both cases are handled |
| CGWindowList during a Space or fullscreen animation | Unknown; probably windows of one, both or neither Space for a few hundred ms | **Confirm on a Mac** | No |
| `activeSpaceDidChange` arrives before or after the windows come back | Unknown, likely after the animation ends | **Confirm on a Mac** | No: grace and blackout detection don't need the order |
| A window closed with `orderOut` but kept alive (Slack, Discord and other "close to background" apps, `isReleasedWhenClosed = false`) keeps a valid element, leaves AXWindows and sends no UIElementDestroyed | Yes | Medium-high | Yes: this is the one case that needs the grace rule |
| AXHidden on the application element is true while the app is hidden (⌘H) | Yes | High | Yes |
| `CGDirectDisplayID` can change across sleep or reconnect (docks, DisplayLink, MST, port swaps) | Yes for some external setups; built-in display ids are stable | Medium | Yes: fallback below |

## Options weighed

Scenarios: W wake, L lock, F fullscreen enter/exit animation, S Space switch,
M move a window to another Space, H ⌘H, Mi minimize, T native tabs, C close,
Q app quit, B busy app.

**(a) Positive evidence only.** Probe each known window that a read did not
list, through its cached element: invalid → destroyed; minimized; app
hidden; app quit (NSWorkspace termination, or the pid is gone).
W, L, F, S, B: kept (element valid). H, Mi, C, Q: correct and immediate. T:
unchanged (tabs are listed). M: kept forever as a ghost column. **Fails one
real case:** "close to background" apps keep a valid element, so their closed
windows stay as ghost columns.

**(b) Grace period alone.** W and L: fails whenever windows take longer than
the grace to return, or the Mac stays locked longer. S: drops after the grace.
Only moves the cliff; this is what STILL_BUSY did.

**(c) Spaces awareness.** Public API cannot tell a window on another Space
from an ordered-out one: `kCGWindowListOptionAll` lists both with
`kCGWindowIsOnscreen` false. The private `CGSCopySpacesForWindows` (read-only,
works with SIP on, used by Hammerspoon, yabai and AltTab) returns no Space
for an ordered-out window, which would settle the one case (a) cannot.
Justifiable, but a private symbol can vanish in a macOS update, and the
public heuristic below may be enough. Kept in reserve (open question 1).

**(d) Blackout detection.** A read where at least half of the managed windows
on shown workspaces are missing is unreliable. Handles W, L, F, S well. On
its own it cannot handle a single window, or C, H, Mi, Q at all.

**Recommendation: (a) as the rule, with (d) and a transition-aware grace
only for the one ambiguous case (valid element, not listed), and (c) held
back unless the logs show that heuristic misfires.**

| | W | L | F | S | M | H | Mi | T | C | Q | B |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Today | drops | drops | drops | drops | drops | drops | drops | ok | ok | ok | drops after 15 s |
| Proposed | kept | kept | kept | kept | dropped after grace (as today, later) | dropped at once (as today) | dropped at once (as today) | as today | dropped at once; ordered-out after grace | dropped at once | kept until it answers or quits |

## Design

### 1. The backend reports departures, not just a list

`WindowBackend` gains one method and loses `skipped_apps`:

```rust
/// Why a window the backend reported before has left for good.
pub enum Departure {
    Destroyed,  // its element is invalid
    AppQuit,    // forget_app, or its pid no longer exists
    AppHidden,  // AXHidden on its app
    Closed,     // valid element, absent past the grace with nothing to explain it
}

/// Windows reported before that the last `windows()` call found gone, and why.
fn departures(&self) -> Vec<(WindowId, Departure)>;
```

`windows()` keeps returning the windows it read. Minimized and non-tileable
windows stay in that list with their flags, as today, so the engine's
existing release path handles them.

### 2. The engine removes only what departed

In `sync_dropping`, `current` becomes: the windows read, plus the previous
info of every managed window that was neither read nor departed. That
replaces the `skipped_apps` merge; a skipped (unchanged or busy) app is just
one whose windows were not read. `fresh` stays "windows read now", so
windows kept from before are not observed or put back, exactly as skipped
apps' windows are today. Windows hidden as background tabs are not kept: a
window listed in another window's `tabs` is treated as not current, so
`follow_tabs` still hands its place to the shown tab.

The `gone` loop is unchanged; it now only ever sees departed windows,
windows that became floating or minimized, and tabs replaced by
`follow_tabs`. Each removal is logged with its reason (section 6).

### 3. `AxBackend::windows()` probes absent windows

- Keep the cached element of every known window, whether or not its app has
  anything on screen (today they are dropped with the app).
- After reading the on-screen apps as today, for each cached window that was
  not listed and whose app was read, notified (`handler.changed`), or
  `everything` is set: one batched `values()` call on its element for
  subrole and minimized, and AXHidden on its app element once per app.
  - `kAXErrorInvalidUIElement` → `Destroyed`.
  - app hidden → `AppHidden`.
  - minimized → listed with `is_minimized`, as for on-screen apps today.
  - `kAXErrorCannotComplete` or stalled → busy: nothing reported, kept.
  - answers, not minimized → *absent*: fed to the absence tracker (4).
- Windows of apps not probed this time are left alone. Destruction,
  minimizing, hiding and focus moves all notify and mark the app, so this
  stays cheap during a long stay on another Space.
- `forget_app` queues `AppQuit` for the app's cached windows. A cheap
  `kill(pid, 0)` check (`ESRCH`) on probed apps catches a termination missed
  during sleep.
- Removed: the `away` list and fullscreen guard (f1b850f), the `busy` map and
  `STILL_BUSY` (4e2fa4e), the "known app is busy → skip" branches (d224f75),
  `skipped`. Kept: `stalled` back-off, `seen` skipping, settling, the error
  when the window server gives no list (4831525), the fullscreen app being
  read with nothing on screen.

### 4. The absence tracker (new `absence.rs`, pure, like `settle.rs`)

The one ambiguous case is a window whose element answers but that is not
listed: another Space, a fullscreen Space, a blackout, or an ordered-out
("closed to background") window. The tracker decides, from what reads show
and from transitions, when such a window counts as `Closed`.

Inputs per read: windows listed, windows absent (probed and valid), the ids
of managed windows on shown workspaces (from the engine via the backend, or
simply every window reported before), whether a transition was noted since
the last read, and `now`.

- A **transition** is anything that legitimately empties the screen: a
  Swift note (`activeSpaceDidChange`, sleep, wake, screens sleep/wake,
  session resign/active, lock, unlock, screen parameters change), a window
  entering or leaving fullscreen, or a **non-home read** (fewer than half of
  the previously listed windows listed, nothing new) following a home read.
- `Closed(X)` when X has been absent in every read for at least `GRACE`
  (proposed 5 s), and either
  1. no transition happened since X was last listed — it left while the
     user was looking at its Space: closed to background, or moved to
     another Space; or
  2. reads have been home (at least half of the windows listed) for at
     least `GRACE` since the last transition — the user is back on X's
     Space and X did not come back.
- While the user is on a fullscreen Space, another Space, the lock screen or
  in a wake blackout, reads are non-home, so nothing matures and windows
  are kept indefinitely. A busy app's windows are never absent, only busy.
- `next_check()` gives the time until the earliest verdict could mature, so
  the app can sync once then (a closed-to-background window would otherwise
  leave its gap until the next notification).

Corner it gets wrong, deliberately: on another Space with more managed
windows than the home Space, reads count as home and the home Space's
windows close after the grace. That is today's behaviour, in a setup the
README already asks users to avoid.

### 5. Swift changes

- `WindowManager`: replace the sync holds (`asleep`, `heldUntil`,
  `isHeld`, `wakeIfUnlocked`, `isUnlockedOnConsole`, the reset in
  `perform` and `retile`) with `engine.noteTransition(name)` on the same
  notifications, plus `activeSpaceDidChange` and screen changes. Syncs run
  normally at all times.
- Keep one re-read after the last wake or unlock (`updateMonitors()` at
  about 1 s and 5 s), since macOS moves windows when displays come back.
  It is now harmless if it hits a blackout.
- After each sync, if `engine.nextCheck` is non-negative, schedule one
  `scheduleSync()` that far ahead (coalesced, like `settle()`).
- Drain the engine log after every engine call (section 6).

FFI additions (`ffi.rs`, `paperwm_engine.h`, `Engine.swift`):
`pwm_engine_note_transition(engine, const char *name)`,
`double pwm_engine_next_check(engine)` (negative for none),
`char *pwm_engine_take_log(engine)` (NULL when empty, freed with
`pwm_string_free`).

### Reverting the wake commits

Not as `git revert`: the commits are interleaved with keepers and with
2efab51. The implementation commit removes, by hand:

- d224f75: the Swift sleep/lock hold (keep the notification subscriptions,
  now feeding `noteTransition`), and the "known app busy → skipped" and
  "known on-screen window busy" branches in the backend.
- 4831525: the hold refinements in Swift. Keep the immediate delivery of
  distributed notifications and the `None` → error in `window_list.rs`.
- 4e2fa4e: `STILL_BUSY`, `busy`, `still_busy`, and the Swift hold changes.
- f1b850f: `away`, `present` and the fullscreen-Space skipping.

### Display ids

`set_monitors` keeps matching by id first. New fallback, for a display whose
id is neither connected nor remembered: before taking "the first workspace
not shown anywhere", it takes the workspace of a display that disconnected
and is not shown now, the most recently disconnected first (`remembered`
records order). A single display whose id changes across wake then keeps its
workspace; a genuinely new display picks up an orphaned workspace, which is
harmless. If logs show ids churning a lot, the next step is matching on
`CGDisplayCreateUUIDFromDisplayID` instead (public, stable across reboots).

### 6. Diagnostic logging

Goal: one run on the Mac shows why each window left.

- The engine keeps a `Vec<String>` of log lines; `pwm_engine_take_log`
  returns and clears them. Swift writes each with
  `Logger(subsystem: "io.github.antoine-sh.PaperWM", category: "engine")`,
  `privacy: .public`. Lines carry window ids and app names, never titles.
- Lines:
  - `remove <id> <app> ws<w> col<c>: <reason>`, reason one of `destroyed`,
    `app quit`, `app hidden`, `minimized`, `closed (absent <s> s, left while
    on screen)`, `closed (absent <s> s, home again <s> s)`, `floats (rule |
    fixed size | not standard)`, `replaced by tab <id>`.
  - `adopt <id> <app> ws<w> col<c>` for every new window, so "came back as
    new" is visible.
  - `read: listed <n>, absent <n>, busy <n>, departed <n>, on-screen list
    <n>, home <yes|no>` whenever one of those counts changes.
  - `transition: <name>` for each Swift note and engine-detected one
    (`fullscreen on <id>`, `fullscreen off <id>`, `non-home read`).
  - `monitors: <id> <w>x<h> -> ws<w> (<kept | returned | new | inherited
    from <old id>>)` on each `set_monitors` call.
  - `backend: <error>` for failed reads (no window list, not trusted).
- How to read it: `log stream --level info --predicate 'subsystem ==
  "io.github.antoine-sh.PaperWM"'` in Terminal, then sleep, wake, enter and
  leave fullscreen, switch Space, ⌘H, minimize, close, quit.

## Test plan (MockBackend)

### Mock extensions (`mock.rs`)

The mock keeps one list of windows and gains per-window and global states:

- `set_away(id, bool)`: not listed, element valid, no departure (other
  Space, fullscreen Space).
- `blackout(bool)`: lists nothing, reports no departures (lock, wake,
  animation). `blackout_except(ids)` for partial lists.
- `hide_app(pid, bool)`: not listed; reports `AppHidden` once.
- `remove(id)` now reports `Destroyed` once (today the window just vanishes).
- `quit_app(pid)`: removes the app's windows, reports `AppQuit`.
- `close_to_background(id)`: not listed, valid, no departure; the verdict is
  the tracker's job, tested in `absence.rs`. The mock takes a
  `depart(id, Departure)` for engine tests that need `Closed`.
- `set_unresponsive` keeps meaning busy: not listed, no departure.
- `departures()` returns and clears what was queued since the last
  `windows()`; the `Shared` wrapper forwards it.

### Engine tests (`engine.rs`)

Each sets up three or more windows with a pin and a tab group, records
`columns()`, pins and tabs, and asserts all are unchanged afterwards unless
stated.

1. `blackout_keeps_every_window_in_place`: blackout, sync several times,
   lift it, sync: same columns, pins, tabs, focus, no new columns.
2. `partial_list_keeps_missing_windows`: only one window listed (fullscreen
   exit mid-animation), then all back.
3. `fullscreen_exit_keeps_the_desktop_layout`: one window fullscreen, the
   others away; it leaves fullscreen while the others are still away; sync;
   they come back: layout intact.
4. `away_windows_keep_their_columns_until_they_return` (Space switch).
5. `busy_app_keeps_its_windows_for_good`: unresponsive for many syncs (no
   15 s cutoff), then answers: unchanged. Replaces
   `busy_app_keeps_its_windows_in_place`.
6. `destroyed_window_leaves_at_once`, and on a hidden workspace (existing
   `closed_window_on_hidden_workspace_is_forgotten` keeps passing via
   `Destroyed`).
7. `quit_app_removes_all_its_windows`, including away ones.
8. `hidden_app_leaves_and_returns_as_new` (today's ⌘H behaviour, kept).
9. `minimized_window_leaves` (existing test still passes).
10. `closed_departure_removes_the_window`.
11. `background_tab_is_not_kept_as_absent`: tab switch while the old tab is
    not listed still moves the column to the shown tab; existing tab tests
    keep passing.
12. `new_window_during_blackout_is_adopted_after`: a window that appears
    while others are blacked out joins next to focus, nothing else moves.
13. `removals_are_logged_with_reason` and `adoptions_are_logged`.
14. Display ids: `display_with_new_id_inherits_the_vanished_displays_workspace`
    (one display, id 1 showing ws 3 → id 2), and
    `new_display_takes_an_orphaned_workspace_first`. Existing monitor tests
    keep passing.

### Absence tracker tests (`absence.rs`, explicit `Instant`s)

1. Absent window with no transition closes after `GRACE`, not before.
2. A transition after its last listing keeps it while reads are non-home,
   for any time.
3. After home reads resume for `GRACE` and it is still absent, it closes.
4. Coming back before the verdict resets its clock.
5. A non-home read following a home read counts as a transition.
6. A single-window setup: its one window closed to background closes after
   `GRACE` (rule 1) even though every read is non-home.
7. `next_check` reports the earliest maturing verdict, and none while
   everything waits on a home read.

### On the Mac (after CI is green)

With the log streaming: sleep and wake (lid and menu), lock and unlock,
enter and leave fullscreen (green button and ⌃⌘F), swipe to another Space
and back, ⌘H and unhide, minimize and restore, close a window, close a
Slack or Discord window to background, quit an app, beachball an app (e.g.
a long Script Editor loop), switch tabs in Safari and Finder, unplug and
replug an external display. Each should leave the layout intact or log the
expected reason.

## Open questions

1. Is the transition-aware grace enough for "close to background" windows,
   or should we use the private `CGSCopySpacesForWindows` to tell an
   ordered-out window from one on another Space? Decide after one run of the
   logs.
2. `GRACE` of 5 s: long enough for wake laggards, short enough that a ghost
   gap after a close-to-background is tolerable?
3. ⌘H and minimize keep today's behaviour (leave, come back as new). Should
   they instead keep their slot and restore into it? That is a feature, not
   this fix.
4. Should frames still be written to absent windows when the layout changes
   while they are away? Today skipped apps' windows are; writing to a
   window on another Space moves it there, which seems right for one-Space
   use.
