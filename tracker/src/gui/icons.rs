//! Small vector icons painted directly, since egui's default fonts no longer include emoji.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke};

const SIZE: f32 = 22.0;

/// A square icon button that flips `value` when clicked.
pub fn toggle(
    ui: &mut egui::Ui,
    value: &mut bool,
    paint: fn(&Painter, Rect, Color32, Color32, bool),
    tooltip: &str,
) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let background = if response.hovered() {
            ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
            visuals.weak_bg_fill
        } else {
            ui.visuals().panel_fill
        };
        paint(ui.painter(), rect.shrink(3.0), visuals.fg_stroke.color, background, *value);
    }
    response.on_hover_text(tooltip)
}

/// Open eye when `visible`, otherwise the same eye struck through.
pub fn eye(painter: &Painter, rect: Rect, colour: Color32, _background: Color32, visible: bool) {
    let c = rect.center();
    let (w, h) = (rect.width() / 2.0, rect.height() * 0.3);
    let stroke = Stroke::new(1.5, colour);
    let lid = |sign: f32| -> Vec<Pos2> {
        (0..=16)
            .map(|i| {
                let x = -1.0 + 2.0 * i as f32 / 16.0;
                c + egui::vec2(x * w, sign * h * (1.0 - x * x))
            })
            .collect()
    };
    painter.add(Shape::line(lid(-1.0), stroke));
    painter.add(Shape::line(lid(1.0), stroke));
    if visible {
        painter.circle_filled(c, h * 0.6, colour);
    } else {
        painter.circle_stroke(c, h * 0.6, stroke);
        painter.line_segment([rect.left_top() + egui::vec2(2.0, 1.0), rect.right_bottom() - egui::vec2(2.0, 1.0)], stroke);
    }
}

/// Closed padlock when `locked`, otherwise with the shackle raised and swung open.
pub fn padlock(painter: &Painter, rect: Rect, colour: Color32, background: Color32, locked: bool) {
    let stroke = Stroke::new(1.5, colour);
    let body = Rect::from_min_max(
        egui::pos2(rect.left() + rect.width() * 0.15, rect.top() + rect.height() * 0.45),
        egui::pos2(rect.right() - rect.width() * 0.15, rect.bottom()),
    );
    painter.rect_filled(body, 2.0, colour);

    let r = body.width() * 0.3;
    let lift = if locked { 0.0 } else { rect.height() * 0.18 };
    let top = body.top() - lift;
    let centre = egui::pos2(body.center().x, top - r * 0.4);
    // Arc over the top, then the two legs down towards the body.
    let mut points: Vec<Pos2> = vec![centre + egui::vec2(-r, r * 0.4 + lift)];
    points.extend((0..=12).map(|i| {
        let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
        centre + egui::vec2(r * a.cos(), r * a.sin())
    }));
    // When unlocked the right leg stops short, showing the gap.
    points.push(centre + egui::vec2(r, if locked { r * 0.4 } else { 0.0 }));
    painter.add(Shape::line(points, stroke));

    painter.circle_filled(body.center() - egui::vec2(0.0, 0.5), body.height() * 0.14, background); // keyhole
}
