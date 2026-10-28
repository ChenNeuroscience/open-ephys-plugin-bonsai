//! Live view window: the latest frame with overlays (see `view`), plus a side panel of controls.

mod icons;
mod strip;
mod video_gpu;
mod view;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use eframe::egui::{self, RichText};

use crate::camera::Range;
use crate::config::{Control, Crop, Overlay, PipelineConfig, Ruler, TrackerState};
use crate::tracker::Shared;

/// Used for sizing new shapes before the first frame has arrived.
const FALLBACK_IMAGE_SIZE: [f32; 2] = [640.0, 480.0];

pub fn run(shared: Arc<Shared>, options: eframe::NativeOptions) -> eframe::Result {
    eframe::run_native(
        "Tracker",
        options,
        Box::new(move |cc| {
            *shared.repaint.lock().unwrap() = Some(cc.egui_ctx.clone());
            let target_format = cc.wgpu_render_state.as_ref().map(|rs| rs.target_format);
            Ok(Box::new(TrackerApp::new(shared, target_format)))
        }),
    )
}

pub fn default_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Tracker").with_inner_size([1150.0, 750.0]),
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum CropKind {
    None,
    Circle,
    Square,
    Rectangle,
}

impl CropKind {
    const ALL: [(CropKind, &'static str); 4] =
        [(CropKind::None, "None"), (CropKind::Circle, "Circle"), (CropKind::Square, "Square"), (CropKind::Rectangle, "Rectangle")];

    fn of(crop: &Crop) -> CropKind {
        match crop {
            Crop::None => CropKind::None,
            Crop::Circle { .. } => CropKind::Circle,
            Crop::Square { .. } => CropKind::Square,
            Crop::Rectangle { .. } => CropKind::Rectangle,
        }
    }

    /// Converts `crop` to this kind, keeping its centre, angle and rough size where possible.
    fn convert(self, crop: &Crop, image: [f32; 2]) -> Crop {
        let center = crop.center().unwrap_or([image[0] / 2.0, image[1] / 2.0]);
        let angle = crop.angle();
        let [hw, hh] = crop.half_extents().unwrap_or([0.4 * image[0].min(image[1]); 2]);
        match self {
            CropKind::None => Crop::None,
            CropKind::Circle => Crop::Circle { center, radius: hw.min(hh) },
            CropKind::Square => Crop::Square { center, size: 2.0 * hw.min(hh), angle },
            CropKind::Rectangle if hw == hh => Crop::Rectangle { center, width: 2.0 * hw, height: 1.5 * hh, angle },
            CropKind::Rectangle => Crop::Rectangle { center, width: 2.0 * hw, height: 2.0 * hh, angle },
        }
    }
}

struct TrackerApp {
    shared: Arc<Shared>,
    /// Local copy, written back to `shared` whenever it changes.
    state: TrackerState,
    video: view::VideoView,
    /// The frame rate warning is shown until this time (egui seconds), with the rate that triggered it.
    rate_warning: Option<(f64, f64)>,
}

/// The frame rate warning to show at `now`: (shown until, triggering rate), renewed whenever the measured
/// rate (0 = not measured yet) is off by more than `RATE_TOLERANCE`, and held for `RATE_WARNING_HOLD`.
fn held_rate_warning(current: Option<(f64, f64)>, now: f64, measured: f64, active: f64) -> Option<(f64, f64)> {
    if measured > 0.0 && (measured - active).abs() > RATE_TOLERANCE * active {
        return Some((now + RATE_WARNING_HOLD, measured));
    }
    current.filter(|&(until, _)| now < until)
}

/// Height of the strip below the video (LED readout, warnings); fixed so the video doesn't jump.
const STRIP_HEIGHT: f32 = 150.0;

/// A measured frame rate tidied for use as the setting: whole numbers when within 0.05% (the measurement,
/// from camera timestamps over several seconds, is far more precise than that), otherwise 2 decimals, so
/// 29.97 stays 29.97.
fn snap_frame_rate(measured: f64) -> f64 {
    let whole = measured.round();
    if (measured - whole).abs() <= 0.0005 * measured { whole } else { (measured * 100.0).round() / 100.0 }
}

/// Measured and declared frame rates may differ by this fraction before the warning appears.
const RATE_TOLERANCE: f64 = 0.02;
/// Seconds the frame rate warning stays up after the last time it applied.
const RATE_WARNING_HOLD: f64 = 10.0;

impl TrackerApp {
    fn new(shared: Arc<Shared>, target_format: Option<eframe::wgpu::TextureFormat>) -> Self {
        let state = shared.state.lock().unwrap().clone();
        TrackerApp { shared, state, video: view::VideoView::new(target_format), rate_warning: None }
    }

    fn image_size(&self) -> [f32; 2] {
        self.video.image_size.unwrap_or(FALLBACK_IMAGE_SIZE)
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tracker");
        ui.label(self.shared.status.lock().unwrap().as_str());
        let recording = self.shared.recording.load(Ordering::Relaxed);
        if recording {
            let red = egui::Color32::from_rgb(230, 70, 70);
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.5, red);
                ui.label(RichText::new("Recording: settings are locked").color(red));
            });
        }
        ui.add_space(4.0);

