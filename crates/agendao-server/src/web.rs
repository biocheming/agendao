use std::borrow::Cow;
use std::fs;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::{OnceLock, RwLock};

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use once_cell::sync::Lazy;

pub type EmbeddedWebAssetLoader = fn(&str) -> Option<Cow<'static, [u8]>>;

static WEB_DIST_OVERRIDE: Lazy<RwLock<Option<PathBuf>>> = Lazy::new(|| RwLock::new(None));
static EMBEDDED_WEB_ASSET_LOADER: Lazy<RwLock<Option<EmbeddedWebAssetLoader>>> =
    Lazy::new(|| RwLock::new(None));
static WEB_DIST_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn configure_web_dist_root(path: Option<PathBuf>) {
    if let Ok(mut guard) = WEB_DIST_OVERRIDE.write() {
        *guard = path;
    }
}

pub fn configure_embedded_web_assets(loader: Option<EmbeddedWebAssetLoader>) {
    if let Ok(mut guard) = EMBEDDED_WEB_ASSET_LOADER.write() {
        *guard = loader;
    }
}

pub async fn web_index() -> Response {
    serve_html("index.html")
}

pub async fn web_file(Path(path): Path<String>) -> Response {
    serve_path(path.as_str())
}

pub async fn root_favicon() -> Response {
    serve_path("favicon.ico")
}

pub async fn root_apple_touch_icon() -> Response {
    serve_path("apple-touch-icon.png")
}

fn serve_html(relative: &str) -> Response {
    match load_web_file(relative) {
        Ok(bytes) => {
            let html = String::from_utf8_lossy(&bytes).into_owned();
            (
                [
                    (header::CONTENT_TYPE, HeaderValueStatic::HTML_UTF8),
                    (header::CACHE_CONTROL, HeaderValueStatic::NO_STORE),
                ],
                Html(html),
            )
                .into_response()
        }
        Err(WebServeError::Unavailable) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Html(web_frontend_unavailable_page()),
        )
            .into_response(),
        Err(WebServeError::InvalidPath) | Err(WebServeError::MissingFile) => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(WebServeError::Io(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read web asset: {error}"),
        )
            .into_response(),
    }
}

fn serve_path(relative: &str) -> Response {
    match load_web_file(relative) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, mime_for_path(relative)),
                (header::CACHE_CONTROL, HeaderValueStatic::NO_STORE),
            ],
            bytes,
        )
            .into_response(),
        Err(WebServeError::Unavailable) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "AgenDao Web frontend assets are unavailable.",
        )
            .into_response(),
        Err(WebServeError::InvalidPath) | Err(WebServeError::MissingFile) => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(WebServeError::Io(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read web asset: {error}"),
        )
            .into_response(),
    }
}

fn load_web_file(relative: &str) -> Result<Vec<u8>, WebServeError> {
    let relative = sanitize_relative_path(relative).ok_or(WebServeError::InvalidPath)?;

    // 1. Runtime override (development / AGENDAO_WEB_DIST)
    if let Ok(guard) = WEB_DIST_OVERRIDE.read() {
        if let Some(root) = guard.as_ref() {
            let absolute = root.join(&relative);
            if absolute.is_file() {
                return fs::read(absolute).map_err(WebServeError::Io);
            }
        }
    }

    // 2. Embedded assets supplied by the product shell.
    let path_str = relative.to_string_lossy().replace('\\', "/");
    if let Ok(guard) = EMBEDDED_WEB_ASSET_LOADER.read() {
        if let Some(loader) = *guard {
            if let Some(bytes) = loader(path_str.as_str()) {
                return Ok(bytes.into_owned());
            }
        }
    }

    // 3. Development fallback (search nearby directories)
    let web_root = resolved_web_dist_root().ok_or(WebServeError::Unavailable)?;
    let absolute = web_root.join(&relative);
    if !absolute.is_file() {
        return Err(WebServeError::MissingFile);
    }
    fs::read(absolute).map_err(WebServeError::Io)
}

fn sanitize_relative_path(raw: &str) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for component in FsPath::new(raw).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

