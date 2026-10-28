//! The capture thread, plus the state it shares with the GUI and the Open Ephys side.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};

use crate::camera::{self, CameraControls, Frame};
use crate::config::{CameraConfig, Config, TrackerState};
use crate::pipeline::{Blob, Pipeline, TrackerSample};

/// Enough for tens of seconds at typical frame rates if Open Ephys stalls.
const SAMPLE_QUEUE_LEN: usize = 4096;

/// The most recent processed frame, for display.
pub struct View {
    pub frame: Frame,
    pub blobs: Vec<Blob>,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub frames: u64,
    /// Inferred from gaps in camera frame ids.
    pub camera_dropped: u64,
    /// Samples discarded because Open Ephys wasn't draining the queue.
    pub queue_dropped: u64,
    pub fps: f32,
    pub process_ms: f32,
}

#[derive(Default)]
pub struct Shared {
    /// Written by the GUI; the pipeline part is read by the capture thread every frame.
    pub state: Mutex<TrackerState>,
    pub latest: Mutex<Option<Arc<View>>>,
    pub status: Mutex<String>,
    pub stats: Mutex<Stats>,
    pub stop: AtomicBool,
    /// The frame rate this acquisition runs at (the stream's declared sample rate). Changing the frame rate
    /// in the state takes effect from the next acquisition.
    pub active_frame_rate: f64,
    /// Open Ephys is recording: the window locks all settings.
    pub recording: AtomicBool,
    /// The tracker window is open, or has been asked to open.
    pub window_open: AtomicBool,
    /// Size of the latest frame.
    pub frame_size: Mutex<Option<[u32; 2]>>,
    /// What the open camera supports, refreshed after settings are applied.
    pub controls: Mutex<CameraControls>,
    /// Set by the GUI so the capture thread can wake it when a new frame arrives.
    pub repaint: Mutex<Option<eframe::egui::Context>>,
}

impl Shared {
    /// The state to save and to describe in stream metadata: the window's settings plus the frame size
    /// last seen by the capture thread.
    pub fn export_state(&self) -> TrackerState {
        let mut state = self.state.lock().unwrap().clone();
        if let Some(size) = *self.frame_size.lock().unwrap() {
            state.frame_size = Some(size);
        }
        state
    }

    pub fn set_status(&self, status: impl Into<String>) {
        *self.status.lock().unwrap() = status.into();
    }

    pub fn request_repaint(&self) {
        if let Some(ctx) = self.repaint.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }
}

pub struct Tracker {
    shared: Arc<Shared>,
    samples: Receiver<TrackerSample>,
    thread: Option<JoinHandle<()>>,
}

impl Tracker {
    pub fn start(config: Config) -> Tracker {
        let shared = Arc::new(Shared {
            active_frame_rate: config.state.camera.frame_rate,
            state: Mutex::new(config.state),
            ..Default::default()
        });
        let (tx, rx) = crossbeam_channel::bounded(SAMPLE_QUEUE_LEN);
        let thread_shared = shared.clone();
        let camera_config = config.camera;
        let thread = std::thread::Builder::new()
            .name("tracker-capture".into())
            .spawn(move || capture_loop(&thread_shared, &camera_config, &tx))
            .expect("spawn capture thread");

        let tracker = Tracker { shared, samples: rx, thread: Some(thread) };
        if config.show_window {
            if let Err(e) = tracker.open_window() {
                tracker.shared.set_status(format!("no window: {e}"));
            }
        }
        tracker
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Opens the tracker window (e.g. after the user closed it), or brings it to the front.
    pub fn open_window(&self) -> Result<(), String> {
        crate::window::open(self.shared.clone())
    }

    /// Blocks up to `timeout` for the first sample, then takes whatever else is queued.
    /// Returns `None` if the capture thread has exited (e.g. it panicked).
    pub fn poll(&self, out: &mut [TrackerSample], timeout: Duration) -> Option<usize> {
        if out.is_empty() {
            return Some(0);
        }
        match self.samples.recv_timeout(timeout) {
            Ok(s) => out[0] = s,
            Err(RecvTimeoutError::Timeout) => return Some(0),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
        let mut n = 1;
        while n < out.len() {
            match self.samples.try_recv() {
                Ok(s) => {
                    out[n] = s;
                    n += 1;
                }
                Err(_) => break,
            }
        }
        Some(n)
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.request_repaint(); // lets the window notice and close
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn capture_loop(shared: &Shared, camera_config: &CameraConfig, tx: &Sender<TrackerSample>) {
    let stopping = || shared.stop.load(Ordering::Relaxed);
    let mut pipeline = Pipeline::default();

    while !stopping() {
        let mut camera = match camera::open(camera_config, shared.active_frame_rate) {
            Ok(c) => c,
            Err(e) => {
                shared.set_status(format!("waiting for camera: {e}"));
                let retry_at = Instant::now() + Duration::from_millis(500);
                while !stopping() && Instant::now() < retry_at {
                    std::thread::sleep(Duration::from_millis(20));
                }
                continue;
            }
        };
        let description = camera.description();
        shared.set_status(if description.is_empty() { "running".to_string() } else { format!("running: {description}") });
        *shared.controls.lock().unwrap() = camera.controls();
        let mut last: Option<(u64, f64)> = None;
        let mut applied = None;

        while !stopping() {
            let settings = shared.state.lock().unwrap().camera;
            if applied != Some(settings) {
                if let Err(e) = camera.apply(&settings) {
                    shared.set_status(format!("running: {description} [couldn't apply camera settings: {e}]"));
                }
                *shared.controls.lock().unwrap() = camera.controls();
                applied = Some(settings);
            }

            let frame = match camera.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => continue,
                Err(e) => {
                    shared.set_status(format!("camera error, reconnecting: {e}"));
                    break;
                }
            };

            let t0 = Instant::now();
            let config = shared.state.lock().unwrap().pipeline.clone();
            let blobs = pipeline.process(&frame.image, &config);
            let process_ms = t0.elapsed().as_secs_f32() * 1e3;

            let queue_full = match tx.try_send(TrackerSample::new(frame.timestamp, &blobs)) {
                Ok(()) => false,
                Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => return,
            };

            {
                let mut stats = shared.stats.lock().unwrap();
                stats.frames += 1;
                stats.queue_dropped += queue_full as u64;
                stats.process_ms = 0.9 * stats.process_ms + 0.1 * process_ms;
                if let Some((last_id, last_ts)) = last {
                    stats.camera_dropped += frame.id.saturating_sub(last_id + 1);
                    let dt = (frame.timestamp - last_ts) as f32;
                    if dt > 0.0 {
                        stats.fps = if stats.fps == 0.0 { 1.0 / dt } else { 0.9 * stats.fps + 0.1 / dt };
                    }
                }
            }
            last = Some((frame.id, frame.timestamp));

            *shared.frame_size.lock().unwrap() = Some([frame.image.width(), frame.image.height()]);
            *shared.latest.lock().unwrap() = Some(Arc::new(View { frame, blobs }));
            shared.request_repaint();
        }
    }
}
