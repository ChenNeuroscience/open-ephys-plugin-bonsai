//! Runs capture, pipeline and window standalone, with the GUI on the main thread (needed on macOS).
//! Samples are drained and summarised on stdout in place of Open Ephys.
//!
//!     cargo run --release --bin tracker-dev                 # fake camera (run tools/fake_camera.py)
//!     cargo run --release --bin tracker-dev -- synthetic    # built-in test scene
//!     cargo run --release --bin tracker-dev -- '<json>'     # full Config as JSON

use std::sync::Arc;
use std::time::{Duration, Instant};

use tracker::config::{CameraConfig, Config};
use tracker::pipeline::TrackerSample;
use tracker::tracker::Tracker;

fn main() {
    let config = match std::env::args().nth(1).as_deref() {
        None => Config::default(),
        Some("synthetic") => Config {
            camera: CameraConfig::Synthetic { width: 1024, height: 768, fps: 50.0 },
            ..Default::default()
        },
        Some(json) => serde_json::from_str(json).expect("config JSON"),
    };
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
}
