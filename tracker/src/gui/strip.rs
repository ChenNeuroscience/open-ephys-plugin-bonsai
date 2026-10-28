//! The strip below the video: a chart of the two LEDs' sizes (bottom right) and the frame rate warning
//! beside it.

use std::collections::VecDeque;

use eframe::egui::{self, Color32};
use egui_plot::{uniform_grid_spacer, Corner, Legend, Line, Plot, PlotBounds};

use crate::tracker::LED_HISTORY;

pub const AMBER: Color32 = Color32::from_rgb(230, 170, 60);
/// Width of the LED chart at the right of the strip.
pub const READOUT_WIDTH: f32 = 440.0;

/// Big and small LED colours, for dark and light themes: the first two slots of a categorical palette
/// validated for colour-vision deficiency against egui's panel colours (unlike green/red).
pub fn led_colours(dark_mode: bool) -> [Color32; 2] {
    if dark_mode {
        [Color32::from_rgb(0x39, 0x87, 0xe5), Color32::from_rgb(0xd9, 0x59, 0x26)]
    } else {
        [Color32::from_rgb(0x2a, 0x78, 0xd6), Color32::from_rgb(0xeb, 0x68, 0x34)]
    }
}

/// Areas of the big and small LED over the last `LED_HISTORY` seconds, with time relative to the latest
/// frame. Frames where an LED wasn't seen leave a gap in its line.
pub fn led_chart(ui: &mut egui::Ui, history: &VecDeque<(f64, [u32; 2])>) {
    let colours = led_colours(ui.visuals().dark_mode);
    let end = history.back().map_or(0.0, |&(t, _)| t);
    // Headroom above the data keeps the legend (top left) clear of the lines.
    let top = history.iter().map(|&(_, a)| a[0].max(a[1])).max().unwrap_or(0).max(10) as f64 * 1.6;

    Plot::new("led_sizes")
        .width(READOUT_WIDTH)
        .height(ui.available_height())
        .legend(Legend::default().position(Corner::LeftTop))
        .x_axis_label("time (s)")
        .x_grid_spacer(uniform_grid_spacer(|_| [1.0, 2.0, 10.0]))
        .y_axis_label("area (px)")
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show(ui, |plot| {
            plot.set_plot_bounds(PlotBounds::from_min_max([-LED_HISTORY - 0.2, 0.0], [0.4, top])); // room for the end ticks
            for (i, name) in ["Big", "Small"].into_iter().enumerate() {
                for run in seen_runs(history, i, end) {
                    plot.line(Line::new(name, run).color(colours[i]).width(2.0));
                }
            }
        });
}

/// Runs of consecutive frames in which LED `i` was seen, as (seconds before `end`, area) points.
fn seen_runs(history: &VecDeque<(f64, [u32; 2])>, i: usize, end: f64) -> Vec<Vec<[f64; 2]>> {
    let mut runs = vec![Vec::new()];
    for &(t, areas) in history {
        if areas[i] > 0 {
            runs.last_mut().unwrap().push([t - end, areas[i] as f64]);
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
        let big = seen_runs(&history, 0, 1.3);
        assert_eq!(big.len(), 2);
        assert!(close(big[0][0][0], -0.3) && big[0].len() == 2 && big[1].len() == 1);
        assert_eq!(seen_runs(&history, 1, 1.3).iter().map(Vec::len).collect::<Vec<_>>(), [1, 2]);
    }
}
