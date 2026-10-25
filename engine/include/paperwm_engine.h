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

typedef enum {
    PWM_STATUS_OK = 0,
    PWM_STATUS_NOT_TRUSTED = 1,
    PWM_STATUS_WINDOW_NOT_FOUND = 2,
    PWM_STATUS_PLATFORM_ERROR = 3,
    PWM_STATUS_NULL_POINTER = 4,
    PWM_STATUS_INVALID_ARGUMENT = 5,
} PwmStatus;

// Keyboard actions for pwm_engine_command. Mirrors engine::Command.
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

PwmStatus pwm_engine_set_window_frame(const PwmEngine *engine, uint32_t id, PwmRect frame);

// Usable area of each monitor (Accessibility coordinates), main one first.
PwmStatus pwm_engine_set_monitors(PwmEngine *engine, const PwmRect *areas, size_t count);

// Brings the layout up to date with the current windows and focus.
PwmStatus pwm_engine_sync(PwmEngine *engine);

// `command` is a PwmCommand value.
PwmStatus pwm_engine_command(PwmEngine *engine, uint32_t command);

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
