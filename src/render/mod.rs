//! Drawing helpers on top of tiny-skia: text, glow, SVG.

pub mod easing;
pub mod glow;
pub mod gradient;
pub mod svg;
pub mod text;

use tiny_skia::{Color, Path, PathBuilder, Pixmap};

/// Highlights under the base layer, each at its own opacity, into `out` (same size).
pub fn composite_layers(
    out: &mut Pixmap,
    base: &Pixmap,
    base_opacity: f32,
    highlight: &Pixmap,
    highlight_opacity: f32,
) {
    let (bo, ho) = (alpha_u8(base_opacity), alpha_u8(highlight_opacity));
    let pixels = base.data().as_chunks::<4>().0;
    let highlights = highlight.data().as_chunks::<4>().0;
    for ((o, b), h) in out
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(pixels)
        .zip(highlights)
    {
        *o = over(*b, bo, *h, ho);
    }
}

/// Opacity 0..=1 as 0..=255.
pub fn alpha_u8(opacity: f32) -> u32 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u32
}

/// One premultiplied RGBA pixel: `base` (times `bo`/255) over `highlight` (times `ho`/255).
#[inline]
pub fn over(base: [u8; 4], bo: u32, highlight: [u8; 4], ho: u32) -> [u8; 4] {
    // Most of an overlay is empty on both layers.
    if u32::from_ne_bytes(base) == 0 && u32::from_ne_bytes(highlight) == 0 {
        return [0; 4];
    }
    let scaled = |p: [u8; 4], o: u32| p.map(|c| mul255(c as u32, o) as u8);
    if highlight[3] == 0 {
        return scaled(base, bo);
    }
    let (b, h) = (scaled(base, bo), scaled(highlight, ho));
    let rest = 255 - b[3] as u32;
    [0, 1, 2, 3].map(|i| (b[i] as u32 + mul255(h[i] as u32, rest)) as u8)
}

/// `x * y / 255`, rounded (x, y in 0..=255).
#[inline]
pub fn mul255(x: u32, y: u32) -> u32 {
    let t = x * y + 128;
    (t + (t >> 8)) >> 8
}

/// Rounded rectangle path; radius is clamped to half the shorter side.
pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = r * 0.552_284_8; // cubic Bézier circle approximation
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// `#RRGGBB` → color with the given alpha (0..=1).
pub fn rgb(hex: u32, alpha: f32) -> Color {
    let channel = |shift: u32| ((hex >> shift) & 0xFF) as f32 / 255.0;
    Color::from_rgba(channel(16), channel(8), channel(0), alpha.clamp(0.0, 1.0))
        .unwrap_or(Color::TRANSPARENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_get_their_own_opacity() {
        let layer = |hex: u32| {
            let mut p = Pixmap::new(2, 1).unwrap();
            p.fill(rgb(hex, 1.0));
            p
        };
        let (base, highlight) = (layer(0xFFFFFF), layer(0x00FF00));
        let mut base_only = Pixmap::new(2, 1).unwrap();
        base_only.fill(Color::TRANSPARENT);
        let mut out = Pixmap::new(2, 1).unwrap();

        composite_layers(&mut out, &base_only, 0.3, &highlight, 1.0);
        assert_eq!(
            out.pixel(0, 0).unwrap().alpha(),
            255,
            "highlight alone at 1.0"
        );

        composite_layers(&mut out, &base, 0.3, &base_only, 1.0);
        let alpha = out.pixel(0, 0).unwrap().alpha();
        assert!((75..=78).contains(&alpha), "base alone at 0.3: {alpha}");

        composite_layers(&mut out, &base, 0.3, &highlight, 1.0);
        let p = out.pixel(0, 0).unwrap().demultiply();
        assert_eq!(p.alpha(), 255, "base drawn over the highlight");
        assert!(
            p.green() == 255 && p.red() > 60 && p.red() < 90,
            "30% white over green: {p:?}"
        );
    }
}
