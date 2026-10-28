//! Camera backends. Each produces 8-bit grayscale frames tagged with the camera's own timestamp.

mod fake;
mod spinnaker;
mod synthetic;

use std::io;

use image::GrayImage;

use crate::config::{CameraConfig, CameraSettings};

pub use fake::FakeCamera;
pub use spinnaker::SpinnakerCamera;
pub use synthetic::SyntheticCamera;

pub struct Frame {
    /// Monotonic frame counter as reported by the camera (gaps imply dropped frames).
    pub id: u64,
    /// Seconds, in the camera's clock.
    pub timestamp: f64,
    pub image: GrayImage,
}

pub trait Camera: Send {
    /// Waits briefly for the next frame. `Ok(None)` means nothing arrived yet; the caller
    /// should check whether it has been asked to stop and then call again.
    fn next_frame(&mut self) -> io::Result<Option<Frame>>;

    /// Shown in the status line, e.g. the camera model and serial number.
    fn description(&self) -> String {
        String::new()
    }

    /// Applies exposure and gain (the frame rate is only set when the camera is opened).
    fn apply(&mut self, _settings: &CameraSettings) -> io::Result<()> {
        Ok(())
    }

    /// Which settings this camera supports, their ranges and current values.
    fn controls(&self) -> CameraControls {
        CameraControls::default()
    }
}

/// Supported settings, as reported by the camera. `None` means not supported.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraControls {
    pub frame_rate: Option<Range>,
    pub exposure: Option<Range>,
    pub gain: Option<Range>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    /// What the camera reports now, which may differ from what was asked (e.g. under auto exposure).
    pub current: f64,
}

pub fn open(config: &CameraConfig, frame_rate: f64) -> io::Result<Box<dyn Camera>> {
    Ok(match config {
        CameraConfig::Synthetic { width, height } => Box::new(SyntheticCamera::new(*width, *height, frame_rate)),
        // The fake camera's rate is set by tools/fake_camera.py --fps.
        CameraConfig::Fake { address } => Box::new(FakeCamera::connect(address)?),
        CameraConfig::Spinnaker { serial, library } => Box::new(SpinnakerCamera::open(
            serial.as_deref().filter(|s| !s.trim().is_empty()),
            frame_rate,
            library.as_deref(),
        )?),
    })
}