fn resolved_web_dist_root() -> Option<PathBuf> {
    WEB_DIST_ROOT.get_or_init(resolve_web_dist_root).clone()
}

fn resolve_web_dist_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(value) = std::env::var("AGENDAO_WEB_DIST") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            if dir.file_name().map(|name| name == "bin").unwrap_or(false) {
                if let Some(prefix) = dir.parent() {
                    candidates.push(prefix.join("share").join("agendao").join("web"));
                }
            }
            if dir.file_name().map(|name| name == "MacOS").unwrap_or(false) {
                if let Some(contents) = dir.parent() {
                    candidates.push(contents.join("Resources").join("web"));
                }
            }
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("../../apps/agendao-web/dist"));

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("apps/agendao-web/dist"));
        candidates.push(current_dir.join("agendao/apps/agendao-web/dist"));
    }

    candidates.into_iter().find(|path| has_web_dist(path))
}

fn has_web_dist(path: &FsPath) -> bool {
    path.join("index.html").is_file()
        && path.join("app.js").is_file()
        && path.join("app.css").is_file()
}

fn mime_for_path(path: &str) -> &'static str {
    match FsPath::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "css" => HeaderValueStatic::CSS_UTF8,
        "html" => HeaderValueStatic::HTML_UTF8,
        "ico" => HeaderValueStatic::ICON,
        "js" => HeaderValueStatic::JS_UTF8,
        "png" => HeaderValueStatic::PNG,
        "svg" => HeaderValueStatic::SVG_UTF8,
        "ttf" => HeaderValueStatic::TTF,
        "woff" => HeaderValueStatic::WOFF,
        "woff2" => HeaderValueStatic::WOFF2,
        _ => HeaderValueStatic::OCTET_STREAM,
    }
}

fn web_frontend_unavailable_page() -> String {
    [
        "<!doctype html>",
        "<html lang=\"en\">",
        "  <head>",
        "    <meta charset=\"utf-8\" />",
        "    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />",
        "    <title>AgenDao Web Unavailable</title>",
        "  </head>",
        "  <body>",
        "    <h1>AgenDao Web frontend is unavailable</h1>",
        "    <p>Build apps/agendao-web first: <code>cd apps/agendao-web && npm ci && npx vite build</code></p>",
        "  </body>",
        "</html>",
    ]
    .join("\n")
}

struct HeaderValueStatic;

impl HeaderValueStatic {
    const CSS_UTF8: &'static str = "text/css; charset=utf-8";
    const HTML_UTF8: &'static str = "text/html; charset=utf-8";
    const ICON: &'static str = "image/x-icon";
    const JS_UTF8: &'static str = "application/javascript; charset=utf-8";
    const NO_STORE: &'static str = "no-store";
    const OCTET_STREAM: &'static str = "application/octet-stream";
    const PNG: &'static str = "image/png";
    const SVG_UTF8: &'static str = "image/svg+xml";
    const TTF: &'static str = "font/ttf";
    const WOFF: &'static str = "font/woff";
    const WOFF2: &'static str = "font/woff2";
}

enum WebServeError {
    InvalidPath,
    Io(std::io::Error),
    MissingFile,
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_relative_path_rejects_parent_segments() {
        assert!(sanitize_relative_path("../dist/index.html").is_none());
        assert!(sanitize_relative_path("/etc/passwd").is_none());
    }

    #[test]
    fn web_root_detection_accepts_complete_dist() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::write(root.join("index.html"), "ok").expect("index");
        fs::write(root.join("app.js"), "ok").expect("app js");
        fs::write(root.join("app.css"), "ok").expect("app css");

        assert!(has_web_dist(root));
    }

    #[test]
    fn configured_web_dist_root_overrides_fallback_resolution() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        fs::write(root.join("index.html"), "override").expect("index");
        fs::write(root.join("app.js"), "ok").expect("app js");
        fs::write(root.join("app.css"), "ok").expect("app css");

        configure_web_dist_root(Some(root.to_path_buf()));
        let bytes = load_web_file("index.html").unwrap_or_else(|_| panic!("index should load"));
        assert_eq!(bytes, b"override");
        configure_web_dist_root(None);
    }
}
