// Modified for Water Data Studio (Apache-2.0, section 4b): a dialog moves by
// its title, can open centred over a part of the window, and fades in
// without sliding. Changes are marked "WDS patch".
// Modified for SparkCommander: the title is smaller and ends in "…" when
// long; on macOS the frame is a window's. Changes are marked "SparkCommander patch".

use super::entrance::{DialogEntrance, EntranceSurface};
use crate::root::WindowState;
use gpui_base::TestSupportExt as _;
use std::{cell::Cell, rc::Rc, sync::LazyLock, time::Duration};

use gpui::{
    Action, Anchor, Animation, AnimationExt as _, AnyElement, App, Bounds, BoxShadow, ClickEvent,
    DispatchPhase, Edges, FocusHandle, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, RenderOnce,
    SharedString, StyleRefinement, Styled, Window, WindowControlArea, anchored, canvas, div, hsla,
    point, prelude::FluentBuilder, px,
};
use gpui_base::{ElementExt as _, TextSelectionScopeId};
use rust_i18n::t;

use crate::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt, TITLE_BAR_HEIGHT, WindowExt as _,
    animation::cubic_bezier,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::{DialogContent, DialogDispatchAnchor, DialogTitle},
    scroll::ScrollableElement as _,
    v_flex,
};

pub static ANIMATION_DURATION: LazyLock<Duration> = LazyLock::new(|| Duration::from_secs_f64(0.25));
pub use gpui_base::actions::{Cancel, Confirm};

type OkHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static>;
type CancelHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static>;
type CloseHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// WDS patch: how tall the band with the title is.
const TITLE_BAND_HEIGHT: f32 = 40.;

/// WDS patch: where a dialog moved by its title is, kept across frames.
#[derive(Default)]
struct DialogMove {
    /// The top left corner it was moved to.
    origin: Cell<Option<Point<Pixels>>>,
    /// The top left corner it was last laid out at.
    laid_out: Cell<Point<Pixels>>,
    /// Where the mouse holds it, from its top left corner, while it moves.
    grab: Cell<Option<Point<Pixels>>>,
}

/// Overwrite `slot` only when `value` was explicitly set.
fn merge_field<T>(slot: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *slot = value;
    }
}

/// Dialog button props.
///
/// Every field is unset until a builder sets it, and an unset field falls back
/// to its documented default when the dialog renders. Handing a value to
/// [`Dialog::button_props`] or [`crate::dialog::AlertDialog::button_props`]
/// therefore overrides only the fields that value sets: whatever the dialog
/// already carries — the Cancel button `AlertDialog::confirm` asked for, a
/// callback an earlier `on_ok` installed — survives.
#[derive(Clone, Default)]
pub struct DialogButtonProps {
    pub(crate) ok_text: Option<SharedString>,
    pub(crate) ok_variant: Option<ButtonVariant>,
    pub(crate) cancel_text: Option<SharedString>,
    pub(crate) cancel_variant: Option<ButtonVariant>,
    pub(crate) show_cancel: Option<bool>,
    pub(crate) on_ok: Option<OkHandler>,
    pub(crate) on_cancel: Option<CancelHandler>,
    pub(crate) on_close: Option<CloseHandler>,
}

impl DialogButtonProps {
    /// Sets the text of the OK button. Default is `OK`.
    pub fn ok_text(mut self, ok_text: impl Into<SharedString>) -> Self {
        self.ok_text = Some(ok_text.into());
        self
    }

    /// Sets the variant of the OK button. Default is `ButtonVariant::Primary`.
    pub fn ok_variant(mut self, ok_variant: ButtonVariant) -> Self {
        self.ok_variant = Some(ok_variant);
        self
    }

    /// Sets the text of the Cancel button. Default is `Cancel`.
    pub fn cancel_text(mut self, cancel_text: impl Into<SharedString>) -> Self {
        self.cancel_text = Some(cancel_text.into());
        self
    }

    /// Sets the variant of the Cancel button. Default is `ButtonVariant::default()`.
    pub fn cancel_variant(mut self, cancel_variant: ButtonVariant) -> Self {
        self.cancel_variant = Some(cancel_variant);
        self
    }

    /// Sets whether to show the Cancel button. Default is `false`.
    pub fn show_cancel(mut self, show_cancel: bool) -> Self {
        self.show_cancel = Some(show_cancel);
        self
    }

    /// Sets the callback for when the dialog is has been confirmed.
    ///
    /// The callback should return `true` to close the dialog, if return `false` the dialog will not be closed.
    pub fn on_ok(
        mut self,
        on_ok: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.on_ok = Some(Rc::new(on_ok));
        self
    }

