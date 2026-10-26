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

// Number of workspaces; workspace indices run from 0 to PWM_WORKSPACE_COUNT - 1.
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
    PWM_COMMAND_SWITCH_WORKSPACE = 13,          // arg: workspace index
    PWM_COMMAND_MOVE_TO_WORKSPACE = 14,         // arg: workspace index
    PWM_COMMAND_NEXT_WORKSPACE = 15,
    PWM_COMMAND_PREVIOUS_WORKSPACE = 16,
    PWM_COMMAND_MOVE_TO_NEXT_WORKSPACE = 17,
    PWM_COMMAND_MOVE_TO_PREVIOUS_WORKSPACE = 18,
    PWM_COMMAND_LAST_WORKSPACE = 19,
    PWM_COMMAND_FOCUS_MONITOR_LEFT = 20,
    PWM_COMMAND_FOCUS_MONITOR_RIGHT = 21,
    PWM_COMMAND_MOVE_TO_MONITOR_LEFT = 22,
    PWM_COMMAND_MOVE_TO_MONITOR_RIGHT = 23,
    PWM_COMMAND_TOGGLE_SCRATCH = 24,
    PWM_COMMAND_MOVE_TO_SCRATCH = 25,
    PWM_COMMAND_FOCUS_WINDOW = 26,              // arg: window id
    PWM_COMMAND_SCROLL_BY = 27,                 // arg: points, positive pans right
    PWM_COMMAND_SCROLL_END = 28,
    PWM_COMMAND_FOCUS_FIRST = 29,
    PWM_COMMAND_FOCUS_LAST = 30,
    PWM_COMMAND_TOGGLE_TABBED = 31,
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
// next failing call.
const char *pwm_last_error(void);

// Usable area of each monitor (Accessibility coordinates), main one first.
PwmStatus pwm_engine_set_monitors(PwmEngine *engine, const PwmRect *areas, size_t count);

// Brings the layout up to date with the current windows and focus.
PwmStatus pwm_engine_sync(PwmEngine *engine);

// Syncs after a drag ended with the pointer at x, y (Accessibility
// coordinates): a tiled window dragged there moves into the column under it.
PwmStatus pwm_engine_drop(PwmEngine *engine, double x, double y);

// Puts every window back in its place and syncs.
PwmStatus pwm_engine_retile(PwmEngine *engine);

// `command` is a PwmCommand value; `arg` is ignored by commands without one.
// `changed`, if not null, is set to whether the command changed anything.
PwmStatus pwm_engine_command(PwmEngine *engine, uint32_t command, double arg, bool *changed);

// Advances strip slides by `seconds` and moves the windows. Returns whether a
// strip is still sliding; call again on the next frame if so. `status`, if not
// null, receives the outcome.
bool pwm_engine_tick(PwmEngine *engine, double seconds, PwmStatus *status);

// Whether a strip is sliding, so pwm_engine_tick should run every frame.
bool pwm_engine_is_animating(const PwmEngine *engine);

// Settings as JSON: {"gap": 8, "width_presets": [0.33, 0.5], "workspace_names": ["1", ...],
// "animation_duration": 0.25, "center_focused": "never" | "on_overflow" | "always",
// "app_rules": [{"app": "Calculator", "action": "float"},
//               {"app": "Mail", "action": "slot", "workspace": 1, "column": 0}]}.
// Missing fields keep their defaults; malformed JSON returns INVALID_ARGUMENT.
PwmStatus pwm_engine_set_config(PwmEngine *engine, const char *json);

// Layout snapshot as JSON (engine::State). Free with pwm_string_free.
char *pwm_engine_state_json(const PwmEngine *engine);
void pwm_string_free(char *string);

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
