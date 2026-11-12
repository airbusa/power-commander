//! A debug build's preview of the controls, laid out as KDE's Application
//! Style page previews a style (`PC_STYLE_PREVIEW=1`): for matching the look
//! of a desktop's style.

use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, NumberInput};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, IndexPath, h_flex, v_flex};
use gpui_kit::*;
use pc_core::settings::Settings;

pub(crate) struct StylePreview {
    combo: Entity<SelectState<Vec<&'static str>>>,
    spin: Entity<InputState>,
    text: Entity<InputState>,
    slider: Entity<SliderState>,
    scroll: ScrollHandle,
}

pub(crate) fn open(cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(330.), px(190.)), cx)),
        app_id: Some("powercommander".into()),
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let settings = Settings::load().unwrap_or_default();
        crate::appearance::apply(&crate::palette(&settings, window), cx);
        let combo = cx.new(|cx| SelectState::new(vec!["Combo box"], Some(IndexPath::new(0)), window, cx));
        let spin = cx.new(|cx| InputState::new(window, cx).default_value("0"));
        let text = cx.new(|cx| InputState::new(window, cx).placeholder("Text Field"));
        let slider = cx.new(|_| SliderState::new().min(0.).max(100.).default_value(30.));
        cx.new(|_| StylePreview { combo, spin, text, slider, scroll: ScrollHandle::new() })
    });
    if let Err(err) = opened {
        eprintln!("style preview: {err}");
    }
}

impl Render for StylePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = |text: &'static str| div().w(px(110.)).flex_none().child(text);
        let row = || h_flex().gap_2().items_center();
        let controls = v_flex()
            .flex_1()
            .gap_2()
            .p_2()
            .child(row().child(label("").child(Checkbox::new("check").label("Checkbox").checked(true))).child(div().flex_1().child(Select::new(&self.combo))))
            .child(
                row()
                    .child(label("").child(Radio::new("radio-1").label("Radio button").checked(true)))
                    .child(div().flex_1().child(Button::new("button").label("Button")))
                    .child(div().w(px(100.)).child(NumberInput::new(&self.spin))),
            )
            .child(row().child(label("").child(Radio::new("radio-2").label("Radio button"))).child(div().flex_1().child(Input::new(&self.text))))
            .child(
                row()
                    .child(label("").child(Slider::new(&self.slider)))
                    .child(div().flex_1().child(Progress::new("progress").value(70.)))
                    .child("70%"),
            );
        let tall = div().id("tall").h_full().overflow_y_scroll().track_scroll(&self.scroll).child(div().h(px(400.)));
        let page = h_flex()
            .flex_1()
            .min_h_0()
            .items_start()
            .border_1()
            .border_color(cx.theme().border)
            .child(controls)
            .child(div().relative().w(px(12.)).h_full().child(tall).child(div().absolute().inset_0().child(Scrollbar::vertical(&self.scroll))));
        // As the app's dialogs.
        v_flex()
            .size_full()
            .p_1()
            .text_sm()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(TabBar::new("tabs").selected_index(0).child(Tab::new().label("Tab 1")).child(Tab::new().label("Tab 2")))
            .child(page)
    }
}
