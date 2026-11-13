//! Windows: a file's icon as Explorer draws it (a program's, a shortcut's).

use std::path::Path;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetObjectW,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY};
use windows::core::HSTRING;

/// `path`'s icon, about `pixels` square: its width, height and straight
/// (not premultiplied) BGRA, the top row first.
pub fn bgra(path: &Path, pixels: u32) -> Option<(u32, u32, Vec<u8>)> {
    // SAFETY: COM is ended only where it was started here; `draw` frees what it makes.
    unsafe {
        let started = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let drawn = draw(path, pixels);
        if started {
            CoUninitialize();
        }
        drawn
    }
}

unsafe fn draw(path: &Path, pixels: u32) -> Option<(u32, u32, Vec<u8>)> {
    let side = pixels as i32;
    // SAFETY: the bitmap and the DC are freed before returning.
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let bitmap = factory.GetImage(SIZE { cx: side, cy: side }, SIIGBF_ICONONLY).ok()?;
        let mut shape = BITMAP::default();
        GetObjectW(bitmap.into(), size_of::<BITMAP>() as i32, Some(&mut shape as *mut BITMAP as *mut _));
        let (width, height) = (shape.bmWidth, shape.bmHeight.abs());
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative: the top row first.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut data = vec![0u8; (width.max(0) * height * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let rows = GetDIBits(dc, bitmap, 0, height as u32, Some(data.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(bitmap.into());
        if rows == 0 || width <= 0 {
            return None;
        }
        // The shell's are premultiplied; an old icon with no alpha at all is opaque.
        let opaque = data.chunks(4).all(|pixel| pixel[3] == 0);
        for pixel in data.chunks_mut(4) {
            match (opaque, pixel[3]) {
                (true, _) => pixel[3] = 255,
                (_, 0) => {}
                (_, alpha) => pixel[..3].iter_mut().for_each(|c| *c = (u32::from(*c) * 255 / u32::from(alpha)).min(255) as u8),
            }
        }
        Some((width as u32, height as u32, data))
    }
}
