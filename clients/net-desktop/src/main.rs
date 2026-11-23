#![cfg_attr(all(windows, feature = "window"), windows_subsystem = "windows")]
//! vØrk's desktop window: the node's own dashboard (`src/web/index.html`, the
//! page `web =` serves) in the system's web view, with a tray icon.
//!
//! The page's `/api/*` requests don't go over the network: they go straight to
//! the machine's node through its control endpoint (the named pipe or Unix
//! socket `vork status` uses), so no port, certificate or token is involved.
//!
//! Where there is no web view (Haiku), or with `--browser`, the same page is
//! served on 127.0.0.1 with a random key and opened in the system's browser.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;

/// The dashboard as this window was built with it: shown only when the node
/// doesn't give its own (one older than this window).
const PAGE: &str = include_str!("../../../src/web/index.html");

/// The node's own dashboard: it updates itself (ADR-0028), this window may not,
/// and the page must fit the node it talks to.
fn page() -> String {
    ask(r#"{"cmd":"page"}"#)
        .ok()
        .and_then(|a| serde_json::from_str::<serde_json::Value>(&a).ok())
        .and_then(|v| v["page"].as_str().map(str::to_string))
        .unwrap_or_else(|| PAGE.to_string())
}

const USAGE: &str = "usage: vork-net [--config FILE] [--browser] [--hidden]
  --config FILE  the node's configuration (default: the system node's)
  --browser      open the dashboard in the browser instead of a window
  --hidden       start in the tray (for starting with the session)";

struct Opts {
    config: Option<PathBuf>,
    browser: bool,
    hidden: bool,
}

fn opts() -> Opts {
    let mut o = Opts { config: None, browser: !cfg!(feature = "window"), hidden: false };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => o.config = args.next().map(PathBuf::from),
            "--browser" => o.browser = true,
            "--hidden" => o.hidden = true,
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }
    o
}

fn main() {
    let o = opts();
    if let Some(c) = &o.config {
        // SAFETY: set before any other thread exists.
        unsafe { std::env::set_var("VORK_CONFIG", c) };
    }
    if o.browser {
        if let Err(e) = browser::run() {
            eprintln!("vork-net: {e}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(feature = "window")]
    window::run(o.hidden);
}

// ---- the node: where its control endpoint is, and asking it ----

#[derive(serde::Deserialize, Default)]
struct NodeConfig {
    listen: Option<String>,
    state: Option<PathBuf>,
    control: Option<String>,
}

/// The node's configuration file: `--config` / `$VORK_CONFIG`; else the
/// system node's, or the user's own when there is no system node.
fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("VORK_CONFIG").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        let system = PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into())).join(r"vork\vork.toml");
        let user = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join(r"vork\node\vork.toml");
        if !system.is_file() && user.is_file() { user } else { system }
    }
    #[cfg(not(windows))]
    {
        let system = PathBuf::from("/etc/vork/vork.toml");
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let user = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("vork/vork.toml");
        if !system.is_file() && user.is_file() { user } else { system }
    }
}

/// The control endpoint, worked out the way the node does (`control`, else a
/// pipe per listening port on Windows, else `control.sock` in the state directory).
fn endpoint() -> Result<String, String> {
    let path = config_path();
    let text = std::fs::read_to_string(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let c: NodeConfig = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(c) = c.control {
        return Ok(c);
    }
    #[cfg(windows)]
    {
        let _ = c.state;
        let port = c.listen.as_deref().and_then(|l| l.parse::<std::net::SocketAddr>().ok()).map_or(51820, |a| a.port());
        Ok(format!(r"\\.\pipe\vork-{port}"))
    }
    #[cfg(not(windows))]
    {
        let _ = c.listen;
        let dir = path.parent().map(PathBuf::from).unwrap_or_default();
        let state = c.state.map(|s| dir.join(s)).unwrap_or_else(|| dir.join("state"));
        Ok(state.join("control.sock").to_string_lossy().into_owned())
    }
}

/// One request line to the node, one answer line back.
fn ask(line: &str) -> Result<String, String> {
    let ep = endpoint()?;
    let conn = connect(&ep).map_err(|e| format!("no node answers at {ep} ({e}); is it running?"))?;
    talk(conn, line).map_err(|e| format!("the node at {ep}: {e}"))
}

fn talk<S: Read + Write>(mut conn: S, line: &str) -> std::io::Result<String> {
    conn.write_all(format!("{line}\n").as_bytes())?;
    let mut answer = String::new();
    BufReader::new(conn).read_line(&mut answer)?;
    if answer.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "no answer"));
    }
    Ok(answer)
}

