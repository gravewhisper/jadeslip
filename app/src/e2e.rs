use crate::schemes::UI_URL;
use serde_json::{Value, json};
use jadeslip_core::{ChapterIx, Extracted, Reader, RenderedChapter};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, Url};

pub struct Switches {
    pub csp: bool,
    pub sanitizer: bool,
    pub nav_guard: bool,
}

pub fn switches() -> &'static Switches {
    static SWITCHES: OnceLock<Switches> = OnceLock::new();
    SWITCHES.get_or_init(|| {
        let off = std::env::var("JADESLIP_E2E_OFF").unwrap_or_default();
        let on = |name: &str| !off.split(',').any(|item| item == name);
        Switches {
            csp: on("csp"),
            sanitizer: on("sanitizer"),
            nav_guard: on("nav_guard"),
        }
    })
}

const RAIL_WIDTH: usize = 296;

static PHASES: Mutex<Vec<(&'static str, f64)>> = Mutex::new(Vec::new());
static NAVIGATIONS: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
static EXTERNAL: Mutex<Vec<String>> = Mutex::new(Vec::new());
static PWNED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static PENDING: OnceLock<Mutex<HashMap<u64, mpsc::Sender<String>>>> = OnceLock::new();
static NEXT_EVAL: AtomicU64 = AtomicU64::new(1);
static INJECTED_FAILURE: Mutex<Option<String>> = Mutex::new(None);

fn wall_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1e3)
}

pub fn mark(name: &'static str) {
    if let Ok(mut phases) = PHASES.lock() {
        phases.push((name, wall_ms()));
    }
}

pub enum SaveAnswer {
    Path(std::path::PathBuf),
    Cancel,
    RealDialog,
}

static SAVE_ANSWER: Mutex<Option<SaveAnswer>> = Mutex::new(None);

pub fn take_save_answer() -> SaveAnswer {
    logged_lock(&SAVE_ANSWER)
        .take()
        .unwrap_or(SaveAnswer::Cancel)
}

fn logged_lock<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn before_gtk() {
    glib::set_prgname(Some("jadeslip-test"));
}

pub fn navigation_decision(url: &Url, allowed: bool) -> bool {
    if let Ok(mut log) = NAVIGATIONS.lock() {
        log.push((url.as_str().to_owned(), allowed));
    }
    allowed || !switches().nav_guard
}

pub fn unsanitized_if_switched(
    reader: &Reader,
    ix: ChapterIx,
    rendered: RenderedChapter,
) -> RenderedChapter {
    if switches().sanitizer {
        return rendered;
    }
    match reader.extract(ix) {
        Ok(Extracted::Document(bytes)) => RenderedChapter {
            xhtml: String::from_utf8_lossy(&bytes).into_owned(),
            css: Vec::new(),
            ..rendered
        },
        _ => rendered,
    }
}

pub fn record_external(url: &str) {
    if let Ok(mut log) = EXTERNAL.lock() {
        log.push(url.to_owned());
    }
    let Ok(parsed) = Url::parse(url) else { return };
    if parsed.host_str() != Some("127.0.0.1") || !parsed.path().starts_with("/C/") {
        return;
    }
    let port = parsed.port().unwrap_or(80);
    let path = parsed.path().to_owned();
    std::thread::spawn(move || {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = write!(stream, "GET {path} HTTP/1.0\r\n\r\n");
            let _ = stream.read_to_end(&mut Vec::new());
        }
    });
}

pub fn injected_save_failure() -> Result<(), crate::commands::SaveFailure> {
    let injected = INJECTED_FAILURE.lock().ok().and_then(|slot| slot.clone());
    match injected.as_deref() {
        Some("wouldBreak") => Err(jadeslip_core::Error::WouldBreak(
            "This edit could not be saved safely, so the book was not changed.".to_owned(),
        )
        .into()),
        Some("delay") => {
            std::thread::sleep(Duration::from_millis(400));
            Ok(())
        }
        _ => Ok(()),
    }
}

pub fn injected_apply_failure() -> Option<String> {
    let injected = INJECTED_FAILURE.lock().ok().and_then(|slot| slot.clone());
    (injected.as_deref() == Some("wouldBreak"))
        .then(|| "This save failed its final check, so the book was not changed.".to_owned())
}

pub fn spell_delay() {
    if let Some(millis) = std::env::var("JADESLIP_E2E_SPELL_DELAY_MS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        std::thread::sleep(Duration::from_millis(millis));
    }
}