    /// Sets the callback for when the dialog is has been canceled.
    ///
    /// The callback should return `true` to close the dialog, if return `false` the dialog will not be closed.
    pub fn on_cancel(
        mut self,
        on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    /// Takes over every field `other` sets and keeps the rest.
    pub(crate) fn merge(&mut self, other: Self) {
        merge_field(&mut self.ok_text, other.ok_text);
        merge_field(&mut self.ok_variant, other.ok_variant);
        merge_field(&mut self.cancel_text, other.cancel_text);
        merge_field(&mut self.cancel_variant, other.cancel_variant);
        merge_field(&mut self.show_cancel, other.show_cancel);
        merge_field(&mut self.on_ok, other.on_ok);
        merge_field(&mut self.on_cancel, other.on_cancel);
        merge_field(&mut self.on_close, other.on_close);
    }

    /// Whether the default footer renders a Cancel button. Default is `false`.
    pub(crate) fn is_cancel_shown(&self) -> bool {
        self.show_cancel.unwrap_or(false)
    }

    /// The confirm callback, defaulting to one that closes the dialog.
    pub(crate) fn ok_handler(&self) -> OkHandler {
        self.on_ok
            .clone()
            .unwrap_or_else(|| Rc::new(|_, _, _| true))
    }

    /// The cancel callback, defaulting to one that closes the dialog.
    pub(crate) fn cancel_handler(&self) -> CancelHandler {
        self.on_cancel
            .clone()
            .unwrap_or_else(|| Rc::new(|_, _, _| true))
    }

    /// The close callback, defaulting to one that does nothing.
    pub(crate) fn close_handler(&self) -> CloseHandler {
        self.on_close
            .clone()
            .unwrap_or_else(|| Rc::new(|_, _, _| {}))
    }

    pub(crate) fn render_ok(&self, _: &mut Window, _: &mut App) -> AnyElement {
        let ok_text = self
            .ok_text
            .clone()
            .unwrap_or_else(|| t!("Dialog.ok").into());

        DialogButton {
            anchor_key: "dialog-ok-anchor",
            button: Button::new("ok")
                .label(ok_text)
                .with_variant(self.ok_variant.unwrap_or(ButtonVariant::Primary)),
            action: Rc::new(Confirm { secondary: false }),
        }
        .into_any_element()
    }

    pub(crate) fn render_cancel(&self, _: &mut Window, _: &mut App) -> AnyElement {
        let cancel_text = self
            .cancel_text
            .clone()
            .unwrap_or_else(|| t!("Dialog.cancel").into());

        DialogButton {
            anchor_key: "dialog-cancel-anchor",
            button: Button::new("cancel")
                .label(cancel_text)
                .with_variant(self.cancel_variant.unwrap_or_default()),
            action: Rc::new(Cancel),
        }
        .into_any_element()
    }
}

/// A default dialog button: activating it dispatches `action` on the dialog
/// it sits in, whatever holds focus at that moment.
#[derive(IntoElement)]
struct DialogButton {
    /// Distinct per button: OK and Cancel render as siblings in one scope.
    anchor_key: &'static str,
    button: Button,
    action: Rc<dyn Action>,
}

impl RenderOnce for DialogButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let anchor = DialogDispatchAnchor::new(self.anchor_key, window, cx);
        self.button
            .child(anchor.element())
            .on_click(move |_, window, cx| anchor.dispatch(&*self.action, window, cx))
    }
}

type ContentBuilderFn = Rc<dyn Fn(DialogContent, &mut Window, &mut App) -> DialogContent + 'static>;

#[derive(Clone)]
pub(crate) struct DialogProps {
    width: Pixels,
    max_width: Option<Pixels>,
    margin_top: Option<Pixels>,
    // WDS patch: where the dialog opens centred.
    center_over: Option<Bounds<Pixels>>,
    close_button: bool,

    overlay: bool,
    overlay_closable: bool,
    pub(crate) overlay_visible: bool,
    keyboard: bool,
    entrance: DialogEntrance,
}

impl Default for DialogProps {
    fn default() -> Self {
        Self {
            margin_top: None,
            center_over: None,
            width: px(448.),
            max_width: None,
            overlay: true,
            keyboard: true,
            overlay_visible: false,
            close_button: true,
            overlay_closable: true,
            // WDS patch: a dialog fades in where it opens, without sliding.
            entrance: DialogEntrance::Fade,
        }
    }
}

enum BaseDialogRoot {
    Dialog(gpui_base::Dialog),
    AlertDialog(gpui_base::AlertDialog),
}

macro_rules! map_base_root {
    ($self:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match $self {
            BaseDialogRoot::Dialog(root) => BaseDialogRoot::Dialog(root.$method($($arg),*)),
            BaseDialogRoot::AlertDialog(root) => {
                BaseDialogRoot::AlertDialog(root.$method($($arg),*))
            }
        }
    };
}

impl BaseDialogRoot {
    fn layer(self, index: usize, topmost: bool) -> Self {
        map_base_root!(self, layer(index, topmost))
    }
    fn focus_handle(self, focus: FocusHandle) -> Self {
        map_base_root!(self, focus_handle(focus))
    }
    fn close_on_escape(self, value: bool) -> Self {
        map_base_root!(self, close_on_escape(value))
    }
    fn close_on_backdrop_press(self, value: bool) -> Self {
        match self {
            Self::Dialog(root) => Self::Dialog(root.close_on_backdrop_press(value)),
            Self::AlertDialog(root) => Self::AlertDialog(root),
        }
    }
    fn dismiss_below_y(self, value: Pixels) -> Self {
        map_base_root!(self, dismiss_below_y(value))
    }
    fn backdrop(self, element: impl IntoElement) -> Self {
        map_base_root!(self, backdrop(element))
    }
    fn popup(self, element: impl IntoElement) -> Self {
        map_base_root!(self, popup(element))
    }
    fn on_ok(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static) -> Self {
        map_base_root!(self, on_ok(handler))
    }
    fn on_cancel(
        self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        map_base_root!(self, on_cancel(handler))
    }
    fn on_close(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        map_base_root!(self, on_close(handler))
    }
    fn request_close(self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        map_base_root!(self, request_close(handler))
    }
}

