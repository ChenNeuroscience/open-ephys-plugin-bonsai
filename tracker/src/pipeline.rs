//! Mirrors the Bonsai workflow: Grayscale → CropPolygon → Threshold → FindContours →
//! BinaryRegionAnalysis → SortBinaryRegions, reduced to the two largest blobs.

use image::{GrayImage, Luma};
use imageproc::drawing::draw_polygon_mut;
use imageproc::point::Point;
use imageproc::region_labelling::{connected_components, Connectivity};

use crate::config::PipelineConfig;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub x: f32,
    pub y: f32,
    pub area: u32,
}

/// One row of output for Open Ephys. Layout matches the old Bonsai/OSC message.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TrackerSample {
    /// Camera timestamp, seconds.
    pub timestamp: f64,
    /// x1, y1, x2, y2, area1, area2. Missing blobs have NaN coordinates and zero area.
    pub values: [f32; 6],
}

impl TrackerSample {
    pub fn new(timestamp: f64, blobs: &[Blob]) -> Self {
        let mut values = [f32::NAN, f32::NAN, f32::NAN, f32::NAN, 0.0, 0.0];
        for (i, b) in blobs.iter().take(2).enumerate() {
            values[i * 2] = b.x;
            values[i * 2 + 1] = b.y;
            values[4 + i] = b.area as f32;
        }
        TrackerSample { timestamp, values }
    }
}

#[derive(Default)]
pub struct Pipeline {
    /// Cached polygon mask, rebuilt when the polygon or frame size changes.
    mask: Option<(Vec<[i32; 2]>, GrayImage)>,
}

#[derive(Default, Clone, Copy)]
struct Accum {
    area: u32,
    sum_x: u64,
    sum_y: u64,
}

impl Pipeline {
    /// Returns blobs passing the area filter, largest first.
    pub fn process(&mut self, image: &GrayImage, config: &PipelineConfig) -> Vec<Blob> {
        let mask = self.mask_for(image, config);
        let binary = GrayImage::from_fn(image.width(), image.height(), |x, y| {
            let inside = mask.map_or(true, |m| m.get_pixel(x, y)[0] != 0);
            Luma([if inside && image.get_pixel(x, y)[0] > config.threshold { 255 } else { 0 }])
        });

        let labels = connected_components(&binary, Connectivity::Eight, Luma([0]));
        let mut accums: Vec<Accum> = Vec::new();
        for (x, y, label) in labels.enumerate_pixels() {
            let label = label[0] as usize;
            if label == 0 {
                continue;
            }
            if label > accums.len() {
                accums.resize(label, Accum::default());
            }
            let a = &mut accums[label - 1];
            a.area += 1;
            a.sum_x += x as u64;
            a.sum_y += y as u64;
        }

        let max_area = config.max_area.unwrap_or(u32::MAX);
        let mut blobs: Vec<Blob> = accums
            .iter()
            .filter(|a| a.area >= config.min_area && a.area <= max_area)
            .map(|a| Blob {
                x: (a.sum_x as f64 / a.area as f64) as f32,
                y: (a.sum_y as f64 / a.area as f64) as f32,
                area: a.area,
            })
            .collect();
        blobs.sort_by(|a, b| b.area.cmp(&a.area));
        blobs
    }

    fn mask_for(&mut self, image: &GrayImage, config: &PipelineConfig) -> Option<&GrayImage> {
        let Some(polygon) = config.mask_polygon.as_ref().filter(|p| p.len() >= 3) else {
            self.mask = None;
            return None;
        };
        let stale = match &self.mask {
            Some((p, m)) => p != polygon || m.dimensions() != image.dimensions(),
            None => true,
        };
        if stale {
            let mut m = GrayImage::new(image.width(), image.height());
            let mut points: Vec<Point<i32>> = polygon.iter().map(|&[x, y]| Point::new(x, y)).collect();
            // draw_polygon_mut closes the polygon itself and panics if first == last.
            if points.first() == points.last() {
                points.pop();
            }
            draw_polygon_mut(&mut m, &points, Luma([255]));
            self.mask = Some((polygon.clone(), m));
        }
        self.mask.as_ref().map(|(_, m)| m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imageproc::drawing::draw_filled_rect_mut;
    use imageproc::rect::Rect;

    fn scene() -> GrayImage {
        let mut img = GrayImage::from_pixel(100, 80, Luma([10]));
        draw_filled_rect_mut(&mut img, Rect::at(10, 10).of_size(4, 4), Luma([255])); // area 16, centre (11.5, 11.5)
        draw_filled_rect_mut(&mut img, Rect::at(60, 40).of_size(6, 5), Luma([255])); // area 30, centre (62.5, 42)
        draw_filled_rect_mut(&mut img, Rect::at(90, 70).of_size(1, 2), Luma([255])); // area 2, below min_area
        img
    }

    #[test]
    fn finds_blobs_largest_first() {
        let blobs = Pipeline::default().process(&scene(), &PipelineConfig::default());
        assert_eq!(
            blobs,
            vec![Blob { x: 62.5, y: 42.0, area: 30 }, Blob { x: 11.5, y: 11.5, area: 16 }]
        );
    }

    #[test]
    fn mask_excludes_outside_pixels() {
        let config = PipelineConfig {
            mask_polygon: Some(vec![[0, 0], [40, 0], [40, 40], [0, 40]]),
            ..Default::default()
        };
        let blobs = Pipeline::default().process(&scene(), &config);
        assert_eq!(blobs, vec![Blob { x: 11.5, y: 11.5, area: 16 }]);
    }

    #[test]
    fn sample_fills_missing_with_nan() {
        let s = TrackerSample::new(1.5, &[Blob { x: 1.0, y: 2.0, area: 9 }]);
        assert_eq!(s.values[..2], [1.0, 2.0]);
        assert!(s.values[2].is_nan() && s.values[3].is_nan());
        assert_eq!(s.values[4..], [9.0, 0.0]);
    }
}
