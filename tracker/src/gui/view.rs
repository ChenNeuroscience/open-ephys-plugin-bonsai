//! The video panel: frame texture, crop/ruler/blob overlays, and dragging of crop and ruler handles.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use eframe::egui::{
    self, Align2, Color32, ColorImage, CursorIcon, FontId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    TextureHandle, TextureOptions,
};

use crate::config::{from_local, to_local, Crop, PipelineConfig, TrackerState};
use crate::pipeline::MaskCache;
use crate::tracker::View;

const BLOB_COLOURS: [Color32; 2] = [Color32::GREEN, Color32::RED];
const CROP_COLOUR: Color32 = Color32::YELLOW;
const RULER_COLOUR: Color32 = Color32::from_rgb(0, 200, 255);
const HIGHLIGHT_COLOUR: Color32 = Color32::from_rgb(255, 0, 255);

// Screen-space sizes, in points.
const HANDLE_RADIUS: f32 = 5.0;
const HIT_RADIUS: f32 = 9.0;
const RULER_HIT_DISTANCE: f32 = 6.0;
const ROTATE_STALK: f32 = 28.0;
/// Smallest crop dimension, in image pixels.
const MIN_SIZE: f32 = 4.0;
const ROTATE_SNAP: f32 = 15.0 * PI / 180.0;

#[derive(Clone, Copy)]
pub struct Options {
    pub highlight_threshold: bool,
    pub edit_crop: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    CropMove,
    CropRadius,
    CropCorner,
    /// Rectangle side: axis 0 = width, 1 = height; sign says which of the two sides.
    CropEdge { axis: usize, sign: f32 },
    CropRotate,
    RulerEnd { ruler: usize, end: usize },
    RulerMove(usize),
}

struct Drag {
    target: Target,
    /// Pointer position at the previous step, in image coordinates.
    last: [f32; 2],
}

#[derive(Clone, Copy, PartialEq)]
struct TextureKey {
    frame: u64,
    highlight: bool,
    threshold: u8,
    crop: Crop,
}

/// Maps between image pixels and screen points for the displayed frame.
#[derive(Clone, Copy)]
struct Xform {
    origin: Pos2,
    scale: f32,
}

impl Xform {
    fn to_screen(self, p: [f32; 2]) -> Pos2 {
        self.origin + egui::vec2(p[0], p[1]) * self.scale
    }

    fn to_image(self, p: Pos2) -> [f32; 2] {
        let v = (p - self.origin) / self.scale;
        [v.x, v.y]
    }
}

#[derive(Default)]
pub struct VideoView {
    texture: Option<TextureHandle>,
    texture_key: Option<TextureKey>,
    mask: MaskCache,
    drag: Option<Drag>,
    /// Size of the most recent frame, once one has arrived.
    pub image_size: Option<[f32; 2]>,
}

