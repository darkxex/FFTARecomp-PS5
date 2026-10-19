//! Resolution-independent, antialiased native UI primitives.
use ab_glyph::{Font, FontRef, ScaleFont, VariableFont};
use std::sync::OnceLock;
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};
pub(super) struct Canvas {
    pub pix: Pixmap,
}
impl Canvas {
    pub fn new(px: &[[u8; 4]], w: usize, h: usize) -> Self {
        let mut pix = Pixmap::new(w as u32, h as u32).unwrap();
        for (d, s) in pix.data_mut().chunks_exact_mut(4).zip(px) {
            d.copy_from_slice(s);
            d[3] = 255;
        }
        Self { pix }
    }
    pub fn finish(self, px: &mut [[u8; 4]]) {
        for (d, s) in px.iter_mut().zip(self.pix.data().chunks_exact(4)) {
            d.copy_from_slice(s);
        }
    }
    pub fn path(&mut self, p: tiny_skia::Path, c: [u8; 3], stroke: f32) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], 255);
        if stroke > 0. {
            self.pix.stroke_path(
                &p,
                &paint,
                &Stroke {
                    width: stroke,
                    line_cap: tiny_skia::LineCap::Round,
                    line_join: tiny_skia::LineJoin::Round,
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        } else {
            self.pix.fill_path(
                &p,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    pub fn rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        top: [u8; 3],
        bottom: [u8; 3],
        border: [u8; 3],
    ) {
        if w <= 0. || h <= 0. {
            return;
        }
        let r = r.min(w / 2.).min(h / 2.);
        let mut p = PathBuilder::new();
        p.move_to(x + r, y);
        p.line_to(x + w - r, y);
        p.quad_to(x + w, y, x + w, y + r);
        p.line_to(x + w, y + h - r);
        p.quad_to(x + w, y + h, x + w - r, y + h);
        p.line_to(x + r, y + h);
        p.quad_to(x, y + h, x, y + h - r);
        p.line_to(x, y + r);
        p.quad_to(x, y, x + r, y);
        p.close();
        let path = p.finish().unwrap();
        let mut paint = Paint::default();
        paint.shader = tiny_skia::LinearGradient::new(
            tiny_skia::Point::from_xy(x, y),
            tiny_skia::Point::from_xy(x, y + h),
            vec![
                tiny_skia::GradientStop::new(
                    0.,
                    tiny_skia::Color::from_rgba8(top[0], top[1], top[2], 255),
                ),
                tiny_skia::GradientStop::new(
                    1.,
                    tiny_skia::Color::from_rgba8(bottom[0], bottom[1], bottom[2], 255),
                ),
            ],
            tiny_skia::SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        self.pix.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
        self.path(path, border, 1.);
    }
    pub fn circle(&mut self, x: f32, y: f32, r: f32, c: [u8; 3], edge: [u8; 3]) {
        let p = PathBuilder::from_circle(x, y, r).unwrap();
        self.path(p.clone(), c, 0.);
        self.path(p, edge, 1.5);
    }
    pub fn line(&mut self, x: f32, y: f32, xx: f32, yy: f32, c: [u8; 3], width: f32) {
        let mut p = PathBuilder::new();
        p.move_to(x, y);
        p.line_to(xx, yy);
        self.path(p.finish().unwrap(), c, width);
    }
    pub fn text(&mut self, x: f32, y: f32, s: &str, c: [u8; 3], size: f32, title: bool, max: f32) {
        static BODY: OnceLock<FontRef<'static>> = OnceLock::new();
        static TITLE: OnceLock<FontRef<'static>> = OnceLock::new();
        let font = if title {
            TITLE.get_or_init(|| {
                let mut f =
                    FontRef::try_from_slice(include_bytes!("../assets/fonts/Cinzel.ttf")).unwrap();
                f.set_variation(b"wght", 700.);
                f
            })
        } else {
            BODY.get_or_init(|| {
                let mut f =
                    FontRef::try_from_slice(include_bytes!("../assets/fonts/Inter.ttf")).unwrap();
                f.set_variation(b"wght", 600.);
                f
            })
        };
        let scaled = font.as_scaled(size);
        let mut pen = x;
        let mut last = None;
        for ch in s.chars() {
            let id = font.glyph_id(ch);
            if let Some(prev) = last {
                pen += scaled.kern(prev, id);
            }
            let adv = scaled.h_advance(id);
            if pen + adv > x + max {
                break;
            }
            let glyph = id.with_scale_and_position(size, ab_glyph::point(pen, y + scaled.ascent()));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                let w = self.pix.width() as i32;
                let h = self.pix.height() as i32;
                outline.draw(|gx, gy, a| {
                    let xx = bounds.min.x as i32 + gx as i32;
                    let yy = bounds.min.y as i32 + gy as i32;
                    if xx >= 0 && yy >= 0 && xx < w && yy < h {
                        let i = (yy as usize * w as usize + xx as usize) * 4;
                        let data = self.pix.data_mut();
                        for k in 0..3 {
                            data[i + k] =
                                (data[i + k] as f32 * (1. - a) + c[k] as f32 * a).round() as u8;
                        }
                    }
                });
            }
            pen += adv;
            last = Some(id);
        }
    }
}
