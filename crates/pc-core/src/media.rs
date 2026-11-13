//! Video and sound by libmpv, loaded when there as PDFium is: mpv draws each
//! frame with OpenGL into a hidden framebuffer at the viewer's size, read
//! back for the front end to show, and plays the sound itself.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::thread::JoinHandle;

/// Video, and sound alone.
pub const EXTENSIONS: &[&str] = &[
    "3gp", "avi", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "ogv", "ts",
    "vob", "webm", "wmv", "aac", "aiff", "ape", "flac", "m4a", "mka", "mp3", "oga", "ogg",
    "opus", "wav", "wma",
];

pub fn is_media(path: &Path) -> bool {
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    extension.is_some_and(|extension| EXTENSIONS.contains(&extension.as_str()))
}

/// libmpv is there, and this system's OpenGL is drawn with.
pub fn available() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos", windows)) && library().is_ok()
}

/// A frame in BGRA, top row first.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Seconds.
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    /// Percent.
    pub volume: f64,
    pub muted: bool,
    /// Pictures are drawn; not for sound alone.
    pub video: bool,
    pub error: Option<String>,
}

/// `1:05` or `1:02:05`.
pub fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.) as u64;
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    match h {
        0 => format!("{m}:{s:02}"),
        _ => format!("{h}:{m:02}:{s:02}"),
    }
}