        egui::CollapsingHeader::new(RichText::new("Stats").strong()).default_open(true).show(ui, |ui| self.stats(ui));

        let mut locked = self.state.camera_locked;
        section(ui, "Camera", recording, None, (&mut locked, "Lock the camera settings"), |ui| self.camera(ui));
        self.state.camera_locked = locked;

        let mut overlay = self.state.crop_overlay;
        section(
            ui,
            "Crop", recording,
            Some((&mut overlay.visible, "Show the whole frame with the crop outline, or hide it and mask out everything outside the crop")),
            (&mut overlay.locked, "Lock the crop. Unlocked: drag inside to move, drag the square handles to resize, and the round \
             handle on the stalk to rotate (hold shift to snap to 15°)"),
            |ui| self.crop(ui),
        );
        self.state.crop_overlay = overlay;

        let mut overlay = self.state.detection_overlay;
        section(
            ui,
            "Detection", recording,
            Some((&mut overlay.visible, "Highlight pixels above the threshold and circle the detected regions")),
            (&mut overlay.locked, "Lock the detection settings"),
            |ui| self.detection(ui),
        );
        self.state.detection_overlay = overlay;

        let mut overlay = self.state.calibration.overlay;
        section(
            ui,
            "Calibration", recording,
            Some((&mut overlay.visible, "Show or hide the rulers (they still count towards px/m)")),
            (&mut overlay.locked, "Lock the rulers so they can't be dragged or edited"),
            |ui| self.calibration(ui),
        );
        self.state.calibration.overlay = overlay;
    }

    fn stats(&mut self, ui: &mut egui::Ui) {
        let stats = self.shared.stats.lock().unwrap().clone();
        egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
            for (name, value) in [
                ("Frames", stats.frames.to_string()),
                ("Rate", format!("{:.1} fps", if stats.measured_fps > 0.0 { stats.measured_fps } else { stats.fps })),
                ("Processing", format!("{:.2} ms", stats.process_ms)),
                ("Camera dropped", stats.camera_dropped.to_string()),
                ("Queue dropped", stats.queue_dropped.to_string()),
            ] {
                ui.label(name);
                ui.label(value);
                ui.end_row();
            }
        });
    }

    fn camera(&mut self, ui: &mut egui::Ui) {
        if self.state.camera_locked {
            ui.disable();
        }
        let controls = *self.shared.controls.lock().unwrap();
        let measured = self.shared.stats.lock().unwrap().measured_fps as f64;
        let settings = &mut self.state.camera;
        egui::Grid::new("camera").num_columns(2).show(ui, |ui| {
            ui.label("Frame rate");
            ui.horizontal(|ui| {
                let range = controls.frame_rate.map_or(1.0..=1000.0, |r| r.min..=r.max);
                ui.add(egui::DragValue::new(&mut settings.frame_rate).range(range).speed(0.1).max_decimals(2).suffix(" fps"))
                    .on_hover_text("Also the sample rate of the Open Ephys data stream");
                // Only needed when the camera won't take a frame rate, so its own rate has to be matched.
                if !controls.frame_rate_set {
                    let detect = ui.add_enabled(measured > 0.0, egui::Button::new("Detect")).on_hover_text(
                        "This camera won't take a frame rate, so set this to the rate it's delivering (measured over \
                         the last few seconds). Like any frame rate change, it applies from the next acquisition.",
                    );
                    if detect.clicked() {
                        settings.frame_rate = snap_frame_rate(measured);
                    }
                }
            });
            ui.end_row();
            control_row(ui, "Exposure", &mut settings.exposure, controls.exposure, " µs", true);
            control_row(ui, "Gain", &mut settings.gain, controls.gain, " dB", false);
        });
        let active = self.shared.active_frame_rate;
        if settings.frame_rate != active {
            ui.weak(format!("Running at {active} fps; the new frame rate applies from the next acquisition."));
        }

        for warning in self.shared.camera_warnings.lock().unwrap().iter() {
            ui.weak(RichText::new(warning).small());
        }
    }

    /// Below the video: the LED size chart at the right (while detection is visible) and, beside it, a
    /// warning when frames arrive at a different rate from the stream's declared one (which is what actually
    /// matters). The warning uses the rate over the last few seconds and, once shown, stays for at least
    /// `RATE_WARNING_HOLD` seconds so it doesn't flicker.
    fn bottom_strip(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|i| i.time);
        let active = self.shared.active_frame_rate;
        let measured = self.shared.stats.lock().unwrap().measured_fps as f64;
        self.rate_warning = held_rate_warning(self.rate_warning, now, measured, active);
        if let Some((until, _)) = self.rate_warning {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(until - now));
        }

        // The readout takes a fixed width at the right; the warning sits right-aligned beside it.
        let rect = ui.available_rect_before_wrap();
        let readout_width = if self.state.detection_overlay.visible { strip::READOUT_WIDTH } else { 0.0 };
        let split = rect.right() - readout_width;
        let region = |ui: &mut egui::Ui, min_x: f32, max_x: f32, layout, add: &mut dyn FnMut(&mut egui::Ui)| {
            let r = egui::Rect::from_min_max(egui::pos2(min_x, rect.top()), egui::pos2(max_x, rect.bottom()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(r).layout(layout), |ui| add(ui));
        };
        if self.state.detection_overlay.visible {
            let history = self.shared.led_history.lock().unwrap();
            let ltr = egui::Layout::left_to_right(egui::Align::Min);
            region(ui, split, rect.right(), ltr, &mut |ui| strip::led_chart(ui, &history));
        }
        if let Some((_, measured)) = self.rate_warning {
            let rtl = egui::Layout::right_to_left(egui::Align::Center);
            region(ui, rect.left(), split - 12.0, rtl, &mut |ui| {
                let width = (ui.available_width() - 16.0).clamp(120.0, 380.0);
                egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.0, strip::AMBER)).show(ui, |ui| {
                    ui.set_max_width(width);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!("Camera running at {measured:.1} fps, stream declared at {active} fps"))
                                .color(strip::AMBER)
                                .strong(),
                        );
                        let advice = if self.shared.controls.lock().unwrap().frame_rate_set {
                            "Samples will be dropped or gap-filled. The camera may be triggered externally, or not \
                             keeping up (e.g. exposure too long for this rate)."
                        } else {
                            "Samples will be dropped or gap-filled. This camera won't take a frame rate: use Detect in \
                             the Camera section to match it (applies from the next acquisition)."
                        };
                        ui.label(RichText::new(advice).small());
                    });
                });
            });
        }
    }

    fn detection(&mut self, ui: &mut egui::Ui) {
        if self.state.detection_overlay.locked {
            ui.disable();
        }
        let p = &mut self.state.pipeline;
        ui.horizontal(|ui| {
            let label = ui.label("Threshold");
            ui.add(egui::Slider::new(&mut p.threshold, 0..=255)).labelled_by(label.id);
        });
        if p.crop != Crop::None {
            ui.horizontal(|ui| {
                ui.checkbox(&mut p.edge.enabled, "Threshold edge").on_hover_text(
                    "A higher threshold for pixels near the crop boundary, so LED reflections off the walls don't \
                     count. The dashed line shows where it stops.",
                );
                if p.edge.enabled {
                    p.edge.threshold = p.edge.threshold.max(p.threshold);
                    ui.add(egui::DragValue::new(&mut p.edge.threshold).range(p.threshold..=255).prefix("≥ "))
                        .on_hover_text("Threshold within the band (at least the main threshold)");
                    ui.label("within");
                    ui.add(egui::DragValue::new(&mut p.edge.width).range(1.0..=1000.0).speed(0.5).suffix(" px"));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("Min area");
            ui.add(egui::DragValue::new(&mut p.min_area).range(0..=100_000).suffix(" px"));
        });
        ui.horizontal(|ui| {
            let mut bounded = p.max_area.is_some();
            ui.checkbox(&mut bounded, "Max area");
            p.max_area = bounded.then(|| {
                let mut v = p.max_area.unwrap_or(1000);
                ui.add(egui::DragValue::new(&mut v).range(p.min_area..=1_000_000).suffix(" px"));
                v
            });
        });
    }

    fn crop(&mut self, ui: &mut egui::Ui) {
        let image = self.image_size();
        let TrackerState { pipeline: PipelineConfig { crop, .. }, crop_overlay: overlay, .. } = &mut self.state;
        if overlay.locked {
            ui.disable();
        }
        let current = CropKind::of(crop);
        ui.horizontal_wrapped(|ui| {
            for (kind, name) in CropKind::ALL {
                if ui.selectable_label(current == kind, name).clicked() && current != kind {
                    *crop = kind.convert(crop, image);
                    *overlay = Overlay { visible: true, locked: false };
                }
            }
        });

        egui::Grid::new("crop").num_columns(2).show(ui, |ui| {
            match crop {
                Crop::None => {}
                Crop::Circle { center, radius } => {
                    center_row(ui, center);
                    ui.label("Radius");
                    ui.add(px(radius));
                    ui.end_row();
                }
                Crop::Square { center, size, angle } => {
                    center_row(ui, center);
                    ui.label("Size");
                    ui.add(px(size));
                    ui.end_row();
                    angle_row(ui, angle);
                }
                Crop::Rectangle { center, width, height, angle } => {
                    center_row(ui, center);
                    ui.label("Width");
                    ui.add(px(width));
                    ui.end_row();
                    ui.label("Height");
                    ui.add(px(height));
                    ui.end_row();
                    angle_row(ui, angle);
                }
            }
        });
    }

    fn calibration(&mut self, ui: &mut egui::Ui) {
        let image = self.image_size();
        let cal = &mut self.state.calibration;
        match cal.pixels_per_meter() {
            Some(ppm) => {
                ui.label(RichText::new(format!("{ppm:.1} px/m")).size(18.0).strong());
                ui.weak(format!("{:.2} mm/px, mean of {} ruler(s)", 1000.0 / ppm, cal.rulers.iter().filter(|r| r.length_cm > 0.0).count()));
            }
            None => {
                ui.weak("Not calibrated: add a ruler, drag its ends onto something of known size and enter its length.");
            }
        }
        // Only the editing controls below are disabled when locked; the readout above stays prominent.
        if cal.overlay.locked {
            ui.disable();
        }
        if ui.button("Add ruler").clicked() {
            let y = image[1] / 2.0 + 20.0 * (cal.rulers.len() % 8) as f32;
            cal.rulers.push(Ruler { a: [image[0] / 3.0, y], b: [image[0] * 2.0 / 3.0, y], length_cm: 0.0 });
            cal.overlay.visible = true;
        }

        let mut remove = None;
        egui::Grid::new("rulers").num_columns(4).striped(true).show(ui, |ui| {
            for (i, ruler) in cal.rulers.iter_mut().enumerate() {
                ui.label(format!("R{}", i + 1));
                ui.label(format!("{:.1} px", ruler.pixels()));
                ui.add(egui::DragValue::new(&mut ruler.length_cm).speed(0.5).range(0.0..=10_000.0).suffix(" cm"))
                    .on_hover_text("Physical length; 0 leaves this ruler out of the calibration");
                if ui.small_button("×").on_hover_text("Remove ruler").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            cal.rulers.remove(i);
        }
    }
}

/// A collapsible section whose header carries eye (overlay visibility) and padlock (lock) toggles.
/// Locking stops the overlay being dragged on the video; each section also disables its own controls.
/// While `recording`, the section is forced locked whatever its own padlock says (the eye still works).
fn section(
    ui: &mut egui::Ui,
    title: &str,
    recording: bool,
    visible: Option<(&mut bool, &str)>,
    (locked, lock_tip): (&mut bool, &str),
    body: impl FnOnce(&mut egui::Ui),
) {
    let id = ui.make_persistent_id(title);
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if recording {
                    ui.add_enabled_ui(false, |ui| icons::toggle(ui, &mut true, icons::padlock, "Locked while recording"));
                } else {
                    icons::toggle(ui, locked, icons::padlock, lock_tip);
                }
                if let Some((visible, eye_tip)) = visible {
                    icons::toggle(ui, visible, icons::eye, eye_tip);
                }
            });
        })
        .body(|ui| {
            if recording {
                ui.disable();
            }
            body(ui)
        });
}

