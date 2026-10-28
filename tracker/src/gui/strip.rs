//! The strip below the video: LED size readout (bottom right) and the frame rate warning beside it.

use std::collections::VecDeque;

use eframe::egui::{self, Color32, RichText, Sense, Stroke};

use crate::tracker::LED_HISTORY;

pub const AMBER: Color32 = Color32::from_rgb(230, 170, 60);
const LED_COLOURS: [Color32; 2] = [Color32::GREEN, Color32::RED];
/// Width of the LED readout at the right of the strip.
pub const READOUT_WIDTH: f32 = 440.0;
/// Below this big/small area ratio the two LEDs are getting hard to tell apart.
const LOW_RATIO: f32 = 1.5;

/// Summary of the two LEDs' areas over the recent history.
#[derive(Debug, PartialEq)]
pub struct LedSummary {
    /// Latest areas (big, small); 0 if not seen.
    pub current: [u32; 2],
    /// Median area of each while seen; 0 if never seen.
    pub median: [u32; 2],
    /// Big/small area ratio over frames where both were seen: (median, lowest).
    pub ratio: Option<(f32, f32)>,
    /// Fraction of frames in which both were seen.
    pub both_seen: f32,
}

pub fn led_summary(history: &VecDeque<(f64, [u32; 2])>) -> Option<LedSummary> {
    let &(_, current) = history.back()?;
    let median = |mut v: Vec<f32>| -> Option<f32> {
        if v.is_empty() {
            return None;
        }
        v.sort_by(f32::total_cmp);
        Some(v[v.len() / 2])
    };
    let areas = |i: usize| history.iter().map(|&(_, a)| a[i]).filter(|&a| a > 0).map(|a| a as f32).collect();
    let ratios: Vec<f32> =
        history.iter().filter(|(_, a)| a[0] > 0 && a[1] > 0).map(|&(_, a)| a[0] as f32 / a[1] as f32).collect();
    Some(LedSummary {
        current,
        median: [0, 1].map(|i| median(areas(i)).unwrap_or(0.0) as u32),
        both_seen: ratios.len() as f32 / history.len() as f32,
        ratio: ratios.iter().copied().reduce(f32::min).map(|lowest| (median(ratios).unwrap(), lowest)),
    })
}

/// Coloured dot, painted (egui's fonts have no bullet glyph).
fn dot(ui: &mut egui::Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, colour);
}

pub fn led_readout(ui: &mut egui::Ui, history: &VecDeque<(f64, [u32; 2])>) {
    let Some(summary) = led_summary(history) else {
        ui.weak("No LEDs yet");
        return;
    };
    ui.horizontal(|ui| {
        sparkline(ui, history);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, name) in ["Big", "Small"].into_iter().enumerate() {
                ui.horizontal(|ui| {
                    dot(ui, LED_COLOURS[i]);
                    ui.label(format!("{name} {} px", summary.current[i]));
                    ui.weak(format!("median {}", summary.median[i]));
                });
            }
            ui.horizontal(|ui| {
                let ratio = match summary.ratio {
                    Some((median, lowest)) => {
                        let text = RichText::new(format!("Ratio {median:.1}×, lowest {lowest:.1}×"));
                        if lowest < LOW_RATIO { text.color(AMBER) } else { text }
                    }
                    None => RichText::new("Ratio –"),
                };
                ui.label(ratio).on_hover_text(format!(
                    "Big/small area ratio over the last {LED_HISTORY:.0} s. Near 1 the two LEDs are hard to tell \
                     apart: zoom the lens in (or defocus slightly) so each covers more pixels."
                ));
                ui.weak(format!("both seen {:.0}%", 100.0 * summary.both_seen));
            });
        });
    });
}

/// Both LEDs' areas over the last `LED_HISTORY` seconds; gaps where an LED wasn't seen.
fn sparkline(ui: &mut egui::Ui, history: &VecDeque<(f64, [u32; 2])>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(180.0, 52.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let Some(&(end, _)) = history.back() else { return };
    let top = history.iter().map(|&(_, a)| a[0].max(a[1])).max().unwrap_or(1).max(1) as f32;
    let to_screen = |t: f64, area: u32| {
        let x = rect.left() + rect.width() * (1.0 - ((end - t) / LED_HISTORY) as f32);
        let y = rect.bottom() - 2.0 - (rect.height() - 4.0) * area as f32 / top;
        egui::pos2(x, y)
    };
    for i in 0..2 {
        let mut line = Vec::new();
        for &(t, areas) in history {
            if areas[i] > 0 {
                line.push(to_screen(t, areas[i]));
            } else if !line.is_empty() {
                painter.add(egui::Shape::line(std::mem::take(&mut line), Stroke::new(1.2, LED_COLOURS[i])));
            }
        }
        painter.add(egui::Shape::line(line, Stroke::new(1.2, LED_COLOURS[i])));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_ignores_unseen_frames() {
        let history: VecDeque<_> =
            [(0.0, [30, 10]), (0.1, [32, 0]), (0.2, [28, 14]), (0.3, [0, 0]), (0.4, [31, 12])].into_iter().collect();
        let s = led_summary(&history).unwrap();
        assert_eq!(s.current, [31, 12]);
        assert_eq!(s.median, [31, 12]);
        let (median, lowest) = s.ratio.unwrap();
        assert!((median - 2.583).abs() < 0.01 && (lowest - 2.0).abs() < 1e-6);
        assert_eq!(s.both_seen, 0.6);
    }
}