impl IntoElement for BaseDialogRoot {
    type Element = <gpui_base::Dialog as IntoElement>::Element;
    fn into_element(self) -> Self::Element {
        match self {
            Self::Dialog(root) => root.into_element(),
            Self::AlertDialog(root) => root.into_element(),
        }
    }
}

/// A modal to display content in a dialog box.
#[derive(IntoElement)]
pub struct Dialog {
    base: Option<BaseDialogRoot>,
    pub(crate) style: StyleRefinement,
    children: Vec<AnyElement>,
    trigger: Option<AnyElement>,
    title: Option<AnyElement>,
    pub(crate) header: Option<AnyElement>,
    pub(crate) footer: Option<AnyElement>,
    pub(crate) content_builder: Option<ContentBuilderFn>,
    pub(crate) props: DialogProps,

    pub(super) button_props: DialogButtonProps,

    /// This will be change when open the dialog, the focus handle is create when open the dialog.
    pub(crate) focus_handle: FocusHandle,
    pub(crate) layer_ix: usize,
    pub(crate) selection_scope: TextSelectionScopeId,
}

pub(crate) fn overlay_color(overlay: bool, cx: &App) -> Hsla {
    if !overlay {
        return hsla(0., 0., 0., 0.);
    }

    cx.theme().overlay
}

/// SparkCommander patch: AppKit's red close button, grey while the window
/// is not in front, its cross shown under the pointer.
fn mac_close_button(button: gpui_base::Button, mac: crate::MacControls, active: bool) -> impl IntoElement {
    let (fill, rim) = match active {
        true => (gpui::rgb(0xff5f57).into(), gpui::rgb(0xe0443e).into()),
        false => (mac.well, mac.well),
    };
    button
        .group("dialog-close")
        .size(px(14.))
        .rounded_full()
        .bg(fill)
        .border(px(0.5))
        .border_color(rim)
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .opacity(0.)
                .group_hover("dialog-close", |this| this.opacity(1.))
                .child(crate::Icon::new(IconName::Close).size(px(10.)).text_color(gpui::black().opacity(0.55))),
        )
}

fn dialog_shadow(delta: f32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: hsla(0., 0., 0., 0.1 * delta),
            offset: point(px(0.), px(20.)),
            blur_radius: px(25.),
            spread_radius: px(-5.),
            inset: false,
        },
        BoxShadow {
            color: hsla(0., 0., 0., 0.1 * delta),
            offset: point(px(0.), px(8.)),
            blur_radius: px(10.),
            spread_radius: px(-6.),
            inset: false,
        },
    ]
}

impl Dialog {
    /// Create a new dialog.
    pub fn new(cx: &mut App) -> Self {
        Self {
            base: Some(BaseDialogRoot::Dialog(gpui_base::Dialog::new(cx))),
            focus_handle: cx.focus_handle(),
            style: StyleRefinement::default(),
            trigger: None,
            title: None,
            header: None,
            footer: None,
            content_builder: None,
            props: DialogProps::default(),
            children: Vec::new(),
            layer_ix: 0,
            selection_scope: TextSelectionScopeId::default(),
            button_props: DialogButtonProps::default(),
        }
    }

    /// Sets the trigger element for the dialog.
    ///
    /// When a trigger is set, the dialog will render as a trigger button that opens the dialog when clicked.
    pub fn trigger(mut self, trigger: impl IntoElement) -> Self {
        self.trigger = Some(trigger.into_any_element());
        self
    }

    /// Sets the content of the dialog.
    pub fn content<F>(mut self, builder: F) -> Self
    where
        F: Fn(DialogContent, &mut Window, &mut App) -> DialogContent + 'static,
    {
        self.content_builder = Some(Rc::new(builder));
        self
    }

    /// Sets the title of the dialog.
    pub fn title(mut self, title: impl IntoElement) -> Self {
        self.title = Some(title.into_any_element());
        self
    }