/// A camera setting that may be left as the camera has it, automatic, or set manually.
fn control_row(ui: &mut egui::Ui, name: &str, control: &mut Control, range: Option<Range>, unit: &str, log: bool) {
    ui.label(name);
    ui.vertical(|ui| {
        let Some(range) = range else {
            ui.weak("not supported by this camera");
            return;
        };
        ui.horizontal(|ui| {
            let manual = Control::Manual(range.current.clamp(range.min, range.max));
            for (option, label) in [(Control::Unchanged, "Camera"), (Control::Auto, "Auto"), (manual, "Manual")] {
                let selected = std::mem::discriminant(control) == std::mem::discriminant(&option);
                if ui.selectable_label(selected, label).clicked() && !selected {
                    *control = option;
                }
            }
        })
        .response
        .on_hover_text("Camera: leave it as the camera is configured (e.g. in SpinView)");
        match control {
            Control::Manual(v) => {
                ui.add(egui::Slider::new(v, range.min..=range.max).logarithmic(log).suffix(unit));
            }
            _ => {
                ui.weak(format!("now {:.1}{unit}", range.current));
            }
        }
    });
    ui.end_row();
}

fn center_row(ui: &mut egui::Ui, center: &mut [f32; 2]) {
    ui.label("Centre");
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut center[0]).speed(1.0).prefix("x "));
        ui.add(egui::DragValue::new(&mut center[1]).speed(1.0).prefix("y "));
    });
    ui.end_row();
}

