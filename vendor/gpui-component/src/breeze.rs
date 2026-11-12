// PowerCommander patch: this file is new.
use gpui::{Global, Hsla};

/// Controls drawn as KDE's Breeze style draws them (`breezehelper.cpp`), in
/// these colours of the user's scheme; without it, GPUI Kit's own look.
#[derive(Clone, Copy, Debug)]
pub struct BreezeControls {
    /// Buttons', check boxes' and radio buttons' fill.
    pub button: Hsla,
    /// Frames: a fifth of the text over the window.
    pub frame: Hsla,
    /// A checked box's or radio button's fill.
    pub checked: Hsla,
    /// The selection colour: a checked box's frame, a slider's or progress bar's fill.
    pub highlight: Hsla,
    /// Check marks and radio dots: the text colour.
    pub mark: Hsla,
}

impl Global for BreezeControls {}
