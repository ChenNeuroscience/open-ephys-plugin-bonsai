//! C ABI used by the Open Ephys plugin. See `include/tracker.h`.
//! No panic may unwind across this boundary, so every entry point is wrapped in `catch_unwind`.

use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

use crate::config::Config;
use crate::pipeline::TrackerSample;
use crate::tracker::Tracker;

pub struct TrackerHandle(Tracker);

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

/// Copies `s` into `buf` as a NUL-terminated string, truncating if needed. Returns the full length of `s`.
unsafe fn write_str(s: &str, buf: *mut c_char, len: usize) -> usize {
    if !buf.is_null() && len > 0 {
        let n = s.len().min(len - 1);
        std::ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    s.len()
}

/// # Safety
/// `config_json` must be a valid NUL-terminated string; `error` must be null or point to `error_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn tracker_start(
    config_json: *const c_char,
    error: *mut c_char,
    error_len: usize,
) -> *mut TrackerHandle {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<_, String> {
        let json = CStr::from_ptr(config_json).to_str().map_err(|e| e.to_string())?;
        let config: Config = serde_json::from_str(json).map_err(|e| format!("bad config: {e}"))?;
        Ok(Box::into_raw(Box::new(TrackerHandle(Tracker::start(config)))))
    }))
    .unwrap_or_else(|_| Err("panic in tracker_start".into()));
    match result {
        Ok(handle) => handle,
        Err(e) => {
            write_str(&e, error, error_len);
            std::ptr::null_mut()
        }
    }
}

/// Waits up to `timeout_ms` for samples and copies up to `max` of them into `out`.
/// Returns the number copied, or -1 if the capture thread has died.
///
/// # Safety
/// `handle` must come from `tracker_start`; `out` must point to `max` samples.
#[no_mangle]
pub unsafe extern "C" fn tracker_poll(
    handle: *const TrackerHandle,
    out: *mut TrackerSample,
    max: usize,
    timeout_ms: u32,
) -> i64 {
    guard(-1, || {
        let out = std::slice::from_raw_parts_mut(out, max);
        match (*handle).0.poll(out, Duration::from_millis(timeout_ms as u64)) {
            Some(n) => n as i64,
            None => -1,
        }
    })
}

/// Copies a human-readable status line into `buf`. Returns its full length.
///
/// # Safety
/// `handle` must come from `tracker_start`; `buf` must be null or point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn tracker_status(handle: *const TrackerHandle, buf: *mut c_char, len: usize) -> usize {
    guard(0, || {
        let status = (*handle).0.shared().status.lock().unwrap().clone();
        write_str(&status, buf, len)
    })
}

/// Copies the current [`crate::config::TrackerState`] (crop, rulers, threshold...) as JSON into `buf`, for the
/// host to persist and pass back as `state` in the next config. Returns its full length; call again with a
/// bigger buffer if that's >= `len`.
///
/// # Safety
/// `handle` must come from `tracker_start`; `buf` must be null or point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn tracker_state_json(handle: *const TrackerHandle, buf: *mut c_char, len: usize) -> usize {
    guard(0, || {
        let state = (*handle).0.shared().state.lock().unwrap().clone();
        write_str(&serde_json::to_string(&state).unwrap_or_default(), buf, len)
    })
}

/// Stops capture, closes the window and frees the handle. Blocks until the capture thread exits.
///
/// # Safety
/// `handle` must come from `tracker_start` and not be used afterwards. Null is ignored.
#[no_mangle]
pub unsafe extern "C" fn tracker_stop(handle: *mut TrackerHandle) {
    if !handle.is_null() {
        guard((), || drop(Box::from_raw(handle)));
    }
}