impl VideoView {
    pub fn show(&mut self, ui: &mut egui::Ui, state: &mut TrackerState, view: Option<&View>, opts: Options) {
        let Some(view) = view else {
            ui.centered_and_justified(|ui| ui.label("No frames yet"));
            return;
        };
        self.update_texture(ui.ctx(), view, &state.pipeline, opts.highlight_threshold);
        let Some(texture) = &self.texture else { return };

        // Fit the frame in the available space, preserving aspect ratio.
        let img_size = texture.size_vec2();
        self.image_size = Some([img_size.x, img_size.y]);
        let avail = ui.available_size();
        let scale = (avail.x / img_size.x).min(avail.y / img_size.y);
        let (response, painter) = ui.allocate_painter(img_size * scale, Sense::click_and_drag());
        let xf = Xform { origin: response.rect.min, scale };
        painter.image(texture.id(), response.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);

        self.interact(ui, &response, xf, state, opts);

        draw_crop(&painter, xf, &state.pipeline.crop, opts.edit_crop);
        if state.calibration.show_rulers {
            for (i, ruler) in state.calibration.rulers.iter().enumerate() {
                draw_ruler(&painter, xf, i, ruler);
            }
        }
        for (blob, colour) in view.blobs.iter().zip(BLOB_COLOURS) {
            let r = (blob.area as f32 / PI).sqrt() * scale + 4.0;
            // Centroids are in pixel indices; +0.5 puts them at pixel centres on screen.
            painter.circle_stroke(xf.to_screen([blob.x + 0.5, blob.y + 0.5]), r, Stroke::new(2.0, colour));
        }
        if let Some(ppm) = state.calibration.pixels_per_meter() {
            label(&painter, response.rect.left_top() + egui::vec2(8.0, 8.0), Align2::LEFT_TOP, format!("{ppm:.1} px/m"), Color32::WHITE);
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context, view: &View, pipeline: &PipelineConfig, highlight: bool) {
        let key = TextureKey { frame: view.frame.id, highlight, threshold: pipeline.threshold, crop: pipeline.crop };
        if self.texture_key == Some(key) && self.texture.is_some() {
            return;
        }
        let img = &view.frame.image;
        let mask = self.mask.get(&pipeline.crop, img.width(), img.height()).map(|m| m.as_raw());
        let threshold = pipeline.threshold;
        let pixels = img
            .as_raw()
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                if mask.is_some_and(|m| m[i] == 0) {
                    Color32::from_gray(v / 3) // outside the crop: dimmed
                } else if highlight && v > threshold {
                    HIGHLIGHT_COLOUR
                } else {
                    Color32::from_gray(v)
                }
            })
            .collect();
        let image = ColorImage::new([img.width() as usize, img.height() as usize], pixels);
        match &mut self.texture {
            Some(tex) => tex.set(image, TextureOptions::NEAREST),
            None => self.texture = Some(ctx.load_texture("frame", image, TextureOptions::NEAREST)),
        }
        self.texture_key = Some(key);
    }

    fn interact(&mut self, ui: &egui::Ui, response: &Response, xf: Xform, state: &mut TrackerState, opts: Options) {
        if response.drag_started() {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                self.drag = hit(origin, xf, state, opts).map(|target| Drag { target, last: xf.to_image(origin) });
            }
        }
        if response.dragged() {
            if let (Some(drag), Some(pos)) = (&mut self.drag, response.interact_pointer_pos()) {
                let p = xf.to_image(pos);
                let snap = ui.input(|i| i.modifiers.shift);
                apply_drag(drag.target, p, [p[0] - drag.last[0], p[1] - drag.last[1]], snap, state);
                drag.last = p;
            }
        }
        if response.drag_stopped() {
            self.drag = None;
        }

        let hovered = || response.hover_pos().and_then(|p| hit(p, xf, state, opts));
        if self.drag.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if let Some(target) = hovered() {
            ui.ctx().set_cursor_icon(if target == Target::CropMove { CursorIcon::Move } else { CursorIcon::Grab });
        }
    }
}

/// Point handles in priority order, in image coordinates.
fn handles(xf: Xform, state: &TrackerState, opts: Options) -> Vec<(Target, [f32; 2])> {
    let mut out = Vec::new();
    if state.calibration.show_rulers {
        for (i, r) in state.calibration.rulers.iter().enumerate() {
            out.push((Target::RulerEnd { ruler: i, end: 0 }, r.a));
            out.push((Target::RulerEnd { ruler: i, end: 1 }, r.b));
        }
    }
    if opts.edit_crop {
        out.extend(crop_handles(&state.pipeline.crop, ROTATE_STALK / xf.scale));
    }
    out
}

fn crop_handles(crop: &Crop, stalk: f32) -> Vec<(Target, [f32; 2])> {
    match *crop {
        Crop::None => vec![],
        Crop::Circle { center, radius } => vec![(Target::CropRadius, [center[0] + radius, center[1]])],
        Crop::Square { center, size, angle } => vec![
            (Target::CropCorner, from_local(center, angle, [size / 2.0, size / 2.0])),
            (Target::CropRotate, from_local(center, angle, [0.0, -size / 2.0 - stalk])),
        ],
        Crop::Rectangle { center, width, height, angle } => {
            let (hw, hh) = (width / 2.0, height / 2.0);
            vec![
                (Target::CropEdge { axis: 0, sign: 1.0 }, from_local(center, angle, [hw, 0.0])),
                (Target::CropEdge { axis: 0, sign: -1.0 }, from_local(center, angle, [-hw, 0.0])),
                (Target::CropEdge { axis: 1, sign: 1.0 }, from_local(center, angle, [0.0, hh])),
                (Target::CropEdge { axis: 1, sign: -1.0 }, from_local(center, angle, [0.0, -hh])),
                (Target::CropRotate, from_local(center, angle, [0.0, -hh - stalk])),
            ]
        }
    }
}

