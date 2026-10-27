//! Live view: the latest frame with blob/mask overlays, plus pipeline controls.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions};

use crate::config::PipelineConfig;
use crate::tracker::Shared;

const BLOB_COLOURS: [Color32; 2] = [Color32::GREEN, Color32::RED];

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
        viewport: egui::ViewportBuilder::default().with_title("Tracker").with_inner_size([1100.0, 750.0]),
        ..Default::default()
    }
}

struct TrackerApp {
    shared: Arc<Shared>,
    config: PipelineConfig,
    texture: Option<TextureHandle>,
    shown_frame: Option<(u64, bool)>,
    show_threshold: bool,
    editing_mask: bool,
}

impl TrackerApp {
    fn new(shared: Arc<Shared>) -> Self {
        let config = shared.pipeline.lock().unwrap().clone();
        TrackerApp { shared, config, texture: None, shown_frame: None, show_threshold: true, editing_mask: false }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        let status = self.shared.status.lock().unwrap().clone();
        let stats = self.shared.stats.lock().unwrap().clone();
        ui.heading("Tracker");
        ui.label(status);
        ui.separator();
        egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
            ui.label("Frames");
            ui.label(stats.frames.to_string());
            ui.end_row();
            ui.label("Rate");
            ui.label(format!("{:.1} fps", stats.fps));
            ui.end_row();
            ui.label("Processing");
            ui.label(format!("{:.2} ms", stats.process_ms));
            ui.end_row();
            ui.label("Camera dropped");
            ui.label(stats.camera_dropped.to_string());
            ui.end_row();
            ui.label("Queue dropped");
            ui.label(stats.queue_dropped.to_string());
            ui.end_row();
        });
        ui.separator();

        let before = self.config.clone();
        ui.add(egui::Slider::new(&mut self.config.threshold, 0..=255).text("threshold"));
        ui.checkbox(&mut self.show_threshold, "Highlight above threshold");
        ui.horizontal(|ui| {
            ui.label("Min area");
            ui.add(egui::DragValue::new(&mut self.config.min_area).range(0..=100_000));
        });
        ui.horizontal(|ui| {
            let mut bounded = self.config.max_area.is_some();
            ui.checkbox(&mut bounded, "Max area");
            if bounded {
                let mut v = self.config.max_area.unwrap_or(1000);
                ui.add(egui::DragValue::new(&mut v).range(self.config.min_area..=1_000_000));
                self.config.max_area = Some(v);
            } else {
                self.config.max_area = None;
            }
        });
        ui.separator();

        ui.toggle_value(&mut self.editing_mask, "Edit mask (click image to add points)");
        if ui.button("Clear mask").clicked() {
            self.config.mask_polygon = None;
        }

        if self.config != before {
            *self.shared.pipeline.lock().unwrap() = self.config.clone();
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context, view: &crate::tracker::View) {
        let key = (view.frame.id, self.show_threshold);
        if self.shown_frame == Some(key) && self.texture.is_some() {
            return;
        }
        let img = &view.frame.image;
        let size = [img.width() as usize, img.height() as usize];
        let image = if self.show_threshold {
            let t = self.config.threshold;
            let pixels = img
                .as_raw()
                .iter()
                .map(|&v| if v > t { Color32::from_rgb(255, 0, 255) } else { Color32::from_gray(v) })
                .collect();
            ColorImage::new(size, pixels)
        } else {
            ColorImage::from_gray(size, img.as_raw())
        };
        match &mut self.texture {
            Some(tex) => tex.set(image, TextureOptions::NEAREST),
            None => self.texture = Some(ctx.load_texture("frame", image, TextureOptions::NEAREST)),
        }
        self.shown_frame = Some(key);
    }

    fn video(&mut self, ui: &mut egui::Ui) {
        let view = self.shared.latest.lock().unwrap().clone();
        let Some(view) = view else {
            ui.centered_and_justified(|ui| ui.label("No frames yet"));
            return;
        };
        self.update_texture(ui.ctx(), &view);
        let Some(tex) = &self.texture else { return };

        // Fit the frame in the available space, preserving aspect ratio.
        let img_size = tex.size_vec2();
        let avail = ui.available_size();
        let scale = (avail.x / img_size.x).min(avail.y / img_size.y);
        let (response, painter) = ui.allocate_painter(img_size * scale, Sense::click());
        let rect = response.rect;
        painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let to_screen = |x: f32, y: f32| rect.min + egui::vec2(x, y) * scale;

        if let Some(poly) = &self.config.mask_polygon {
            let points: Vec<Pos2> = poly.iter().map(|&[x, y]| to_screen(x as f32, y as f32)).collect();
            painter.add(egui::Shape::closed_line(points, Stroke::new(1.5, Color32::YELLOW)));
        }
        for (blob, colour) in view.blobs.iter().zip(BLOB_COLOURS) {
            let r = (blob.area as f32 / std::f32::consts::PI).sqrt() * scale + 4.0;
            painter.circle_stroke(to_screen(blob.x, blob.y), r, Stroke::new(2.0, colour));
        }

        if self.editing_mask && response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let p = (pos - rect.min) / scale;
                self.config.mask_polygon.get_or_insert_with(Vec::new).push([p.x as i32, p.y as i32]);
                *self.shared.pipeline.lock().unwrap() = self.config.clone();
            }
        }
    }
}

impl eframe::App for TrackerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.shared.stop.load(Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::Panel::left("controls").resizable(true).default_size(260.0).show(ui, |ui| self.controls(ui));
        egui::CentralPanel::default().show(ui, |ui| self.video(ui));
    }
}

impl Drop for TrackerApp {
    fn drop(&mut self) {
        *self.shared.repaint.lock().unwrap() = None;
    }
}
