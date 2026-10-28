//! Mirrors the original Bonsai workflow this replaced: Grayscale → Crop → Threshold → FindContours →
//! BinaryRegionAnalysis → SortBinaryRegions, reduced to the two largest blobs.

use std::sync::Arc;

use image::{GrayImage, Luma};

use crate::config::{Crop, PipelineConfig};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub x: f32,
    pub y: f32,
    pub area: u32,
}

/// One row of output for Open Ephys. Layout matches the original plugin's channels.
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

/// Finds bright regions without building per-frame images: each row of the crop is scanned for runs of
/// above-threshold pixels, and runs that touch (8-connected) across rows are merged with union-find while
/// their area and centroid sums are accumulated. Most rows contain no LED at all, and those are rejected
/// with a single (SIMD-friendly) max over the row.
#[derive(Default)]
pub struct Pipeline {
    spans: SpanCache,
    runs: Vec<Run>,
    /// Union-find parent of each run.
    parent: Vec<u32>,
}

/// A horizontal run of foreground pixels, carrying the totals of its region once merged.
#[derive(Clone, Copy)]
struct Run {
    x0: u32,
    /// Exclusive.
    x1: u32,
    area: u32,
    sum_x: u64,
    sum_y: u64,
}

/// The crop as one span of columns per row (crops are convex, so the inside of a row is contiguous),
/// rebuilt only when the crop or frame size changes.
#[derive(Default)]
struct SpanCache {
    key: Option<(Crop, u32, u32)>,
    /// Per row, start..end columns; empty if start >= end.
    spans: Vec<(u32, u32)>,
}

impl SpanCache {
    fn get(&mut self, crop: &Crop, width: u32, height: u32) -> &[(u32, u32)] {
        let key = (*crop, width, height);
        if self.key != Some(key) {
            self.spans = (0..height).map(|y| row_span(crop, y, width)).collect();
            self.key = Some(key);
        }
        &self.spans
    }
}

/// Rasterised crop (255 inside, 0 outside), rebuilt only when the crop or frame size changes.
#[derive(Default)]
pub struct MaskCache {
    key: Option<(Crop, u32, u32)>,
    mask: Option<Arc<GrayImage>>,
}

/// Columns of row `y` whose pixel centres are inside `crop`, as start..end (empty if start >= end).
/// Worked out from the shape rather than by testing every pixel, so it's cheap enough to redo on every
/// frame while a crop is being dragged; the ends are then checked with [`Crop::contains`] so they agree
/// with it exactly.
pub fn row_span(crop: &Crop, y: u32, width: u32) -> (u32, u32) {
    let yc = y as f32 + 0.5;
    let (lo, hi) = match *crop {
        Crop::None => return (0, width),
        Crop::Circle { center, radius } => {
            let dy = yc - center[1];
            if dy.abs() > radius {
                return (0, 0);
            }
            let dx = (radius * radius - dy * dy).sqrt();
            (center[0] - dx, center[0] + dx)
        }
        Crop::Square { .. } | Crop::Rectangle { .. } => {
            let (center, [hw, hh]) = (crop.center().unwrap(), crop.half_extents().unwrap());
            let (sin, cos) = crop.angle().sin_cos();
            let dy = yc - center[1];
            // In the shape's frame, local x = dx*cos + dy*sin and local y = -dx*sin + dy*cos (see to_local);
            // each must lie within its half extent, which bounds dx.
            let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
            for (slope, offset, half) in [(cos, dy * sin, hw), (-sin, dy * cos, hh)] {
                if slope.abs() < 1e-6 {
                    if offset.abs() > half {
                        return (0, 0);
                    }
                } else {
                    let (a, b) = ((-half - offset) / slope, (half - offset) / slope);
                    lo = lo.max(a.min(b));
                    hi = hi.min(a.max(b));
                }
            }
            (center[0] + lo, center[0] + hi)
        }
    };
    let inside = |x: u32| crop.contains([x as f32 + 0.5, yc]);
    let clamp = |v: f32| v.clamp(0.0, width as f32) as u32;
    let (mut start, mut end) = (clamp((lo - 0.5).ceil()), clamp((hi - 0.5).floor() + 1.0));
    // Float rounding can leave the ends a pixel out from the exact test; nudge them to agree.
    while start < end && !inside(start) {
        start += 1;
    }
    while start > 0 && start <= end && inside(start - 1) {
        start -= 1;
    }
    while end > start && !inside(end - 1) {
        end -= 1;
    }
    while end > start && end < width && inside(end) {
        end += 1;
    }
    if start < end { (start, end) } else { (0, 0) }
}