fn px(v: &mut f32) -> egui::DragValue<'_> {
    egui::DragValue::new(v).speed(1.0).range(1.0..=f32::MAX).suffix(" px")
}

fn angle_row(ui: &mut egui::Ui, angle: &mut f32) {
    ui.label("Angle");
    ui.drag_angle(angle);
    ui.end_row();
}

impl eframe::App for TrackerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.shared.stop.load(Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.show(ui);
    }
}

impl TrackerApp {
    fn show(&mut self, ui: &mut egui::Ui) {
        let before = self.state.clone();

        egui::Panel::left("controls").resizable(true).default_size(290.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
        });
        let latest = self.shared.latest.lock().unwrap().clone();
        if self.shared.recording.load(Ordering::Relaxed) {
            // Draw from a copy with every overlay locked, so nothing can be dragged while recording.
            let mut locked = self.state.clone();
            locked.crop_overlay.locked = true;
            locked.calibration.overlay.locked = true;
            egui::CentralPanel::default().show(ui, |ui| {
                egui::Panel::bottom("strip").exact_size(STRIP_HEIGHT).show(ui, |ui| self.bottom_strip(ui));
                self.video.show(ui, &mut locked, latest.as_ref())
            });
        } else {
            egui::CentralPanel::default().show(ui, |ui| {
                egui::Panel::bottom("strip").exact_size(STRIP_HEIGHT).show(ui, |ui| self.bottom_strip(ui));
                self.video.show(ui, &mut self.state, latest.as_ref())
            });
        }