    /// Sets the header of the dialog, rendered above the title and content.
    pub(crate) fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }

    /// Sets the footer of the dialog, the footer will render at the bottom of the dialog, usually for action buttons.
    ///
    /// A plain `Dialog` never renders buttons from `button_props`: the props
    /// only install the callbacks behind the `Confirm` and `Cancel` actions.
    /// Compose the action buttons yourself — typically with a
    /// [`DialogFooter`](crate::dialog::DialogFooter) — or use an
    /// [`AlertDialog`](crate::dialog::AlertDialog), which builds a default
    /// footer from `button_props` when no footer is set.
    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    /// Set the button props of the dialog.
    ///
    /// This overrides only the fields `button_props` sets; the rest of the
    /// dialog's button configuration is kept, so the call order does not
    /// matter.
    ///
    /// On a plain `Dialog` these props install the callbacks behind the
    /// `Confirm` and `Cancel` actions and the labels an
    /// [`AlertDialog`](crate::dialog::AlertDialog) default footer would use;
    /// the dialog surface itself renders no buttons from them, see
    /// [`Self::footer`].
    pub fn button_props(mut self, button_props: DialogButtonProps) -> Self {
        self.button_props.merge(button_props);
        self
    }

    pub(crate) fn with_base_alert_dialog(mut self, base: gpui_base::AlertDialog) -> Self {
        self.base = Some(BaseDialogRoot::AlertDialog(base));
        self.props.overlay_closable = false;
        self
    }

    /// Sets the callback for when the dialog is closed.
    ///
    /// Called after [`Self::on_ok`] or [`Self::on_cancel`] callback.
    pub fn on_close(
        mut self,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.button_props.on_close = Some(Rc::new(on_close));
        self
    }

    /// Sets the callback for when the dialog is has been confirmed.
    ///
    /// The callback should return `true` to close the dialog, if return `false` the dialog will not be closed.
    pub fn on_ok(
        mut self,
        on_ok: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.button_props = self.button_props.on_ok(on_ok);
        self
    }

    /// Sets the callback for when the dialog is has been canceled.
    ///
    /// The callback should return `true` to close the dialog, if return `false` the dialog will not be closed.
    pub fn on_cancel(
        mut self,
        on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.button_props = self.button_props.on_cancel(on_cancel);
        self
    }

    /// Sets the false to hide close icon, default: true
    pub fn close_button(mut self, close_button: bool) -> Self {
        self.props.close_button = close_button;
        self
    }

    /// Set the top offset of the dialog, defaults to None, will use the 1/10 of the viewport height.
    pub fn margin_top(mut self, margin_top: impl Into<Pixels>) -> Self {
        self.props.margin_top = Some(margin_top.into());
        self
    }

    /// WDS patch: opens the dialog centred over `bounds`, in window
    /// coordinates, instead of at `margin_top`.
    pub fn center_over(mut self, bounds: Bounds<Pixels>) -> Self {
        self.props.center_over = Some(bounds);
        self
    }

    /// Sets the width of the dialog, defaults to 448px.
    ///
    /// The dialog is never wider than the viewport minus a margin on each side.
    ///
    /// See also [`Self::width`]
    pub fn w(mut self, width: impl Into<Pixels>) -> Self {
        self.props.width = width.into();
        self
    }

    /// Sets the width of the dialog, defaults to 448px.
    ///
    /// The dialog is never wider than the viewport minus a margin on each side.
    pub fn width(mut self, width: impl Into<Pixels>) -> Self {
        self.props.width = width.into();
        self
    }

    /// Set the maximum width of the dialog, defaults to `None`.
    pub fn max_w(mut self, max_width: impl Into<Pixels>) -> Self {
        self.props.max_width = Some(max_width.into());
        self
    }

    /// Set the overlay of the dialog, defaults to `true`.
    pub fn overlay(mut self, overlay: bool) -> Self {
        self.props.overlay = overlay;
        self
    }

    /// Set the overlay closable of the dialog, defaults to `true`.
    ///
    /// When the overlay is clicked, the dialog will be closed.
    pub fn overlay_closable(mut self, overlay_closable: bool) -> Self {
        self.props.overlay_closable = overlay_closable;
        self
    }

    /// Set whether to support keyboard esc to close the dialog, defaults to `true`.
    pub fn keyboard(mut self, keyboard: bool) -> Self {
        self.props.keyboard = keyboard;
        self
    }

    /// Sets how the dialog enters, defaulting to [`DialogEntrance::SlideDown`].
    ///
    /// This controls the surface and backdrop together. Reduced motion skips
    /// the entrance and immediately shows the final state. Closing remains immediate.
    pub fn entrance(mut self, entrance: DialogEntrance) -> Self {
        self.props.entrance = entrance;
        self
    }

    pub(crate) fn has_overlay(&self) -> bool {
        self.props.overlay
    }

    pub(crate) fn with_props(mut self, props: DialogProps) -> Self {
        self.props = props;
        self
    }

    fn defer_close_dialog(window: &mut Window, cx: &mut App) {
        WindowState::update(window, cx, |root, window, cx| {
            root.defer_close_dialog(window, cx);
        });
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Dialog {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl Dialog {
    fn render_trigger(self, trigger: AnyElement, _: &mut Window, _: &mut App) -> AnyElement {
        let content_builder = self.content_builder.clone();
        let style = self.style.clone();
        let props = self.props.clone();
        let button_props = self.button_props.clone();

        gpui_base::DialogTrigger::new(trigger)
            .on_open(move |window, cx| {
                let content_builder = content_builder.clone();
                let style = style.clone();
                let props = props.clone();
                let button_props = button_props.clone();
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog
                        .refine_style(&style)
                        .button_props(button_props.clone())
                        .with_props(props.clone())
                        .content({
                            let content_builder = content_builder.clone();
                            move |content, window, cx| {
                                if let Some(builder) = content_builder.clone() {
                                    builder(content, window, cx)
                                } else {
                                    content
                                }
                            }
                        })
                });
            })
            .into_any_element()
    }
}

impl RenderOnce for Dialog {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if let Some(trigger) = self.trigger.take() {
            return self.render_trigger(trigger, window, cx);
        }

        let layer_ix = self.layer_ix;
        let selection_scope = self.selection_scope;
        // WDS patch: where the dialog was moved to.
        let moving = window
            .use_keyed_state(("dialog-move", layer_ix), cx, |_, _| {
                Rc::new(DialogMove::default())
            })
            .read(cx)
            .clone();
        let on_close = self.button_props.close_handler();
        let on_ok = self.button_props.ok_handler();
        let on_cancel = self.button_props.cancel_handler();

