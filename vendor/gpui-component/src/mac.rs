// SparkCommander patch: this file is new.
use gpui::{Global, Hsla};

/// Controls drawn as AppKit draws them (macOS 26, measured), in the system's
/// colours; without it, GPUI Kit's own look.
#[derive(Clone, Copy, Debug)]
pub struct MacControls {
    /// The user's accent: checked boxes, the default button, a slider's fill.
    pub accent: Hsla,
    /// Marks and text on the accent.
    pub on_accent: Hsla,
    /// Push buttons', pop-up buttons' and segments' fill.
    pub button: Hsla,
    pub button_pressed: Hsla,
    pub button_disabled: Hsla,
    /// Unchecked boxes and radio buttons, a slider's groove.
    pub well: Hsla,
    /// A progress bar's groove and its rim.
    pub groove: Hsla,
    pub groove_rim: Hsla,
    /// A slider's knob.
    pub knob: Hsla,
    /// A group box's fill.
    pub group: Hsla,
    /// A text field's frame.
    pub field_frame: Hsla,
    pub text: Hsla,
    pub disabled_text: Hsla,
    /// A dialog's rim.
    pub window_frame: Hsla,
    /// Window tabs: the bar's track, the selected tab and its rim.
    pub tab_track: Hsla,
    pub tab: Hsla,
    pub tab_rim: Hsla,
    /// Tooltips' fill and rim.
    pub tooltip: Hsla,
    pub tooltip_frame: Hsla,
}

impl Global for MacControls {}

/// The check box's mark, bolder than GPUI Kit's, as AppKit's.
pub(crate) const CHECK_MARK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><path d="M2.4 6.4 4.9 8.9 9.7 3.2" fill="none" stroke="black" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;