pub fn clock_ahead_ms() -> u64 {
    std::env::var("JADESLIP_E2E_CLOCK_AHEAD_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

pub fn search_delay() -> Option<jadeslip_core::ChapterDelay> {
    let spec = std::env::var("JADESLIP_E2E_SEARCH_DELAY").ok()?;
    let (limit, millis) = spec.split_once(':')?;
    let limit: u32 = limit.parse().ok()?;
    let millis: u64 = millis.parse().ok()?;
    Some(std::sync::Arc::new(move |chapter: ChapterIx| {
        if chapter.0 < limit {
            Duration::from_millis(millis)
        } else {
            Duration::ZERO
        }
    }))
}

#[tauri::command]
pub fn pwned(from: Option<String>) -> &'static str {
    if let Ok(mut log) = PWNED.lock() {
        log.push(from.unwrap_or_default());
    }
    "pwned"
}

#[tauri::command]
pub fn ctl_reply(id: u64, v: String) {
    let sender = PENDING
        .get()
        .and_then(|pending| pending.lock().ok())
        .and_then(|mut pending| pending.remove(&id));
    if let Some(sender) = sender {
        let _ = sender.send(v);
    }
}

fn on_webview(app: &AppHandle, work: impl FnOnce(&webkit2gtk::WebView) + Send + 'static) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let (done, wait) = mpsc::channel();
    let _ = window.with_webview(move |platform| {
        work(&platform.inner());
        let _ = done.send(());
    });
    let _ = wait.recv_timeout(Duration::from_secs(5));
}

mod synth {
    use glib::translate::ToGlibPtr;
    use gtk::prelude::WidgetExt;
    use webkit2gtk::WebView;

    fn raw_window(view: &WebView) -> Option<*mut gdk_sys::GdkWindow> {
        let window = view.window()?;
        let raw: *mut gdk_sys::GdkWindow = window.to_glib_none().0;
        Some(raw)
    }

    pub fn button(view: &WebView, x: f64, y: f64, kind: gdk_sys::GdkEventType, state: u32) {
        numbered_button(view, x, y, kind, state, 1);
    }

    pub fn numbered_button(
        view: &WebView,
        x: f64,
        y: f64,
        kind: gdk_sys::GdkEventType,
        state: u32,
        number: u32,
    ) {
        let Some(window) = raw_window(view) else {
            return;
        };
        unsafe {
            let event = gdk_sys::gdk_event_new(kind);
            let fields = &mut (*event).button;
            fields.window = gobject_sys::g_object_ref(window.cast()).cast();
            fields.time = gdk_sys::GDK_CURRENT_TIME as u32;
            fields.x = x;
            fields.y = y;
            fields.state = state;
            fields.button = number;
            let display = gdk_sys::gdk_window_get_display(window);
            let seat = gdk_sys::gdk_display_get_default_seat(display);
            gdk_sys::gdk_event_set_device(event, gdk_sys::gdk_seat_get_pointer(seat));
            let (mut root_x, mut root_y) = (0, 0);
            gdk_sys::gdk_window_get_root_coords(
                window,
                x as i32,
                y as i32,
                &mut root_x,
                &mut root_y,
            );
            fields.x_root = f64::from(root_x);
            fields.y_root = f64::from(root_y);
            gtk_sys::gtk_main_do_event(event);
            gdk_sys::gdk_event_free(event);
        }
    }

    pub fn motion(view: &WebView, x: f64, y: f64) {
        let Some(window) = raw_window(view) else {
            return;
        };
        unsafe {
            let event = gdk_sys::gdk_event_new(gdk_sys::GDK_MOTION_NOTIFY);
            let fields = &mut (*event).motion;
            fields.window = gobject_sys::g_object_ref(window.cast()).cast();
            fields.time = gdk_sys::GDK_CURRENT_TIME as u32;
            fields.x = x;
            fields.y = y;
            let display = gdk_sys::gdk_window_get_display(window);
            let seat = gdk_sys::gdk_display_get_default_seat(display);
            gdk_sys::gdk_event_set_device(event, gdk_sys::gdk_seat_get_pointer(seat));
            gtk_sys::gtk_main_do_event(event);
            gdk_sys::gdk_event_free(event);
        }
    }

    pub fn wheel(view: &WebView, x: f64, y: f64, steps: u32, state: u32, up: bool) {
        let Some(window) = raw_window(view) else {
            return;
        };
        for _ in 0..steps {
            unsafe {
                let event = gdk_sys::gdk_event_new(gdk_sys::GDK_SCROLL);
                let fields = &mut (*event).scroll;
                fields.window = gobject_sys::g_object_ref(window.cast()).cast();
                fields.time = gdk_sys::GDK_CURRENT_TIME as u32;
                fields.x = x;
                fields.y = y;
                fields.direction = if up {
                    gdk_sys::GDK_SCROLL_UP
                } else {
                    gdk_sys::GDK_SCROLL_DOWN
                };
                fields.state = state;
                let display = gdk_sys::gdk_window_get_display(window);
                let seat = gdk_sys::gdk_display_get_default_seat(display);
                gdk_sys::gdk_event_set_device(event, gdk_sys::gdk_seat_get_pointer(seat));
                gtk_sys::gtk_main_do_event(event);
                gdk_sys::gdk_event_free(event);
            }
        }
    }

    pub fn key(view: &WebView, keyval: u32, state: u32, press: bool) {
        let Some(window) = raw_window(view) else {
            return;
        };
        let kind = if press {
            gdk_sys::GDK_KEY_PRESS
        } else {
            gdk_sys::GDK_KEY_RELEASE
        };
        unsafe {
            let event = gdk_sys::gdk_event_new(kind);
            let fields = &mut (*event).key;
            fields.window = gobject_sys::g_object_ref(window.cast()).cast();
            fields.time = gdk_sys::GDK_CURRENT_TIME as u32;
            fields.state = state;
            fields.keyval = keyval;
            let display = gdk_sys::gdk_window_get_display(window);
            let keymap = gdk_sys::gdk_keymap_get_for_display(display);
            let mut keys = std::ptr::null_mut();
            let mut count = 0;
            if gdk_sys::gdk_keymap_get_entries_for_keyval(keymap, keyval, &mut keys, &mut count)
                != 0
                && count > 0
            {
                fields.hardware_keycode = (*keys).keycode as u16;
                fields.group = (*keys).group as u8;
                glib_sys::g_free(keys.cast());
            }
            let seat = gdk_sys::gdk_display_get_default_seat(display);
            gdk_sys::gdk_event_set_device(event, gdk_sys::gdk_seat_get_keyboard(seat));
            gtk_sys::gtk_main_do_event(event);
            gdk_sys::gdk_event_free(event);
        }
    }
}

fn eval(app: &AppHandle, script: &str, timeout_ms: u64) -> String {
    let Some(window) = app.get_webview_window("main") else {
        return json!({"ok": false, "err": "no window"}).to_string();
    };
    let id = NEXT_EVAL.fetch_add(1, Ordering::SeqCst);
    let (sender, reply) = mpsc::channel();
    if let Some(mut pending) = PENDING.get().and_then(|pending| pending.lock().ok()) {
        pending.insert(id, sender);
    }
    let wrapped = format!(
        "(async()=>{{let r;try{{r={{ok:true,v:await (async()=>{{return ({script})}})()}}}}catch(e){{r={{ok:false,err:String(e)}}}};window.__TAURI_INTERNALS__.invoke('ctl_reply',{{id:{id},v:JSON.stringify(r)}})}})()"
    );
    if window.eval(&wrapped).is_err() {
        return json!({"ok": false, "err": "eval refused"}).to_string();
    }
    reply
        .recv_timeout(Duration::from_millis(timeout_ms))
        .unwrap_or_else(|_| json!({"ok": false, "err": "timeout"}).to_string())
}

fn snapshot_ink(app: &AppHandle, png: Option<String>) -> String {
    let Some(window) = app.get_webview_window("main") else {
        return json!({"ok": false, "err": "no window"}).to_string();
    };
    let (sender, reply) = mpsc::channel();
    let _ = window.with_webview(move |platform| {
        use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
        platform.inner().snapshot(
            SnapshotRegion::Visible,
            SnapshotOptions::NONE,
            None::<&gtk::gio::Cancellable>,
            move |result| {
                let _ = sender.send(
                    result
                        .map(|surface| ink_counts(&surface, png.as_deref()))
                        .map_err(|error| error.to_string()),
                );
            },
        );
    });
    match reply.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok((width, height, ink))) => {
            json!({"ok": true, "width": width, "height": height, "ink": ink}).to_string()
        }
        Ok(Err(error)) => json!({"ok": false, "err": error}).to_string(),
        Err(_) => json!({"ok": false, "err": "snapshot timeout"}).to_string(),
    }
}