#[cfg(windows)]
fn connect(ep: &str) -> std::io::Result<std::fs::File> {
    // The node makes the next pipe instance just after one is taken: a
    // client arriving in between finds every instance busy for a moment.
    const ERROR_PIPE_BUSY: i32 = 231;
    let mut tries = 0;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(ep) {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < 40 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            r => return r,
        }
    }
}

#[cfg(unix)]
fn connect(ep: &str) -> std::io::Result<std::os::unix::net::UnixStream> {
    let s = std::os::unix::net::UnixStream::connect(ep)?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(15)))?;
    Ok(s)
}

// ---- the page's requests, the same in the window and the browser ----

struct Answer {
    status: u16,
    kind: &'static str,
    body: Vec<u8>,
}

fn answer(status: u16, kind: &'static str, body: impl Into<Vec<u8>>) -> Answer {
    Answer { status, kind, body: body.into() }
}

/// The command a page put in the address (`?c=…`, percent-encoded): WebKitGTK
/// gives a custom scheme's requests no body, so the page sends it there too.
fn command_in(query: Option<&str>) -> Option<Vec<u8>> {
    let v = query?.split('&').find_map(|p| p.strip_prefix("c="))?;
    let (b, mut out, mut i) = (v.as_bytes(), Vec::new(), 0);
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).ok()?, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(out)
}

/// The routes of the node's own dashboard server (`src/web.rs`).
fn route(method: &str, path: &str, body: &[u8]) -> Answer {
    let line = match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => return answer(200, "text/html; charset=utf-8", page()),
        ("GET", "/api/status") => r#"{"cmd":"status"}"#.to_string(),
        ("GET", "/api/metrics") => r#"{"cmd":"metrics"}"#.to_string(),
        ("GET", "/api/log") => r#"{"cmd":"log"}"#.to_string(),
        ("POST", "/api/cmd") => match serde_json::from_slice::<serde_json::Value>(body) {
            Ok(v) => v.to_string(),
            Err(_) => return answer(400, "application/json", r#"{"error":"bad request"}"#),
        },
        _ => return answer(404, "text/plain", "not found"),
    };
    match ask(&line) {
        Ok(a) => answer(200, "application/json", a),
        // Not JSON on purpose: the page then shows the node as not answering.
        Err(e) => answer(503, "text/plain; charset=utf-8", e),
    }
}

// ---- the window (WebView2 / WebKitGTK) and the tray icon ----

