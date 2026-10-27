//! Configuration, passed across the FFI boundary as JSON so the C ABI doesn't change as knobs are added.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub camera: CameraConfig,
    pub pipeline: PipelineConfig,
    /// Open the live-view window (ignored where an in-process window isn't supported, i.e. macOS).
    pub show_window: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraConfig {
    /// In-process test scene with two moving LEDs.
    Synthetic { width: u32, height: u32, fps: f64 },
    /// Raw frames streamed over TCP by `tools/fake_camera.py`.
    Fake { address: String },
}

impl Default for CameraConfig {
    fn default() -> Self {
        CameraConfig::Fake { address: "127.0.0.1:27030".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineConfig {
    /// Pixels strictly brighter than this are foreground.
    pub threshold: u8,
    pub min_area: u32,
    /// `None` means unbounded.
    pub max_area: Option<u32>,
    /// Only pixels inside this polygon (image coordinates) are considered. `None` means the whole frame.
    pub mask_polygon: Option<Vec<[i32; 2]>>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        PipelineConfig { threshold: 200, min_area: 5, max_area: None, mask_polygon: None }
    }
}
