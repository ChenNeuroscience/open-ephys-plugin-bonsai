//! Camera backends. Each produces 8-bit grayscale frames tagged with the camera's own timestamp.

mod fake;
mod synthetic;

use std::io;

use image::GrayImage;

use crate::config::CameraConfig;

pub use fake::FakeCamera;
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
}

pub fn open(config: &CameraConfig) -> io::Result<Box<dyn Camera>> {
    Ok(match config {
        CameraConfig::Synthetic { width, height, fps } => {
            Box::new(SyntheticCamera::new(*width, *height, *fps))
        }
        CameraConfig::Fake { address } => Box::new(FakeCamera::connect(address)?),
    })
}
