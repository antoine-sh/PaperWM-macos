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
} PwmStatus;

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

// Tiles the windows on `area` into equal columns; writes the count to
// `tiled` when it is not NULL.
PwmStatus pwm_engine_tile_columns(const PwmEngine *engine, PwmRect area, double gap, size_t *tiled);

#ifdef __cplusplus
}
#endif

#endif
