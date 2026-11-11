// C interface of the PaperWM engine. Mirrors engine/src/ffi.rs.
// All functions must be called from the main thread.

#ifndef PAPERWM_ENGINE_H
#define PAPERWM_ENGINE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Number of strips the engine keeps (one per display); indices run from 0 to PWM_WORKSPACE_COUNT - 1.
#define PWM_WORKSPACE_COUNT 9

typedef enum {
    PWM_STATUS_OK = 0,
    PWM_STATUS_NOT_TRUSTED = 1,
    PWM_STATUS_WINDOW_NOT_FOUND = 2,
    PWM_STATUS_PLATFORM_ERROR = 3,
    PWM_STATUS_NULL_POINTER = 4,
    PWM_STATUS_INVALID_ARGUMENT = 5,
} PwmStatus;

// Actions for pwm_engine_command. Mirrors engine::Command::parse.
typedef enum {
    PWM_COMMAND_FOCUS_LEFT = 0,
    PWM_COMMAND_FOCUS_RIGHT = 1,
    PWM_COMMAND_FOCUS_UP = 2,
    PWM_COMMAND_FOCUS_DOWN = 3,
    PWM_COMMAND_MOVE_LEFT = 4,
    PWM_COMMAND_MOVE_RIGHT = 5,
    PWM_COMMAND_MOVE_UP = 6,
    PWM_COMMAND_MOVE_DOWN = 7,
    PWM_COMMAND_CYCLE_WIDTH = 8,
    PWM_COMMAND_TOGGLE_FULL_WIDTH = 9,
    PWM_COMMAND_CENTER = 10,
    PWM_COMMAND_CONSUME = 11,
    PWM_COMMAND_EXPEL = 12,
    // 13 to 21 were the workspace and monitor-focus commands.
    PWM_COMMAND_MOVE_TO_MONITOR_LEFT = 22,
    PWM_COMMAND_MOVE_TO_MONITOR_RIGHT = 23,
    PWM_COMMAND_TOGGLE_SCRATCH = 24,
    PWM_COMMAND_MOVE_TO_SCRATCH = 25,
    PWM_COMMAND_FOCUS_WINDOW = 26,              // arg: window id
    PWM_COMMAND_SCROLL_BY = 27,                 // arg: points, positive pans right
    PWM_COMMAND_SCROLL_END = 28,
    PWM_COMMAND_FOCUS_FIRST = 29,
    PWM_COMMAND_FOCUS_LAST = 30,
    PWM_COMMAND_WIDEN_COLUMN = 31,
    PWM_COMMAND_NARROW_COLUMN = 32,
    PWM_COMMAND_TOGGLE_TABBED = 33,
    PWM_COMMAND_TOGGLE_PIN_LEFT = 34,
    PWM_COMMAND_TOGGLE_PIN_RIGHT = 35,
    PWM_COMMAND_TALLER = 36,
    PWM_COMMAND_SHORTER = 37,
    PWM_COMMAND_EQUAL_HEIGHTS = 38,
    PWM_COMMAND_FOCUS_NEXT_MONITOR = 39,
    PWM_COMMAND_UNSTACK = 40,
    PWM_COMMAND_SET_WIDTH = 41,                 // arg: fraction of the screen, in (0, 1]
} PwmCommand;

// Top-left origin, y grows downwards (Accessibility coordinates).
typedef struct {
    double x;
    double y;
    double width;
    double height;
} PwmRect;

typedef struct {
    uint32_t id;
    int32_t pid;
    PwmRect frame;
    char *app_name;
    char *title;
    bool is_standard;
    bool is_resizable;
    bool is_minimized;
} PwmWindow;

typedef struct {
    PwmWindow *windows;
    size_t len;
    PwmStatus status;
} PwmWindowList;

typedef struct PwmEngine PwmEngine;

const char *pwm_version(void);

// With `prompt`, macOS shows its dialog when permission is missing.
bool pwm_accessibility_is_trusted(bool prompt);

PwmEngine *pwm_engine_new(void);
void pwm_engine_free(PwmEngine *engine);

// Free the result with pwm_window_list_free.
PwmWindowList pwm_engine_list_windows(const PwmEngine *engine);
void pwm_window_list_free(PwmWindowList list);

