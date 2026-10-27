use std::io;
use std::time::{Duration, Instant};

use image::{GrayImage, Luma};
use imageproc::drawing::draw_filled_circle_mut;

use super::{Camera, Frame};

/// Two LEDs (a big bright one and a smaller one behind it) circling the frame.
pub struct SyntheticCamera {
    width: u32,
    height: u32,
    interval: Duration,
    start: Instant,
    next_due: Instant,
    id: u64,
}

impl SyntheticCamera {
    pub fn new(width: u32, height: u32, fps: f64) -> Self {
        let now = Instant::now();
        SyntheticCamera {
            width,
            height,
            interval: Duration::from_secs_f64(1.0 / fps),
            start: now,
            next_due: now,
            id: 0,
        }
    }
}

impl Camera for SyntheticCamera {
    fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        let now = Instant::now();
        if now < self.next_due {
            std::thread::sleep((self.next_due - now).min(Duration::from_millis(50)));
            if Instant::now() < self.next_due {
                return Ok(None);
            }
        }
        self.next_due += self.interval;

        let t = self.id as f64 * self.interval.as_secs_f64();
        let (w, h) = (self.width as f64, self.height as f64);
        let theta = t * 0.7;
        let x = w / 2.0 + w / 3.0 * theta.cos();
        let y = h / 2.0 + h / 3.0 * (theta * 1.3).sin();
        let heading = theta + std::f64::consts::FRAC_PI_2;

        let mut image = GrayImage::from_pixel(self.width, self.height, Luma([20]));
        draw_filled_circle_mut(&mut image, (x as i32, y as i32), 5, Luma([255]));
        draw_filled_circle_mut(
            &mut image,
            ((x - 25.0 * heading.cos()) as i32, (y - 25.0 * heading.sin()) as i32),
            3,
            Luma([240]),
        );

        let frame = Frame { id: self.id, timestamp: self.start.elapsed().as_secs_f64(), image };
        self.id += 1;
        Ok(Some(frame))
    }
}
