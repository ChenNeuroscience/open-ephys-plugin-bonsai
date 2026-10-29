//! End-to-end: tools/fake_camera.py → FakeCamera → pipeline → samples. Needs python3 on PATH.
//!     cargo test --release -- --ignored

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tracker::config::{CameraConfig, Config, Crop};
use tracker::pipeline::TrackerSample;
use tracker::tracker::Tracker;

#[test]
#[ignore]
fn fake_camera_end_to_end() {
    let port = 27399;
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/fake_camera.py");
    let mut child = Command::new("python3")
        .args([script, "--port", &port.to_string(), "--fps", "50"])
        .stdout(Stdio::null())
        .spawn()
        .expect("run python3");

    let mut config = Config { camera: CameraConfig::Fake { address: format!("127.0.0.1:{port}") }, ..Default::default() };
    // Exclude the glint on the room floor to the right of the arena, as the crop would on the rig.
    config.state.pipeline.crop = Crop::Square { center: [320.0, 240.0], size: 450.0, angle: 0.0 };
    let tracker = Tracker::start(config);

    let mut buf = [TrackerSample { timestamp: 0.0, values: [0.0; 6] }; 64];
    let mut samples = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while samples.len() < 100 && Instant::now() < deadline {
        let n = tracker.poll(&mut buf, Duration::from_millis(100)).expect("capture thread alive");
        samples.extend_from_slice(&buf[..n]);
    }
    let stats = tracker.shared().stats.lock().unwrap().clone();
    drop(tracker);
    child.kill().ok();

    assert!(samples.len() >= 100, "only got {} samples; stats {stats:?}", samples.len());
    let span = samples.last().unwrap().timestamp - samples[0].timestamp;
    assert!((1.5..2.5).contains(&span), "100 samples at 50fps should span ~2s, got {span}");
    let with_led = samples.iter().filter(|s| s.values[4] > 15.0 && s.values[4] < 60.0).count();
    assert!(with_led > 80, "big LED (r~3, ~30px) should be seen in most frames; got {with_led}/100");
    let in_arena = samples.iter().filter(|s| s.values[0] > 90.0 && s.values[0] < 550.0).count();
    assert_eq!(in_arena, with_led, "the glint outside the arena should be cropped out");
    println!("{stats:?}\nfirst: {:?}", samples[0]);
}
