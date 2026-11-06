//! SVG via usvg, flattened to logical-px shapes tagged with element/group ids so widgets can restyle parts.

use anyhow::{Context, Result};
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Mask, Paint, Path, Pixmap, Stroke, Transform};

use super::glow::{Sprite, blur_into_svg_glow};

pub struct SvgShape {
    pub path: Path,
    pub fill: Option<Color>,
    pub stroke: Option<(Color, Stroke)>,
    /// Ids of the element and its ancestor groups, innermost first.
    pub ids: Vec<String>,
}

impl SvgShape {
    pub fn has_id(&self, id: &str) -> bool {
        self.ids.iter().any(|i| i == id)
    }

    /// Draws fill (or `fill` override, like an inline `style.fill`) then stroke, both times `opacity`.
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        ts: Transform,
        opacity: f32,
        fill: Option<Color>,
        mask: Option<&Mask>,
    ) {
        self.draw_fill(pixmap, ts, opacity, fill, mask);
        self.draw_stroke(pixmap, ts, opacity, mask);
    }

    /// Fill only: `fill` override, else the SVG's own fill.
    pub fn draw_fill(
        &self,
        pixmap: &mut Pixmap,
        ts: Transform,
        opacity: f32,
        fill: Option<Color>,
        mask: Option<&Mask>,
    ) {
        if let Some(mut color) = fill.or(self.fill) {
            color.apply_opacity(opacity);
            let mut paint = Paint::default();
            paint.set_color(color);
            pixmap.fill_path(&self.path, &paint, FillRule::Winding, ts, mask);
        }
    }

    pub fn draw_stroke(
        &self,
        pixmap: &mut Pixmap,
        ts: Transform,
        opacity: f32,
        mask: Option<&Mask>,
    ) {
        if let Some((mut color, stroke)) = self.stroke.clone() {
            color.apply_opacity(opacity);
            let mut paint = Paint::default();
            paint.set_color(color);
            pixmap.stroke_path(&self.path, &paint, &stroke, ts, mask);
        }
    }
}

/// Glow (standard deviation in SVG units) around `parts` filled with `fill`; offset is from the widget origin.
pub fn glow_sprite(
    parts: &[&SvgShape],
    fill: [u8; 3],
    std_dev: f32,
    unit: f32,
    scale: f32,
) -> Sprite {
    let sigma = std_dev * unit * scale;
    let pad = (3.0 * sigma).ceil();
    let bounds = parts
        .iter()
        .map(|s| s.path.bounds())
        .reduce(|a, b| a.join(&b).unwrap_or(a));
    let Some(bounds) = bounds else {
        return Sprite {
            pixmap: Pixmap::new(1, 1).expect("1x1"),
            offset: (0, 0),
        };
    };
    let (bx, by) = (
        (bounds.left() * scale).floor(),
        (bounds.top() * scale).floor(),
    );
    let w = (bounds.width() * scale).ceil() + 2.0 * pad + 2.0;
    let h = (bounds.height() * scale).ceil() + 2.0 * pad + 2.0;
    let mut mask = Pixmap::new(w as u32, h as u32).expect("glow sprite size");
    let ts = Transform::from_row(scale, 0.0, 0.0, scale, pad - bx, pad - by);
    let color = Color::from_rgba8(fill[0], fill[1], fill[2], 255);
    for part in parts {
        part.draw(&mut mask, ts, 1.0, Some(color), None);
    }
    // Blur, then the filter's color matrix (linearRGB) pushes dense areas to white.
    blur_into_svg_glow(&mut mask, sigma, fill);
    Sprite {
        pixmap: mask,
        offset: ((bx - pad) as i32, (by - pad) as i32),
    }
}

pub struct SvgImage {
    /// Document order (paint order).
    pub shapes: Vec<SvgShape>,
    /// Logical px, excluding `origin`.
    pub size: (f32, f32),
    /// One SVG user unit in logical px.
    pub unit: f32,
}

/// Parses `svg` and fits it to `width` logical px with its top-left at `origin`.
pub fn load(svg: &str, width: f32, origin: (f32, f32)) -> Result<SvgImage> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).context("parse svg")?;
    let size = tree.size();
    let scale = width / size.width();
    let base = Transform::from_translate(origin.0, origin.1).pre_scale(scale, scale);

    let mut shapes = Vec::new();
    let mut unit = scale;
    collect(
        tree.root(),
        base,
        1.0,
        &mut Vec::new(),
        &mut shapes,
        &mut unit,
    );
    Ok(SvgImage {
        shapes,
        size: (width, size.height() * scale),
        unit,
    })
}

fn collect(
    group: &usvg::Group,
    base: Transform,
    opacity: f32,
    ids: &mut Vec<String>,
    out: &mut Vec<SvgShape>,
    unit: &mut f32,
) {
    let opacity = opacity * group.opacity().get();
    let pushed = !group.id().is_empty();
    if pushed {
        ids.push(group.id().to_owned());
    }
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect(g, base, opacity, ids, out, unit),
            usvg::Node::Path(p) if p.is_visible() => {
                let ts = base.pre_concat(p.abs_transform());
                let Some(path) = p.data().clone().transform(ts) else {
                    continue;
                };
                let linear = (ts.sx * ts.sy - ts.kx * ts.ky).abs().sqrt();
                *unit = linear; // user unit → logical px (same for every path in our SVGs)
                let fill = p
                    .fill()
                    .and_then(|f| paint_color(f.paint(), f.opacity().get() * opacity));
                let stroke = p.stroke().and_then(|s| {
                    let color = paint_color(s.paint(), s.opacity().get() * opacity)?;
                    Some((color, stroke_style(s, linear)))
                });
                let mut shape_ids = vec![p.id().to_owned()];
                shape_ids.extend(ids.iter().rev().cloned());
                shape_ids.retain(|i| !i.is_empty());
                out.push(SvgShape {
                    path,
                    fill,
                    stroke,
                    ids: shape_ids,
                });
            }
            _ => {}
        }
    }
    if pushed {
        ids.pop();
    }
}

fn paint_color(paint: &usvg::Paint, opacity: f32) -> Option<Color> {
    match paint {
        usvg::Paint::Color(c) => Some(Color::from_rgba8(
            c.red,
            c.green,
            c.blue,
            (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
        )),
        _ => None, // gradients/patterns not used by the bundled SVGs
    }
}

fn stroke_style(s: &usvg::Stroke, scale: f32) -> Stroke {
    Stroke {
        width: s.width().get() * scale,
        miter_limit: s.miterlimit().get(),
        line_cap: match s.linecap() {
            usvg::LineCap::Butt => LineCap::Butt,
            usvg::LineCap::Round => LineCap::Round,
            usvg::LineCap::Square => LineCap::Square,
        },
        line_join: match s.linejoin() {
            usvg::LineJoin::Miter => LineJoin::Miter,
            usvg::LineJoin::MiterClip => LineJoin::MiterClip,
            usvg::LineJoin::Round => LineJoin::Round,
            usvg::LineJoin::Bevel => LineJoin::Bevel,
        },
        dash: None,
    }
}