        if self.state != before {
            *self.shared.state.lock().unwrap() = self.state.clone();
        }
    }
}

impl Drop for TrackerApp {
    fn drop(&mut self) {
        *self.shared.repaint.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, SyntheticCamera};
    use crate::tracker::View;

    /// A wgpu renderer for the harness, plus its target format so the app's video callback can draw into it.
    fn test_renderer() -> (egui_kittest::wgpu::WgpuTestRenderer, eframe::wgpu::TextureFormat) {
        let state = egui_kittest::wgpu::create_render_state(egui_kittest::wgpu::default_wgpu_setup(), Default::default());
        let format = state.target_format;
        (egui_kittest::wgpu::WgpuTestRenderer::from_render_state(state), format)
    }

    /// Renders the whole window. Run with UPDATE_SNAPSHOTS=1 to accept changes; see tests/snapshots/.
    fn window_snapshot(
        name: &str,
        recording: bool,
        setup: impl FnOnce(&Shared),
        configure: impl FnOnce(&mut TrackerState),
    ) {
        let mut camera = SyntheticCamera::new(640, 480, 50.0);
        let frame = camera.next_frame().unwrap().unwrap();
        let mut state = TrackerState::default();
        state.pipeline.crop = Crop::Rectangle { center: [330.0, 250.0], width: 420.0, height: 300.0, angle: 0.3 };
        state.pipeline.edge = crate::config::EdgeThreshold { enabled: true, width: 25.0, threshold: 250 };
        state.calibration.rulers = vec![
            Ruler { a: [120.0, 420.0], b: [480.0, 420.0], length_cm: 100.0 },
            Ruler { a: [560.0, 80.0], b: [560.0, 260.0], length_cm: 0.0 },
        ];
        configure(&mut state);
        let blobs = crate::pipeline::Pipeline::default().process(&frame.image, &state.pipeline);
        let shared = Arc::new(Shared {
            active_frame_rate: 50.0,
            controls: camera.controls().into(),
            recording: recording.into(),
            stats: crate::tracker::Stats { measured_fps: 50.0, ..Default::default() }.into(),
            // 10 s at 50 fps of plausible LED areas, with a few dropouts.
            led_history: (0..500)
                .map(|i| {
                    let wobble = |p: f64| ((i as f64 * p).sin() * 3.0) as i32;
                    let big = if i % 97 < 3 { 0 } else { (30 + wobble(0.11)) as u32 };
                    let small = if i % 61 < 4 { 0 } else { (12 + wobble(0.07) / 2) as u32 };
                    (i as f64 / 50.0, [big, small])
                })
                .collect::<std::collections::VecDeque<_>>()
                .into(),
            state: state.into(),
            ..Default::default()
        });
        *shared.latest.lock().unwrap() = Some(Arc::new(View { frame, blobs }));
        setup(&shared);

        let (renderer, target_format) = test_renderer();
        let mut app = TrackerApp::new(shared, Some(target_format));
        let mut harness =
            egui_kittest::Harness::builder().with_size([1150.0, 650.0]).renderer(renderer).build_ui(|ui| app.show(ui));
        harness.run();
        harness.snapshot(name);
    }