impl MaskCache {
    /// `None` when there's no crop, i.e. everything is inside.
    pub fn get(&mut self, crop: &Crop, width: u32, height: u32) -> Option<&GrayImage> {
        self.update(crop, width, height);
        self.mask.as_deref()
    }

    /// Like [`MaskCache::get`], but shareable; the same `Arc` is returned until the mask changes.
    pub fn get_shared(&mut self, crop: &Crop, width: u32, height: u32) -> Option<Arc<GrayImage>> {
        self.update(crop, width, height);
        self.mask.clone()
    }

    fn update(&mut self, crop: &Crop, width: u32, height: u32) {
        let key = (*crop, width, height);
        if self.key != Some(key) {
            self.mask = (*crop != Crop::None).then(|| {
                let mut mask = GrayImage::new(width, height);
                for (y, row) in mask.rows_mut().enumerate() {
                    let (start, end) = row_span(crop, y as u32, width);
                    for p in row.skip(start as usize).take(end.saturating_sub(start) as usize) {
                        *p = Luma([255]);
                    }
                }
                Arc::new(mask)
            });
            self.key = Some(key);
        }
    }
}

impl Pipeline {
    /// Returns blobs passing the area filter, largest first.
    pub fn process(&mut self, image: &GrayImage, config: &PipelineConfig) -> Vec<Blob> {
        let (width, height) = image.dimensions();
        let spans = self.spans.get(&config.crop, width, height);
        let raw = image.as_raw();
        let threshold = config.threshold;
        let (runs, parent) = (&mut self.runs, &mut self.parent);
        runs.clear();
        parent.clear();

        let mut prev_row = 0..0; // indices of the previous row's runs
        for (y, &(start, end)) in spans.iter().enumerate() {
            let first_run = runs.len();
            if start < end {
                let offset = y * width as usize;
                let row = &raw[offset + start as usize..offset + end as usize];
                if row.iter().fold(0u8, |m, &v| m.max(v)) > threshold {
                    let mut x = 0;
                    while x < row.len() {
                        if row[x] <= threshold {
                            x += 1;
                            continue;
                        }
                        let run_start = x;
                        while x < row.len() && row[x] > threshold {
                            x += 1;
                        }
                        let (x0, x1) = (start + run_start as u32, start + x as u32);
                        let len = (x1 - x0) as u64;
                        parent.push(runs.len() as u32);
                        runs.push(Run {
                            x0,
                            x1,
                            area: len as u32,
                            sum_x: len * (x0 as u64 + x1 as u64 - 1) / 2,
                            sum_y: len * y as u64,
                        });
                    }
                }
            }

            // Merge with runs in the previous row that overlap or touch diagonally. Both rows are sorted by x.
            let row_runs = first_run..runs.len();
            let mut j = prev_row.start;
            for i in row_runs.clone() {
                let (x0, x1) = (runs[i].x0, runs[i].x1);
                while j < prev_row.end && runs[j].x1 < x0 {
                    j += 1;
                }
                let mut k = j;
                while k < prev_row.end && runs[k].x0 <= x1 {
                    union(parent, k, i);
                    k += 1;
                }
            }
            prev_row = row_runs;
        }

        // Add each run's totals into its region's root (roots come first, as unions keep the lower index).
        for i in 0..runs.len() {
            let root = find(parent, i);
            if root != i {
                let run = runs[i];
                let r = &mut runs[root];
                r.area += run.area;
                r.sum_x += run.sum_x;
                r.sum_y += run.sum_y;
            }
        }

        let max_area = config.max_area.unwrap_or(u32::MAX);
        let mut blobs: Vec<Blob> = (0..runs.len())
            .filter(|&i| parent[i] as usize == i)
            .map(|i| runs[i])
            .filter(|r| r.area >= config.min_area && r.area <= max_area)
            .map(|r| Blob {
                x: (r.sum_x as f64 / r.area as f64) as f32,
                y: (r.sum_y as f64 / r.area as f64) as f32,
                area: r.area,
            })
            .collect();
        blobs.sort_by(|a, b| b.area.cmp(&a.area));
        blobs
    }
}

