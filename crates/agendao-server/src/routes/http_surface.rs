use axum::{routing::get, Json, Router};
use serde::Serialize;

use crate::web;

pub fn attach_http_shell_routes<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .route("/", get(web::web_index))
        .route("/favicon.ico", get(web::root_favicon))
        .route("/apple-touch-icon.png", get(web::root_apple_touch_icon))
        .route("/web", get(web::web_index))
        .route("/web/", get(web::web_index))
        .route("/web/{*path}", get(web::web_file))
        .route("/doc", get(get_doc))
}

#[derive(Debug, Serialize)]
struct DocInfo {
    title: String,
    version: String,
    description: String,
    openapi: String,
}

#[derive(Debug, Serialize)]
struct DocResponse {
    info: DocInfo,
}

async fn get_doc() -> Json<DocResponse> {
    Json(DocResponse {
        info: DocInfo {
            title: "agendao".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            description: "agendao api".to_string(),
            openapi: "3.1.1".to_string(),
        },
    })
}
