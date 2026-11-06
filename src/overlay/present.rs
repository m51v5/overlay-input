//! Copies a tiny-skia pixmap into a DIB and pushes it with `UpdateLayeredWindow` (per-pixel alpha).

use std::ffi::c_void;
use std::ptr;

use anyhow::{Context, Result};
use tiny_skia::Pixmap;

use crate::render::{alpha_u8, over};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};

pub struct LayeredSurface {
    dib: Option<Dib>,
}

struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: u32,
    height: u32,
}

impl LayeredSurface {
    pub fn new() -> Self {
        Self { dib: None }
    }

    /// `highlight` (layer, its opacity) is blended under `pixmap` per pixel; then the window adds no opacity.
    pub fn present(
        &mut self,
        hwnd: HWND,
        pos: POINT,
        pixmap: &Pixmap,
        opacity: f32,
        highlight: Option<(&Pixmap, f32)>,
    ) -> Result<()> {
        let (width, height) = (pixmap.width(), pixmap.height());
        if !matches!(&self.dib, Some(d) if d.width == width && d.height == height) {
            self.dib = None; // free old DIB before allocating the new one
            self.dib = Some(Dib::new(width, height)?);
        }
        let dib = self.dib.as_ref().expect("dib allocated above");

        // tiny-skia: premultiplied RGBA; GDI: premultiplied BGRA.
        let len = (width * height * 4) as usize;
        let dst = unsafe { std::slice::from_raw_parts_mut(dib.bits, len) };
        let dst = dst.as_chunks_mut::<4>().0.iter_mut();
        let src = pixmap.data().as_chunks::<4>().0;
        let window_opacity = match highlight {
            Some((layer, highlight_opacity)) => {
                let (bo, ho) = (alpha_u8(opacity), alpha_u8(highlight_opacity));
                for ((d, s), h) in dst.zip(src).zip(layer.data().as_chunks::<4>().0) {
                    let [r, g, b, a] = over(*s, bo, *h, ho);
                    *d = [b, g, r, a];
                }
                1.0
            }
            None => {
                for (d, s) in dst.zip(src) {
                    *d = [s[2], s[1], s[0], s[3]];
                }
                opacity
            }
        };

        let size = SIZE {
            cx: width as i32,
            cy: height as i32,
        };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha_u8(window_opacity) as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&pos),
                Some(&size),
                Some(dib.dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
        .context("UpdateLayeredWindow")
    }
}

impl Dib {
    fn new(width: u32, height: u32) -> Result<Self> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down rows, same as tiny-skia
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                anyhow::bail!("CreateCompatibleDC failed");
            }
            let mut bits: *mut c_void = ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(b) => b,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(e).context("CreateDIBSection");
                }
            };
            let previous = SelectObject(dc, bitmap.into());
            Ok(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
