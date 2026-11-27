//! Minimal software renderer for the calibration window: a u32 pixel
//! buffer (softbuffer's XRGB8888), rectangles with optional alpha blending,
//! and text rasterized with ab_glyph from the embedded DejaVu fonts (the
//! `dejavu` crate), so no system font is needed on the target machine.

use ab_glyph::{Font, FontRef, PxScale, point};

/// Pack an RGB color into softbuffer's u32 XRGB8888 pixel.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

pub struct Canvas {
    pub buf: Vec<u32>,
    pub w: usize,
    pub h: usize,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas {
            buf: vec![0; w * h],
            w,
            h,
        }
    }

    pub fn fill(&mut self, c: u32) {
        self.buf.fill(c);
    }

    /// Opaque rectangle, clipped to the canvas.
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        self.blend_rect(x, y, w, h, c, 1.0);
    }

    /// Rectangle border (thickness `t`), clipped to the canvas.
    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, t: i32, c: u32) {
        self.rect(x, y, w, t, c);
        self.rect(x, y + h - t, w, t, c);
        self.rect(x, y, t, h, c);
        self.rect(x + w - t, y, t, h, c);
    }

    /// Source-over alpha-blended rectangle, clipped to the canvas.
    pub fn blend_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32, alpha: f32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = (x + w).min(self.w as i32) as usize;
        let y1 = (y + h).min(self.h as i32) as usize;
        let (sr, sg, sb) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
        for py in y0..y1 {
            for px in x0..x1 {
                let dst = self.buf[py * self.w + px];
                let (dr, dg, db) = ((dst >> 16) & 0xFF, (dst >> 8) & 0xFF, dst & 0xFF);
                let a = alpha.clamp(0.0, 1.0);
                let blend = |s: u32, d: u32| (s as f32 * a + d as f32 * (1.0 - a)).round() as u32;
                self.buf[py * self.w + px] =
                    0xFF00_0000 | blend(sr, dr) << 16 | blend(sg, dg) << 8 | blend(sb, db);
            }
        }
    }

    fn plot_blend(&mut self, x: i32, y: i32, c: u32, v: f32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 || v <= 0.0 {
            return;
        }
        let i = y as usize * self.w + x as usize;
        let dst = self.buf[i];
        let (sr, sg, sb) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
        let (dr, dg, db) = ((dst >> 16) & 0xFF, (dst >> 8) & 0xFF, dst & 0xFF);
        let blend = |s: u32, d: u32| (s as f32 * v + d as f32 * (1.0 - v)).round() as u32;
        self.buf[i] = 0xFF00_0000 | blend(sr, dr) << 16 | blend(sg, dg) << 8 | blend(sb, db);
    }

    /// Width of `s` at pixel size `px`, in pixels.
    pub fn text_width(&self, font: &FontRef, px: f32, s: &str) -> f32 {
        let upem = font.units_per_em().unwrap_or(1000.0);
        s.chars()
            .map(|c| font.h_advance_unscaled(font.glyph_id(c)) * px / upem)
            .sum()
    }

    /// Draw `s` with its baseline at `(x, y_baseline)`. Returns the advance
    /// width in pixels.
    pub fn text(
        &mut self,
        font: &FontRef,
        px: f32,
        x: f32,
        y_baseline: f32,
        s: &str,
        c: u32,
    ) -> f32 {
        let upem = font.units_per_em().unwrap_or(1000.0);
        let scale = PxScale::from(px);
        let mut pen = x;
        for ch in s.chars() {
            let id = font.glyph_id(ch);
            let glyph = id.with_scale_and_position(scale, point(pen, y_baseline));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|dx, dy, v| {
                    self.plot_blend(
                        bounds.min.x as i32 + dx as i32,
                        bounds.min.y as i32 + dy as i32,
                        c,
                        v,
                    );
                });
            }
            pen += font.h_advance_unscaled(id) * px / upem;
        }
        pen - x
    }

    /// Draw `s` centered horizontally around `cx`, baseline at `y`.
    pub fn text_centered(&mut self, font: &FontRef, px: f32, cx: f32, y: f32, s: &str, c: u32) {
        let w = self.text_width(font, px, s);
        self.text(font, px, cx - w / 2.0, y, s, c);
    }

    /// Ascent of the font at pixel size `px` (top of the line to baseline).
    pub fn ascent(font: &FontRef, px: f32) -> f32 {
        let upem = font.units_per_em().unwrap_or(1000.0);
        font.ascent_unscaled() * px / upem
    }
}