fn hit(pos: Pos2, xf: Xform, state: &TrackerState, opts: Options) -> Option<Target> {
    let nearest = handles(xf, state, opts)
        .into_iter()
        .map(|(t, p)| (t, xf.to_screen(p).distance(pos)))
        .filter(|&(_, d)| d <= HIT_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((target, _)) = nearest {
        return Some(target);
    }
    if state.calibration.show_rulers {
        for (i, r) in state.calibration.rulers.iter().enumerate() {
            if distance_to_segment(pos, xf.to_screen(r.a), xf.to_screen(r.b)) <= RULER_HIT_DISTANCE {
                return Some(Target::RulerMove(i));
            }
        }
    }
    let crop = &state.pipeline.crop;
    (opts.edit_crop && *crop != Crop::None && crop.contains(xf.to_image(pos))).then_some(Target::CropMove)
}

fn apply_drag(target: Target, p: [f32; 2], delta: [f32; 2], snap: bool, state: &mut TrackerState) {
    let crop = &mut state.pipeline.crop;
    let rulers = &mut state.calibration.rulers;
    match (target, crop) {
        (Target::CropMove, crop) => {
            if let Some(c) = crop.center() {
                crop.set_center([c[0] + delta[0], c[1] + delta[1]]);
            }
        }
        (Target::CropRadius, Crop::Circle { center, radius }) => {
            *radius = (p[0] - center[0]).hypot(p[1] - center[1]).max(MIN_SIZE);
        }
        (Target::CropCorner, Crop::Square { center, size, angle }) => {
            let l = to_local(*center, *angle, p);
            *size = (2.0 * l[0].abs().max(l[1].abs())).max(MIN_SIZE);
        }
        (Target::CropEdge { axis, sign }, Crop::Rectangle { center, width, height, angle }) => {
            // Move one side, keeping the opposite side where it is.
            let l = to_local(*center, *angle, p);
            let half = if axis == 0 { *width / 2.0 } else { *height / 2.0 };
            let fixed = -sign * half;
            let moving = if sign > 0.0 { l[axis].max(fixed + MIN_SIZE) } else { l[axis].min(fixed - MIN_SIZE) };
            let mut mid = [0.0; 2];
            mid[axis] = (moving + fixed) / 2.0;
            *center = from_local(*center, *angle, mid);
            *if axis == 0 { width } else { height } = (moving - fixed).abs();
        }
        (Target::CropRotate, Crop::Square { center, angle, .. } | Crop::Rectangle { center, angle, .. }) => {
            // The handle sits "above" the shape, i.e. at -90 degrees in its own frame.
            let mut a = (p[1] - center[1]).atan2(p[0] - center[0]) + FRAC_PI_2;
            if snap {
                a = (a / ROTATE_SNAP).round() * ROTATE_SNAP;
            }
            *angle = (a + PI).rem_euclid(TAU) - PI;
        }
        (Target::RulerEnd { ruler, end }, _) => {
            if let Some(r) = rulers.get_mut(ruler) {
                *if end == 0 { &mut r.a } else { &mut r.b } = p;
            }
        }
        (Target::RulerMove(i), _) => {
            if let Some(r) = rulers.get_mut(i) {
                for q in [&mut r.a, &mut r.b] {
                    q[0] += delta[0];
                    q[1] += delta[1];
                }
            }
        }
        _ => {} // the crop changed shape mid-drag
    }
}

fn draw_crop(painter: &Painter, xf: Xform, crop: &Crop, editing: bool) {
    let stroke = Stroke::new(1.5, CROP_COLOUR);
    let (Some(center), Some([hw, hh])) = (crop.center(), crop.half_extents()) else { return };
    let angle = crop.angle();
    match crop {
        Crop::Circle { .. } => {
            painter.circle_stroke(xf.to_screen(center), hw * xf.scale, stroke);
        }
        _ => {
            let corners = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]];
            let points = corners.iter().map(|&l| xf.to_screen(from_local(center, angle, l))).collect();
            painter.add(Shape::closed_line(points, stroke));
        }
    }
    if !editing {
        return;
    }
    let c = xf.to_screen(center);
    painter.line_segment([c - egui::vec2(6.0, 0.0), c + egui::vec2(6.0, 0.0)], stroke);
    painter.line_segment([c - egui::vec2(0.0, 6.0), c + egui::vec2(0.0, 6.0)], stroke);
    for (target, p) in crop_handles(crop, ROTATE_STALK / xf.scale) {
        let p = xf.to_screen(p);
        if target == Target::CropRotate {
            let top = xf.to_screen(from_local(center, angle, [0.0, -hh]));
            painter.line_segment([top, p], stroke);
            painter.circle(p, HANDLE_RADIUS, CROP_COLOUR, Stroke::new(1.0, Color32::BLACK));
        } else {
            painter.rect(
                Rect::from_center_size(p, egui::vec2(2.0, 2.0) * HANDLE_RADIUS),
                0.0,
                Color32::WHITE,
                Stroke::new(1.0, Color32::BLACK),
                egui::StrokeKind::Middle,
            );
        }
    }
}