// What went wrong in the latest call that failed, for logs. Valid until the
// next failing call. A panic in the engine fails the call with PLATFORM_ERROR.
const char *pwm_last_error(void);

// Display id (CGDirectDisplayID) and usable area (Accessibility coordinates)
// of each monitor, main one first. Each display keeps its strip by id.
PwmStatus pwm_engine_set_monitors(PwmEngine *engine, const uint32_t *ids, const PwmRect *areas, size_t count);

// Brings the layout up to date with the current windows and focus.
PwmStatus pwm_engine_sync(PwmEngine *engine);

// Syncs after a drag ended with the pointer at x, y (Accessibility
// coordinates): a tiled window dragged there moves into the column under it.
PwmStatus pwm_engine_drop(PwmEngine *engine, double x, double y);

// `command` is a PwmCommand value; `arg` is ignored by commands without one.
// `changed`, if not null, is set to whether the command changed anything.
PwmStatus pwm_engine_command(PwmEngine *engine, uint32_t command, double arg, bool *changed);

// Regroups the active strip's windows, in order, into `count` columns: column
// i takes rows[i] windows (at least 1) and fractions[i] of the screen (in
// (0, 1]). Windows past the layout keep a column each; pinned columns stay.
// `changed`, if not null, is set to whether anything changed.
PwmStatus pwm_engine_arrange(PwmEngine *engine, const uint32_t *rows, const double *fractions, size_t count,
                             bool *changed);

// Brings every managed window fully on screen, for when tiling is turned off.
// The engine keeps its layout; make a new one to tile again.
PwmStatus pwm_engine_release(PwmEngine *engine);

// Advances strip slides by `seconds` and moves the windows. Returns whether a
// strip is still sliding; call again on the next frame if so. `status`, if not
// null, receives the outcome.
bool pwm_engine_tick(PwmEngine *engine, double seconds, PwmStatus *status);

// Whether a strip is sliding, so pwm_engine_tick should run every frame.
bool pwm_engine_is_animating(const PwmEngine *engine);

// Writes where the engine last put managed window `id` into `out`; false if it
// put it nowhere (unmanaged, fullscreen). The window server lags behind this.
bool pwm_engine_placed_frame(const PwmEngine *engine, uint32_t id, PwmRect *out);

// Whether the last sync met a window still being created, so pwm_engine_sync
// should be called again shortly.
bool pwm_engine_is_settling(const PwmEngine *engine);

// Notes something that empties the screen for a while (a Space change, sleep,
// wake, lock, unlock, a display change), so windows missing meanwhile are
// taken for away rather than closed.
void pwm_engine_note_transition(const PwmEngine *engine);

// Seconds until pwm_engine_sync may find a missing window closed though
// nothing notifies, so call it then; negative if no window is waiting.
double pwm_engine_next_check(const PwmEngine *engine);

// Settings as JSON: {"gap": 8, "width_presets": [0.33, 0.5],
// "animation_duration": 0.25, "center_focused": "never" | "on_overflow" | "always",
// "app_rules": [{"app": "Calculator", "action": "float"},
//               {"app": "Mail", "action": "slot", "column": 0}]}.
// Missing fields keep their defaults; malformed JSON returns INVALID_ARGUMENT.
PwmStatus pwm_engine_set_config(PwmEngine *engine, const char *json);

// Layout snapshot as JSON (engine::State), or null on failure. Free with
// pwm_string_free.
char *pwm_engine_state_json(const PwmEngine *engine);
void pwm_string_free(char *string);

// Diagnostic lines logged since the last call (why windows joined or left the
// layout, what each read saw), separated by newlines, or null when there are
// none. Free with pwm_string_free.
char *pwm_take_log(void);

// Start or stop watching an app's windows for changes.
void pwm_engine_observe_app(const PwmEngine *engine, int32_t pid);
void pwm_engine_forget_app(const PwmEngine *engine, int32_t pid);

// `handler(context)` runs on the main run loop when observed windows change.
// It must not call into the engine directly; schedule pwm_engine_sync.
void pwm_engine_set_change_handler(const PwmEngine *engine, void (*handler)(void *context), void *context);

#ifdef __cplusplus
}
#endif

#endif
