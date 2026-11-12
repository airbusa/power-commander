// PowerCommander patch: this file is new.
use gpui::{Global, Hsla, Pixels};

/// Controls drawn as the desktop's own style draws them (KDE's Breeze,
/// Windows 11's): its colours and the sizes that differ; without it, GPUI
/// Kit's own look.
#[derive(Clone, Copy, Debug)]
pub struct NativeControls {
    /// Check boxes and radio buttons, and a radio button's dot.
    pub check_size: Pixels,
    pub radio_dot: Pixels,
    /// Unchecked, their frame and fill; checked; the mark or dot on them.
    pub check_frame: Hsla,
    pub check_fill: Hsla,
    pub checked_frame: Hsla,
    pub checked_fill: Hsla,
    pub mark: Hsla,
    /// Sliders' and progress bars' groove, and what fills it.
    pub groove: Hsla,
    pub fill: Hsla,
    /// A slider's groove; a progress bar's groove and fill.
    pub slider_groove: Pixels,
    pub progress: [Pixels; 2],
    /// A slider's 20 px handle, its middle, and the rim around the middle.
    pub handle: Hsla,
    pub handle_middle: Hsla,
    pub handle_rim: Pixels,
    /// Windows': the handle's frame.
    pub handle_frame: Option<Hsla>,
    /// Push buttons' fill and frame, normal, hovered and pressed; the default
    /// button's (a dialog's OK); their text.
    pub button: [(Hsla, Hsla); 3],
    pub default_button: [(Hsla, Hsla); 3],
    pub button_text: Hsla,
    pub default_button_text: Hsla,
    /// Windows': the bottom edge of a button's frame, of the default button's.
    pub button_edge: Option<[Hsla; 2]>,
    /// Breeze's document-mode tabs: the selected one, the others, the selected one's line.
    pub tabs: Option<[Hsla; 3]>,
    /// Windows': a text field's bottom line; focused, the checked fill.
    pub field_line: Option<Hsla>,
}

impl Global for NativeControls {}