/// Plays a file until dropped.
pub struct Player {
    mpv: &'static Mpv,
    handle: Raw,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Player {
    /// Starts playing, or `paused`; `notify`, called on other threads, says a frame or the state changed.
    pub fn open(path: &Path, paused: bool, notify: impl Fn() + Send + Sync + 'static) -> Result<Player, String> {
        let mpv = library()?;
        // SAFETY: a plain constructor; null when it fails.
        let handle = Raw(unsafe { (mpv.create)() });
        if handle.0.is_null() {
            return Err("libmpv could not start".into());
        }
        let shared = Arc::new(Shared {
            wake: Mutex::default(),
            cond: Condvar::new(),
            frame: Mutex::default(),
            error: Mutex::default(),
            quit: AtomicBool::new(false),
            notify: Box::new(notify),
        });
        let mut player = Player {
            mpv,
            handle,
            shared,
            threads: Vec::new(),
        };
        // Pictures to us; the last frame stays at the end; the GPU decodes where safe.
        for (name, value) in [
            ("vo", "libmpv"),
            ("keep-open", "yes"),
            ("hwdec", "auto-safe"),
            // mpv's probe of NVIDIA's prints "Cannot load libcuda.so.1" where it is missing.
            ("gpu-hwdec-interop", if cfg!(target_os = "linux") && !nvidia() { "vaapi" } else { "auto" }),
            ("input-default-bindings", "no"),
            ("terminal", "no"),
            ("pause", if paused { "yes" } else { "no" }),
        ] {
            let (name, value) = (text(name)?, text(value)?);
            // SAFETY: valid strings, a live handle.
            player.check(unsafe { (mpv.set_option_string)(handle.0, name.as_ptr(), value.as_ptr()) })?;
        }
        // SAFETY: a live handle.
        player.check(unsafe { (mpv.initialize)(handle.0) })?;
        for name in ["time-pos", "duration", "pause", "volume", "mute", "vo-configured"] {
            let name = text(name)?;
            // SAFETY: as above; changes come as events.
            unsafe { (mpv.observe_property)(handle.0, 0, name.as_ptr(), FORMAT_NONE) };
        }
        // The render context must be there before the file starts.
        let (ready, started) = mpsc::channel();
        let shared = player.shared.clone();
        player
            .threads
            .push(std::thread::spawn(move || render(mpv, handle, shared, ready)));
        started
            .recv()
            .map_err(|_| "the video thread ended".to_string())??;
        let shared = player.shared.clone();
        player
            .threads
            .push(std::thread::spawn(move || events(mpv, handle, shared)));
        player.command(&["loadfile", &path.to_string_lossy()])?;
        Ok(player)
    }

    pub fn toggle_pause(&self) {
        let _ = self.command(&["cycle", "pause"]);
    }

    /// Seconds forward, or back when negative.
    pub fn seek_by(&self, seconds: f64) {
        let _ = self.command(&["seek", &seconds.to_string(), "relative"]);
    }

    /// To `seconds` from the start, exactly there.
    pub fn seek_seconds(&self, seconds: f64) {
        let _ = self.command(&["seek", &seconds.max(0.).to_string(), "absolute+exact"]);
    }

    /// To a share of the whole, 0 to 1: exactly there, not at the keyframe before.
    pub fn seek_to(&self, share: f64) {
        let percent = (share.clamp(0., 1.) * 100.).to_string();
        let _ = self.command(&["seek", &percent, "absolute-percent+exact"]);
    }

    pub fn change_volume(&self, by: f64) {
        let _ = self.command(&["add", "volume", &by.to_string()]);
    }

    pub fn toggle_mute(&self) {
        let _ = self.command(&["cycle", "mute"]);
    }

    /// The size frames are drawn at, in pixels: the view's.
    pub fn set_size(&self, width: u32, height: u32) {
        self.shared.poke(|wake| {
            if wake.size != (width, height) {
                (wake.size, wake.resized) = ((width, height), true);
            }
        });
    }

    /// The newest frame not taken yet.
    pub fn take_frame(&self) -> Option<Frame> {
        self.shared.frame.lock().unwrap().take()
    }

    pub fn state(&self) -> State {
        State {
            position: self.double("time-pos").unwrap_or(0.),
            duration: self.double("duration").unwrap_or(0.),
            paused: self.flag("pause"),
            volume: self.double("volume").unwrap_or(100.),
            muted: self.flag("mute"),
            video: self.flag("vo-configured"),
            error: self.shared.error.lock().unwrap().clone(),
        }
    }

    fn double(&self, name: &str) -> Option<f64> {
        let mut value = 0f64;
        let found = self.property(name, FORMAT_DOUBLE, &mut value as *mut f64 as *mut c_void);
        found.then_some(value)
    }

    fn flag(&self, name: &str) -> bool {
        let mut value: c_int = 0;
        self.property(name, FORMAT_FLAG, &mut value as *mut c_int as *mut c_void) && value != 0
    }

    fn property(&self, name: &str, format: c_int, value: *mut c_void) -> bool {
        let Ok(name) = text(name) else { return false };
        // SAFETY: `value` fits the format.
        unsafe { (self.mpv.get_property)(self.handle.0, name.as_ptr(), format, value) >= 0 }
    }

    fn command(&self, args: &[&str]) -> Result<(), String> {
        let args = args.iter().map(|arg| text(arg)).collect::<Result<Vec<_>, _>>()?;
        let mut pointers: Vec<*const c_char> = args.iter().map(|arg| arg.as_ptr()).collect();
        pointers.push(std::ptr::null());
        // SAFETY: a null-ended list of strings that outlive the call.
        self.check(unsafe { (self.mpv.command)(self.handle.0, pointers.as_mut_ptr()) })
    }

    fn check(&self, code: c_int) -> Result<(), String> {
        match code {
            0.. => Ok(()),
            _ => Err(error(self.mpv, code)),
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.poke(|wake| wake.stop = true);
        self.shared.quit.store(true, Ordering::Release);
        // SAFETY: wakes the event thread's wait; the handle is still alive.
        unsafe { (self.mpv.wakeup)(self.handle.0) };
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        // SAFETY: the render context is freed and no thread uses the handle now.
        unsafe { (self.mpv.terminate_destroy)(self.handle.0) };
    }
}

/// NVIDIA's CUDA is installed: its hardware decoding is worth probing.
fn nvidia() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    // SAFETY: loads NVIDIA's driver library, as mpv would.
    *FOUND.get_or_init(|| cfg!(target_os = "linux") && unsafe { libloading::Library::new("libcuda.so.1") }.is_ok())
}

fn text(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|err| err.to_string())
}

fn error(mpv: &Mpv, code: c_int) -> String {
    // SAFETY: mpv's static message for the code.
    let message = unsafe { CStr::from_ptr((mpv.error_string)(code)) };
    message.to_string_lossy().into_owned()
}

#[derive(Default)]
struct Wake {
    frame: bool,
    size: (u32, u32),
    resized: bool,
    stop: bool,
}

struct Shared {
    wake: Mutex<Wake>,
    cond: Condvar,
    frame: Mutex<Option<Frame>>,
    error: Mutex<Option<String>>,
    /// For the event thread.
    quit: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn poke(&self, change: impl FnOnce(&mut Wake)) {
        change(&mut self.wake.lock().unwrap());
        self.cond.notify_one();
    }
}

/// mpv has a frame to draw; called on its threads.
unsafe extern "C" fn on_update(shared: *mut c_void) {
    // SAFETY: the render thread's Arc keeps it until the callback is removed.
    let shared = unsafe { &*(shared as *const Shared) };
    shared.poke(|wake| wake.frame = true);
}

