//! Configuration, passed across the FFI boundary as JSON so the C ABI doesn't change as knobs are added.
//!
//! Image coordinates are continuous pixels: pixel (i, j) covers [i, i+1) x [j, j+1).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub camera: CameraConfig,
    /// Open the live-view window (ignored where an in-process window isn't supported, i.e. macOS).
    pub show_window: bool,
    pub state: TrackerState,
}

/// Everything adjustable from the tracker window. Round-trips through the host so it persists between runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackerState {
    /// Width and height of the last frames seen, so the stream metadata can be filled in before the camera
    /// is opened. Kept up to date by the capture thread (see `Shared::export_state`), not the window.
    pub frame_size: Option<[u32; 2]>,
    pub camera: CameraSettings,
    pub camera_locked: bool,
    pub pipeline: PipelineConfig,
    /// Visible: above-threshold pixels are highlighted and detected regions circled. Locked: settings can't be changed.
    pub detection_overlay: Overlay,
    /// Hidden: the outside of the crop is masked out. Visible: the whole frame is shown with the crop outline.
    pub crop_overlay: Overlay,
    pub calibration: Calibration,
}

/// Display state of something drawn over the video.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    pub visible: bool,
    /// Locked overlays are drawn but can't be dragged.
    pub locked: bool,
}

impl Default for Overlay {
    fn default() -> Self {
        Overlay { visible: true, locked: false }
    }
}

impl Overlay {
    pub fn editable(&self) -> bool {
        self.visible && !self.locked
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraConfig {
    /// In-process test scene with two moving LEDs.
    Synthetic { width: u32, height: u32 },
    /// Raw frames streamed over TCP by `tools/fake_camera.py`.
    Fake { address: String },
    /// A Teledyne FLIR camera via the Spinnaker SDK.
    Spinnaker {
        /// `None` (or blank) for the first camera found.
        #[serde(default)]
        serial: Option<String>,
        /// Path to the Spinnaker C library, if it isn't found automatically.
        #[serde(default)]
        library: Option<String>,
    },
}

impl Default for CameraConfig {
    fn default() -> Self {
        CameraConfig::Fake { address: "127.0.0.1:27030".into() }
    }
}

/// Camera settings adjustable from the tracker window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    /// Frames per second. Fixed for the length of an acquisition, because Open Ephys declares it as the
    /// stream's sample rate before acquisition starts; set on the camera where it allows that.
    pub frame_rate: f64,
    /// Microseconds.
    pub exposure: Control,
    /// Decibels.
    pub gain: Control,
}

impl Default for CameraSettings {
    fn default() -> Self {
        CameraSettings { frame_rate: 50.0, exposure: Control::Unchanged, gain: Control::Unchanged }
    }
}

/// How to set a camera feature that can be automatic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
pub enum Control {
    /// Leave it as the camera is configured (e.g. in SpinView).
    #[default]
    Unchanged,
    Auto,
    Manual(f64),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelineConfig {
    /// Pixels strictly brighter than this are foreground.
    pub threshold: u8,
    pub min_area: u32,
    /// `None` means unbounded.
    pub max_area: Option<u32>,
    /// Only pixels inside this region are considered.
    pub crop: Crop,
    /// Reduced gain near the crop boundary, to suppress LED reflections off the walls.
    pub edge: EdgeGain,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        PipelineConfig { threshold: 200, min_area: 5, max_area: None, crop: Crop::None, edge: EdgeGain::default() }
    }
}

impl PipelineConfig {
    /// The threshold for pixels within the edge band: a pixel counts if its value, scaled down by the edge
    /// gain, is still above `threshold`. 255 means nothing there can count.
    pub fn edge_threshold(&self) -> u8 {
        if self.edge.width <= 0.0 || self.crop == Crop::None {
            return self.threshold;
        }
        let gain = 10f32.powf(-self.edge.reduction_db.max(0.0) / 20.0);
        (self.threshold as f32 / gain).floor().min(255.0) as u8
    }
}

/// Pixels within `width` of the crop boundary have their gain reduced by `reduction_db`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeGain {
    /// Pixels; 0 turns it off.
    pub width: f32,
    pub reduction_db: f32,
}

impl Default for EdgeGain {
    fn default() -> Self {
        EdgeGain { width: 0.0, reduction_db: 6.0 }
    }
}

/// Region of interest. Angles are radians, clockwise on screen (since y points down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Crop {
    #[default]
    None,
    Circle { center: [f32; 2], radius: f32 },
    Square { center: [f32; 2], size: f32, angle: f32 },
    Rectangle { center: [f32; 2], width: f32, height: f32, angle: f32 },
}

impl Crop {
    pub fn center(&self) -> Option<[f32; 2]> {
        match *self {
            Crop::None => None,
            Crop::Circle { center, .. } | Crop::Square { center, .. } | Crop::Rectangle { center, .. } => Some(center),
        }
    }

    pub fn angle(&self) -> f32 {
        match *self {
            Crop::Square { angle, .. } | Crop::Rectangle { angle, .. } => angle,
            _ => 0.0,
        }
    }

