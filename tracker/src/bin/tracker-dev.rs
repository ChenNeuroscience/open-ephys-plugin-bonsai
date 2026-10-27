//! Runs capture, pipeline and window standalone, with the GUI on the main thread (needed on macOS).
//! Samples are drained and summarised on stdout in place of Open Ephys.
//!
//!     cargo run --release --bin tracker-dev                 # fake camera (run tools/fake_camera.py)
//!     cargo run --release --bin tracker-dev -- synthetic    # built-in test scene
//!     cargo run --release --bin tracker-dev -- '<json>'     # full Config as JSON
//!
//! The window's state (crop, rulers, threshold...) is kept in `tracker-dev.state.json` in the working
//! directory, standing in for the plugin settings that Open Ephys saves.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tracker::config::{CameraConfig, Config, TrackerState};
use tracker::pipeline::TrackerSample;
use tracker::tracker::Tracker;

const STATE_FILE: &str = "tracker-dev.state.json";

fn main() {
    let mut config = match std::env::args().nth(1).as_deref() {
        None => Config::default(),
        Some("synthetic") => Config {
            camera: CameraConfig::Synthetic { width: 1024, height: 768, fps: 50.0 },
            ..Default::default()
        },
        Some(json) => serde_json::from_str(json).expect("config JSON"),
    };
    if let Ok(saved) = std::fs::read_to_string(STATE_FILE) {
        match serde_json::from_str::<TrackerState>(&saved) {
            Ok(state) => config.state = state,
            Err(e) => eprintln!("ignoring {STATE_FILE}: {e}"),
        }
    }
    let tracker = Arc::new(Tracker::start(Config { show_window: false, ..config }));

    let drain = tracker.clone();
    std::thread::spawn(move || {
        let mut buf = [TrackerSample { timestamp: 0.0, values: [0.0; 6] }; 64];
        let (mut count, mut last, mut report_at) = (0, None, Instant::now() + Duration::from_secs(1));
        loop {
            let Some(n) = drain.poll(&mut buf, Duration::from_millis(100)) else { break };
            count += n;
            if n > 0 {
                last = Some(buf[n - 1]);
            }
            if Instant::now() >= report_at {
                println!("{count} samples/s, last: {last:?}");
                count = 0;
                report_at += Duration::from_secs(1);
            }
        }
    });

    tracker::gui::run(tracker.shared().clone(), tracker::gui::default_options()).expect("window");

    let state = tracker.shared().state.lock().unwrap().clone();
    match std::fs::write(STATE_FILE, serde_json::to_string_pretty(&state).unwrap()) {
        Ok(()) => println!("saved {STATE_FILE}"),
        Err(e) => eprintln!("couldn't save {STATE_FILE}: {e}"),
    }
}
