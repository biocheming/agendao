use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

pub async fn web_index() -> Html<&'static str> {
    Html(include_str!("../web/dist/index.html"))
}

pub async fn web_app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValueStatic::CSS_UTF8),
            (header::CACHE_CONTROL, HeaderValueStatic::NO_STORE),
        ],
        include_str!("../web/dist/app.css"),
    )
}

pub async fn web_app_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValueStatic::JS_UTF8),
            (header::CACHE_CONTROL, HeaderValueStatic::NO_STORE),
        ],
        include_str!("../web/dist/app.js"),
    )
}

pub async fn web_asset(Path(path): Path<String>) -> impl IntoResponse {
    match web_asset_bytes(path.as_str()) {
        Some((bytes, content_type)) => (
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, HeaderValueStatic::NO_STORE),
            ],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn web_favicon_ico() -> impl IntoResponse {
    binary_asset(
        include_bytes!("../web/public/favicon.ico"),
        HeaderValueStatic::ICON,
    )
}

pub async fn web_favicon_png_32() -> impl IntoResponse {
    binary_asset(
        include_bytes!("../web/public/favicon-32.png"),
        HeaderValueStatic::PNG,
    )
}

pub async fn web_favicon_png_192() -> impl IntoResponse {
    binary_asset(
        include_bytes!("../web/public/favicon-192.png"),
        HeaderValueStatic::PNG,
    )
}

pub async fn web_apple_touch_icon() -> impl IntoResponse {
    binary_asset(
        include_bytes!("../web/public/apple-touch-icon.png"),
        HeaderValueStatic::PNG,
    )
}

pub async fn web_rocode_logo_png() -> impl IntoResponse {
    binary_asset(
        include_bytes!("../../../icons/rocode_logo.png"),
        HeaderValueStatic::PNG,
    )
}

fn binary_asset(bytes: &'static [u8], content_type: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, HeaderValueStatic::NO_STORE),
        ],
        bytes,
    )
}

struct HeaderValueStatic;

impl HeaderValueStatic {
    const CSS_UTF8: &'static str = "text/css; charset=utf-8";
    const ICON: &'static str = "image/x-icon";
    const JS_UTF8: &'static str = "application/javascript; charset=utf-8";
    const NO_STORE: &'static str = "no-store";
    const PNG: &'static str = "image/png";
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn response_text(response: impl IntoResponse) -> String {
        let response = response.into_response();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable");
        String::from_utf8(bytes.to_vec()).expect("response body should be utf-8")
    }

    #[tokio::test]
    async fn web_index_route_serves_react_frontend() {
        let root = response_text(web_index().await).await;

        assert!(root.contains("/web/app.js"));
        assert!(root.contains("/web/app.css"));
    }
}
