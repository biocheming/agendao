use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;

use crate::{ApiError, Result, ServerState};
use rocode_config::{discover_web_plugins, get_plugin_roots, WebPluginInfo};

pub(crate) fn web_plugin_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_web_plugins))
        .route("/serve", get(serve_web_plugin_query))
        .route("/serve/{plugin}/{*file}", get(serve_web_plugin_path))
}

#[derive(Debug, Serialize)]
struct WebPluginEntry {
    name: String,
    entry: String,
}

#[derive(Debug, Deserialize, Default)]
struct WorkspaceQuery {
    workspace: Option<String>,
}

async fn list_web_plugins(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<WorkspaceQuery>,
) -> Result<Json<Vec<WebPluginEntry>>> {
    let roots = web_plugin_roots(&state, query.workspace.as_deref());
    let plugins = discover_web_plugins(&roots);
    let entries: Vec<WebPluginEntry> = plugins
        .into_iter()
        .map(|p| {
            let entry = p.entry();
            WebPluginEntry {
                name: p.name,
                entry,
            }
        })
        .collect();
    Ok(Json(entries))
}

#[derive(Debug, Deserialize)]
struct ServeQuery {
    plugin: String,
    file: String,
    workspace: Option<String>,
}

async fn serve_web_plugin_query(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ServeQuery>,
) -> Result<impl IntoResponse> {
    serve_web_plugin_inner(
        &state,
        &query.plugin,
        &query.file,
        query.workspace.as_deref(),
    )
}

async fn serve_web_plugin_path(
    State(state): State<Arc<ServerState>>,
    Path((plugin, file)): Path<(String, String)>,
    Query(query): Query<WorkspaceQuery>,
) -> Result<impl IntoResponse> {
    serve_web_plugin_inner(&state, &plugin, &file, query.workspace.as_deref())
}

fn serve_web_plugin_inner(
    state: &Arc<ServerState>,
    plugin_name: &str,
    requested_file: &str,
    workspace_path: Option<&str>,
) -> Result<impl IntoResponse> {
    let roots = web_plugin_roots(state, workspace_path);
    let plugins = discover_web_plugins(&roots);

    let plugin = plugins
        .iter()
        .find(|p| p.name == plugin_name)
        .ok_or_else(|| ApiError::NotFound(format!("Web plugin '{}' not found", plugin_name)))?;

    let asset_path = resolve_web_plugin_asset_path(plugin, requested_file)?;
    let bytes = std::fs::read(&asset_path)
        .map_err(|e| ApiError::BadRequest(format!("Failed to read plugin file: {}", e)))?;

    Ok((
        [
            (header::CONTENT_TYPE, web_plugin_content_type(&asset_path)),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from(bytes),
    ))
}

fn web_plugin_roots(state: &ServerState, workspace_path: Option<&str>) -> Vec<PathBuf> {
    let config = state.config_store.config();
    let project_dir = workspace_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| state.config_store.project_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    get_plugin_roots(&project_dir, &config.plugin_paths)
}

fn resolve_web_plugin_asset_path(plugin: &WebPluginInfo, requested_file: &str) -> Result<PathBuf> {
    let relative_path = sanitize_relative_file_path(requested_file)?;
    let candidate = plugin.serve_root.join(relative_path);
    let canonical_root =
        std::fs::canonicalize(&plugin.serve_root).unwrap_or_else(|_| plugin.serve_root.clone());
    let canonical_candidate = std::fs::canonicalize(&candidate)
        .map_err(|_| ApiError::NotFound(format!("Plugin asset '{}' not found", requested_file)))?;

    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(ApiError::BadRequest("Invalid path".to_string()));
    }

    Ok(canonical_candidate)
}

fn sanitize_relative_file_path(requested_file: &str) -> Result<PathBuf> {
    let mut relative = PathBuf::new();
    for component in FsPath::new(requested_file).components() {
        match component {
            Component::Normal(segment) => relative.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ApiError::BadRequest("Invalid path".to_string()));
            }
        }
    }

    if relative.as_os_str().is_empty() {
        return Err(ApiError::BadRequest("Invalid path".to_string()));
    }

    Ok(relative)
}

fn web_plugin_content_type(path: &FsPath) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let unique = format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock error")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("failed to create temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn sanitize_relative_file_path_rejects_parent_segments() {
        let error =
            sanitize_relative_file_path("../secret.js").expect_err("path should be rejected");
        assert!(matches!(error, ApiError::BadRequest(_)));
    }

    #[test]
    fn resolve_web_plugin_asset_path_supports_directory_assets() {
        let temp = TestDir::new("rocode_web_plugin_assets");
        let plugin_dir = temp.path.join("web").join("molstar");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        fs::write(plugin_dir.join("index.js"), "import './config.js';\n").expect("index");
        fs::write(plugin_dir.join("config.js"), "export const config = {};\n").expect("config");

        let plugin = WebPluginInfo {
            name: "molstar".to_string(),
            entry_path: plugin_dir.join("index.js"),
            serve_root: plugin_dir.clone(),
        };

        let asset_path =
            resolve_web_plugin_asset_path(&plugin, "config.js").expect("asset path should resolve");

        assert_eq!(
            asset_path,
            fs::canonicalize(plugin_dir.join("config.js")).expect("canonical")
        );
    }
}
