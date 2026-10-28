/* C interface to the Rust tracker library (tracker/src/ffi.rs). */
#ifndef TRACKER_H
#define TRACKER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct TrackerHandle TrackerHandle;

typedef struct TrackerSample {
    double timestamp; /* camera clock, seconds */
    float values[6];  /* x1, y1, x2, y2, area1, area2; missing blobs are NaN with area 0 */
} TrackerSample;

/* Starts capture (and the live-view window if config.show_window). Returns NULL on failure,
   with a message written to `error`. Config is JSON, see tracker/src/config.rs. */
TrackerHandle* tracker_start(const char* config_json, char* error, size_t error_len);

/* Waits up to timeout_ms, then copies up to `max` samples. Returns count, or -1 if capture died. */
int64_t tracker_poll(const TrackerHandle* handle, TrackerSample* out, size_t max, uint32_t timeout_ms);

/* Writes a NUL-terminated status line into buf; returns its untruncated length. */
size_t tracker_status(const TrackerHandle* handle, char* buf, size_t len);

/* Opens the tracker window if it was closed, or brings it to the front. Returns false where the window isn't
   supported (macOS). */
bool tracker_open_window(const TrackerHandle* handle);

/* Writes the window-adjustable state (crop, rulers, threshold...) as NUL-terminated JSON, to be persisted and
   passed back as config.state next time. Returns the untruncated length; retry with a bigger buffer if >= len. */
size_t tracker_state_json(const TrackerHandle* handle, char* buf, size_t len);

/* Stops capture, closes the window, frees the handle. NULL is ignored. */
void tracker_stop(TrackerHandle* handle);

#ifdef __cplusplus
}
#endif

#endif