        let window_paddings = crate::window_border::window_paddings(window);
        let view_size = window.viewport_size()
            - gpui::size(
                window_paddings.left + window_paddings.right,
                window_paddings.top + window_paddings.bottom,
            );
        // The dialog keeps this much of the viewport clear around its edges,
        // so a small window shrinks the surface instead of letting it run off
        // the window. A dialog that fits keeps `margin_top` (a tenth of the
        // viewport by default); an overflowing one is snapped up to this edge
        // margin so that the preferred offset does not waste usable height.
        let margin = cx.theme().spacing_tokens().lg;
        let layer_offset = px(layer_ix as f32 * 16.);
        let y = self.props.margin_top.unwrap_or(view_size.height / 10.) + layer_offset;
        let entrance = self.props.entrance;
        let motion = cx.theme().motion_tokens();
        let animated = entrance != DialogEntrance::None
            && !cx.reduce_motion()
            && (entrance == DialogEntrance::SlideDown || !motion.duration_normal.is_zero());
        let travel = motion.distance_short.to_pixels(window.rem_size());
        let width = self
            .props
            .width
            .min((view_size.width - margin * 2.).max(px(0.)));
        let x = (view_size.width - width) / 2.;
        let max_height = (view_size.height - margin * 2. - layer_offset).max(px(0.));

        let base_size = window.text_style().font_size;
        let rem_size = window.rem_size();

        // WDS patch: moved, the dialog stays where it was put; else centred
        // over `center_over`, else at `margin_top`.
        let placed = moving.origin.get().is_some() || self.props.center_over.is_some();
        let (anchor, position) = match (moving.origin.get(), self.props.center_over) {
            (Some(origin), _) => (Anchor::TopLeft, origin),
            (None, Some(over)) => (
                Anchor::LeftCenter,
                point(over.center().x - width / 2., over.center().y),
            ),
            (None, None) => (
                Anchor::TopLeft,
                point(window_paddings.left + x, window_paddings.top + y),
            ),
        };
        let laid_out = moving.clone();
        let has_title = self.title.is_some();
        let has_footer = self.footer.is_some();
        // SparkCommander patch: AppKit's window, see `MacControls`: 16 px corners,
        // a 32 px title bar of the dialog's colour, the close button on the left.
        let mac = cx
            .try_global::<crate::MacControls>()
            .copied()
            .filter(|_| !cx.has_global::<crate::NativeControls>());
        // SparkCommander patch: WinUI's ContentDialog, see `NativeControls::command_bar`.
        let command_bar = cx.try_global::<crate::NativeControls>().and_then(|native| native.command_bar);
        let band = px(if mac.is_some() { 32. } else if command_bar.is_some() { 52. } else { TITLE_BAND_HEIGHT });
        let corner = mac.map_or(cx.theme().radius_lg, |_| px(16.));
        let active = window.is_window_active();

        let mut paddings = Edges::all(px(16.));
        if let Some(pl) = self.style.padding.left {
            paddings.left = pl.to_pixels(base_size, rem_size);
        }
        if let Some(pr) = self.style.padding.right {
            paddings.right = pr.to_pixels(base_size, rem_size);
        }
        if let Some(pt) = self.style.padding.top {
            paddings.top = pt.to_pixels(base_size, rem_size);
        }
        if let Some(pb) = self.style.padding.bottom {
            paddings.bottom = pb.to_pixels(base_size, rem_size);
        }

        // x1 = 1/3, x2 = 2/3 make the bezier's time mapping the identity,
        // preserving the trajectory this dialog was tuned with before
        // `cubic_bezier` solved for x; vaul's (0.32, 0.72, 0., 1.) is far
        // more front-loaded under the CSS-correct solver.
        let animation = if entrance == DialogEntrance::SlideDown {
            Animation::new(*ANIMATION_DURATION).with_easing(cubic_bezier(
                1. / 3.,
                0.72,
                2. / 3.,
                1.,
            ))
        } else {
            let easing = motion.easing_enter.clone();
            Animation::new(motion.duration_normal).with_easing(move |delta| easing.sample(delta))
        };

