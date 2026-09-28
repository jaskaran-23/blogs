//! Development server with file watching and live reload.
//!
//! `sblog --serve` builds the site, serves the output directory over
//! HTTP, watches the source directories, rebuilds changed pages, and
//! tells connected browsers to reload. Browsers long-poll `/__reload`;
//! the request returns when the site changes or after a timeout. The
//! client script is injected into served HTML at request time, so
//! files on disk stay free of JavaScript.

use std::io::Cursor;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};
use tiny_http::{Header, Response, Server};

use crate::{SiteConfig, build_site, build_stale, copy_static, load_config};

/// Default bind address when `--serve` gets no argument.
const DEFAULT_HOST: &str = "localhost";
/// Default port, matching the README preview example.
const DEFAULT_PORT: u16 = 8123;
/// URL path of the long-poll reload channel.
const RELOAD_PATH: &str = "/__reload";
/// How long the watcher waits for an event burst to settle.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// How long one `/__reload` request waits before answering 204.
const POLL_TIMEOUT: Duration = Duration::from_secs(30);

/// The client half of live reload: long-poll `/__reload` in a loop and
/// refresh the page when the server answers `reload`. Injected before
/// `</body>` at serve time.
const RELOAD_SCRIPT: &str = "<script>(function poll(){fetch(\"/__reload\").then(function(r){return r.status==200?r.text():r.status==204?Promise.reject(\"timeout\"):Promise.reject(r.status)}).then(function(b){if(b==\"reload\")location.reload()}).catch(function(){setTimeout(poll,1000)})})();</script>";

/// Registry of connected reload clients.
///
/// Each client owns a channel. `broadcast` pushes one message per
/// client and drops channels whose receiver is gone.
#[derive(Default)]
struct ReloadHub {
    senders: Mutex<Vec<Sender<()>>>,
}

impl ReloadHub {
    fn new() -> Self {
        Self::default()
    }

    /// Register a new client and return its receive half.
    fn subscribe(&self) -> Receiver<()> {
        let (tx, rx) = mpsc::channel();
        self.senders.lock().expect("lock reload hub").push(tx);
        rx
    }

    /// Wake every connected client. Dead channels are removed.
    fn broadcast(&self) {
        let mut senders = self.senders.lock().expect("lock reload hub");
        senders.retain(|tx| tx.send(()).is_ok());
    }
}

/// Parse the optional `--serve` argument into a bind address.
///
/// Accepted forms: `host:port`, `:port`, a bare port number, or a bare
/// host name. Missing parts fall back to `localhost:8123`.
fn parse_bind(arg: Option<&str>) -> (String, u16) {
    let Some(arg) = arg else {
        return (DEFAULT_HOST.to_string(), DEFAULT_PORT);
    };
    if let Some((host, port)) = arg.rsplit_once(':') {
        let port = port.parse().unwrap_or_else(|_| {
            eprintln!("Invalid port in {arg:?}; using {DEFAULT_PORT}");
            DEFAULT_PORT
        });
        let host = if host.is_empty() { DEFAULT_HOST } else { host };
        (host.to_string(), port)
    } else if let Ok(port) = arg.parse::<u16>() {
        (DEFAULT_HOST.to_string(), port)
    } else {
        (arg.to_string(), DEFAULT_PORT)
    }
}

/// Entry point of `sblog --serve [host[:port]]`.
pub fn run_serve(root: &Path, bind: Option<&str>) {
    let (host, port) = parse_bind(bind);
    let config = load_config(root);

    // Start from a clean, complete site so orphaned pages go away.
    build_site(root, &config);

    let hub = Arc::new(ReloadHub::new());
    start_watcher(root, &config, Arc::clone(&hub));

    let addr = format!("{host}:{port}");
    let addrs: Vec<_> = match addr.to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(e) => {
            eprintln!("Cannot resolve {addr}: {e}");
            std::process::exit(1);
        }
    };
    let server = match Server::http(addrs[0]) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("Cannot bind {addr}: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "Serving {} at http://{addr}",
        root.join(&config.output_dir).display()
    );
    println!("Watching for changes. Press Ctrl-C to stop.");

    // A few worker threads share the server. tiny_http hands each
    // incoming connection to one of them.
    let public_dir = root.join(&config.output_dir);
    let mut guards = Vec::new();
    for _ in 0..4 {
        let server = Arc::clone(&server);
        let public_dir = public_dir.clone();
        let hub = Arc::clone(&hub);
        guards.push(thread::spawn(move || {
            loop {
                match server.recv() {
                    Ok(request) => handle_request(request, &public_dir, &hub),
                    Err(e) => eprintln!("Request error: {e}"),
                }
            }
        }));
    }
    for g in guards {
        let _ = g.join();
    }
}

