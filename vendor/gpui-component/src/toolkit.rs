// SparkCommander patch: this file is new.
use std::sync::Arc;

use gpui::{
    Bounds, Corners, Div, Global, InteractiveElement, ParentElement, Pixels, RenderImage,
    StatefulInteractiveElement, Styled, canvas, div, point, px,
};

/// Controls drawn by the desktop's own toolkit: its Qt style (KDE's
/// Application Style) or GTK 3 theme (Xfce's, MATE's, Cinnamon's); images of
/// each state, shown in place of GPUI Kit's drawing, or of
/// `NativeControls`'. Images are in device pixels, `scale` to a pixel.
#[derive(Clone)]
pub struct ToolkitControls {
    pub scale: f32,
    /// Check boxes, unchecked and checked.
    pub checkbox: [StateImages; 2],
    /// Push buttons: their left end, a strip of the middle, the right end;
    /// plain, and the default button (a dialog's OK).
    pub button: [[StateImages; 3]; 2],
}

/// A control's look when normal, hovered, pressed and disabled.
#[derive(Clone)]
pub struct StateImages(pub [Arc<RenderImage>; 4]);

impl Global for ToolkitControls {}

impl ToolkitControls {
    /// An image's size in the window's pixels.
    pub fn size(&self, image: &RenderImage) -> (Pixels, Pixels) {
        let size = image.size(0);
        let logical = |pixels: i32| px(pixels as f32 / self.scale);
        (logical(size.width.0), logical(size.height.0))
    }
}

/// The groups whose hover and press the layers follow.
pub(crate) const CHECKBOX: &str = "toolkit-checkbox";
pub(crate) const BUTTON: &str = "toolkit-button";

pub(crate) const NORMAL: usize = 0;
pub(crate) const HOVERED: usize = 1;
pub(crate) const PRESSED: usize = 2;
pub(crate) const DISABLED: usize = 3;

/// A control's layers in `group`, `layer` making each state's: the normal
/// one, the hovered and pressed ones shown while the pointer is over it or
/// presses it; only the disabled one when disabled.
pub(crate) fn layers(
    group: &'static str,
    disabled: bool,
    layer: impl Fn(usize) -> Div,
) -> Div {
    let base = div().absolute().inset_0();
    if disabled {
        return base.child(layer(DISABLED));
    }
    let over = |state| layer(state).absolute().inset_0().opacity(0.);
    base.child(layer(NORMAL))
        .child(over(HOVERED).group_hover(group, |style| style.opacity(1.)))
        .child(
            over(PRESSED)
                .id("qt-pressed")
                // A cursor gives it a hitbox, without which GPUI tracks no press.
                .cursor_default()
                .group_active(group, |style| style.opacity(1.)),
        )
}

/// An image filling its place.
pub(crate) fn picture(image: Arc<RenderImage>) -> Div {
    painted(move |bounds| vec![(image, bounds, bounds)])
}

/// A button's background in `state`: its ends as drawn, the middle stretched between.
pub(crate) fn button_layer(controls: &ToolkitControls, parts: &[StateImages; 3], state: usize) -> Div {
    let [left, middle, right] = parts.clone().map(|part| part.0[state].clone());
    let (left_width, _) = controls.size(&left);
    let (right_width, _) = controls.size(&right);
    painted(move |Bounds { origin, size }| {
        let middle_width = (size.width - left_width - right_width).max(px(0.));
        let part = |x: Pixels, width: Pixels| Bounds::new(point(origin.x + x, origin.y), gpui::size(width, size.height));
        let (left_part, middle_part, right_part) =
            (part(px(0.), left_width), part(left_width, middle_width), part(left_width + middle_width, right_width));
        // The strip's middle half stretched: its edges would blend in the atlas' next image.
        let strip = part(left_width - middle_width / 2., middle_width * 2.);
        vec![(left, left_part, left_part), (middle, middle_part, strip), (right, right_part, right_part)]
    })
}

/// Images painted where `place` puts them in the element's bounds, each
/// shown in its first bounds, laid out in its second: an `img` would take
/// its height from its own shape.
type Placed = (Arc<RenderImage>, Bounds<Pixels>, Bounds<Pixels>);
fn painted(place: impl FnOnce(Bounds<Pixels>) -> Vec<Placed> + 'static) -> Div {
    let canvas = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            for (image, shown, at) in place(bounds) {
                let _ = window.paint_image(shown, at, Corners::default(), image, 0, false);
            }
        },
    );
    div().size_full().child(canvas.absolute().inset_0())
}