/// Draws frames on a thread of its own, where its OpenGL context is current;
/// on the CPU where OpenGL is too old for mpv (Windows' own 1.1, in Remote Desktop).
fn render(mpv: &'static Mpv, handle: Raw, shared: Arc<Shared>, ready: mpsc::Sender<Result<(), String>>) {
    let mut context = null_mut();
    let opengl = gl::Context::new().ok().filter(|opengl| {
        let mut init = GlInit {
            get_proc_address: gl::proc_address,
            context: &**opengl as *const gl::Context as *mut c_void,
        };
        let mut params = [
            Param::new(PARAM_API_TYPE, c"opengl".as_ptr() as *mut c_void),
            Param::new(PARAM_OPENGL_INIT, &mut init as *mut GlInit as *mut c_void),
            Param::END,
        ];
        // SAFETY: the parameters live through the call; the context is current.
        unsafe { (mpv.render_create)(&mut context, handle.0, params.as_mut_ptr()) >= 0 }
    });
    if opengl.is_none() {
        let mut params = [Param::new(PARAM_API_TYPE, c"sw".as_ptr() as *mut c_void), Param::END];
        // SAFETY: as above.
        let code = unsafe { (mpv.render_create)(&mut context, handle.0, params.as_mut_ptr()) };
        if code < 0 {
            return drop(ready.send(Err(format!("mpv cannot draw: {}", error(mpv, code)))));
        }
    }
    let callback = Arc::as_ptr(&shared) as *mut c_void;
    // SAFETY: `shared` outlives the callback, removed below.
    unsafe { (mpv.render_set_update_callback)(context, Some(on_update), callback) };
    let _ = ready.send(Ok(()));
    let mut target = opengl.as_deref().map(gl::Target::new);
    loop {
        let (resized, (width, height)) = {
            let mut wake = shared.wake.lock().unwrap();
            while !(wake.frame || wake.resized || wake.stop) {
                wake = shared.cond.wait(wake).unwrap();
            }
            if wake.stop {
                break;
            }
            let resized = wake.resized;
            (wake.frame, wake.resized) = (false, false);
            (resized, wake.size)
        };
        // SAFETY: on the context's thread.
        let flags = unsafe { (mpv.render_update)(context) };
        if flags & UPDATE_FRAME == 0 && !resized || width == 0 || height == 0 {
            continue;
        }
        let bgra = match &mut target {
            Some(target) => {
                let mut fbo = Fbo {
                    fbo: target.bind(width, height) as c_int,
                    w: width as c_int,
                    h: height as c_int,
                    internal_format: GL_RGBA8,
                };
                // Not flipped: in a framebuffer of its own mpv draws the top row first.
                let mut params = [
                    Param::new(PARAM_OPENGL_FBO, &mut fbo as *mut Fbo as *mut c_void),
                    Param::END,
                ];
                // SAFETY: as above; waits for the frame's time.
                unsafe { (mpv.render_render)(context, params.as_mut_ptr()) };
                target.read(width, height)
            }
            None => {
                let mut bgra = vec![0u8; width as usize * height as usize * 4];
                let mut size = [width as c_int, height as c_int];
                let mut stride = width as usize * 4;
                let mut params = [
                    Param::new(PARAM_SW_SIZE, size.as_mut_ptr() as *mut c_void),
                    Param::new(PARAM_SW_FORMAT, c"bgr0".as_ptr() as *mut c_void),
                    Param::new(PARAM_SW_STRIDE, &mut stride as *mut usize as *mut c_void),
                    Param::new(PARAM_SW_POINTER, bgra.as_mut_ptr() as *mut c_void),
                    Param::END,
                ];
                // SAFETY: as above; the buffer has `stride` bytes for each row.
                unsafe { (mpv.render_render)(context, params.as_mut_ptr()) };
                // The fourth byte is left as it was.
                bgra.chunks_exact_mut(4).for_each(|pixel| pixel[3] = 255);
                bgra
            }
        };
        *shared.frame.lock().unwrap() = Some(Frame {
            width,
            height,
            bgra,
        });
        (shared.notify)();
    }
    // SAFETY: freed on its thread, before the handle is destroyed.
    unsafe {
        (mpv.render_set_update_callback)(context, None, null_mut());
        (mpv.render_free)(context);
    }
    drop(target);
}

