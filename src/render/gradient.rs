//! Color gradient tables.

use serde::Deserialize;
use tiny_skia::Color;

#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stop {
    /// Position on the scale (intensity 0 to 1, speed, ...).
    pub at: f32,
    #[schemars(extend("format" = "color"))]
    pub rgb: [u8; 3],
    #[serde(default = "one")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub opacity: f32,
}

fn one() -> f32 {
    1.0
}

impl Stop {
    pub const fn new(at: f32, rgb: [u8; 3], opacity: f32) -> Self {
        Self { at, rgb, opacity }
    }
}

/// Interpolates between the stops around `x`, rounding channels.
/// Below the first stop → first stop; above the last → last. `stops` must be sorted by `at`.
pub fn sample(stops: &[Stop], x: f32) -> Color {
    let Some(first) = stops.first() else {
        return Color::TRANSPARENT;
    };
    let next_i = stops.iter().position(|s| s.at >= x);
    let (curr, next) = match next_i {
        Some(0) => (first, first),
        Some(i) => (&stops[i - 1], &stops[i]),
        None => (stops.last().unwrap_or(first), stops.last().unwrap_or(first)),
    };
    let delta = next.at - curr.at;
    let t = if delta > 0.0 {
        (x - curr.at) / delta
    } else {
        0.0
    };
    let lerp =
        |a: u8, b: u8| (a as f32 + ((b as f32 - a as f32) * t).round()).clamp(0.0, 255.0) as u8;
    let opacity = curr.opacity + (next.opacity - curr.opacity) * t;
    Color::from_rgba8(
        lerp(curr.rgb[0], next.rgb[0]),
        lerp(curr.rgb[1], next.rgb[1]),
        lerp(curr.rgb[2], next.rgb[2]),
        (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_and_clamps() {
        let stops = [
            Stop::new(0.0, [0, 0, 0], 0.0),
            Stop::new(1.0, [200, 100, 50], 1.0),
        ];
        let mid = sample(&stops, 0.5).to_color_u8();
        assert_eq!(
            (mid.red(), mid.green(), mid.blue(), mid.alpha()),
            (100, 50, 25, 128)
        );
        assert_eq!(sample(&stops, 2.0).to_color_u8().red(), 200);
        assert_eq!(sample(&stops, -1.0).to_color_u8().alpha(), 0);
    }
}