fn draw_ruler(painter: &Painter, xf: Xform, index: usize, ruler: &crate::config::Ruler) {
    let (a, b) = (xf.to_screen(ruler.a), xf.to_screen(ruler.b));
    let stroke = Stroke::new(2.0, RULER_COLOUR);
    painter.line_segment([a, b], stroke);
    let dir = (b - a).normalized();
    let normal = if dir.is_finite() { egui::vec2(-dir.y, dir.x) } else { egui::vec2(0.0, -1.0) };
    for p in [a, b] {
        painter.line_segment([p - normal * 7.0, p + normal * 7.0], stroke);
        painter.circle(p, HANDLE_RADIUS - 1.5, Color32::WHITE, Stroke::new(1.0, Color32::BLACK));
    }
    let mut text = format!("R{}  {:.1} px", index + 1, ruler.pixels());
    if ruler.length_cm > 0.0 {
        text += &format!(" = {} cm", ruler.length_cm);
    }
    // Put the label on whichever side of the line is "up" on screen.
    let up = if normal.y > 0.0 { -normal } else { normal };
    label(painter, a + (b - a) / 2.0 + up * 14.0, Align2::CENTER_CENTER, text, RULER_COLOUR);
}

/// Text on a translucent dark background, so it's readable over any part of the frame.
fn label(painter: &Painter, pos: Pos2, anchor: Align2, text: String, colour: Color32) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), colour);
    let rect = anchor.anchor_size(pos, galley.size());
    painter.rect_filled(rect.expand(3.0), 3.0, Color32::from_black_alpha(170));
    painter.galley(rect.min, galley, colour);
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_corners(crop: &Crop) -> Vec<[f32; 2]> {
        let (c, [hw, hh], a) = (crop.center().unwrap(), crop.half_extents().unwrap(), crop.angle());
        [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].iter().map(|&l| from_local(c, a, l)).collect()
    }

    #[test]
    fn dragging_a_rotated_rectangle_edge_keeps_the_opposite_edge() {
        let mut state = TrackerState::default();
        state.pipeline.crop = Crop::Rectangle { center: [100.0, 100.0], width: 60.0, height: 40.0, angle: 0.5 };
        let before = rect_corners(&state.pipeline.crop);
        // Drag the +x side outwards by 20px along the rectangle's own x axis.
        let target = from_local([100.0, 100.0], 0.5, [50.0, 0.0]);
        apply_drag(Target::CropEdge { axis: 0, sign: 1.0 }, target, [0.0, 0.0], false, &mut state);

        let Crop::Rectangle { width, height, .. } = state.pipeline.crop else { unreachable!() };
        assert!((width - 80.0).abs() < 1e-3 && (height - 40.0).abs() < 1e-3);
        let after = rect_corners(&state.pipeline.crop);
        for i in [0, 3] {
            // the -x side's corners
            assert!((after[i][0] - before[i][0]).abs() < 1e-3 && (after[i][1] - before[i][1]).abs() < 1e-3);
        }
    }

    #[test]
    fn rotate_snaps_with_shift() {
        let mut state = TrackerState::default();
        state.pipeline.crop = Crop::Square { center: [0.0, 0.0], size: 10.0, angle: 0.0 };
        // Pointer slightly clockwise of straight right, i.e. ~90 degrees from the handle's rest position.
        apply_drag(Target::CropRotate, [10.0, 0.4], [0.0, 0.0], true, &mut state);
        assert!((state.pipeline.crop.angle() - FRAC_PI_2).abs() < 1e-5);
    }
}