/// Errors and changes of the state, to the front end.
fn events(mpv: &'static Mpv, handle: Raw, shared: Arc<Shared>) {
    loop {
        // SAFETY: the handle lives until this thread is joined.
        let event = unsafe { &*(mpv.wait_event)(handle.0, -1.) };
        if shared.quit.load(Ordering::Acquire) || event.id == EVENT_SHUTDOWN {
            break;
        }
        match event.id {
            EVENT_END_FILE => {
                // SAFETY: an end-file event's data.
                let end = unsafe { &*(event.data as *const EndFile) };
                if end.reason == END_FILE_ERROR {
                    *shared.error.lock().unwrap() = Some(error(mpv, end.error));
                }
                (shared.notify)();
            }
            EVENT_FILE_LOADED | EVENT_PROPERTY_CHANGE => (shared.notify)(),
            _ => {}
        }
    }
}

// libmpv's C interface (client.h, render.h, render_gl.h).

#[repr(C)]
struct Handle {
    _opaque: [u8; 0],
}

#[repr(C)]
struct RenderContext {
    _opaque: [u8; 0],
}

#[derive(Clone, Copy)]
struct Raw(*mut Handle);

// SAFETY: libmpv's client API may be called from any thread.
unsafe impl Send for Raw {}
unsafe impl Sync for Raw {}

#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

/// The first fields of mpv_event_end_file.
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct Param {
    kind: c_int,
    data: *mut c_void,
}

impl Param {
    const END: Param = Param {
        kind: 0,
        data: null_mut(),
    };

    fn new(kind: c_int, data: *mut c_void) -> Self {
        Param { kind, data }
    }
}

#[repr(C)]
struct GlInit {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    context: *mut c_void,
}

#[repr(C)]
struct Fbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

const FORMAT_NONE: c_int = 0;
const FORMAT_FLAG: c_int = 3;
const FORMAT_DOUBLE: c_int = 5;
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PROPERTY_CHANGE: c_int = 22;
const END_FILE_ERROR: c_int = 4;
const PARAM_API_TYPE: c_int = 1;
const PARAM_OPENGL_INIT: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_SW_SIZE: c_int = 17;
const PARAM_SW_FORMAT: c_int = 18;
const PARAM_SW_STRIDE: c_int = 19;
const PARAM_SW_POINTER: c_int = 20;
const UPDATE_FRAME: u64 = 1;
const GL_RGBA8: c_int = 0x8058;

type Create = unsafe extern "C" fn() -> *mut Handle;
type OfHandle = unsafe extern "C" fn(*mut Handle);
type Initialize = unsafe extern "C" fn(*mut Handle) -> c_int;
type SetOption = unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int;
type Command = unsafe extern "C" fn(*mut Handle, *mut *const c_char) -> c_int;
type GetProperty = unsafe extern "C" fn(*mut Handle, *const c_char, c_int, *mut c_void) -> c_int;
type Observe = unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int) -> c_int;
type WaitEvent = unsafe extern "C" fn(*mut Handle, f64) -> *mut Event;
type ErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type RenderCreate = unsafe extern "C" fn(*mut *mut RenderContext, *mut Handle, *mut Param) -> c_int;
type UpdateCallback = Option<unsafe extern "C" fn(*mut c_void)>;
type SetCallback = unsafe extern "C" fn(*mut RenderContext, UpdateCallback, *mut c_void);
type RenderUpdate = unsafe extern "C" fn(*mut RenderContext) -> u64;
type RenderRender = unsafe extern "C" fn(*mut RenderContext, *mut Param) -> c_int;
type RenderFree = unsafe extern "C" fn(*mut RenderContext);

struct Mpv {
    create: Create,
    initialize: Initialize,
    terminate_destroy: OfHandle,
    wakeup: OfHandle,
    set_option_string: SetOption,
    command: Command,
    get_property: GetProperty,
    observe_property: Observe,
    wait_event: WaitEvent,
    error_string: ErrorString,
    render_create: RenderCreate,
    render_set_update_callback: SetCallback,
    render_update: RenderUpdate,
    render_render: RenderRender,
    render_free: RenderFree,
    _library: libloading::Library,
}

/// Linux's; macOS and Windows have none, Power Commander ships its own (vendor/mpv).
const NAMES: &[&str] = &["libmpv.so.2", "libmpv.so"];
/// The shipped one, and what makes it.
pub(crate) const BUNDLED: Option<(&str, &str)> = match () {
    _ if cfg!(target_os = "macos") => Some(("libmpv.2.dylib", "build.sh")),
    _ if cfg!(windows) => Some(("libmpv-2.dll", "fetch.bat")),
    _ => None,
};

