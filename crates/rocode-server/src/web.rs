use std::fs;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::OnceLock;

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};

static WEB_DIST_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();

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
            "ROCode Web frontend assets are unavailable. Build apps/rocode-web or set ROCODE_WEB_DIST.",
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
    let web_root = resolved_web_dist_root().ok_or(WebServeError::Unavailable)?;
    let relative = sanitize_relative_path(relative).ok_or(WebServeError::InvalidPath)?;
    let absolute = web_root.join(relative);
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

fn resolved_web_dist_root() -> Option<&'static PathBuf> {
    WEB_DIST_ROOT.get_or_init(resolve_web_dist_root).as_ref()
}

fn resolve_web_dist_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(value) = std::env::var("ROCODE_WEB_DIST") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            if dir.file_name().map(|name| name == "bin").unwrap_or(false) {
                if let Some(prefix) = dir.parent() {
                    candidates.push(prefix.join("share").join("rocode").join("web"));
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
    candidates.push(manifest_dir.join("../../apps/rocode-web/dist"));

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("apps/rocode-web/dist"));
        candidates.push(current_dir.join("rocode/apps/rocode-web/dist"));
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
        "    <title>ROCode Web Unavailable</title>",
        "  </head>",
        "  <body>",
        "    <h1>ROCode Web frontend is unavailable</h1>",
        "    <p>The backend no longer embeds Web assets.</p>",
        "    <p>Build <code>apps/rocode-web</code> separately and point the server at its <code>dist/</code> directory with <code>ROCODE_WEB_DIST</code>, or install a release package that includes <code>share/rocode/web</code>.</p>",
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
}
