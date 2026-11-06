//! Overlay window: the overlay on a solid color at the window's size, for OBS Window Capture + Color Key.

use std::time::Instant;

use anyhow::{Context, Result, bail};
use tiny_skia::{Color, Pixmap};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, ReleaseDC, SetDIBitsToDevice,
};
use winit::dpi::{PhysicalSize, Size};
use winit::event_loop::ActiveEventLoop;
use winit::platform::windows::{IconExtWindows, WindowAttributesExtWindows};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Icon, Window, WindowId};

use crate::render::glow::GlowCache;
use crate::render::{mul255, over};
use crate::scene::Scene;
use crate::widgets::Canvas;

pub struct ColorWindow {
    window: Window,
    hwnd: HWND,
    background: [u8; 3],
    /// Overlay layers at the window's scale.
    base: Option<Pixmap>,
    highlight: Option<Pixmap>,
    /// Whole client area, BGRA; the background is only refilled when `FrameLayout` changes.
    frame: Vec<u8>,
    filled: Option<FrameLayout>,
}

/// What the frame was filled for. A move of the overlay's area leaves old pixels outside the new one.
#[derive(Clone, Copy, PartialEq)]
struct FrameLayout {
    width: u32,
    height: u32,
    background: [u8; 3],
    origin: (u32, u32),
    overlay: (u32, u32),
}