/// Loaded once found; tried again while missing, as an add-on may come meanwhile.
fn library() -> Result<&'static Mpv, String> {
    static MPV: OnceLock<Mpv> = OnceLock::new();
    if let Some(mpv) = MPV.get() {
        return Ok(mpv);
    }
    let mpv = load()?;
    Ok(MPV.get_or_init(|| mpv))
}

fn load() -> Result<Mpv, String> {
    let library = if let Some((name, script)) = BUNDLED {
        let path = crate::bundled::path("mpv", name);
        if !path.exists() {
            return Err(format!("libmpv is missing: Settings > Add-ons downloads it (a source build: vendor/mpv/{script})"));
        }
        // SAFETY: loading libmpv runs only its libraries' initialisers.
        unsafe { libloading::Library::new(&path) }.map_err(|err| format!("libmpv: {err}"))?
    } else {
        // SAFETY: as above.
        NAMES
            .iter()
            .find_map(|name| unsafe { libloading::Library::new(name) }.ok())
            .ok_or_else(|| format!("libmpv is not installed ({})", NAMES[0]))?
    };
    /// A function of libmpv's, typed as the header says.
    fn get<T: Copy>(library: &libloading::Library, name: &str) -> Result<T, String> {
        // SAFETY: the types above follow libmpv's headers.
        let symbol = unsafe { library.get::<T>(name.as_bytes()) };
        symbol.map(|symbol| *symbol).map_err(|err| format!("libmpv: {err}"))
    }
    Ok(Mpv {
        create: get(&library, "mpv_create")?,
        initialize: get(&library, "mpv_initialize")?,
        terminate_destroy: get(&library, "mpv_terminate_destroy")?,
        wakeup: get(&library, "mpv_wakeup")?,
        set_option_string: get(&library, "mpv_set_option_string")?,
        command: get(&library, "mpv_command")?,
        get_property: get(&library, "mpv_get_property")?,
        observe_property: get(&library, "mpv_observe_property")?,
        wait_event: get(&library, "mpv_wait_event")?,
        error_string: get(&library, "mpv_error_string")?,
        render_create: get(&library, "mpv_render_context_create")?,
        render_set_update_callback: get(&library, "mpv_render_context_set_update_callback")?,
        render_update: get(&library, "mpv_render_context_update")?,
        render_render: get(&library, "mpv_render_context_render")?,
        render_free: get(&library, "mpv_render_context_free")?,
        _library: library,
    })
}

