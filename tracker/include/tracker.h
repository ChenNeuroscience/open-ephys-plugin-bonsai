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

/* Summary of the tracker state for stream metadata (see tracker/src/metadata.rs). */
typedef struct TrackerMetadata {
    double pixels_per_metre; /* NaN if not calibrated */
    int32_t window[4];       /* crop bounding box clipped to the frame: min_x, min_y, max_x, max_y */
    char crop[128];          /* e.g. "circle(cx,cy,r) 320 240 200": format then values; NUL-terminated, zero-padded */
    int32_t frame_size[2];   /* width, height; 0 if not known yet */
} TrackerMetadata;

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

/* Whether Open Ephys is recording; the tracker window locks all settings while it is. */
void tracker_set_recording(const TrackerHandle* handle, bool recording);

/* Opens the tracker window if it was closed, or brings it to the front. Returns false where the window isn't
   supported (macOS). */
bool tracker_open_window(const TrackerHandle* handle);

/* Writes the window-adjustable state (crop, rulers, threshold...) as NUL-terminated JSON, to be persisted and
   passed back as config.state next time. Returns the untruncated length; retry with a bigger buffer if >= len. */
size_t tracker_state_json(const TrackerHandle* handle, char* buf, size_t len);

/* Fills `out` from a tracker state JSON (from tracker_state_json, or saved; empty for defaults).
   Returns false if it doesn't parse. */
bool tracker_metadata(const char* state_json, TrackerMetadata* out);

/* Stops capture, closes the window, frees the handle. NULL is ignored. */
void tracker_stop(TrackerHandle* handle);

#ifdef __cplusplus
}
#endif

#endif
