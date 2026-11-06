//! Shadows (blur, spread, color) as cached blurred sprites.

use std::collections::HashMap;

use tiny_skia::{Color, FillRule, Paint, Pixmap, PixmapPaint, PremultipliedColorU8, Transform};

use super::rounded_rect;

#[derive(Clone, Copy)]
pub struct GlowSpec {
    pub blur: f32,
    pub spread: f32,
    pub rgb: [u8; 3],
}

#[derive(Hash, PartialEq, Eq)]
struct SpriteKey {
    w: u32,
    h: u32,
    radius: u32,
    spread: u32,
    blur: u32,
    rgb: [u8; 3],
}

/// Pre-blurred image plus where its top-left sits relative to the owner's origin (physical px).
pub struct Sprite {
    pub pixmap: Pixmap,
    pub offset: (i32, i32),
}

/// Sprites are built per physical size, so cache is keyed on scaled pixels.
#[derive(Default)]
pub struct GlowCache {
    sprites: HashMap<SpriteKey, (Pixmap, i32)>,
    custom: HashMap<(String, u32), Sprite>,
}

impl GlowCache {
    /// Widget-defined sprite (e.g. SVG glow), built once per `key` and scale.
    pub fn sprite(&mut self, key: &str, scale: f32, build: impl FnOnce() -> Sprite) -> &Sprite {
        self.custom
            .entry((key.to_owned(), scale.to_bits()))
            .or_insert_with(build)
    }

    /// Draws a sprite with its owner's origin at physical `(x, y)`.
    pub fn blit(pixmap: &mut Pixmap, sprite: &Sprite, x: f32, y: f32, opacity: f32) {
        pixmap.draw_pixmap(
            x.round() as i32 + sprite.offset.0,
            y.round() as i32 + sprite.offset.1,
            sprite.pixmap.as_ref(),
            &PixmapPaint {
                opacity,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    /// Draws glow behind the logical box (x, y, w, h) with corner `radius`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        spec: GlowSpec,
        scale: f32,
    ) {
        let key = SpriteKey {
            w: (w * scale).round() as u32,
            h: (h * scale).round() as u32,
            radius: (radius * scale).round() as u32,
            spread: (spec.spread * scale).round() as u32,
            blur: (spec.blur * scale).round() as u32,
            rgb: spec.rgb,
        };
        let (sprite, origin) = self.sprites.entry(key).or_insert_with_key(build_sprite);
        pixmap.draw_pixmap(
            (x * scale).round() as i32 - *origin,
            (y * scale).round() as i32 - *origin,
            sprite.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
}

/// Returns the sprite and how far its top-left sits outside the box.
fn build_sprite(key: &SpriteKey) -> (Pixmap, i32) {
    let spread = key.spread as f32;
    let blur = key.blur as f32;
    let pad = (blur * 1.5).ceil(); // gaussian tail beyond the blur radius
    let origin = spread + pad;
    let (w, h) = (key.w as f32 + 2.0 * origin, key.h as f32 + 2.0 * origin);

    let mut mask = Pixmap::new(w as u32, h as u32).expect("glow sprite size");
    if let Some(shape) = rounded_rect(
        pad,
        pad,
        key.w as f32 + 2.0 * spread,
        key.h as f32 + 2.0 * spread,
        key.radius as f32 + spread,
    ) {
        let mut paint = Paint::default();
        paint.set_color(Color::WHITE);
        mask.fill_path(
            &shape,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    // Blur radius B => gaussian sigma B/2.
    blur_into_color(&mut mask, blur / 2.0, key.rgb);
    (mask, origin as i32)
}

/// Gaussian-blurs the alpha of `mask` (approximated by three box blurs) and recolors it to `rgb`.
/// Leave transparent padding of about 3·sigma so the blur isn't clipped.
pub fn blur_into_color(mask: &mut Pixmap, sigma: f32, rgb: [u8; 3]) {
    blur_with(mask, sigma, |_| rgb);
}

/// Glow: blur, then a color matrix of rgb + 2·alpha in linearRGB.
/// Faint tails stay close to `source`; dense areas saturate to white.
pub fn blur_into_svg_glow(mask: &mut Pixmap, sigma: f32, source: [u8; 3]) {
    let linear = source.map(|c| srgb_to_linear(c as f32 / 255.0));
    blur_with(mask, sigma, |a| {
        linear.map(|l| (linear_to_srgb((l + 2.0 * a).min(1.0)) * 255.0).round() as u8)
    });
}

/// Blurs the alpha of `mask`; `color(alpha 0..=1)` gives the straight rgb for each pixel.
fn blur_with(mask: &mut Pixmap, sigma: f32, color: impl Fn(f32) -> [u8; 3]) {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let mut alpha: Vec<f32> = mask.pixels().iter().map(|p| p.alpha() as f32).collect();
    for size in box_sizes_for_gauss(sigma, 3) {
        let r = (size - 1) / 2;
        alpha = box_blur(&alpha, w, h, r, true);
        alpha = box_blur(&alpha, w, h, r, false);
    }
    for (px, a) in mask.pixels_mut().iter_mut().zip(alpha) {
        let a = a.round().clamp(0.0, 255.0) as u8;
        let [r, g, b] = color(a as f32 / 255.0);
        let pm = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
        *px = PremultipliedColorU8::from_rgba(pm(r), pm(g), pm(b), a).expect("premultiplied");
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// Box widths whose 3-pass convolution approximates a gaussian (Kovesi / Ivan Kutskir).
fn box_sizes_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    if sigma <= 0.0 {
        return vec![1; n];
    }
    let nf = n as f32;
    let w_ideal = (12.0 * sigma * sigma / nf + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let wlf = wl as f32;
    let m = ((12.0 * sigma * sigma - nf * wlf * wlf - 4.0 * nf * wlf - 3.0 * nf)
        / (-4.0 * wlf - 4.0))
        .round() as i32;
    (0..n as i32)
        .map(|i| if i < m { wl } else { wu }.max(1) as usize)
        .collect()
}

/// One box-blur pass (running sum); pixels outside the buffer count as transparent.
fn box_blur(src: &[f32], w: usize, h: usize, r: usize, horizontal: bool) -> Vec<f32> {
    let mut out = vec![0.0; src.len()];
    let norm = 1.0 / (2 * r + 1) as f32;
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let index = |line: usize, i: usize| {
        if horizontal {
            line * w + i
        } else {
            i * w + line
        }
    };
    for line in 0..lines {
        let at = |i: isize| {
            if i >= 0 && (i as usize) < len {
                src[index(line, i as usize)]
            } else {
                0.0
            }
        };
        let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for i in 0..len {
            out[index(line, i)] = sum * norm;
            sum += at(i as isize + r as isize + 1) - at(i as isize - r as isize);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_blur_preserves_mass_and_spreads() {
        let mut src = vec![0.0; 9];
        src[4] = 9.0;
        let out = box_blur(&src, 9, 1, 1, true);
        assert_eq!(&out[3..6], &[3.0, 3.0, 3.0]);
        assert!((out.iter().sum::<f32>() - 9.0).abs() < 1e-4);
    }
}