/// OpenGL with no window, EGL's on Linux, CGL's on macOS, WGL's on Windows;
/// a framebuffer to draw in.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod gl {
    use std::ffi::{CStr, c_char, c_void};

    use glow::HasContext;

    #[cfg(target_os = "macos")]
    pub use cgl::Context;
    #[cfg(target_os = "linux")]
    pub use egl::Context;
    #[cfg(windows)]
    pub use wgl::Context;

    #[cfg(target_os = "linux")]
    mod egl {
        use std::ffi::c_void;

        use khronos_egl as egl;

        pub struct Context {
            instance: egl::DynamicInstance<egl::EGL1_4>,
            display: egl::Display,
            context: egl::Context,
        }

        impl Context {
            /// Current on this thread from now on; boxed, as mpv keeps its address.
            pub fn new() -> Result<Box<Context>, String> {
                let failed = |err: egl::Error| format!("OpenGL (EGL): {err}");
                // SAFETY: loads the system's libEGL.
                let instance = unsafe { egl::DynamicInstance::<egl::EGL1_4>::load_required() }
                    .map_err(|err| format!("OpenGL (EGL) is not there: {err}"))?;
                // SAFETY: the default display needs no native handle.
                let display = unsafe { instance.get_display(egl::DEFAULT_DISPLAY) }
                    .ok_or("OpenGL (EGL): no display")?;
                instance.initialize(display).map_err(failed)?;
                instance.bind_api(egl::OPENGL_API).map_err(failed)?;
                // Any surface type: frames are drawn in framebuffers (the default asks for windows).
                let attributes = [egl::RENDERABLE_TYPE, egl::OPENGL_BIT, egl::SURFACE_TYPE, 0, egl::NONE];
                let config = instance
                    .choose_first_config(display, &attributes)
                    .map_err(failed)?
                    .ok_or("OpenGL (EGL): no configuration for OpenGL")?;
                let context = instance
                    .create_context(display, config, None, &[egl::NONE])
                    .map_err(failed)?;
                // No surface: everything is drawn in framebuffers.
                instance
                    .make_current(display, None, None, Some(context))
                    .map_err(failed)?;
                Ok(Box::new(Context {
                    instance,
                    display,
                    context,
                }))
            }

            pub fn address(&self, name: &str) -> *const c_void {
                let found = self.instance.get_proc_address(name);
                found.map_or(std::ptr::null(), |function| function as *const c_void)
            }
        }

        impl Drop for Context {
            fn drop(&mut self) {
                // The display stays initialised: other players may share it.
                let _ = self.instance.make_current(self.display, None, None, None);
                let _ = self.instance.destroy_context(self.display, self.context);
            }
        }
    }

    /// Apple deprecated OpenGL for Metal, but every Mac still has it, as mpv
    /// draws with it: a context of its core profile, with no window.
    #[cfg(target_os = "macos")]
    mod cgl {
        use std::ffi::{c_int, c_void};
        use std::ptr::{null, null_mut};

        type PixelFormat = *mut c_void;
        type Cgl = *mut c_void;

        // CGLTypes.h.
        const PROFILE: c_int = 99;
        const CORE_3_2: c_int = 0x3200;
        const ACCELERATED: c_int = 73;
        const OFFLINE_RENDERERS: c_int = 96;

        pub struct Context {
            context: Cgl,
            set_current: unsafe extern "C" fn(Cgl) -> c_int,
            destroy: unsafe extern "C" fn(Cgl) -> c_int,
            /// OpenGL's functions are found in it.
            library: libloading::Library,
        }

        /// A function of CGL's, typed as its header says.
        fn get<T: Copy>(library: &libloading::Library, name: &str) -> Result<T, String> {
            // SAFETY: the types above follow CGL's headers.
            let symbol = unsafe { library.get::<T>(name.as_bytes()) };
            symbol.map(|symbol| *symbol).map_err(|err| format!("OpenGL (CGL): {err}"))
        }

        impl Context {
            /// Current on this thread from now on; boxed, as mpv keeps its address.
            pub fn new() -> Result<Box<Context>, String> {
                let path = "/System/Library/Frameworks/OpenGL.framework/OpenGL";
                // SAFETY: loads the system's OpenGL.
                let library = unsafe { libloading::Library::new(path) }
                    .map_err(|err| format!("OpenGL is not there: {err}"))?;
                let choose: unsafe extern "C" fn(*const c_int, *mut PixelFormat, *mut c_int) -> c_int =
                    get(&library, "CGLChoosePixelFormat")?;
                let create: unsafe extern "C" fn(PixelFormat, Cgl, *mut Cgl) -> c_int =
                    get(&library, "CGLCreateContext")?;
                let release: unsafe extern "C" fn(PixelFormat) -> c_int = get(&library, "CGLDestroyPixelFormat")?;
                let set_current = get(&library, "CGLSetCurrentContext")?;
                let destroy = get(&library, "CGLDestroyContext")?;
                // The GPU, a laptop's other one too; everything is drawn in framebuffers.
                let attributes = [PROFILE, CORE_3_2, ACCELERATED, OFFLINE_RENDERERS, 0];
                let (mut format, mut formats) = (null_mut(), 0);
                // SAFETY: a zero-ended list; the format is released below.
                let code = unsafe { choose(attributes.as_ptr(), &mut format, &mut formats) };
                if code != 0 || format.is_null() {
                    return Err(format!("OpenGL (CGL): no pixel format ({code})"));
                }
                let mut context = null_mut();
                // SAFETY: as above; no context to share.
                let code = unsafe { create(format, null_mut(), &mut context) };
                // SAFETY: the context keeps what it needs of it.
                unsafe { release(format) };
                if code != 0 {
                    return Err(format!("OpenGL (CGL): no context ({code})"));
                }
                let context = Context {
                    context,
                    set_current,
                    destroy,
                    library,
                };
                // SAFETY: a live context; dropped, it is no longer current.
                let code = unsafe { (context.set_current)(context.context) };
                if code != 0 {
                    return Err(format!("OpenGL (CGL): the context cannot be used ({code})"));
                }
                Ok(Box::new(context))
            }

            pub fn address(&self, name: &str) -> *const c_void {
                // SAFETY: only the address is taken; mpv and glow type it.
                let found = unsafe { self.library.get::<*const c_void>(name.as_bytes()) };
                found.map_or(null(), |function| *function)
            }
        }

        impl Drop for Context {
            fn drop(&mut self) {
                // SAFETY: made current on this thread, which drops it.
                unsafe {
                    (self.set_current)(null_mut());
                    (self.destroy)(self.context);
                }
            }
        }
    }

    /// WGL wants a window's device context: a hidden one, never shown.
    #[cfg(windows)]
    mod wgl {
        use std::ffi::{CString, c_void};
        use std::ptr::null;

        use windows::Win32::Foundation::{HMODULE, HWND};
        use windows::Win32::Graphics::Gdi::{GetDC, HDC, ReleaseDC};
        use windows::Win32::Graphics::OpenGL::{
            ChoosePixelFormat, HGLRC, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA,
            PIXELFORMATDESCRIPTOR, SetPixelFormat, wglCreateContext, wglDeleteContext, wglGetProcAddress,
            wglMakeCurrent,
        };
        use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP};
        use windows::core::{PCSTR, w};

        pub struct Context {
            window: HWND,
            dc: HDC,
            context: HGLRC,
            /// OpenGL 1.1's functions, which wglGetProcAddress does not give.
            opengl32: HMODULE,
        }

        impl Context {
            /// Current on this thread from now on; boxed, as mpv keeps its address.
            pub fn new() -> Result<Box<Context>, String> {
                let failed = |err: windows::core::Error| format!("OpenGL (WGL): {err}");
                // SAFETY: a hidden window of this thread, destroyed with the context.
                let window = unsafe {
                    CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 1, 1, None, None, None, None)
                }
                .map_err(failed)?;
                // SAFETY: the window's own device context, released in drop.
                let dc = unsafe { GetDC(Some(window)) };
                let mut context = Context { window, dc, context: HGLRC::default(), opengl32: HMODULE::default() };
                let format = PIXELFORMATDESCRIPTOR {
                    nSize: size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                    nVersion: 1,
                    dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                    iPixelType: PFD_TYPE_RGBA,
                    cColorBits: 32,
                    ..Default::default()
                };
                // SAFETY: OpenGL calls on a live device context; the context is current from here.
                unsafe {
                    let chosen = ChoosePixelFormat(dc, &format);
                    if chosen == 0 {
                        return Err("OpenGL (WGL): no pixel format".into());
                    }
                    SetPixelFormat(dc, chosen, &format).map_err(failed)?;
                    context.context = wglCreateContext(dc).map_err(failed)?;
                    wglMakeCurrent(dc, context.context).map_err(failed)?;
                    context.opengl32 = GetModuleHandleW(w!("opengl32.dll")).map_err(failed)?;
                }
                Ok(Box::new(context))
            }

            pub fn address(&self, name: &str) -> *const c_void {
                let Ok(name) = CString::new(name) else { return null() };
                let name = PCSTR(name.as_ptr() as *const u8);
                // SAFETY: the context is current; some drivers answer 1, 2, 3 or -1 for none.
                let found = unsafe { wglGetProcAddress(name) }.filter(|f| !matches!(*f as usize, 1..=3 | usize::MAX));
                let found = found.or_else(|| unsafe { GetProcAddress(self.opengl32, name) });
                found.map_or(null(), |function| function as *const c_void)
            }
        }

        impl Drop for Context {
            fn drop(&mut self) {
                // SAFETY: made current on this thread, which drops it.
                unsafe {
                    let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
                    let _ = wglDeleteContext(self.context);
                    ReleaseDC(Some(self.window), self.dc);
                    let _ = DestroyWindow(self.window);
                }
            }
        }
    }

    /// For mpv: OpenGL's functions by name.
    pub unsafe extern "C" fn proc_address(context: *mut c_void, name: *const c_char) -> *mut c_void {
        // SAFETY: mpv passes back the Context it was given, and a C string.
        let (context, name) = unsafe { (&*(context as *const Context), CStr::from_ptr(name)) };
        context.address(&name.to_string_lossy()) as *mut c_void
    }

    pub struct Target {
        gl: glow::Context,
        framebuffer: Option<(glow::Framebuffer, glow::Texture, u32, u32)>,
    }

    impl Target {
        pub fn new(context: &Context) -> Self {
            // SAFETY: the context is current on this thread.
            let gl = unsafe { glow::Context::from_loader_function(|name| context.address(name)) };
            Target {
                gl,
                framebuffer: None,
            }
        }

        /// A framebuffer of this size, made anew when it changes; its name.
        pub fn bind(&mut self, width: u32, height: u32) -> u32 {
            let gl = &self.gl;
            // SAFETY: OpenGL calls on the context's thread.
            unsafe {
                match self.framebuffer {
                    Some((framebuffer, _, w, h)) if (w, h) == (width, height) => {
                        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                    }
                    _ => {
                        if let Some((framebuffer, texture, ..)) = self.framebuffer.take() {
                            gl.delete_framebuffer(framebuffer);
                            gl.delete_texture(texture);
                        }
                        let texture = gl.create_texture().expect("an OpenGL texture");
                        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                        gl.tex_image_2d(
                            glow::TEXTURE_2D,
                            0,
                            glow::RGBA8 as i32,
                            width as i32,
                            height as i32,
                            0,
                            glow::RGBA,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(None),
                        );
                        let framebuffer = gl.create_framebuffer().expect("an OpenGL framebuffer");
                        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                        gl.framebuffer_texture_2d(
                            glow::FRAMEBUFFER,
                            glow::COLOR_ATTACHMENT0,
                            glow::TEXTURE_2D,
                            Some(texture),
                            0,
                        );
                        self.framebuffer = Some((framebuffer, texture, width, height));
                    }
                }
            }
            self.framebuffer.map_or(0, |(framebuffer, ..)| framebuffer.0.get())
        }

        /// The framebuffer's pixels in BGRA, the rows as mpv drew them.
        pub fn read(&self, width: u32, height: u32) -> Vec<u8> {
            let mut pixels = vec![0; width as usize * height as usize * 4];
            // SAFETY: as above; the buffer fits the rows read.
            unsafe {
                // mpv leaves its own framebuffers bound.
                let framebuffer = self.framebuffer.map(|(framebuffer, ..)| framebuffer);
                self.gl.bind_framebuffer(glow::FRAMEBUFFER, framebuffer);
                self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
                self.gl.read_pixels(
                    0,
                    0,
                    width as i32,
                    height as i32,
                    glow::BGRA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut pixels)),
                );
            }
            pixels
        }
    }

    impl Drop for Target {
        fn drop(&mut self) {
            if let Some((framebuffer, texture, ..)) = self.framebuffer.take() {
                // SAFETY: as above.
                unsafe {
                    self.gl.delete_framebuffer(framebuffer);
                    self.gl.delete_texture(texture);
                }
            }
        }
    }
}

