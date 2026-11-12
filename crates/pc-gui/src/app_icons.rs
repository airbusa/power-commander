//! Entries' icons: macOS's applications (`….app`) show their own, as Finder
//! draws them, made in the background; everything else the Material theme's.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use pc_core::{Entry, icons};

use crate::Commander;

/// Applications' icons by path and size in pixels: none while being made,
/// or where it failed.
#[derive(Default)]
pub(crate) struct AppIcons(RefCell<HashMap<(PathBuf, u32), Option<Arc<RenderImage>>>>);

impl Commander {
    /// `entry`'s icon in panel `ix`, `size` high.
    pub(crate) fn entry_icon(&self, ix: usize, entry: &Entry, size: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let panel = self.panels[ix].tabs.current();
        if entry.is_app() && panel.remote().is_none() && !panel.in_archive() {
            // Twice as large: sharp on a Retina display.
            let pixels = (f32::from(size) * 2.).round() as u32;
            if let Some(icon) = self.app_icon(panel.folder().join(&entry.name), pixels, cx) {
                return img(icon).size(size).flex_none().into_any_element();
            }
        }
        let folder = entry.is_dir && !entry.is_app();
        let icon = icons::material().icon(&entry.display_name(), folder, !cx.theme().mode.is_dark());
        img(format!("material/{icon}")).size(size).flex_none().into_any_element()
    }

    /// The application's icon once made; the first ask starts making it.
    fn app_icon(&self, path: PathBuf, pixels: u32, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        let key = (path, pixels);
        if let Some(made) = self.app_icons.0.borrow().get(&key) {
            return made.clone();
        }
        self.app_icons.0.borrow_mut().insert(key.clone(), None);
        let path = key.0.clone();
        let making = cx.background_executor().spawn(async move { make(&path, pixels) });
        cx.spawn(async move |this, cx| {
            let made = making.await;
            let _ = this.update(cx, |this, cx| {
                if made.is_some() {
                    this.app_icons.0.borrow_mut().insert(key, made);
                    cx.notify();
                }
            });
        })
        .detach();
        None
    }
}

/// The icon Finder shows for `path`, `pixels` square, in BGRA as GPUI
/// takes images.
#[cfg(target_os = "macos")]
fn make(path: &Path, pixels: u32) -> Option<Arc<RenderImage>> {
    use objc2::AllocAnyThread;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext, NSWorkspace};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    // A thread of the executor's has no pool of its own.
    autoreleasepool(|_| {
        let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path.to_str()?));
        let side = pixels as isize;
        // SAFETY: no planes given, the rep has a buffer of its own; RGBA, 8 bits each.
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                side,
                side,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                0,
                0,
            )
        }?;
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&context));
        image.drawInRect(NSRect::new(NSPoint::new(0., 0.), NSSize::new(side as f64, side as f64)));
        context.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();
        let (data, stride) = (rep.bitmapData(), rep.bytesPerRow() as usize);
        if data.is_null() {
            return None;
        }
        // SAFETY: the rep's buffer, `pixels` rows of `stride` bytes, the top one first.
        let rows = unsafe { std::slice::from_raw_parts(data, stride * pixels as usize) };
        // The rep's colours are multiplied by their alpha; GPUI's are not.
        let mut bgra = image::RgbaImage::new(pixels, pixels);
        for (y, row) in rows.chunks(stride).enumerate() {
            for x in 0..pixels as usize {
                let [r, g, b, a] = [row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]];
                let straight = |c: u8| match a {
                    0 => 0,
                    _ => (u32::from(c) * 255 / u32::from(a)).min(255) as u8,
                };
                bgra.put_pixel(x as u32, y as u32, image::Rgba([straight(b), straight(g), straight(r), a]));
            }
        }
        Some(Arc::new(RenderImage::new([image::Frame::new(bgra)])))
    })
}

#[cfg(not(target_os = "macos"))]
fn make(_: &Path, _: u32) -> Option<Arc<RenderImage>> {
    None
}