fn ink_counts(surface: &gtk::cairo::Surface, png: Option<&str>) -> (i32, i32, u64) {
    let Ok(image) = gtk::cairo::ImageSurface::try_from(surface.clone()) else {
        return (0, 0, 0);
    };
    let (width, height, stride) = (image.width(), image.height(), image.stride() as usize);
    let mut buffer = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height)
        .expect("snapshot copy");
    {
        let context = gtk::cairo::Context::new(&buffer).expect("snapshot context");
        context
            .set_source_surface(&image, 0.0, 0.0)
            .expect("snapshot source");
        context.paint().expect("snapshot paint");
    }
    buffer.flush();
    if let Some(path) = png
        && let Ok(mut file) = std::fs::File::create(path)
    {
        let _ = buffer.write_to_png(&mut file);
    }
    let Ok(pixels) = buffer.data() else {
        return (width, height, 0);
    };
    let mut ink = 0;
    for row in pixels.chunks(stride) {
        let (cells, _) = row.as_chunks::<4>();
        for pixel in cells.iter().skip(RAIL_WIDTH) {
            if pixel[0] > 160 && pixel[1] > 160 && pixel[2] > 160 {
                ink += 1;
            }
        }
    }
    (width, height, ink)
}

fn logged<T: Clone>(log: &Mutex<Vec<T>>) -> Vec<T> {
    log.lock()
        .map(|entries| entries.clone())
        .unwrap_or_default()
}