/// Elsewhere for now: no OpenGL context to draw with.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod gl {
    use std::ffi::{c_char, c_void};

    pub struct Context;

    impl Context {
        pub fn new() -> Result<Box<Context>, String> {
            Err("video is played only on Linux, macOS and Windows yet".into())
        }
    }

    pub unsafe extern "C" fn proc_address(_: *mut c_void, _: *const c_char) -> *mut c_void {
        std::ptr::null_mut()
    }

    pub struct Target;

    impl Target {
        pub fn new(_: &Context) -> Self {
            Target
        }

        pub fn bind(&mut self, _: u32, _: u32) -> u32 {
            0
        }

        pub fn read(&self, width: u32, height: u32) -> Vec<u8> {
            vec![0; width as usize * height as usize * 4]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_media_and_clocks() {
        assert!(is_media(Path::new("/a/Film.MKV")));
        assert!(is_media(Path::new("song.flac")));
        assert!(!is_media(Path::new("notes.txt")));
        assert_eq!(clock(65.4), "1:05");
        assert_eq!(clock(3725.), "1:02:05");
        assert_eq!(clock(-1.), "0:00");
    }

    /// Plays a second of video mpv makes itself (FFmpeg's lavfi), with the shipped or system libmpv.
    #[test]
    #[ignore = "needs libmpv"]
    fn draws_frames() {
        // White above, black below: the frame must come the right way up.
        let file = Path::new("av://lavfi:color=c=black:size=320x240:rate=25:duration=1,drawbox=x=0:y=0:w=320:h=120:color=white:t=fill");
        let (sent, frames) = mpsc::channel();
        let player = Player::open(file, false, move || {
            let _ = sent.send(());
        })
        .unwrap();
        player.set_size(160, 120);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        // The first may be black: drawn for the size, before the video's first.
        let frame = loop {
            let _ = frames.recv_timeout(std::time::Duration::from_millis(100));
            let frame = player.take_frame();
            if let Some(frame) = frame.filter(|f| f.bgra.chunks(4).any(|px| px[..3] != [0; 3])) {
                break frame;
            }
            assert!(std::time::Instant::now() < deadline, "no frame with a picture: {:?}", player.state());
        };
        assert_eq!((frame.width, frame.height, frame.bgra.len()), (160, 120, 160 * 120 * 4));
        let pixel = |x: usize, y: usize| frame.bgra[(y * 160 + x) * 4];
        let (top, bottom) = (pixel(80, 10), pixel(80, 110));
        assert!(top > 200 && bottom < 50, "the right way up: {top} above, {bottom} below");
        let state = player.state();
        assert!(state.duration > 0.9 && state.video && state.error.is_none(), "{state:?}");
        drop(player);
    }
}