/// The embedded fonts used by the calibration UI.
pub struct Fonts {
    pub sans: FontRef<'static>,
    pub sans_bold: FontRef<'static>,
    pub mono: FontRef<'static>,
}

impl Fonts {
    pub fn load() -> Result<Fonts, ab_glyph::InvalidFont> {
        Ok(Fonts {
            sans: FontRef::try_from_slice(dejavu::sans::regular())?,
            sans_bold: FontRef::try_from_slice(dejavu::sans::bold())?,
            mono: FontRef::try_from_slice(dejavu::sans_mono::regular())?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_fills_and_clips() {
        let mut c = Canvas::new(10, 10);
        c.fill(rgb(0, 0, 0));
        c.rect(-5, -5, 8, 8, rgb(255, 0, 0));
        assert_eq!(c.buf[0], rgb(255, 0, 0), "clipped corner is filled");
        assert_eq!(c.buf[2], rgb(255, 0, 0), "still inside at x=2");
        assert_eq!(c.buf[3], rgb(0, 0, 0), "x=3 is outside the rect");
        assert_eq!(c.buf[8], rgb(0, 0, 0), "outside the rect stays black");
        c.rect(8, 8, 10, 10, rgb(0, 255, 0));
        assert_eq!(c.buf[99], rgb(0, 255, 0));
        assert_eq!(c.buf[88], rgb(0, 255, 0), "corner (8,8) is inside");
        assert_eq!(c.buf[77], rgb(0, 0, 0), "just outside stays black");
    }

    #[test]
    fn blend_rect_mixes_colors() {
        let mut c = Canvas::new(4, 4);
        c.fill(rgb(0, 0, 0));
        c.blend_rect(0, 0, 4, 4, rgb(255, 255, 255), 0.5);
        assert_eq!(c.buf[0], rgb(128, 128, 128));
    }

    #[test]
    fn frame_draws_only_the_border() {
        let mut c = Canvas::new(10, 10);
        c.fill(rgb(0, 0, 0));
        c.frame(2, 2, 6, 6, 1, rgb(255, 255, 255));
        assert_eq!(c.buf[2 * 10 + 2], rgb(255, 255, 255));
        assert_eq!(c.buf[4 * 10 + 4], rgb(0, 0, 0), "inside stays empty");
    }

    #[test]
    fn text_renders_pixels_and_measures_width() {
        let fonts = Fonts::load().unwrap();
        let mut c = Canvas::new(400, 60);
        c.fill(rgb(0, 0, 0));
        let w = c.text(
            &fonts.sans,
            28.0,
            10.0,
            40.0,
            "Rouge 12/24",
            rgb(255, 255, 255),
        );
        assert!(w > 50.0, "width {w}");
        assert!(
            c.buf.iter().any(|&p| p != rgb(0, 0, 0)),
            "text must paint pixels"
        );
        let wide = c.text_width(&fonts.sans, 28.0, "WWWW");
        let narrow = c.text_width(&fonts.sans, 28.0, "iiii");
        assert!(wide > narrow, "W is wider than i: {wide} vs {narrow}");
        // Accented French renders (é in "Précédent").
        let mut c2 = Canvas::new(400, 60);
        c2.text(
            &fonts.sans,
            28.0,
            10.0,
            40.0,
            "Précédent",
            rgb(255, 255, 255),
        );
        assert!(c2.buf.iter().any(|&p| p != rgb(0, 0, 0)));
    }

    #[test]
    fn fonts_load_from_the_embedded_dejavu() {
        let f = Fonts::load().unwrap();
        assert!(f.sans.units_per_em().is_some());
        assert!(f.sans.glyph_id('é').0 != 0, "accented glyphs must exist");
    }
}