    /// Half width and half height in the shape's own (rotated) frame; for a circle, the radius twice.
    pub fn half_extents(&self) -> Option<[f32; 2]> {
        match *self {
            Crop::None => None,
            Crop::Circle { radius, .. } => Some([radius, radius]),
            Crop::Square { size, .. } => Some([size / 2.0, size / 2.0]),
            Crop::Rectangle { width, height, .. } => Some([width / 2.0, height / 2.0]),
        }
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        let (Some(center), Some([hw, hh])) = (self.center(), self.half_extents()) else { return true };
        let [x, y] = to_local(center, self.angle(), p);
        match self {
            Crop::Circle { .. } => x * x + y * y <= hw * hw,
            _ => x.abs() <= hw && y.abs() <= hh,
        }
    }

    /// Axis-aligned bounding box as [min_x, min_y, max_x, max_y]; `None` for no crop.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let (center, [hw, hh]) = (self.center()?, self.half_extents()?);
        if let Crop::Circle { .. } = self {
            return Some([center[0] - hw, center[1] - hh, center[0] + hw, center[1] + hh]);
        }
        let corners = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].map(|l| from_local(center, self.angle(), l));
        let (xs, ys) = (corners.map(|c| c[0]), corners.map(|c| c[1]));
        let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
        let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
        Some([min(xs), min(ys), max(xs), max(ys)])
    }

    /// The points at least `d` inside the boundary, which for these shapes is the same shape shrunk by `d`.
    /// `None` if nothing is left (or there's no crop).
    pub fn inset(&self, d: f32) -> Option<Crop> {
        match *self {
            Crop::None => None,
            Crop::Circle { center, radius } => (radius > d).then_some(Crop::Circle { center, radius: radius - d }),
            Crop::Square { center, size, angle } => {
                (size > 2.0 * d).then_some(Crop::Square { center, size: size - 2.0 * d, angle })
            }
            Crop::Rectangle { center, width, height, angle } => (width > 2.0 * d && height > 2.0 * d)
                .then_some(Crop::Rectangle { center, width: width - 2.0 * d, height: height - 2.0 * d, angle }),
        }
    }

    pub fn set_center(&mut self, c: [f32; 2]) {
        match self {
            Crop::None => {}
            Crop::Circle { center, .. } | Crop::Square { center, .. } | Crop::Rectangle { center, .. } => *center = c,
        }
    }
}

/// Rotates `p` about `center` by -angle, giving coordinates in a shape's own frame.
pub fn to_local(center: [f32; 2], angle: f32, p: [f32; 2]) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    let (dx, dy) = (p[0] - center[0], p[1] - center[1]);
    [dx * c + dy * s, -dx * s + dy * c]
}

/// Inverse of [`to_local`].
pub fn from_local(center: [f32; 2], angle: f32, l: [f32; 2]) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    [center[0] + l[0] * c - l[1] * s, center[1] + l[0] * s + l[1] * c]
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calibration {
    pub rulers: Vec<Ruler>,
    /// Applies to all rulers. Hidden rulers still count towards [`Calibration::pixels_per_meter`].
    pub overlay: Overlay,
}

impl Calibration {
    /// Mean over rulers that have a physical length entered.
    pub fn pixels_per_meter(&self) -> Option<f32> {
        let values: Vec<f32> = self.rulers.iter().filter_map(Ruler::pixels_per_meter).collect();
        (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ruler {
    pub a: [f32; 2],
    pub b: [f32; 2],
    /// Physical length; 0 means not entered yet.
    pub length_cm: f32,
}

impl Ruler {
    pub fn pixels(&self) -> f32 {
        (self.b[0] - self.a[0]).hypot(self.b[1] - self.a[1])
    }

    pub fn pixels_per_meter(&self) -> Option<f32> {
        (self.length_cm > 0.0).then(|| self.pixels() / (self.length_cm / 100.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_rectangle_contains() {
        let crop = Crop::Rectangle { center: [50.0, 50.0], width: 40.0, height: 10.0, angle: std::f32::consts::FRAC_PI_2 };
        // Rotated by 90 degrees the long side runs vertically.
        assert!(crop.contains([50.0, 68.0]));
        assert!(!crop.contains([68.0, 50.0]));
    }

    #[test]
    fn local_round_trip() {
        let (c, a, p) = ([3.0, -2.0], 0.7, [10.0, 4.0]);
        let back = from_local(c, a, to_local(c, a, p));
        assert!((back[0] - p[0]).abs() < 1e-4 && (back[1] - p[1]).abs() < 1e-4);
    }

    #[test]
    fn pixels_per_meter_averages_measured_rulers() {
        let calibration = Calibration {
            rulers: vec![
                Ruler { a: [0.0, 0.0], b: [300.0, 0.0], length_cm: 100.0 },
                Ruler { a: [0.0, 0.0], b: [0.0, 160.0], length_cm: 50.0 },
                Ruler { a: [0.0, 0.0], b: [10.0, 0.0], length_cm: 0.0 }, // not measured: ignored
            ],
            overlay: Overlay { visible: false, locked: true },
        };
        assert_eq!(calibration.pixels_per_meter(), Some(310.0));
    }
}