#[cfg(feature = "window")]
mod window {
    use super::*;
    use tao::dpi::LogicalSize;
    use tao::event::{Event, StartCause, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::{Icon, WindowBuilder};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use wry::http::Response;
    use wry::{NewWindowResponse, WebContext, WebViewBuilder};

    /// Where the page lives: WebView2 maps custom schemes to http://<scheme>.localhost.
    #[cfg(windows)]
    const HOME: &str = "http://vork.localhost/";
    #[cfg(not(windows))]
    const HOME: &str = "vork://localhost/";

    /// The page asks for a key before it calls the API; here the node's
    /// control endpoint decides, so any key does.
    const INIT: &str = r#"try { localStorage.setItem("vork-token", "app"); } catch (e) {}"#;

    enum UserEvent {
        Menu(MenuEvent),
        Tray(TrayIconEvent),
        /// Another start of vork-net asked for the window.
        Show,
    }

    pub fn run(hidden: bool) {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        // One per user: a second start shows this one's window and ends
        // (one started at login, `--hidden`, just ends).
        if !single::claim(hidden, move || {
            let _ = proxy.send_event(UserEvent::Show);
        }) {
            return;
        }
        let proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = proxy.send_event(UserEvent::Menu(e));
        }));
        let proxy = event_loop.create_proxy();
        TrayIconEvent::set_event_handler(Some(move |e| {
            let _ = proxy.send_event(UserEvent::Tray(e));
        }));

        let (rgba, size) = icon_rgba(64);
        let window = WindowBuilder::new()
            .with_title("vØrk")
            .with_inner_size(LogicalSize::new(1240.0, 820.0))
            .with_min_inner_size(LogicalSize::new(560.0, 420.0))
            .with_window_icon(Icon::from_rgba(rgba.clone(), size, size).ok())
            .with_visible(!hidden)
            .build(&event_loop)
            .expect("a window");

        // The web view keeps its data (the page's remembered page and range)
        // per user, not next to the program (Program Files isn't writable).
        let mut context = WebContext::new(Some(data_dir()));
        let builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_initialization_script(INIT)
            .with_asynchronous_custom_protocol("vork".into(), |_id, request, responder| {
                // The node may take a moment; never on the UI thread.
                std::thread::spawn(move || {
                    let path = request.uri().path().to_string();
                    let from_query = if request.body().is_empty() { command_in(request.uri().query()) } else { None };
                    let a = route(request.method().as_str(), &path, from_query.as_deref().unwrap_or(request.body()));
                    let response = Response::builder()
                        .status(a.status)
                        .header("Content-Type", a.kind)
                        .header("Cache-Control", "no-store")
                        .body(a.body)
                        .unwrap_or_else(|_| Response::new(Vec::new()));
                    responder.respond(response);
                });
            })
            .with_navigation_handler(|url| url.starts_with(HOME))
            // A web link (a release's download or page) opens in the browser.
            .with_new_window_req_handler(|url, _| {
                if url.starts_with("https://") {
                    browser::open_browser(&url);
                }
                NewWindowResponse::Deny
            })
            .with_url(HOME);
        #[cfg(not(target_os = "linux"))]
        let webview = builder.build(&window);
        #[cfg(target_os = "linux")]
        let webview = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            builder.build_gtk(window.default_vbox().expect("a GTK box"))
        };
        let webview = match webview {
            Ok(w) => w,
            Err(e) => {
                eprintln!("vork-net: no web view ({e}); opening the browser instead");
                if let Err(e) = browser::run() {
                    eprintln!("vork-net: {e}");
                }
                return;
            }
        };

        let open = MenuItem::new("Open vØrk", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let mut tray = None;

        event_loop.run(move |event, _, flow| {
            *flow = ControlFlow::Wait;
            let _ = &webview;
            match event {
                // The tray icon is made once the loop runs (GTK must be up on Linux).
                Event::NewEvents(StartCause::Init) => {
                    let menu = Menu::new();
                    let _ = menu.append_items(&[&open, &quit]);
                    tray = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_menu_on_left_click(false)
                        .with_tooltip("vØrk")
                        .with_icon(tray_icon::Icon::from_rgba(rgba.clone(), size, size).expect("an icon"))
                        .build()
                        .map_err(|e| eprintln!("vork-net: no tray icon ({e})"))
                        .ok();
                }
                // The page fills the window at every size (maximized too).
                #[cfg(not(target_os = "linux"))]
                Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
                    let _ = webview.set_bounds(wry::Rect {
                        position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                        size: wry::dpi::PhysicalSize::new(size.width, size.height).into(),
                    });
                }
                // Closing the window keeps the app in the tray; Quit ends it.
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                    if tray.is_some() {
                        window.set_visible(false);
                    } else {
                        *flow = ControlFlow::Exit;
                    }
                }
                Event::UserEvent(UserEvent::Menu(e)) if e.id == quit.id() => {
                    tray.take();
                    *flow = ControlFlow::Exit;
                }
                Event::UserEvent(UserEvent::Menu(e)) if e.id == open.id() => show(&window),
                Event::UserEvent(UserEvent::Show) => show(&window),
                Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. })) => show(&window),
                _ => {}
            }
        });
    }

    /// One vork-net per user.
    mod single {
        /// Whether this is the first: then `show` is called whenever another
        /// start asks for the window. A later one asks (unless it starts
        /// hidden) and gets false.
        #[cfg(windows)]
        pub fn claim(hidden: bool, show: impl Fn() + Send + 'static) -> bool {
            use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
            use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForSingleObject};
            let name: Vec<u16> = "Local\\vork-net-show\0".encode_utf16().collect();
            // Safe: the name is NUL-terminated; the handle lives as long as the process.
            let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
            if event.is_null() {
                return true;
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                if !hidden {
                    unsafe { SetEvent(event) };
                }
                return false;
            }
            let event = event as usize;
            std::thread::spawn(move || {
                loop {
                    unsafe { WaitForSingleObject(event as _, INFINITE) };
                    show();
                }
            });
            true
        }

        #[cfg(unix)]
        pub fn claim(hidden: bool, show: impl Fn() + Send + 'static) -> bool {
            use std::os::unix::net::{UnixListener, UnixStream};
            let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
            let path = dir.join(format!("vork-net-{}.sock", unsafe { libc_getuid() }));
            use std::io::{Read, Write};
            if let Ok(mut other) = UnixStream::connect(&path) {
                // A byte is the request; one started hidden doesn't make it.
                if !hidden {
                    let _ = other.write_all(b"s");
                }
                return false;
            }
            let _ = std::fs::remove_file(&path);
            let Ok(listener) = UnixListener::bind(&path) else { return true };
            std::thread::spawn(move || {
                for mut conn in listener.incoming().flatten() {
                    let mut b = [0u8; 1];
                    if conn.read(&mut b).is_ok_and(|n| n == 1) {
                        show();
                    }
                }
            });
            true
        }

        #[cfg(unix)]
        unsafe extern "C" {
            #[link_name = "getuid"]
            fn libc_getuid() -> u32;
        }
    }

    fn show(window: &tao::window::Window) {
        window.set_visible(true);
        window.set_minimized(false);
        window.set_focus();
    }

    fn data_dir() -> PathBuf {
        #[cfg(windows)]
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        #[cfg(not(windows))]
        let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
        base.join("vork").join("net-desktop")
    }

    /// The icon (docs/images/icon.svg), drawn and antialiased: a node and its
    /// three neighbors, white on the dashboard's blue-violet rounded square.
    pub fn icon_rgba(n: u32) -> (Vec<u8>, u32) {
        let s = n as f32 / 64.0;
        let (c, radius) = (32.0 * s, 14.0 * s);
        let center = (32.0 * s, 36.0 * s);
        let nodes = [(32.0 * s, 15.0 * s), (13.8 * s, 46.5 * s), (50.2 * s, 46.5 * s)];
        let ends = [(32.0 * s, 21.3 * s), (19.3 * s, 43.3 * s), (44.7 * s, 43.3 * s)];
        let (ring, half, hub) = (5.6 * s, 1.6 * s, 9.0 * s);
        let cover = |d: f32| (0.5 - d).clamp(0.0, 1.0);
        let segment = |p: (f32, f32), a: (f32, f32), b: (f32, f32)| {
            let (vx, vy) = (b.0 - a.0, b.1 - a.1);
            let t = (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
            ((p.0 - a.0 - t * vx).powi(2) + (p.1 - a.1 - t * vy).powi(2)).sqrt()
        };
        let dist = |p: (f32, f32), q: (f32, f32)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
        let mut px = Vec::with_capacity((n * n * 4) as usize);
        for y in 0..n {
            for x in 0..n {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                // Rounded square: signed distance to its edge.
                let (qx, qy) = ((p.0 - c).abs() - (c - radius), (p.1 - c).abs() - (c - radius));
                let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - radius;
                let square = cover(outside);
                let mut white = cover(dist(p, center) - hub);
                for (node, end) in nodes.iter().zip(ends.iter()) {
                    white = white.max(cover((dist(p, *node) - ring).abs() - half));
                    white = white.max(cover(segment(p, center, *end) - half));
                }
                // #4f8dff to #a78bfa along the diagonal.
                let g = ((p.0 + p.1) / (2.0 * n as f32)).clamp(0.0, 1.0);
                let base = [79.0 + (167.0 - 79.0) * g, 141.0 + (139.0 - 141.0) * g, 255.0 + (250.0 - 255.0) * g];
                for ch in base {
                    px.push((ch + (255.0 - ch) * white).round() as u8);
                }
                px.push((square * 255.0).round() as u8);
            }
        }
        (px, n)
    }
}