impl ColorWindow {
    /// `size`: client area in physical px.
    pub fn open(event_loop: &ActiveEventLoop, size: [u32; 2], background: [u8; 3]) -> Result<Self> {
        let icon = Icon::from_resource(1, None).ok();
        let attributes = Window::default_attributes()
            .with_title("Overlay Input — Window")
            .with_inner_size(Size::Physical(PhysicalSize::new(size[0], size[1])))
            .with_min_inner_size(Size::Physical(PhysicalSize::new(120, 80)))
            .with_window_icon(icon.clone())
            .with_taskbar_icon(icon);
        let window = event_loop
            .create_window(attributes)
            .context("create overlay window")?;
        let hwnd = match window.window_handle()?.as_raw() {
            RawWindowHandle::Win32(handle) => HWND(handle.hwnd.get() as *mut _),
            _ => bail!("overlay window is not a Win32 window"),
        };
        window.request_redraw();
        Ok(Self {
            window,
            hwnd,
            background,
            base: None,
            highlight: None,
            frame: Vec::new(),
            filled: None,
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn focus(&self) {
        self.window.set_minimized(false);
        self.window.focus_window();
    }

    /// Client area, physical px.
    pub fn size(&self) -> [u32; 2] {
        let size = self.window.inner_size();
        [size.width, size.height]
    }

    pub fn set_background(&mut self, background: [u8; 3]) {
        self.background = background;
    }

    /// Draws `scene` fitted and centered at full opacity over the background, and shows it.
    pub fn render(&mut self, scene: &Scene, glow: &mut GlowCache, now: Instant) -> Result<()> {
        let [width, height] = self.size();
        let (sw, sh) = scene.size;
        if width == 0 || height == 0 || sw <= 0.0 || sh <= 0.0 {
            return Ok(()); // minimized
        }
        let scale = (width as f32 / sw).min(height as f32 / sh);
        let pw = ((sw * scale).ceil() as u32).clamp(1, width);
        let ph = ((sh * scale).ceil() as u32).clamp(1, height);
        let fits = |p: &Option<Pixmap>| {
            p.as_ref()
                .is_some_and(|p| (p.width(), p.height()) == (pw, ph))
        };
        if !fits(&self.base) {
            self.base = Pixmap::new(pw, ph);
        }
        let split = scene.highlight_opacity.is_some();
        if split && !fits(&self.highlight) {
            self.highlight = Pixmap::new(pw, ph);
        }
        let Some(base) = &mut self.base else {
            return Ok(());
        };
        base.fill(Color::TRANSPARENT);
        let mut highlight = if split { self.highlight.as_mut() } else { None };
        if let Some(layer) = highlight.as_deref_mut() {
            layer.fill(Color::TRANSPARENT);
        }
        let mut canvas = Canvas {
            pixmap: base,
            highlight,
            scale,
            glow,
        };
        scene.draw(&mut canvas, now);

        let origin = ((width - pw) / 2, (height - ph) / 2);
        fill_if_changed(
            &mut self.frame,
            &mut self.filled,
            FrameLayout {
                width,
                height,
                background: self.background,
                origin,
                overlay: (pw, ph),
            },
        );
        let highlight = if split { self.highlight.as_ref() } else { None };
        compose(
            &mut self.frame,
            width,
            origin,
            base,
            highlight,
            self.background,
        );
        self.blit(width, height);
        Ok(())
    }

    fn blit(&self, width: u32, height: u32) {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let dc = GetDC(Some(self.hwnd));
            if dc.is_invalid() {
                return;
            }
            SetDIBitsToDevice(
                dc,
                0,
                0,
                width,
                height,
                0,
                0,
                0,
                height,
                self.frame.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
            );
            ReleaseDC(Some(self.hwnd), dc);
        }
    }
}

fn fill_if_changed(frame: &mut Vec<u8>, filled: &mut Option<FrameLayout>, layout: FrameLayout) {
    if *filled != Some(layout) {
        let [r, g, b] = layout.background;
        *frame = [b, g, r, 255].repeat((layout.width * layout.height) as usize);
        *filled = Some(layout);
    }
}

/// Writes the overlay (base over highlight, both at full opacity) over `background` into the BGRA `frame`
/// at `origin`.
fn compose(
    frame: &mut [u8],
    frame_width: u32,
    origin: (u32, u32),
    base: &Pixmap,
    highlight: Option<&Pixmap>,
    background: [u8; 3],
) {
    let width = base.width() as usize;
    let bases = base.data().as_chunks::<4>().0;
    let highlights = highlight.map(|h| h.data().as_chunks::<4>().0);
    let [br, bg, bb] = background.map(u32::from);
    for (row, pixels) in bases.chunks_exact(width).enumerate() {
        let start = ((origin.1 as usize + row) * frame_width as usize + origin.0 as usize) * 4;
        let out = frame[start..start + width * 4].as_chunks_mut::<4>().0;
        for (x, (o, &p)) in out.iter_mut().zip(pixels).enumerate() {
            let p = match highlights {
                Some(h) => over(p, 255, h[row * width + x], 255),
                None => p,
            };
            let rest = 255 - p[3] as u32;
            // Premultiplied RGBA over an opaque color, out as BGRA.
            *o = [
                (p[2] as u32 + mul255(bb, rest)) as u8,
                (p[1] as u32 + mul255(bg, rest)) as u8,
                (p[0] as u32 + mul255(br, rest)) as u8,
                255,
            ];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::PremultipliedColorU8;

    #[test]
    fn a_smaller_overlay_leaves_nothing_of_the_bigger_one() {
        let (mut frame, mut filled) = (Vec::new(), None);
        let layout = |origin, overlay| FrameLayout {
            width: 4,
            height: 1,
            background: [0, 255, 0],
            origin,
            overlay,
        };
        let white = |origin, width, frame: &mut Vec<u8>| {
            let mut base = Pixmap::new(width, 1).unwrap();
            base.fill(Color::WHITE);
            compose(frame, 4, origin, &base, None, [0, 255, 0]);
        };
        let green = [0, 255, 0, 255];

        fill_if_changed(&mut frame, &mut filled, layout((0, 0), (4, 1)));
        white((0, 0), 4, &mut frame);
        fill_if_changed(&mut frame, &mut filled, layout((1, 0), (2, 1)));
        white((1, 0), 2, &mut frame);
        assert_eq!(frame[0..4], green, "left of the new overlay");
        assert_eq!(frame[4..8], [255, 255, 255, 255]);
        assert_eq!(frame[12..16], green, "right of the new overlay");

        // Same layout: no refill, so frames while nothing moves stay cheap.
        frame[0] = 7;
        fill_if_changed(&mut frame, &mut filled, layout((1, 0), (2, 1)));
        assert_eq!(frame[0], 7);
    }

    #[test]
    fn overlay_blends_onto_the_key_color() {
        let mut base = Pixmap::new(2, 1).unwrap();
        // Opaque white, then half-transparent white (premultiplied 128).
        base.pixels_mut()[0] = PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap();
        base.pixels_mut()[1] = PremultipliedColorU8::from_rgba(128, 128, 128, 128).unwrap();
        let mut frame = [0, 255, 0, 255].repeat(4); // 4x1, green (BGRA)
        compose(&mut frame, 4, (1, 0), &base, None, [0, 255, 0]);
        let px = |i: usize| &frame[i * 4..i * 4 + 4];
        assert_eq!(px(0), [0, 255, 0, 255], "background left alone");
        assert_eq!(px(1), [255, 255, 255, 255]);
        assert_eq!(px(2), [128, 255, 128, 255], "half white over green");
        assert_eq!(px(3), [0, 255, 0, 255]);
    }
}