        anchored()
            .position(point(window_paddings.left, window_paddings.top))
            .snap_to_window()
            .child(
                div()
                    .id("dialog")
                    .test_support()
                    .occlude()
                    .w(view_size.width)
                    .h(view_size.height)
                    .child(
                        self.base
                            .take()
                            .expect("Dialog base host is always present")
                            .layer(
                                layer_ix,
                                (self.layer_ix + 1)
                                    == WindowState::read(window, cx).active_dialogs.len(),
                            )
                            .focus_handle(self.focus_handle.clone())
                            .close_on_escape(self.props.keyboard)
                            .close_on_backdrop_press(self.props.overlay_closable)
                            .dismiss_below_y(TITLE_BAR_HEIGHT)
                            .when(self.props.overlay, |this| {
                                this.backdrop(
                                    div()
                                        .absolute()
                                        .size_full()
                                        .window_control_area(WindowControlArea::Drag)
                                        .when(self.props.overlay_visible, |overlay| {
                                            overlay.bg(overlay_color(true, cx))
                                        }),
                                )
                            })
                            .on_ok(move |event, window, cx| on_ok(event, window, cx))
                            .on_cancel(move |event, window, cx| on_cancel(event, window, cx))
                            .on_close(move |event, window, cx| on_close(event, window, cx))
                            .request_close(move |deferred, window, cx| {
                                if deferred {
                                    Self::defer_close_dialog(window, cx);
                                } else {
                                    window.close_dialog(cx);
                                }
                            })
                            .popup(
                                gpui_base::Positioner::corner(
                                    // WDS patch: see `moving`.
                                    anchor, position,
                                )
                                .on_position(move |resolved| {
                                    laid_out.laid_out.set(resolved.bounds.origin)
                                })
                                .margin(margin)
                                .child(
                                    v_flex()
                                        .id(layer_ix)
                                        .test_support()
                                        .debug_selector(move || format!("dialog-{layer_ix}"))
                                        .bg(cx.theme().tokens.background)
                                        .border_1()
                                        .border_color(mac.map_or(cx.theme().border, |mac| mac.window_frame))
                                        .rounded(corner)
                                        .min_h_24()
                                        .pt(paddings.top)
                                        .pb(paddings.bottom)
                                        .gap(paddings.top.max(px(8.)))
                                        .refine_style(&self.style)
                                        // WDS patch: the title's band is at the top.
                                        .when(has_title, |this| this.pt_0())
                                        .px_0()
                                        .occlude()
                                        .w(width)
                                        .when_some(self.props.max_width, |this, w| this.max_w(w))
                                        .max_h(max_height)
                                        // WDS patch: the title is in a band, which
                                        // shows where the dialog is moved by.
                                        .when_some(self.title, |this, title| {
                                            let moving = moving.clone();
                                            let theme = cx.theme();
                                            this.child(
                                                div()
                                                    .id("dialog-move")
                                                    .flex_none()
                                                    .h(band)
                                                    .flex()
                                                    .items_center()
                                                    .pl(paddings.left)
                                                    .pr(paddings.right + px(28.))
                                                    .map(|this| match mac {
                                                        Some(_) => this.pl(px(36.)).pr(paddings.right),
                                                        None if command_bar.is_some() => this.pt(px(8.)),
                                                        None => this.bg(theme.muted).border_b_1().border_color(theme.border),
                                                    })
                                                    .rounded_t(corner - px(1.))
                                                    // SparkCommander patch: the window title's size; a long one ends in "…".
                                                    .child(
                                                        DialogTitle::new()
                                                            .text_sm()
                                                            .when(mac.is_some(), |this| {
                                                                this.font_semibold().text_color(theme.muted_foreground)
                                                            })
                                                            .when(command_bar.is_some(), |this| this.text_xl().font_semibold())
                                                            .flex_1()
                                                            .min_w_0()
                                                            .truncate()
                                                            .child(title),
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        move |event: &MouseDownEvent, window, _| {
                                                            moving.grab.set(Some(
                                                                event.position
                                                                    - moving.laid_out.get(),
                                                            ));
                                                            window.refresh();
                                                        },
                                                    ),
                                            )
                                        })
                                        .when(moving.grab.get().is_some(), |this| {
                                            this.child(move_listener(moving.clone()))
                                        })
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .overflow_hidden()
                                                .gap_y_2()
                                                .when_some(self.header, |this, header| {
                                                    this.child(
                                                        div()
                                                            .pl(paddings.left)
                                                            .pr(paddings.right)
                                                            .child(header),
                                                    )
                                                })
                                                .when_some(self.content_builder, |this, builder| {
                                                    this.child(builder(
                                                        DialogContent::new()
                                                            .gap(paddings.bottom)
                                                            .pl(paddings.left)
                                                            .pr(paddings.right),
                                                        window,
                                                        cx,
                                                    ))
                                                })
                                                .when(!self.children.is_empty(), |this| {
                                                    this.child(
                                                        div().flex_1().overflow_hidden().child(
                                                            // Body
                                                            v_flex()
                                                                .size_full()
                                                                .overflow_y_scrollbar()
                                                                .pl(paddings.left)
                                                                .pr(paddings.right)
                                                                .children(self.children),
                                                        ),
                                                    )
                                                }),
                                        )
                                        .when_some(self.footer, |this, footer| {
                                            this.child(
                                                div()
                                                    .pl(paddings.left)
                                                    .pr(paddings.right)
                                                    .when_some(command_bar, |this, fill| {
                                                        this.py(paddings.bottom)
                                                            .bg(fill)
                                                            .border_t_1()
                                                            .border_color(cx.theme().border)
                                                            .rounded_b(corner - px(1.))
                                                    })
                                                    .child(footer),
                                            )
                                        })
                                        .when(command_bar.is_some() && has_footer, |this| this.pb_0())
                                        .children(self.props.close_button.then(|| {
                                            // WDS patch: in the middle of the title's band.
                                            let top = if has_title {
                                                (band - px(24.)) / 2.
                                            } else {
                                                (paddings.top - px(10.)).max(px(8.))
                                            };
                                            let right = (paddings.right - px(10.)).max(px(8.));

                                            let close = gpui_base::DialogClose::new().absolute();
                                            match mac {
                                                Some(mac) => close.top((band - px(14.)) / 2.).left(px(11.)).trigger(|button| {
                                                    mac_close_button(button, mac, active)
                                                }),
                                                None => close.top(top).right(right).trigger(|button| {
                                                    Button::new("close")
                                                        .with_base(button)
                                                        .small()
                                                        .ghost()
                                                        .icon(IconName::Close)
                                                }),
                                            }
                                        }))
                                        .map(|surface| {
                                            if animated {
                                                surface
                                                    .with_animation(
                                                        "slide-down-shadow",
                                                        animation.clone(),
                                                        |surface, delta| {
                                                            surface.shadow(dialog_shadow(delta))
                                                        },
                                                    )
                                                    .text_selection_scope(selection_scope)
                                                    .into_any_element()
                                            } else {
                                                surface
                                                    .shadow(dialog_shadow(1.))
                                                    .text_selection_scope(selection_scope)
                                                    .into_any_element()
                                            }
                                        })
                                        .map(|surface| {
                                            if animated && entrance == DialogEntrance::FadeSlide {
                                                EntranceSurface::new(
                                                    surface,
                                                    travel,
                                                    window_paddings.top + margin,
                                                )
                                                .with_animation(
                                                    "dialog-settle",
                                                    animation.clone(),
                                                    |mut surface, delta| {
                                                        surface.progress = delta;
                                                        surface
                                                    },
                                                )
                                                .into_any_element()
                                            } else {
                                                surface
                                            }
                                        }),
                                )
                                .map(|positioner| {
                                    if animated && entrance == DialogEntrance::SlideDown && !placed
                                    {
                                        positioner
                                            .with_animation(
                                                "slide-down",
                                                animation.clone(),
                                                move |this, delta| {
                                                    this.position(point(
                                                        window_paddings.left + x,
                                                        window_paddings.top + y * delta,
                                                    ))
                                                },
                                            )
                                            .into_any_element()
                                    } else {
                                        positioner.into_any_element()
                                    }
                                }),
                            ),
                    )
                    .map(|layer| {
                        if animated {
                            layer
                                .with_animation("fade-in", animation, |layer, delta| {
                                    layer.opacity(delta)
                                })
                                .into_any_element()
                        } else {
                            layer.into_any_element()
                        }
                    }),
            )
            .into_any_element()
    }
}