/// Handle one HTTP request: the reload channel or a static file.
fn handle_request(request: tiny_http::Request, public_dir: &Path, hub: &ReloadHub) {
    let url = request.url().to_string();
    let path = url.split(['?', '#']).next().unwrap_or("/").to_string();

    let result = if path == RELOAD_PATH {
        request.respond(reload_response(hub))
    } else {
        request.respond(file_response(public_dir, &path))
    };
    if let Err(e) = result {
        eprintln!("Response error: {e}");
    }
}

/// Build the response for one `/__reload` long-poll.
///
/// The request blocks on the client's channel until the site changes
/// (`200` + `reload`) or `POLL_TIMEOUT` passes (`204`). One complete
/// response per poll; nothing is streamed.
fn reload_response(hub: &ReloadHub) -> Response<Cursor<Vec<u8>>> {
    let rx = hub.subscribe();
    let (status, body) = match rx.recv_timeout(POLL_TIMEOUT) {
        Ok(()) => (200, "reload"),
        Err(RecvTimeoutError::Timeout) => (204, ""),
        // The hub dropped this client; end the poll with no content.
        Err(RecvTimeoutError::Disconnected) => (204, ""),
    };
    let mut headers = vec![header("Cache-Control", "no-store")];
    if status == 200 {
        headers.push(header("Content-Type", "text/plain; charset=utf-8"));
    }
    Response::new(
        tiny_http::StatusCode(status),
        headers,
        Cursor::new(body.as_bytes().to_vec()),
        Some(body.len()),
        None,
    )
}

/// Build a response for a file under the output directory.
///
/// Returns a 404 response when the file is missing or the path tries
/// to escape the output directory.
fn file_response(public_dir: &Path, url_path: &str) -> Response<Cursor<Vec<u8>>> {
    match resolve_file(public_dir, url_path) {
        Some((full_path, mime)) => {
            let mut body = fs_read(&full_path);
            // Inject the reload client into HTML pages at serve time.
            // Files on disk stay free of JavaScript.
            if mime.starts_with("text/html") {
                body = inject_reload_script(body);
            }
            let len = body.len();
            Response::new(
                tiny_http::StatusCode(200),
                vec![
                    header("Content-Type", &mime),
                    header("Cache-Control", "no-store"),
                ],
                Cursor::new(body),
                Some(len),
                None,
            )
        }
        None => not_found(url_path),
    }
}

/// Insert the reload script before `</body>`. Without that marker the
/// body is returned unchanged.
fn inject_reload_script(body: Vec<u8>) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(&body) else {
        return body;
    };
    match text.to_lowercase().rfind("</body>") {
        Some(pos) => {
            let mut out = String::with_capacity(text.len() + RELOAD_SCRIPT.len());
            out.push_str(&text[..pos]);
            out.push_str(RELOAD_SCRIPT);
            out.push_str(&text[pos..]);
            out.into_bytes()
        }
        None => body,
    }
}

/// Map a URL path to a file under `public_dir` plus its MIME type.
fn resolve_file(public_dir: &Path, url_path: &str) -> Option<(PathBuf, String)> {
    let decoded = percent_decode(url_path);
    let rel = decoded.trim_start_matches('/');
    if rel.split('/').any(|part| part == "..") {
        return None;
    }
    let mut full = public_dir.join(rel);
    if decoded.ends_with('/') || full.is_dir() {
        full = full.join("index.html");
    }
    let mime = mime_for(&full)?;
    if full.is_file() {
        Some((full, mime.to_string()))
    } else {
        None
    }
}