    #[test]
    fn window_crop_editable_rulers_locked() {
        window_snapshot("tracker_window", false, |_| {}, |s| s.calibration.overlay.locked = true);
    }

    #[test]
    fn window_detection_hidden() {
        window_snapshot("tracker_window_detection_hidden", false, |_| {}, |s| s.detection_overlay.visible = false);
    }

    #[test]
    fn detected_rates_are_tidied() {
        assert_eq!(snap_frame_rate(30.004), 30.0);
        assert_eq!(snap_frame_rate(29.9701), 29.97);
        assert_eq!(snap_frame_rate(49.2), 49.2);
    }

    #[test]
    fn rate_warning_is_held_for_ten_seconds() {
        let off = held_rate_warning(None, 100.0, 30.0, 50.0);
        assert_eq!(off, Some((110.0, 30.0)));
        // The rate recovers: still shown until the hold runs out, then gone.
        assert_eq!(held_rate_warning(off, 109.9, 50.0, 50.0), off);
        assert_eq!(held_rate_warning(off, 110.0, 50.0, 50.0), None);
        // Within tolerance (2%) never triggers; nor does an unmeasured rate.
        assert_eq!(held_rate_warning(None, 0.0, 50.9, 50.0), None);
        assert_eq!(held_rate_warning(None, 0.0, 0.0, 50.0), None);
    }