// ---- the browser: the page on 127.0.0.1 with a random key ----

mod browser {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    pub fn run() -> Result<(), String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("can't listen on 127.0.0.1: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let mut key = [0u8; 16];
        getrandom::getrandom(&mut key).map_err(|e| e.to_string())?;
        let key: String = key.iter().map(|b| format!("{b:02x}")).collect();
        // The page keeps the key from the link (#t=…) and sends it with every
        // request: other local users can reach the port, not the key.
        let url = format!("http://127.0.0.1:{port}/#t={key}");
        println!("vØrk dashboard: {url}\n(stop with Ctrl-C)");
        open_browser(&url);
        for conn in listener.incoming().flatten() {
            let key = key.clone();
            std::thread::spawn(move || {
                let _ = serve(conn, &key);
            });
        }
        Ok(())
    }

    fn serve(mut conn: TcpStream, key: &str) -> std::io::Result<()> {
        conn.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        let (head_end, length) = loop {
            let n = conn.read(&mut chunk)?;
            if n == 0 || buf.len() > 64 * 1024 {
                return Ok(());
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                let len = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                break (i + 4, len.min(32 * 1024));
            }
        };
        while buf.len() < head_end + length {
            let n = conn.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let body = &buf[head_end..(head_end + length).min(buf.len())];
        let mut first = head.lines().next().unwrap_or_default().split_whitespace();
        let (method, path) = (first.next().unwrap_or_default(), first.next().unwrap_or("/"));
        let path = path.split('?').next().unwrap_or("/");
        let given = head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case("x-vork-token").then(|| v.trim().to_string())
        });
        // Only this machine, by name: a page elsewhere can't rebind a name to 127.0.0.1 and read this.
        let host_ok = head.lines().any(|l| l.split_once(':').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("host") && v.trim() == format!("127.0.0.1:{}", conn.local_addr().map(|a| a.port()).unwrap_or(0))));
        let a = if !host_ok {
            answer(421, "text/plain", "wrong host")
        } else if path.starts_with("/api/") && given.as_deref().is_none_or(|g| !eq(g.as_bytes(), key.as_bytes())) {
            answer(401, "application/json", r#"{"error":"open the dashboard with the link vork-net printed"}"#)
        } else {
            route(method, path, body)
        };
        let reason = match a.status {
            200 => "OK",
            401 => "Unauthorized",
            404 => "Not Found",
            _ => "Error",
        };
        let head = format!(
            "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
            a.status,
            a.kind,
            a.body.len()
        );
        conn.write_all(head.as_bytes())?;
        conn.write_all(&a.body)?;
        conn.flush()
    }

    fn eq(a: &[u8], b: &[u8]) -> bool {
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    pub fn open_browser(url: &str) {
        #[cfg(windows)]
        let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
        #[cfg(target_os = "macos")]
        let r = std::process::Command::new("open").arg(url).spawn();
        #[cfg(target_os = "haiku")]
        let r = std::process::Command::new("open").arg(url).spawn();
        #[cfg(not(any(windows, target_os = "macos", target_os = "haiku")))]
        let r = std::process::Command::new("xdg-open").arg(url).spawn();
        if let Err(e) = r {
            eprintln!("couldn't open a browser ({e}): open the link above yourself");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_command_comes_from_the_address() {
        let c = super::command_in(Some("c=%7B%22cmd%22%3A%22link%22%2C%22address%22%3A%22vork-link%3Aab%3Fc%3D1%26n%3Dx%22%7D")).unwrap();
        assert_eq!(c, br#"{"cmd":"link","address":"vork-link:ab?c=1&n=x"}"#);
        assert_eq!(super::command_in(Some("x=1")), None);
        // A cut-off escape is kept as it is (then not JSON: a bad request).
        assert_eq!(super::command_in(Some("c=%7")).unwrap(), b"%7");
    }
}
