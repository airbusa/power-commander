//! Video and sound in the viewer, played by libmpv (pc_core::media): the
//! frames mpv draws at this view's size, and the controls below them.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, IconNamed, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use pc_core::media::{Player, clock};

/// The player's icons, Lucide's (assets/media).
#[derive(Clone, Copy)]
pub enum MediaIcon {
    Play,
    Pause,
    Volume,
    Muted,
    FullScreen,
    Window,
}

impl IconNamed for MediaIcon {
    fn path(self) -> SharedString {
        match self {
            MediaIcon::Play => "media/play.svg",
            MediaIcon::Pause => "media/pause.svg",
            MediaIcon::Volume => "media/volume-2.svg",
            MediaIcon::Muted => "media/volume-x.svg",
            MediaIcon::FullScreen => "media/maximize.svg",
            MediaIcon::Window => "media/minimize.svg",
        }
        .into()
    }
}

/// The icons' files, by name.
pub const ICONS: [(&str, &[u8]); 6] = [
    ("play.svg", include_bytes!("../assets/media/play.svg")),
    ("pause.svg", include_bytes!("../assets/media/pause.svg")),
    ("volume-2.svg", include_bytes!("../assets/media/volume-2.svg")),
    ("volume-x.svg", include_bytes!("../assets/media/volume-x.svg")),
    ("maximize.svg", include_bytes!("../assets/media/maximize.svg")),
    ("minimize.svg", include_bytes!("../assets/media/minimize.svg")),
];

pub struct MediaView {
    path: PathBuf,
    player: Result<Rc<Player>, String>,
    image: Option<Arc<RenderImage>>,
    /// Frames shown before, their textures freed at the next draw.
    stale: Vec<Arc<RenderImage>>,
    /// Where the seek bar is, for the mouse.
    bar: Rc<Cell<Bounds<Pixels>>>,
    seeking: bool,
    _frames: Task<()>,
}

impl MediaView {
    /// `paused`: Quick View's, which shows what the cursor passes.
    pub fn new(path: PathBuf, paused: bool, cx: &mut Context<Self>) -> Self {
        let (sent, mut received) = futures::channel::mpsc::unbounded::<()>();
        let player = Player::open(&path, paused, move || {
            let _ = sent.unbounded_send(());
        });
        let frames = cx.spawn(async move |this, cx| {
            while received.next().await.is_some() {
                // Notices come in bursts; one redraw takes them all.
                while received.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.take_frame(cx)).is_err() {
                    break;
                }
            }
        });
        MediaView {
            path,
            player: player.map(Rc::new),
            image: None,
            stale: Vec::new(),
            bar: Rc::default(),
            seeking: false,
            _frames: frames,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn player(&self) -> Option<Rc<Player>> {
        self.player.as_ref().ok().cloned()
    }

    fn take_frame(&mut self, cx: &mut Context<Self>) {
        let frame = self.player().and_then(|player| player.take_frame());
        if let Some(frame) = frame
            && let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)
        {
            let image = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
            self.stale.extend(self.image.replace(image));
        }
        cx.notify();
    }

    /// To where the mouse is on the seek bar.
    fn seek_at(&self, x: Pixels) {
        let bar = self.bar.get();
        if let Some(player) = self.player().filter(|_| bar.size.width > px(0.)) {
            player.seek_to(((x - bar.origin.x) / bar.size.width) as f64);
        }
    }
}

impl Render for MediaView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for old in self.stale.drain(..) {
            let _ = window.drop_image(old);
        }
        let theme = cx.theme().clone();
        let player = match &self.player {
            Ok(player) => player.clone(),
            Err(err) => {
                let text = format!("This cannot be played: {err}. 1 shows it as text.");
                return div().p_4().child(text).into_any_element();
            }
        };
        let state = player.state();
        let image = self.image.clone();
        let sizing = player.clone();
        // mpv draws at the picture's size in pixels; the frame fills it.
        let picture = canvas(
            move |bounds, window, _| {
                let pixels = |side: Pixels| (f32::from(side) * window.scale_factor()).round() as u32;
                sizing.set_size(pixels(bounds.size.width), pixels(bounds.size.height));
            },
            move |bounds, (), window, _| {
                if let Some(image) = image {
                    let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
                }
            },
        )
        .size_full();
        let notice = match (&state.error, state.video) {
            (Some(err), _) => Some(format!("This cannot be played: {err}")),
            (None, false) => Some(self.path.file_name().unwrap_or_default().display().to_string()),
            (None, true) => None,
        };
        let share = match state.duration > 0. {
            true => (state.position / state.duration).clamp(0., 1.) as f32,
            false => 0.,
        };
        let bar = self.bar.clone();
        let full_screen = window.is_fullscreen();
        let button = |id: &'static str, icon: MediaIcon| Button::new(id).icon(icon).small().ghost();
        let controls = h_flex()
            .flex_none()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .child(
                button(
                    "media-play",
                    if state.paused { MediaIcon::Play } else { MediaIcon::Pause },
                )
                .on_click(cx.listener(|this, _, _, _| {
                    this.player().inspect(|player| player.toggle_pause());
                })),
            )
            .child(
                div()
                    .flex_none()
                    .text_sm()
                    .child(format!("{} / {}", clock(state.position), clock(state.duration))),
            )
            .child(
                div()
                    .id("media-seek")
                    .flex_1()
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.seeking = true;
                            this.seek_at(event.position.x);
                        }),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(6.))
                            .rounded_full()
                            .bg(theme.muted)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .h_full()
                                    .w(relative(share))
                                    .rounded_full()
                                    .bg(theme.primary),
                            )
                            .child(
                                canvas(move |bounds, _, _| bar.set(bounds), |_, _, _, _| {})
                                    .absolute()
                                    .size_full(),
                            ),
                    ),
            )
            .child(
                button(
                    "media-mute",
                    if state.muted { MediaIcon::Muted } else { MediaIcon::Volume },
                )
                .on_click(cx.listener(|this, _, _, _| {
                    this.player().inspect(|player| player.toggle_mute());
                })),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(40.))
                    .text_sm()
                    .child(format!("{:.0}%", state.volume)),
            )
            .child(
                button(
                    "media-full-screen",
                    if full_screen { MediaIcon::Window } else { MediaIcon::FullScreen },
                )
                .on_click(|_, window, _| window.toggle_fullscreen()),
            );
        v_flex()
            .size_full()
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, _| {
                if this.seeking && event.pressed_button == Some(MouseButton::Left) {
                    this.seek_at(event.position.x);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.seeking = false),
            )
            .child(
                div()
                    .id("media-picture")
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .bg(gpui_kit::black())
                    .on_click(|event, window, _| {
                        if event.click_count() == 2 {
                            window.toggle_fullscreen();
                        }
                    })
                    .child(picture)
                    .when_some(notice, |el, notice| {
                        el.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(gpui_kit::white())
                                .child(notice),
                        )
                    }),
            )
            .child(controls)
            .into_any_element()
    }
}