    /// A camera that won't take a frame rate, running at 30 fps against a declared 50.
    fn rate_mismatch(shared: &Shared) {
        shared.stats.lock().unwrap().measured_fps = 30.0;
        shared.controls.lock().unwrap().frame_rate_set = false;
    }

    #[test]
    fn window_rate_warning() {
        window_snapshot("tracker_window_rate_warning", false, rate_mismatch, |_| {});
    }

    #[test]
    fn window_recording() {
        window_snapshot("tracker_window_recording", true, |_| {}, |_| {});
    }

    #[test]
    fn window_crop_hidden() {
        window_snapshot("tracker_window_crop_hidden", false, |_| {}, |s| s.crop_overlay.visible = false);
    }

    /// Regenerates docs/screenshot.png from a live frame of tools/fake_camera.py (needs python3):
    ///     cargo test --release readme_screenshot -- --ignored
    #[test]
    #[ignore]
    fn readme_screenshot() {
        let port = 27398;
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/fake_camera.py");
        let mut child = std::process::Command::new("python3")
            .args([script, "--port", &port.to_string()])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("run python3");
        let mut camera = (0..50)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(100));
                crate::camera::FakeCamera::connect(&format!("127.0.0.1:{port}")).ok()
            })
            .expect("connect to fake camera");
        let mut frames = std::iter::from_fn(|| camera.next_frame().unwrap()).take(40);
        let frame = frames.by_ref().last().unwrap(); // let the mouse wander a little first
        child.kill().ok();

        // The arena floor edges are 1 m long, at ~363 px/m with the fake camera's geometry.
        let (lo, hi) = (320.0 - 181.6, 320.0 + 181.6);
        let mut state = TrackerState::default();
        state.pipeline.crop = Crop::Square { center: [320.0, 240.0], size: 452.0, angle: 0.0 };
        state.calibration.rulers = vec![
            Ruler { a: [lo, 240.0 + 181.6], b: [hi, 240.0 + 181.6], length_cm: 100.0 },
            Ruler { a: [320.0 - 181.6, 240.0 - 181.6], b: [320.0 - 181.6, 240.0 + 181.6], length_cm: 100.0 },
        ];
        let blobs = crate::pipeline::Pipeline::default().process(&frame.image, &state.pipeline);
        let shared = Arc::new(Shared {
            active_frame_rate: 50.0,
            controls: SyntheticCamera::new(640, 480, 50.0).controls().into(),
            state: state.into(),
            ..Default::default()
        });
        *shared.status.lock().unwrap() = "running".into();
        *shared.stats.lock().unwrap() = crate::tracker::Stats { frames: 2961, fps: 50.0, process_ms: 1.6, ..Default::default() };
        shared.state.lock().unwrap().crop_overlay.locked = true;
        *shared.latest.lock().unwrap() = Some(Arc::new(View { frame, blobs }));

        let (renderer, target_format) = test_renderer();
        let mut app = TrackerApp::new(shared, Some(target_format));
        let mut harness =
            egui_kittest::Harness::builder().with_size([1050.0, 600.0]).renderer(renderer).build_ui(|ui| app.show(ui));
        harness.run();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/screenshot.png");
        std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
        harness.render().unwrap().save(path).unwrap();
    }

    #[test]
    fn icons_snapshot() {
        let mut values = [true, false, true, false];
        let mut harness = egui_kittest::Harness::builder()
            .with_size([120.0, 30.0])
            .with_pixels_per_point(3.0)
            .wgpu()
            .build_ui(|ui| {
                ui.horizontal(|ui| {
                    let [a, b, c, d] = &mut values;
                    icons::toggle(ui, a, icons::eye, "");
                    icons::toggle(ui, b, icons::eye, "");
                    icons::toggle(ui, c, icons::padlock, "");
                    icons::toggle(ui, d, icons::padlock, "");
                });
            });
        harness.run();
        harness.snapshot("icons");
    }
}
