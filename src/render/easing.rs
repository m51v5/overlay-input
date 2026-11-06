//! Timing functions.

/// `cubic-bezier(x1, y1, x2, y2)` evaluated at progress `t` in 0..=1.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let bez = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let t = t.clamp(0.0, 1.0);
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if bez(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(y1, y2, (lo + hi) / 2.0)
}

/// Ease-out curve.
pub fn ease_out(t: f32) -> f32 {
    cubic_bezier(0.0, 0.0, 0.58, 1.0, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_endpoints_and_shape() {
        assert!(ease_out(0.0).abs() < 1e-4);
        assert!((ease_out(1.0) - 1.0).abs() < 1e-4);
        assert!(ease_out(0.5) > 0.5, "ease-out runs ahead of linear");
    }
}
