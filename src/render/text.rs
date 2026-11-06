//! System fonts with per-glyph fallback, and text → vector path (anti-aliased by tiny-skia at any scale).

use std::path::PathBuf;

use ab_glyph::{Font as _, FontVec, GlyphId, OutlineCurve, Point, PxScale, ScaleFont};
use anyhow::{Context, Result};
use tiny_skia::{Path, PathBuilder};

/// Primary face plus fallbacks for glyphs it lacks.
pub struct Font {
    faces: Vec<FontVec>,
}

impl Font {
    /// Loads every font found in `%WINDIR%\Fonts` from `candidates`; the first found is primary.
    pub fn load_system(candidates: &[&str]) -> Result<Self> {
        let dir = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()))
            .join("Fonts");
        let mut faces = Vec::new();
        for name in candidates {
            let path = dir.join(name);
            let Ok(bytes) = std::fs::read(&path) else {
                tracing::debug!(?path, "font not found");
                continue;
            };
            faces.push(FontVec::try_from_vec(bytes).with_context(|| format!("parse {path:?}"))?);
        }
        anyhow::ensure!(
            !faces.is_empty(),
            "none of the fonts {candidates:?} found in {dir:?}"
        );
        Ok(Self { faces })
    }

    /// Ascent and descent (negative) of the primary face at em size `px`.
    pub fn metrics(&self, px: f32) -> (f32, f32) {
        let scaled = self.faces[0].as_scaled(em_scale(&self.faces[0], px));
        (scaled.ascent(), scaled.descent())
    }

    pub fn width(&self, text: &str, px: f32) -> f32 {
        self.layout(text, px)
            .last()
            .map_or(0.0, |g| g.x + g.advance)
    }

    /// Path of `text` at em size `px`, centered in the box both ways.
    pub fn text_path(&self, text: &str, px: f32, x: f32, y: f32, w: f32, h: f32) -> Option<Path> {
        let (ascent, descent) = self.metrics(px);
        let left = x + (w - self.width(text, px)) / 2.0;
        let baseline = y + (h - (ascent - descent)) / 2.0 + ascent;
        self.line_path(text, px, left, baseline)
    }

    /// Path of `text` starting at `x` on `baseline`.
    pub fn line_path(&self, text: &str, px: f32, x: f32, baseline: f32) -> Option<Path> {
        let mut pb = PathBuilder::new();
        for g in self.layout(text, px) {
            let face = &self.faces[g.face];
            let factor = face.as_scaled(em_scale(face, px)).scale_factor();
            if let Some(outline) = face.outline(g.id) {
                let pen = x + g.x;
                let map = |p: Point| {
                    (
                        pen + p.x * factor.horizontal,
                        baseline - p.y * factor.vertical,
                    )
                };
                append_outline(&mut pb, &outline.curves, map);
            }
        }
        pb.finish()
    }

    fn layout(&self, text: &str, px: f32) -> Vec<Glyph> {
        let mut glyphs = Vec::new();
        let mut pen = 0.0;
        let mut prev: Option<(usize, GlyphId)> = None;
        for c in text.chars() {
            let (face, id) = self.face_for(c);
            let scaled = self.faces[face].as_scaled(em_scale(&self.faces[face], px));
            if let Some((pf, pid)) = prev
                && pf == face
            {
                pen += scaled.kern(pid, id);
            }
            let advance = scaled.h_advance(id);
            glyphs.push(Glyph {
                face,
                id,
                x: pen,
                advance,
            });
            pen += advance;
            prev = Some((face, id));
        }
        glyphs
    }

    fn face_for(&self, c: char) -> (usize, GlyphId) {
        self.faces
            .iter()
            .enumerate()
            .map(|(i, f)| (i, f.glyph_id(c)))
            .find(|(_, id)| id.0 != 0)
            .unwrap_or((0, self.faces[0].glyph_id(c)))
    }
}

struct Glyph {
    face: usize,
    id: GlyphId,
    x: f32,
    advance: f32,
}

/// `px` is the em size; ab_glyph PxScale is ascent-descent height.
fn em_scale(face: &FontVec, px: f32) -> PxScale {
    let upem = face.units_per_em().unwrap_or(1000.0);
    PxScale::from(px * face.height_unscaled() / upem)
}

/// Appends glyph curves; a new contour starts whenever a curve doesn't begin at the previous end.
fn append_outline(
    pb: &mut PathBuilder,
    curves: &[OutlineCurve],
    map: impl Fn(Point) -> (f32, f32),
) {
    let mut last_end: Option<Point> = None;
    for curve in curves {
        let (start, end) = match *curve {
            OutlineCurve::Line(a, b) => (a, b),
            OutlineCurve::Quad(a, _, b) => (a, b),
            OutlineCurve::Cubic(a, _, _, b) => (a, b),
        };
        if last_end != Some(start) {
            if last_end.is_some() {
                pb.close();
            }
            let (x, y) = map(start);
            pb.move_to(x, y);
        }
        match *curve {
            OutlineCurve::Line(_, b) => {
                let (x, y) = map(b);
                pb.line_to(x, y);
            }
            OutlineCurve::Quad(_, c, b) => {
                let ((cx, cy), (x, y)) = (map(c), map(b));
                pb.quad_to(cx, cy, x, y);
            }
            OutlineCurve::Cubic(_, c1, c2, b) => {
                let ((c1x, c1y), (c2x, c2y), (x, y)) = (map(c1), map(c2), map(b));
                pb.cubic_to(c1x, c1y, c2x, c2y, x, y);
            }
        }
        last_end = Some(end);
    }
    if last_end.is_some() {
        pb.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_for_symbols_missing_in_primary() {
        let font = Font::load_system(&["calibri.ttf", "seguisym.ttf"]).unwrap();
        if font.faces.len() < 2 {
            return; // fallback font not installed
        }
        assert_eq!(font.face_for('A').0, 0);
        assert_eq!(
            font.face_for('⏎').0,
            1,
            "Calibri has no ⏎; Segoe UI Symbol does"
        );
    }
}