/// Read a file, returning an empty body on error. The dev server
/// serves what it can instead of failing the request thread.
fn fs_read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

fn not_found(url_path: &str) -> Response<Cursor<Vec<u8>>> {
    let body = format!("404 Not Found: {url_path}").into_bytes();
    let len = body.len();
    Response::new(
        tiny_http::StatusCode(404),
        vec![
            header("Content-Type", "text/plain; charset=utf-8"),
            header("Cache-Control", "no-store"),
        ],
        Cursor::new(body),
        Some(len),
        None,
    )
}

/// Build a tiny_http header, panicking only on the impossible case of
/// an invalid static header pair.
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

/// MIME type for a path, by extension. `None` means "do not serve".
fn mime_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "xml" => "application/xml",
        "txt" | "md" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "map" => "application/json",
        _ => "application/octet-stream",
    })
}

/// Decode percent escapes in a URL path. Invalid sequences pass
/// through unchanged.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Watch the source directories and rebuild on change.
///
/// The watcher thread collects mutation paths from notify and flushes
/// them as one batch after a quiet gap, so one save produces one
/// rebuild. Read-only `Access` events are dropped: the build itself
/// reads the watched files, and on Linux every read would otherwise
/// trigger the next rebuild in an endless loop.
fn start_watcher(root: &Path, config: &SiteConfig, hub: Arc<ReloadHub>) {
    let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();
    let mut watcher = match notify::recommended_watcher(
        move |res: Result<notify::Event, notify::Error>| match res {
            Ok(event) => {
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                let _ = tx.send(event.paths);
            }
            Err(e) => {
                eprintln!("Watch error: {e}");
            }
        },
    ) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Cannot start file watcher: {e}");
            std::process::exit(1);
        }
    };

    let watch_paths: Vec<PathBuf> = vec![
        root.join(&config.posts_dir),
        root.join(&config.templates_dir),
        root.join(&config.static_dir),
        root.join("config.toml"),
    ];
    for path in &watch_paths {
        if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
            eprintln!("Cannot watch {}: {e}", path.display());
        }
    }

    let root = root.to_path_buf();
    let config = config.clone();
    thread::spawn(move || {
        // Keep the watcher alive for the lifetime of the server.
        let _watcher = watcher;
        rebuild_loop(root, config, hub, rx);
    });
}

/// Consume watcher events and rebuild the site.
fn rebuild_loop(
    root: PathBuf,
    config: SiteConfig,
    hub: Arc<ReloadHub>,
    rx: Receiver<Vec<PathBuf>>,
) {
    loop {
        // Wait for the first change, then collect more paths for a
        // short quiet gap so one save produces one rebuild.
        let Ok(first) = rx.recv() else {
            return;
        };
        let mut batch: Vec<PathBuf> = first;
        while let Ok(more) = rx.recv_timeout(DEBOUNCE) {
            batch.extend(more);
        }
        if matches!(rx.try_recv(), Err(TryRecvError::Disconnected)) {
            return;
        }

        let mut static_dirty = false;
        let mut post_changed = false;
        let mut post_removed = false;
        let mut template_changed = false;
        let mut config_changed = false;
        let posts_dir = root.join(&config.posts_dir);
        let templates_dir = root.join(&config.templates_dir);
        let static_dir = root.join(&config.static_dir);
        let config_path = root.join("config.toml");

        for path in &batch {
            if path.starts_with(&static_dir) {
                static_dirty = true;
            } else if path.starts_with(&templates_dir) {
                template_changed = true;
            } else if *path == config_path {
                config_changed = true;
            } else if path.starts_with(&posts_dir)
                && path.extension().and_then(|e| e.to_str()) == Some("md")
            {
                if path.exists() {
                    post_changed = true;
                } else {
                    post_removed = true;
                }
            }
        }

        let started = std::time::Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if template_changed || config_changed || post_removed {
                build_site(&root, &config);
            } else if post_changed {
                build_stale(&root, &config);
            } else if static_dirty {
                copy_static(&root, &config);
            }
        }));

        if result.is_err() {
            eprintln!("Rebuild failed; serving the last good pages.");
            continue;
        }

        println!("Rebuilt in {:?}", started.elapsed());
        hub.broadcast();
    }
}
