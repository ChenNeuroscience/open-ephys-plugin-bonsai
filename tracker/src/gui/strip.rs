//! The strip below the video: a chart of the two LEDs' sizes (bottom right) and the frame rate warning
//! beside it.

use std::collections::VecDeque;

use eframe::egui::{self, Color32};
use egui_plot::{uniform_grid_spacer, Line, Plot, PlotBounds};

use crate::tracker::LED_HISTORY;

pub const AMBER: Color32 = Color32::from_rgb(230, 170, 60);
/// Width of the LED chart at the right of the strip.
pub const READOUT_WIDTH: f32 = 440.0;

/// Big (green) and small (red) LED, the colours tracking setups have long used for the two head LEDs.
/// (Which line is which is also clear from their sizes.)
pub fn led_colours(_dark_mode: bool) -> [Color32; 2] {
    [Color32::GREEN, Color32::RED]
}

/// Areas of the big and small LED over the buffered history (up to `LED_HISTORY` seconds), with 0 s at the
/// oldest buffered frame: the lines fill from the left, then scroll once the buffer is full. Frames where an
/// LED wasn't seen leave a gap in its line.
pub fn led_chart(ui: &mut egui::Ui, history: &VecDeque<(f64, [u32; 2])>) {
    let colours = led_colours(ui.visuals().dark_mode);
    let start = history.front().map_or(0.0, |&(t, _)| t);
    let top = history.iter().map(|&(_, a)| a[0].max(a[1])).max().unwrap_or(0).max(10) as f64 * 1.15;
    let step = nice_step(top / 5.0);

    Plot::new("led_sizes")
        .width(ui.available_width())
        .height(ui.available_height())
        .x_grid_spacer(uniform_grid_spacer(|_| [1.0, 2.0, 10.0]))
        .x_axis_formatter(|mark, _| format!("{}s", mark.value.round()))
        .y_grid_spacer(uniform_grid_spacer(move |_| [step / 2.0, step, step * 10.0]))
        .y_axis_formatter(|mark, _| format!("{}px", mark.value.round()))
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show(ui, |plot| {
            plot.set_plot_bounds(PlotBounds::from_min_max([-0.3, 0.0], [LED_HISTORY + 0.2, top])); // room for the end ticks
            for (i, name) in ["Big", "Small"].into_iter().enumerate() {
                for run in seen_runs(history, i, start) {
                    plot.line(Line::new(name, run).color(colours[i]).width(2.0));
                }
            }
        });
}

/// A round tick spacing (1, 2 or 5 times a power of ten) of at least `raw`.
fn nice_step(raw: f64) -> f64 {
    let power = 10f64.powf(raw.max(1e-9).log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * power).find(|&s| s >= raw).unwrap()
}

/// Runs of consecutive frames in which LED `i` was seen, as (seconds after `start`, area) points.
fn seen_runs(history: &VecDeque<(f64, [u32; 2])>, i: usize, start: f64) -> Vec<Vec<[f64; 2]>> {
    let mut runs = vec![Vec::new()];
    for &(t, areas) in history {
        if areas[i] > 0 {
            runs.last_mut().unwrap().push([t - start, areas[i] as f64]);
        } else if !runs.last().unwrap().is_empty() {
            runs.push(Vec::new());
        }
    }
    runs.retain(|r| !r.is_empty());
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_break_where_the_led_was_not_seen() {
        let history: VecDeque<_> =
            [(1.0, [30, 10]), (1.1, [32, 0]), (1.2, [0, 14]), (1.3, [31, 12])].into_iter().collect();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let big = seen_runs(&history, 0, 1.0);
        assert_eq!(big.len(), 2);
        assert!(close(big[0][0][0], 0.0) && close(big[1][0][0], 0.3) && big[0].len() == 2 && big[1].len() == 1);
        assert_eq!(seen_runs(&history, 1, 1.0).iter().map(Vec::len).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn tick_steps_are_round() {
        assert_eq!(nice_step(9.6), 10.0);
        assert_eq!(nice_step(13.0), 20.0);
        assert_eq!(nice_step(41.0), 50.0);
        assert_eq!(nice_step(0.3), 0.5);
    }
}
