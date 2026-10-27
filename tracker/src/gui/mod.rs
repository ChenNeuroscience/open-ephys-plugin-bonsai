//! Live view window: the latest frame with overlays (see `view`), plus a side panel of controls.

mod icons;
mod view;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use eframe::egui::{self, RichText};

use crate::config::{Crop, Overlay, PipelineConfig, Ruler, TrackerState};
use crate::tracker::Shared;

/// Used for sizing new shapes before the first frame has arrived.
const FALLBACK_IMAGE_SIZE: [f32; 2] = [640.0, 480.0];

pub fn run(shared: Arc<Shared>, options: eframe::NativeOptions) -> eframe::Result {
    eframe::run_native(
        "Tracker",
        options,
        Box::new(move |cc| {
            *shared.repaint.lock().unwrap() = Some(cc.egui_ctx.clone());
            Ok(Box::new(TrackerApp::new(shared)))
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
    highlight_threshold: bool,
}

impl TrackerApp {
    fn new(shared: Arc<Shared>) -> Self {
        let state = shared.state.lock().unwrap().clone();
        TrackerApp { shared, state, video: Default::default(), highlight_threshold: true }
    }

    fn image_size(&self) -> [f32; 2] {
        self.video.image_size.unwrap_or(FALLBACK_IMAGE_SIZE)
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tracker");
        ui.label(self.shared.status.lock().unwrap().as_str());
        ui.add_space(4.0);

        egui::CollapsingHeader::new("Stats").default_open(true).show(ui, |ui| self.stats(ui));
        egui::CollapsingHeader::new("Detection").default_open(true).show(ui, |ui| self.detection(ui));
        egui::CollapsingHeader::new("Crop").default_open(true).show(ui, |ui| self.crop(ui));
        egui::CollapsingHeader::new("Calibration").default_open(true).show(ui, |ui| self.calibration(ui));
    }

    fn stats(&mut self, ui: &mut egui::Ui) {
        let stats = self.shared.stats.lock().unwrap().clone();
        egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
            for (name, value) in [
                ("Frames", stats.frames.to_string()),
                ("Rate", format!("{:.1} fps", stats.fps)),
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

    fn detection(&mut self, ui: &mut egui::Ui) {
        let p = &mut self.state.pipeline;
        ui.add(egui::Slider::new(&mut p.threshold, 0..=255).text("threshold"));
        ui.checkbox(&mut self.highlight_threshold, "Highlight above threshold");
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
        let current = CropKind::of(crop);
        ui.horizontal_wrapped(|ui| {
            for (kind, name) in CropKind::ALL {
                if ui.selectable_label(current == kind, name).clicked() && current != kind {
                    *crop = kind.convert(crop, image);
                    *overlay = Overlay { visible: true, locked: false };
                }
            }
        });
        if *crop == Crop::None {
            return;
        }
        ui.horizontal(|ui| {
            icons::toggle(ui, &mut overlay.visible, icons::eye, "Show the whole frame and the crop outline, or hide them and mask out everything outside the crop");
            icons::toggle(
                ui,
                &mut overlay.locked,
                icons::padlock,
                "Lock the crop so it can't be dragged. Unlocked: drag inside to move, drag the square handles to \
                 resize, and the round handle on the stalk to rotate (hold shift to snap to 15°)",
            );
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
        ui.horizontal(|ui| {
            if ui.button("Add ruler").clicked() {
                let y = image[1] / 2.0 + 20.0 * (cal.rulers.len() % 8) as f32;
                cal.rulers.push(Ruler { a: [image[0] / 3.0, y], b: [image[0] * 2.0 / 3.0, y], length_cm: 0.0 });
                cal.overlay = Overlay { visible: true, locked: false };
            }
            icons::toggle(ui, &mut cal.overlay.visible, icons::eye, "Show or hide the rulers (they still count towards px/m)");
            icons::toggle(ui, &mut cal.overlay.locked, icons::padlock, "Lock the rulers so they can't be dragged");
        });

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
        let options = view::Options { highlight_threshold: self.highlight_threshold };
        egui::CentralPanel::default().show(ui, |ui| self.video.show(ui, &mut self.state, latest.as_deref(), options));

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

    /// Renders the whole window. Run with UPDATE_SNAPSHOTS=1 to accept changes; see tests/snapshots/.
    fn window_snapshot(name: &str, configure: impl FnOnce(&mut TrackerState)) {
        let frame = SyntheticCamera::new(640, 480, 1000.0).next_frame().unwrap().unwrap();
        let mut state = TrackerState::default();
        state.pipeline.crop = Crop::Rectangle { center: [330.0, 250.0], width: 420.0, height: 300.0, angle: 0.3 };
        state.calibration.rulers = vec![
            Ruler { a: [120.0, 420.0], b: [480.0, 420.0], length_cm: 100.0 },
            Ruler { a: [560.0, 80.0], b: [560.0, 260.0], length_cm: 0.0 },
        ];
        configure(&mut state);
        let blobs = crate::pipeline::Pipeline::default().process(&frame.image, &state.pipeline);
        let shared = Arc::new(Shared { state: state.into(), ..Default::default() });
        *shared.latest.lock().unwrap() = Some(Arc::new(View { frame, blobs }));

        let mut app = TrackerApp::new(shared);
        let mut harness = egui_kittest::Harness::builder().with_size([1150.0, 650.0]).wgpu().build_ui(|ui| app.show(ui));
        harness.run();
        harness.snapshot(name);
    }

    #[test]
    fn window_crop_editable_rulers_locked() {
        window_snapshot("tracker_window", |s| s.calibration.overlay.locked = true);
    }

    #[test]
    fn window_crop_hidden() {
        window_snapshot("tracker_window_crop_hidden", |s| s.crop_overlay.visible = false);
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