fn handle(app: &AppHandle, command: &Value) -> String {
    match command["op"].as_str().unwrap_or_default() {
        "eval" => eval(
            app,
            command["js"].as_str().unwrap_or("null"),
            command["timeout"].as_u64().unwrap_or(30_000),
        ),
        "click" => {
            let (x, y) = (
                command["x"].as_f64().unwrap_or(0.0),
                command["y"].as_f64().unwrap_or(0.0),
            );
            on_webview(app, move |view| {
                synth::motion(view, x, y);
                synth::button(view, x, y, gdk_sys::GDK_BUTTON_PRESS, 0);
                synth::button(
                    view,
                    x,
                    y,
                    gdk_sys::GDK_BUTTON_RELEASE,
                    gdk_sys::GDK_BUTTON1_MASK,
                );
            });
            json!({"ok": true}).to_string()
        }
        "right_click" => {
            let (x, y) = (
                command["x"].as_f64().unwrap_or(0.0),
                command["y"].as_f64().unwrap_or(0.0),
            );
            on_webview(app, move |view| {
                synth::motion(view, x, y);
                synth::numbered_button(view, x, y, gdk_sys::GDK_BUTTON_PRESS, 0, 3);
                synth::numbered_button(
                    view,
                    x,
                    y,
                    gdk_sys::GDK_BUTTON_RELEASE,
                    gdk_sys::GDK_BUTTON3_MASK,
                    3,
                );
            });
            json!({"ok": true}).to_string()
        }
        "wheel" => {
            let (x, y) = (
                command["x"].as_f64().unwrap_or(0.0),
                command["y"].as_f64().unwrap_or(0.0),
            );
            let steps = command["steps"].as_u64().unwrap_or(1) as u32;
            let state = command["state"].as_u64().unwrap_or(0) as u32;
            let up = command["up"].as_bool().unwrap_or(false);
            on_webview(app, move |view| {
                synth::motion(view, x, y);
                synth::wheel(view, x, y, steps, state, up);
            });
            json!({"ok": true}).to_string()
        }
        "move" => {
            let (x, y) = (
                command["x"].as_f64().unwrap_or(0.0),
                command["y"].as_f64().unwrap_or(0.0),
            );
            on_webview(app, move |view| synth::motion(view, x, y));
            json!({"ok": true}).to_string()
        }
        "press" => {
            let (x, y) = (
                command["x"].as_f64().unwrap_or(0.0),
                command["y"].as_f64().unwrap_or(0.0),
            );
            on_webview(app, move |view| {
                synth::motion(view, x, y);
                synth::button(view, x, y, gdk_sys::GDK_BUTTON_PRESS, 0);
            });
            json!({"ok": true}).to_string()
        }
        "release" => {
            let (x, y) = (
                command["x"].as_f64().unwrap_or(0.0),
                command["y"].as_f64().unwrap_or(0.0),
            );
            on_webview(app, move |view| {
                synth::button(
                    view,
                    x,
                    y,
                    gdk_sys::GDK_BUTTON_RELEASE,
                    gdk_sys::GDK_BUTTON1_MASK,
                );
            });
            json!({"ok": true}).to_string()
        }
        "key" => {
            let keyval = command["keyval"].as_u64().unwrap_or(0) as u32;
            let state = command["state"].as_u64().unwrap_or(0) as u32;
            on_webview(app, move |view| {
                use gtk::prelude::WidgetExt;
                view.grab_focus();
                synth::key(view, keyval, state, true);
                synth::key(view, keyval, state, false);
            });
            json!({"ok": true}).to_string()
        }
        "double_click" => {
            let (x, y) = (
                command["x"].as_f64().unwrap_or(0.0),
                command["y"].as_f64().unwrap_or(0.0),
            );
            let gtk_sequence = command["gtk_sequence"].as_bool().unwrap_or(false);
            on_webview(app, move |view| {
                synth::motion(view, x, y);
                let gtk_order = [
                    gdk_sys::GDK_BUTTON_PRESS,
                    gdk_sys::GDK_BUTTON_RELEASE,
                    gdk_sys::GDK_BUTTON_PRESS,
                    gdk_sys::GDK_DOUBLE_BUTTON_PRESS,
                    gdk_sys::GDK_BUTTON_RELEASE,
                ];
                let plain = [
                    gdk_sys::GDK_BUTTON_PRESS,
                    gdk_sys::GDK_BUTTON_RELEASE,
                    gdk_sys::GDK_BUTTON_PRESS,
                    gdk_sys::GDK_BUTTON_RELEASE,
                ];
                let kinds: &[gdk_sys::GdkEventType] =
                    if gtk_sequence { &gtk_order } else { &plain };
                for &kind in kinds {
                    let state = if kind == gdk_sys::GDK_BUTTON_RELEASE {
                        gdk_sys::GDK_BUTTON1_MASK
                    } else {
                        0
                    };
                    synth::button(view, x, y, kind, state);
                }
            });
            json!({"ok": true}).to_string()
        }
        "fail_save" => {
            if let Ok(mut slot) = INJECTED_FAILURE.lock() {
                *slot = command["kind"].as_str().map(str::to_owned);
            }
            json!({"ok": true}).to_string()
        }
        "arm_save_answer" => {
            *logged_lock(&SAVE_ANSWER) = Some(match command["path"].as_str() {
                Some(path) => SaveAnswer::Path(path.into()),
                None if command["real"].as_bool() == Some(true) => SaveAnswer::RealDialog,
                None => SaveAnswer::Cancel,
            });
            json!({"ok": true}).to_string()
        }
        "drop_paths" => {
            let paths = command["paths"]
                .as_array()
                .map(|paths| {
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .map(Into::into)
                        .collect()
                })
                .unwrap_or_default();
            crate::library::receive_drop(&app.state::<crate::session::Session>(), paths);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.dispatchEvent(new Event('ss-book-dropped'))");
            }
            json!({"ok": true}).to_string()
        }
        "snapshot" => snapshot_ink(app, command["png"].as_str().map(str::to_owned)),
        "state" => {
            let main = app.get_webview_window("main");
            let url = main
                .as_ref()
                .and_then(|window| window.url().ok())
                .map_or_else(String::new, |url| url.to_string());
            let fullscreen = main
                .as_ref()
                .and_then(|window| window.is_fullscreen().ok())
                .unwrap_or(false);
            let (sender, zoom) = mpsc::channel();
            on_webview(app, move |view| {
                use webkit2gtk::WebViewExt;
                let _ = sender.send(view.zoom_level());
            });
            let zoom = zoom.try_recv().unwrap_or(f64::NAN);
            json!({
                "ok": true,
                "url": url,
                "ui_url": UI_URL,
                "phases": logged(&PHASES),
                "navigations": logged(&NAVIGATIONS),
                "external": logged(&EXTERNAL),
                "pwned": logged(&PWNED),
                "windows": app.webview_windows().len(),
                "fullscreen": fullscreen,
                "zoom": zoom,
            })
            .to_string()
        }
        "quit" => {
            app.exit(0);
            json!({"ok": true}).to_string()
        }
        _ => json!({"ok": false, "err": "unknown op"}).to_string(),
    }
}

fn serve_client(app: AppHandle, stream: UnixStream) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        let Ok(command) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let reply = handle(&app, &command);
        if writeln!(writer, "{reply}").is_err() {
            return;
        }
    }
}

pub fn start_control(app: AppHandle) {
    let Ok(path) = std::env::var("JADESLIP_E2E_SOCKET") else {
        return;
    };
    let _ = PENDING.set(Mutex::new(HashMap::new()));
    let Ok(listener) = UnixListener::bind(path) else {
        eprintln!("jadeslip: the control socket could not bind");
        std::process::exit(3);
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            std::thread::spawn(move || serve_client(app, stream));
        }
    });
}