/// WDS patch: follows the mouse anywhere in the window while the dialog is
/// held by its title.
fn move_listener(moving: Rc<DialogMove>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let follow = moving.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                match (follow.grab.get(), event.pressed_button) {
                    (Some(grab), Some(MouseButton::Left)) => {
                        follow.origin.set(Some(event.position - grab));
                        window.refresh();
                    }
                    _ => follow.grab.set(None),
                }
            });
            let release = moving.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, window, _| {
                if phase == DispatchPhase::Capture {
                    release.grab.set(None);
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .size_0()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::{AppContext as _, Bounds, Context, Render, TestAppContext, VisualTestContext, size};

    struct DialogHost;

    impl Render for DialogHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    /// A window of `window_size` whose root renders the dialog layer, with
    /// motion reduced so the entrance animation settles on its first frame.
    pub(crate) fn window(
        cx: &mut TestAppContext,
        window_size: gpui::Size<Pixels>,
    ) -> &mut VisualTestContext {
        cx.update(|cx| {
            crate::init(cx);
            cx.set_reduce_motion(true);
        });
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| DialogHost);
            crate::Root::new(view, window, cx)
        });
        cx.simulate_resize(window_size);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx
    }

    fn open(
        cx: &mut VisualTestContext,
        build: impl Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
    ) {
        cx.update(|window, cx| window.open_dialog(cx, build));
        cx.run_until_parked();
        // One frame mounts the layer, the next paints it at rest.
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn surface(cx: &mut VisualTestContext, layer_ix: usize) -> Bounds<Pixels> {
        let selector = ["dialog-0", "dialog-1"][layer_ix];
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("dialog layer {layer_ix} was not painted"))
    }

    /// The clamp must not touch a dialog that already fits: the default width
    /// and the tenth-of-the-viewport top offset are the documented contract.
    #[gpui::test]
    fn a_dialog_that_fits_keeps_its_default_width_and_top_offset(cx: &mut TestAppContext) {
        let cx = window(cx, size(px(1000.), px(800.)));
        open(cx, |dialog, _, _| dialog.title("Fits").child("body"));

        let bounds = surface(cx, 0);
        assert_eq!(bounds.size.width, px(448.));
        assert_eq!(bounds.origin.x, px(276.));
        assert_eq!(bounds.origin.y, px(80.));
    }

    /// A dialog wider and taller than the window must shrink to the viewport
    /// instead of running off both edges. It also gives up the usual top
    /// offset so that space is available to its content, while its footer
    /// remains inside the surface rather than clipped below it.
    #[gpui::test]
    fn a_dialog_larger_than_the_window_stays_inside_it(cx: &mut TestAppContext) {
        let viewport = size(px(400.), px(300.));
        let cx = window(cx, viewport);
        open(cx, |dialog, _, _| {
            dialog
                .w(px(800.))
                .title("Too big")
                .child(div().h(px(1000.)).child("tall body"))
                .footer(div().h(px(32.)).debug_selector(|| "footer-probe".into()))
        });

        let bounds = surface(cx, 0);
        let footer = cx.debug_bounds("footer-probe").unwrap();
        let margin = px(16.);
        assert!(
            bounds.origin.x >= margin && bounds.right() <= viewport.width - margin,
            "the dialog ran off the sides: {bounds:?}"
        );
        assert!(
            bounds.bottom() <= viewport.height - margin,
            "the dialog ran off the bottom: {bounds:?}"
        );
        assert_eq!(bounds.origin.y, margin);
        assert!(
            footer.bottom() <= bounds.bottom(),
            "the footer was clipped below the dialog: footer {footer:?}, dialog {bounds:?}"
        );
    }

    /// `Dialog::button_props` overrides only the fields it sets.
    #[gpui::test]
    fn dialog_button_props_merge_with_what_the_dialog_already_carries(cx: &mut TestAppContext) {
        let cx = window(cx, size(px(400.), px(300.)));
        cx.update(|_, cx| {
            let dialog = Dialog::new(cx)
                .button_props(DialogButtonProps::default().cancel_text("Keep"))
                .button_props(DialogButtonProps::default().ok_text("Delete"))
                .button_props(DialogButtonProps::default().ok_variant(ButtonVariant::Danger));

            assert_eq!(dialog.button_props.ok_text.as_deref(), Some("Delete"));
            assert_eq!(dialog.button_props.cancel_text.as_deref(), Some("Keep"));
            assert_eq!(dialog.button_props.ok_variant, Some(ButtonVariant::Danger));
        });
    }

    /// Each stacked dialog steps down 16px; the deepest one must still end
    /// above the bottom margin.
    #[gpui::test]
    fn stacked_dialogs_each_fit_the_window(cx: &mut TestAppContext) {
        let viewport = size(px(400.), px(300.));
        let cx = window(cx, viewport);
        open(cx, |dialog, _, _| {
            dialog.title("First").child(div().h(px(1000.)))
        });
        open(cx, |dialog, _, _| {
            dialog.title("Second").child(div().h(px(1000.)))
        });

        let first = surface(cx, 0);
        let second = surface(cx, 1);
        assert_eq!(second.origin.y, first.origin.y + px(16.));
        assert!(first.bottom() <= viewport.height - px(16.), "{first:?}");
        assert!(second.bottom() <= viewport.height - px(16.), "{second:?}");
        assert!(second.size.height < first.size.height);
    }

    #[gpui::test]
    fn test_dialog_entrance_builder(cx: &mut TestAppContext) {
        cx.update(crate::init);
        cx.update(|cx| {
            assert_eq!(Dialog::new(cx).props.entrance, DialogEntrance::SlideDown);
            for entrance in [
                DialogEntrance::SlideDown,
                DialogEntrance::Fade,
                DialogEntrance::FadeSlide,
                DialogEntrance::None,
            ] {
                assert_eq!(Dialog::new(cx).entrance(entrance).props.entrance, entrance);
            }
        });
    }

    #[gpui::test]
    fn reduced_motion_displays_every_entrance_at_rest(cx: &mut TestAppContext) {
        for entrance in [
            DialogEntrance::SlideDown,
            DialogEntrance::Fade,
            DialogEntrance::FadeSlide,
            DialogEntrance::None,
        ] {
            let cx = window(cx, size(px(1000.), px(800.)));
            open(cx, move |dialog, _, _| {
                dialog
                    .title("Entrance")
                    .child("body")
                    .entrance(entrance)
                    .margin_top(px(300.))
            });
            assert_eq!(surface(cx, 0).top(), px(300.));
        }
    }

    #[gpui::test]
    fn no_entrance_displays_immediately_and_keeps_keyboard_dismissal(cx: &mut TestAppContext) {
        let cx = window(cx, size(px(1000.), px(800.)));
        cx.update(|_, cx| cx.set_reduce_motion(false));
        open(cx, |dialog, _, _| {
            dialog
                .title("Immediate")
                .child("body")
                .entrance(DialogEntrance::None)
                .margin_top(px(300.))
        });
        assert_eq!(surface(cx, 0).top(), px(300.));
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| assert!(WindowState::read(window, cx).active_dialogs.is_empty()));
    }

    #[gpui::test]
    fn zero_theme_duration_displays_fade_entrances_immediately(cx: &mut TestAppContext) {
        for entrance in [DialogEntrance::Fade, DialogEntrance::FadeSlide] {
            let cx = window(cx, size(px(1000.), px(800.)));
            cx.update(|_, cx| {
                cx.set_reduce_motion(false);
                crate::Theme::update(cx, |theme| theme.motion.duration_normal = Duration::ZERO);
            });
            open(cx, move |dialog, _, _| {
                dialog
                    .title("Immediate")
                    .child("body")
                    .entrance(entrance)
                    .margin_top(px(300.))
            });
            assert_eq!(surface(cx, 0).top(), px(300.));
        }
    }

    #[gpui::test]
    fn alert_dialog_uses_the_same_entrance(cx: &mut TestAppContext) {
        for entrance in [
            DialogEntrance::SlideDown,
            DialogEntrance::Fade,
            DialogEntrance::FadeSlide,
            DialogEntrance::None,
        ] {
            let cx = window(cx, size(px(1000.), px(800.)));
            cx.update(|window, cx| {
                let alert = crate::dialog::AlertDialog::new(cx)
                    .title("Alert")
                    .entrance(entrance);
                assert_eq!(alert.build_surface(window, cx).props.entrance, entrance);
                window.open_alert_dialog(cx, move |alert, _, _| {
                    alert.title("Alert").entrance(entrance)
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(surface(cx, 0).top(), px(80.));
        }
    }
}
