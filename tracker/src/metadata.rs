//! Summary of the tracker state for the Open Ephys data stream's metadata, so record engines can put the
//! arena window and calibration into recording headers. Fixed-size numbers only: the plugin creates the
//! metadata fields once per signal chain update and overwrites their values as the state changes.

use crate::config::{Crop, TrackerState};

/// Size of the crop description buffer, including its terminating NUL.
pub const CROP_LEN: usize = 128;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamMetadata {
    /// Mean of the calibration rulers; NaN if none has a length.
    pub pixels_per_metre: f64,
    /// Bounding box of the crop in frame pixels, clipped to the frame: min_x, min_y, max_x, max_y.
    /// The whole frame without a crop; all zero if the frame size isn't known yet and there's no crop.
    pub window: [i32; 4],
    /// The crop as `shape(field,...)` followed by the space-separated values, e.g.
    /// `circle(cx,cy,r) 320 240 200`; see [`crop_description`]. NUL-terminated and zero-padded.
    pub crop: [u8; CROP_LEN],
    /// Frame width and height in pixels; zero if not known yet.
    pub frame_size: [i32; 2],
}

pub fn stream_metadata(state: &TrackerState) -> StreamMetadata {
    let crop = state.pipeline.crop;
    let [fw, fh] = state.frame_size.map_or([0, 0], |[w, h]| [w as i32, h as i32]);

    let window = match crop.bounds() {
        None => [0, 0, fw, fh],
        Some([x0, y0, x1, y1]) => {
            let (x0, y0, x1, y1) = (x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32);
            if fw > 0 && fh > 0 {
                [x0.clamp(0, fw), y0.clamp(0, fh), x1.clamp(0, fw), y1.clamp(0, fh)]
            } else {
                [x0, y0, x1, y1]
            }
        }
    };

    let mut crop_buf = [0u8; CROP_LEN];
    let text = crop_description(&crop);
    let n = text.len().min(CROP_LEN - 1);
    crop_buf[..n].copy_from_slice(&text.as_bytes()[..n]);

    StreamMetadata {
        pixels_per_metre: state.calibration.pixels_per_meter().map_or(f64::NAN, |p| p as f64),
        window,
        crop: crop_buf,
        frame_size: [fw, fh],
    }
}

/// The crop as its format followed by space-separated values, so it can't be misread. Positions and sizes
/// are in frame pixels; angles in degrees, clockwise on screen (y points down).
pub fn crop_description(crop: &Crop) -> String {
    match *crop {
        Crop::None => "none()".into(),
        Crop::Circle { center: [cx, cy], radius } => format!("circle(cx,cy,r) {cx} {cy} {radius}"),
        Crop::Square { center: [cx, cy], size, angle } => {
            format!("square(cx,cy,w,deg) {cx} {cy} {size} {}", angle.to_degrees())
        }
        Crop::Rectangle { center: [cx, cy], width, height, angle } => {
            format!("rectangle(cx,cy,w,h,deg) {cx} {cy} {width} {height} {}", angle.to_degrees())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Ruler;

    #[test]
    fn rotated_square_window_is_its_bounding_box_clipped_to_the_frame() {
        let mut state = TrackerState { frame_size: Some([640, 480]), ..Default::default() };
        state.pipeline.crop = Crop::Square { center: [320.0, 240.0], size: 400.0, angle: std::f32::consts::FRAC_PI_4 };
        state.calibration.rulers = vec![Ruler { a: [0.0, 0.0], b: [363.0, 0.0], length_cm: 100.0 }];
        let m = stream_metadata(&state);
        // A 400 px square at 45 degrees spans 400*sqrt(2) = 565.7 px each way.
        assert_eq!(m.window, [37, 0, 603, 480]);
        let text = std::str::from_utf8(&m.crop).unwrap().trim_end_matches('\0');
        assert_eq!(text, "square(cx,cy,w,deg) 320 240 400 45");
        assert_eq!(m.pixels_per_metre, 363.0);
        assert_eq!(m.frame_size, [640, 480]);
    }

    #[test]
    fn no_crop_is_the_whole_frame() {
        let m = stream_metadata(&TrackerState { frame_size: Some([1440, 1080]), ..Default::default() });
        assert_eq!(m.window, [0, 0, 1440, 1080]);
        assert!(m.crop.starts_with(b"none()\0"));
        assert!(m.pixels_per_metre.is_nan());
    }
}