fn find(parent: &mut [u32], mut i: usize) -> usize {
    while parent[i] as usize != i {
        parent[i] = parent[parent[i] as usize]; // path halving
        i = parent[i] as usize;
    }
    i
}

fn union(parent: &mut [u32], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        let (lo, hi) = (ra.min(rb), ra.max(rb));
        parent[hi] = lo as u32;
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
            crop: Crop::Rectangle { center: [20.0, 20.0], width: 40.0, height: 40.0, angle: 0.0 },
            ..Default::default()
        };
        let blobs = Pipeline::default().process(&scene(), &config);
        assert_eq!(blobs, vec![Blob { x: 11.5, y: 11.5, area: 16 }]);
    }

    /// The previous, straightforward implementation (mask, label every pixel, accumulate), as a reference.
    fn reference_process(image: &GrayImage, config: &PipelineConfig) -> Vec<Blob> {
        use imageproc::region_labelling::{connected_components, Connectivity};
        let binary = GrayImage::from_fn(image.width(), image.height(), |x, y| {
            let inside = config.crop.contains([x as f32 + 0.5, y as f32 + 0.5]);
            Luma([if inside && image.get_pixel(x, y)[0] > config.threshold { 255 } else { 0 }])
        });
        let labels = connected_components(&binary, Connectivity::Eight, Luma([0]));
        let mut accums: Vec<(u32, u64, u64)> = Vec::new();
        for (x, y, label) in labels.enumerate_pixels() {
            let label = label[0] as usize;
            if label == 0 {
                continue;
            }
            if label > accums.len() {
                accums.resize(label, (0, 0, 0));
            }
            let a = &mut accums[label - 1];
            *a = (a.0 + 1, a.1 + x as u64, a.2 + y as u64);
        }
        let max_area = config.max_area.unwrap_or(u32::MAX);
        accums
            .iter()
            .filter(|a| a.0 >= config.min_area && a.0 <= max_area)
            .map(|a| Blob { x: (a.1 as f64 / a.0 as f64) as f32, y: (a.2 as f64 / a.0 as f64) as f32, area: a.0 })
            .collect()
    }

    fn sorted(mut blobs: Vec<Blob>) -> Vec<Blob> {
        blobs.sort_by(|a, b| (b.area, a.x, a.y).partial_cmp(&(a.area, b.x, b.y)).unwrap());
        blobs
    }

    #[test]
    fn matches_reference_on_random_images() {
        let mut seed = 7u64;
        let mut rand = move |n: u32| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) % n as u64) as u32
        };
        for case in 0..2000 {
            let (w, h) = (20 + rand(60), 20 + rand(60));
            // Sparse to dense foreground, so regions range from specks to large tangled shapes.
            let density = rand(100);
            let img = GrayImage::from_fn(w, h, |_, _| Luma([if rand(100) < density { 200 } else { 50 }]));
            let center = [rand(w) as f32, rand(h) as f32];
            let crop = match case % 4 {
                0 => Crop::None,
                1 => Crop::Circle { center, radius: 5.0 + rand(40) as f32 },
                2 => Crop::Square { center, size: 5.0 + rand(60) as f32, angle: rand(628) as f32 / 100.0 },
                _ => Crop::Rectangle {
                    center,
                    width: 5.0 + rand(60) as f32,
                    height: 5.0 + rand(60) as f32,
                    angle: rand(628) as f32 / 100.0,
                },
            };
            let config = PipelineConfig { threshold: 100, min_area: rand(3), max_area: None, crop };
            let ours = sorted(Pipeline::default().process(&img, &config));
            let reference = sorted(reference_process(&img, &config));
            assert_eq!(ours, reference, "case {case}: {w}x{h}, density {density}%, {crop:?}");
        }
    }

    #[test]
    fn row_spans_match_per_pixel_test() {
        let (w, h) = (1440u32, 1080u32);
        let crops = [
            Crop::Circle { center: [700.3, 512.8], radius: 431.7 },
            Crop::Circle { center: [10.0, 10.0], radius: 3.2 },
            Crop::Square { center: [720.0, 540.0], size: 900.0, angle: 0.0 },
            Crop::Square { center: [719.5, 540.5], size: 777.7, angle: std::f32::consts::FRAC_PI_4 },
            Crop::Rectangle { center: [1400.0, 100.0], width: 600.0, height: 90.0, angle: 1.3 },
            Crop::Rectangle { center: [720.0, 540.0], width: 2000.0, height: 400.0, angle: -0.05 },
        ];
        for crop in crops {
            for y in 0..h {
                let inside: Vec<u32> = (0..w).filter(|&x| crop.contains([x as f32 + 0.5, y as f32 + 0.5])).collect();
                let expected = match (inside.first(), inside.last()) {
                    (Some(&a), Some(&b)) => (a, b + 1),
                    _ => (0, 0),
                };
                assert_eq!(row_span(&crop, y, w), expected, "{crop:?}, row {y}");
            }
        }
    }

    #[test]
    fn sample_fills_missing_with_nan() {
        let s = TrackerSample::new(1.5, &[Blob { x: 1.0, y: 2.0, area: 9 }]);
        assert_eq!(s.values[..2], [1.0, 2.0]);
        assert!(s.values[2].is_nan() && s.values[3].is_nan());
        assert_eq!(s.values[4..], [9.0, 0.0]);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use imageproc::drawing::draw_filled_circle_mut;

    /// A 1440x1080 frame (a common Blackfly S resolution): noisy dark floor, a bright wall band, two LEDs.
    pub fn realistic_frame() -> GrayImage {
        let mut seed = 12345u32;
        let mut img = GrayImage::from_fn(1440, 1080, |x, y| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let wall = !(200..1240).contains(&x) || !(100..980).contains(&y);
            Luma([if wall { 140 } else { 12 } + (seed >> 29) as u8])
        });
        draw_filled_circle_mut(&mut img, (700, 500), 6, Luma([255]));
        draw_filled_circle_mut(&mut img, (720, 510), 4, Luma([245]));
        img
    }

    /// cargo test --release bench_process -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_process() {
        let img = realistic_frame();
        for (name, crop) in [
            ("no crop", Crop::None),
            ("square crop", Crop::Square { center: [720.0, 540.0], size: 900.0, angle: 0.3 }),
        ] {
            let config = PipelineConfig { crop, ..Default::default() };
            let mut pipeline = Pipeline::default();
            let blobs = pipeline.process(&img, &config); // warm up (builds the mask)
            let n = 50;
            let t = std::time::Instant::now();
            for _ in 0..n {
                std::hint::black_box(pipeline.process(&img, &config));
            }
            println!("{name}: {:.2} ms/frame, blobs {:?}", t.elapsed().as_secs_f64() * 1e3 / n as f64, &blobs[..blobs.len().min(2)]);
        }

        // Dragging a crop changes it every GUI frame: the pipeline's spans and the display mask are rebuilt.
        let n = 50;
        let t = std::time::Instant::now();
        for i in 0..n {
            let crop = Crop::Rectangle { center: [720.0 + i as f32, 540.0], width: 900.0, height: 700.0, angle: 0.3 };
            std::hint::black_box(Pipeline::default().process(&img, &PipelineConfig { crop, ..Default::default() }));
            std::hint::black_box(MaskCache::default().get(&crop, 1440, 1080).is_some());
        }
        println!("crop change: {:.2} ms", t.elapsed().as_secs_f64() * 1e3 / n as f64);
    }
}
