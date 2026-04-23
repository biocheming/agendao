use axum::{
    body::Body,
    extract::{Query, State},
    http::header,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;

use crate::{ApiError, Result, ServerState};

pub(crate) fn file_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/", get(list_files).delete(delete_file))
        .route("/directory", post(create_directory))
        .route("/content", get(read_file).put(write_file))
        .route("/tree", get(get_file_tree))
        .route("/download", get(download_file))
        .route("/upload", axum::routing::post(upload_files))
        .route("/status", get(get_file_status))
}

pub(crate) fn find_routes() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/text", get(find_text))
        .route("/file", get(find_files))
        .route("/symbol", get(find_symbols))
}

#[derive(Debug, Deserialize)]
pub struct ListFilesQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct FileTreeQuery {
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteFileRequest {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Deserialize)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub create_parents: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateDirectoryRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub size: Option<u64>,
    pub modified: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct FileTreeNode {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub size: Option<u64>,
    pub modified: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<FileTreeNode>,
}

#[derive(Debug, Serialize)]
pub struct UploadFileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UploadFilesResponse {
    pub files: Vec<UploadFileInfo>,
}

#[derive(Debug, Deserialize)]
pub struct UploadFileRequest {
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub encoding: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UploadFilesRequest {
    #[serde(default)]
    pub path: Option<String>,
    pub files: Vec<UploadFileRequest>,
}

#[derive(Debug, Serialize)]
pub struct FileWriteResponse {
    pub path: String,
    pub bytes_written: usize,
}

#[derive(Debug, Serialize)]
pub struct FileDeleteResponse {
    pub path: String,
    #[serde(rename = "type")]
    pub file_type: String,
}

#[derive(Debug, Serialize)]
pub struct DirectoryCreateResponse {
    pub path: String,
}

fn project_root(state: &ServerState) -> Result<PathBuf> {
    Ok(state.project_root())
}

fn modified_millis(path: &FsPath) -> Option<i64> {
    std::fs::metadata(path).ok().and_then(|metadata| {
        metadata.modified().ok().map(|time| {
            time.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64
        })
    })
}

fn normalize_path(path: &FsPath) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn canonical_root(root: &FsPath) -> Result<PathBuf> {
    root.canonicalize()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve project root: {}", e)))
}

fn nearest_existing_ancestor(path: &FsPath) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

fn effective_root_for_input(input: &str, default_root: &FsPath) -> Result<PathBuf> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return canonical_root(default_root);
    }

    let raw = PathBuf::from(trimmed);
    if !raw.is_absolute() {
        return canonical_root(default_root);
    }

    let resolved = resolve_user_path(trimmed, default_root);
    let ancestor = nearest_existing_ancestor(&resolved).ok_or_else(|| {
        ApiError::BadRequest("Failed to resolve an existing ancestor for path".to_string())
    })?;
    let root = if ancestor.is_dir() {
        ancestor
    } else {
        ancestor
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| ancestor.clone())
    };

    canonical_root(&root)
}

fn canonicalize_within_root(path: &FsPath, root: &FsPath) -> Result<PathBuf> {
    let canonical_root = canonical_root(root)?;
    let canonical_path = path
        .canonicalize()
        .map_err(|e| ApiError::BadRequest(format!("Failed to resolve path: {}", e)))?;

    if !canonical_path.starts_with(&canonical_root) {
        return Err(ApiError::BadRequest(
            "Access denied: path escapes project directory".to_string(),
        ));
    }

    Ok(canonical_path)
}

fn resolve_user_path(input: &str, root: &FsPath) -> PathBuf {
    let path = PathBuf::from(input);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn ensure_within_root_nonexistent(path: &FsPath, root: &FsPath) -> Result<PathBuf> {
    let canonical_root = canonical_root(root)?;
    let normalized = normalize_path(path);

    if !normalized.starts_with(&canonical_root) {
        return Err(ApiError::BadRequest(
            "Access denied: path escapes project directory".to_string(),
        ));
    }

    if let Some(parent) = normalized.parent() {
        if parent.exists() {
            let canonical_parent = parent.canonicalize().map_err(|e| {
                ApiError::BadRequest(format!("Failed to resolve parent directory: {}", e))
            })?;
            if !canonical_parent.starts_with(&canonical_root) {
                return Err(ApiError::BadRequest(
                    "Access denied: path escapes project directory".to_string(),
                ));
            }
        }
    }

    Ok(normalized)
}

fn resolve_existing_input_path(input: &str, root: &FsPath) -> Result<PathBuf> {
    let resolved = resolve_user_path(input, root);
    if !resolved.exists() {
        return Err(ApiError::NotFound("File not found".to_string()));
    }
    canonicalize_within_root(&resolved, root)
}

fn resolve_output_path(input: &str, root: &FsPath) -> Result<PathBuf> {
    let resolved = resolve_user_path(input, root);
    ensure_within_root_nonexistent(&resolved, root)
}

fn is_within_root(path: &FsPath, root: &FsPath) -> bool {
    canonicalize_within_root(path, root).is_ok()
}

fn file_info_from_path(path: &FsPath) -> FileInfo {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let file_type = if path.is_dir() { "directory" } else { "file" };
    let size = if path.is_file() {
        std::fs::metadata(path).ok().map(|metadata| metadata.len())
    } else {
        None
    };

    FileInfo {
        name,
        path: path.to_string_lossy().to_string(),
        file_type: file_type.to_string(),
        size,
        modified: modified_millis(path),
    }
}

fn build_tree_node(path: &FsPath, root: &FsPath) -> Result<FileTreeNode> {
    let canonical = canonicalize_within_root(path, root)?;
    let name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| canonical.display().to_string());
    let file_type = if canonical.is_dir() {
        "directory"
    } else {
        "file"
    };
    let size = if canonical.is_file() {
        std::fs::metadata(&canonical)
            .ok()
            .map(|metadata| metadata.len())
    } else {
        None
    };

    let mut children = Vec::new();
    if canonical.is_dir() {
        let mut entries = std::fs::read_dir(&canonical)
            .map_err(|e| ApiError::BadRequest(format!("Failed to read directory: {}", e)))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_within_root(path, root))
            .collect::<Vec<_>>();
        entries.sort();
        for child in entries {
            children.push(build_tree_node(&child, root)?);
        }
    }

    Ok(FileTreeNode {
        name,
        path: canonical.to_string_lossy().to_string(),
        file_type: file_type.to_string(),
        size,
        modified: modified_millis(&canonical),
        children,
    })
}

async fn list_files(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ListFilesQuery>,
) -> Result<Json<Vec<FileInfo>>> {
    let default_root = project_root(state.as_ref())?;
    let root = effective_root_for_input(&query.path, &default_root)?;
    let path = resolve_existing_input_path(&query.path, &root)?;
    let mut files = Vec::new();

    if path.is_dir() {
        let mut entries = std::fs::read_dir(&path)
            .map_err(|e| ApiError::BadRequest(format!("Failed to read directory: {}", e)))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path_buf| is_within_root(path_buf, &root))
            .collect::<Vec<_>>();
        entries.sort();

        for path_buf in entries {
            files.push(file_info_from_path(&path_buf));
        }
    }

    Ok(Json(files))
}

async fn get_file_tree(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<FileTreeQuery>,
) -> Result<Json<FileTreeNode>> {
    let default_root = project_root(state.as_ref())?;
    if let Some(input) = query.path.as_deref() {
        let root = effective_root_for_input(input, &default_root)?;
        Ok(Json(build_tree_node(
            &resolve_existing_input_path(input, &root)?,
            &root,
        )?))
    } else {
        let root = canonical_root(&default_root)?;
        Ok(Json(build_tree_node(&root, &root)?))
    }
}

async fn read_file(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ListFilesQuery>,
) -> Result<Json<serde_json::Value>> {
    let default_root = project_root(state.as_ref())?;
    let root = effective_root_for_input(&query.path, &default_root)?;
    let path = resolve_existing_input_path(&query.path, &root)?;

    if path.is_file() {
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(Json(
                serde_json::json!({ "content": content, "path": query.path }),
            )),
            Err(e) => Err(ApiError::BadRequest(format!("Failed to read file: {}", e))),
        }
    } else {
        Err(ApiError::BadRequest("Path is not a file".to_string()))
    }
}

async fn write_file(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<WriteFileRequest>,
) -> Result<Json<FileWriteResponse>> {
    let default_root = project_root(state.as_ref())?;
    let root = effective_root_for_input(&req.path, &default_root)?;
    let path = resolve_output_path(&req.path, &root)?;

    if path.exists() && path.is_dir() {
        return Err(ApiError::BadRequest(
            "Cannot write file content to a directory".to_string(),
        ));
    }

    let parent = path.parent().ok_or_else(|| {
        ApiError::BadRequest("Target file path must have a parent directory".to_string())
    })?;

    if !parent.exists() {
        if req.create_parents {
            std::fs::create_dir_all(parent).map_err(|e| {
                ApiError::BadRequest(format!("Failed to create parent directories: {}", e))
            })?;
        } else {
            return Err(ApiError::BadRequest(
                "Parent directory does not exist".to_string(),
            ));
        }
    }

    std::fs::write(&path, req.content.as_bytes())
        .map_err(|e| ApiError::BadRequest(format!("Failed to write file: {}", e)))?;

    Ok(Json(FileWriteResponse {
        path: path.to_string_lossy().to_string(),
        bytes_written: req.content.len(),
    }))
}

async fn create_directory(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CreateDirectoryRequest>,
) -> Result<Json<DirectoryCreateResponse>> {
    let default_root = project_root(state.as_ref())?;
    let root = effective_root_for_input(&req.path, &default_root)?;
    let path = resolve_output_path(&req.path, &root)?;

    if path.exists() {
        if path.is_dir() {
            return Ok(Json(DirectoryCreateResponse {
                path: path.to_string_lossy().to_string(),
            }));
        }

        return Err(ApiError::BadRequest(
            "Cannot create directory because a file already exists at the target path".to_string(),
        ));
    }

    std::fs::create_dir_all(&path)
        .map_err(|e| ApiError::BadRequest(format!("Failed to create directory: {}", e)))?;

    Ok(Json(DirectoryCreateResponse {
        path: path.to_string_lossy().to_string(),
    }))
}

async fn delete_file(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<DeleteFileRequest>,
) -> Result<Json<FileDeleteResponse>> {
    let default_root = project_root(state.as_ref())?;
    let root = effective_root_for_input(&req.path, &default_root)?;
    let path = resolve_existing_input_path(&req.path, &root)?;

    if path.is_dir() {
        if req.recursive {
            std::fs::remove_dir_all(&path)
                .map_err(|e| ApiError::BadRequest(format!("Failed to delete directory: {}", e)))?;
        } else {
            std::fs::remove_dir(&path)
                .map_err(|e| ApiError::BadRequest(format!("Failed to delete directory: {}", e)))?;
        }
        return Ok(Json(FileDeleteResponse {
            path: path.to_string_lossy().to_string(),
            file_type: "directory".to_string(),
        }));
    }

    std::fs::remove_file(&path)
        .map_err(|e| ApiError::BadRequest(format!("Failed to delete file: {}", e)))?;

    Ok(Json(FileDeleteResponse {
        path: path.to_string_lossy().to_string(),
        file_type: "file".to_string(),
    }))
}

async fn download_file(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ListFilesQuery>,
) -> Result<impl IntoResponse> {
    let default_root = project_root(state.as_ref())?;
    let root = effective_root_for_input(&query.path, &default_root)?;
    let path = resolve_existing_input_path(&query.path, &root)?;

    if !path.is_file() {
        return Err(ApiError::BadRequest(
            "Only files can be downloaded".to_string(),
        ));
    }

    let bytes = std::fs::read(&path)
        .map_err(|e| ApiError::BadRequest(format!("Failed to read file for download: {}", e)))?;
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "download.bin".to_string());

    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                Box::leak(format!("attachment; filename=\"{}\"", filename).into_boxed_str()),
            ),
        ],
        Body::from(bytes),
    ))
}

fn decode_upload_bytes(file: &UploadFileRequest) -> Result<Vec<u8>> {
    let trimmed = file.content.trim();

    if let Some(rest) = trimmed.strip_prefix("data:") {
        let (_, payload) = rest
            .split_once(',')
            .ok_or_else(|| ApiError::BadRequest("Invalid data URL payload".to_string()))?;
        return base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(|e| ApiError::BadRequest(format!("Failed to decode data URL: {}", e)));
    }

    if matches!(file.encoding.as_deref(), Some("base64")) {
        return base64::engine::general_purpose::STANDARD
            .decode(trimmed)
            .map_err(|e| ApiError::BadRequest(format!("Failed to decode base64 content: {}", e)));
    }

    Ok(file.content.as_bytes().to_vec())
}

fn persist_uploaded_files(
    root: &FsPath,
    target_dir: &FsPath,
    files: Vec<UploadFileRequest>,
) -> Result<Json<UploadFilesResponse>> {
    std::fs::create_dir_all(target_dir)
        .map_err(|e| ApiError::BadRequest(format!("Failed to create upload target: {}", e)))?;

    let mut saved_files = Vec::new();
    for file in files {
        let safe_name = FsPath::new(&file.name)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| ApiError::BadRequest("Uploaded file name is invalid".to_string()))?;
        let bytes = decode_upload_bytes(&file)?;

        let output_path = ensure_within_root_nonexistent(&target_dir.join(&safe_name), root)?;
        std::fs::write(&output_path, &bytes)
            .map_err(|e| ApiError::BadRequest(format!("Failed to write uploaded file: {}", e)))?;

        saved_files.push(UploadFileInfo {
            name: safe_name,
            path: output_path.to_string_lossy().to_string(),
            size: bytes.len() as u64,
            mime: file.mime,
        });
    }

    Ok(Json(UploadFilesResponse { files: saved_files }))
}

async fn upload_files(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<UploadFilesRequest>,
) -> Result<Json<UploadFilesResponse>> {
    let default_root = project_root(state.as_ref())?;
    if req.files.is_empty() {
        return Err(ApiError::BadRequest(
            "No uploaded files were provided".to_string(),
        ));
    }

    if let Some(path) = req.path.as_deref() {
        let root = effective_root_for_input(path, &default_root)?;
        let target_dir = resolve_output_path(path, &root)?;
        persist_uploaded_files(&root, &target_dir, req.files)
    } else {
        let root = canonical_root(&default_root)?;
        persist_uploaded_files(&root, &root, req.files)
    }
}

async fn get_file_status(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<Vec<FileStatusInfo>>> {
    let cwd = state.project_root();
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .arg("status")
        .arg("--porcelain")
        .output()
        .map_err(|e| ApiError::BadRequest(format!("Failed to run git status: {}", e)))?;

    if !output.status.success() {
        return Ok(Json(Vec::new()));
    }

    let mut files = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.len() < 4 {
            continue;
        }
        let status_code = &line[..2];
        let mut path = line[3..].trim().to_string();
        if let Some((_, renamed_to)) = path.rsplit_once(" -> ") {
            path = renamed_to.to_string();
        }

        let staged = status_code.chars().next().unwrap_or(' ') != ' ';
        let status_char = if staged {
            status_code.chars().next().unwrap_or(' ')
        } else {
            status_code.chars().nth(1).unwrap_or(' ')
        };
        let status = match status_char {
            'M' => "modified",
            'A' => "added",
            'D' => "deleted",
            'R' => "renamed",
            'C' => "copied",
            'U' => "unmerged",
            '?' => "untracked",
            _ => "unknown",
        };

        files.push(FileStatusInfo {
            path,
            status: status.to_string(),
            staged,
        });
    }

    Ok(Json(files))
}

#[derive(Debug, Serialize)]
pub struct FileStatusInfo {
    pub path: String,
    pub status: String,
    pub staged: bool,
}

#[derive(Debug, Deserialize)]
pub struct FindTextQuery {
    pub pattern: String,
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub match_text: String,
}

async fn find_text(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<FindTextQuery>,
) -> Result<Json<Vec<SearchResult>>> {
    let default_root = project_root(state.as_ref())?;
    let base_input = query
        .path
        .unwrap_or_else(|| default_root.to_string_lossy().to_string());
    let root = effective_root_for_input(&base_input, &default_root)?;
    let base_path = resolve_existing_input_path(&base_input, &root)?;
    let mut results = Vec::new();

    fn search_in_file(path: &FsPath, pattern: &str, results: &mut Vec<SearchResult>) {
        if let Ok(content) = std::fs::read_to_string(path) {
            for (line_num, line) in content.lines().enumerate() {
                if let Some(col) = line.find(pattern) {
                    results.push(SearchResult {
                        path: path.to_string_lossy().to_string(),
                        line: line_num + 1,
                        column: col + 1,
                        match_text: line.to_string(),
                    });
                }
            }
        }
    }

    fn search_recursive(
        path: &FsPath,
        root: &FsPath,
        pattern: &str,
        results: &mut Vec<SearchResult>,
    ) {
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let path_buf = entry.path();
                    if !is_within_root(&path_buf, root) {
                        continue;
                    }
                    if path_buf.is_dir() {
                        search_recursive(&path_buf, root, pattern, results);
                    } else if path_buf.is_file() {
                        search_in_file(&path_buf, pattern, results);
                    }
                }
            }
        }
    }

    search_recursive(&base_path, &root, &query.pattern, &mut results);
    Ok(Json(results))
}

#[cfg(test)]
mod tests {
    use super::{effective_root_for_input, nearest_existing_ancestor};
    use std::path::PathBuf;

    #[test]
    fn nearest_existing_ancestor_walks_up_for_missing_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let existing = temp.path().join("workspace");
        std::fs::create_dir_all(&existing).expect("mkdir");

        let missing = existing.join("nested").join("file.txt");
        assert_eq!(
            nearest_existing_ancestor(&missing).as_deref(),
            Some(existing.as_path())
        );
    }

    #[test]
    fn effective_root_keeps_relative_paths_inside_default_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("project");
        std::fs::create_dir_all(&root).expect("mkdir");

        let resolved = effective_root_for_input("src/main.rs", &root).expect("root");
        assert_eq!(resolved, root.canonicalize().expect("canonical"));
    }

    #[test]
    fn effective_root_uses_absolute_workspace_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let default_root = temp.path().join("server-root");
        let workspace = temp.path().join("external-workspace");
        std::fs::create_dir_all(&default_root).expect("mkdir default");
        std::fs::create_dir_all(workspace.join("src")).expect("mkdir workspace");

        let resolved =
            effective_root_for_input(workspace.to_string_lossy().as_ref(), &default_root)
                .expect("root");
        assert_eq!(resolved, workspace.canonicalize().expect("canonical"));
    }

    #[test]
    fn effective_root_uses_parent_for_absolute_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let default_root = temp.path().join("server-root");
        let workspace = temp.path().join("external-workspace");
        let file = workspace.join("src").join("lib.rs");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir workspace");
        std::fs::create_dir_all(&default_root).expect("mkdir default");
        std::fs::write(&file, "fn main() {}").expect("write file");

        let resolved =
            effective_root_for_input(file.to_string_lossy().as_ref(), &default_root).expect("root");
        assert_eq!(
            resolved,
            file.parent()
                .map(PathBuf::from)
                .expect("parent")
                .canonicalize()
                .expect("canonical")
        );
    }
}

#[derive(Debug, Deserialize)]
pub struct FindFilesQuery {
    pub query: String,
    #[serde(rename = "type")]
    pub file_type: Option<String>,
    pub limit: Option<usize>,
}

async fn find_files(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<FindFilesQuery>,
) -> Result<Json<Vec<String>>> {
    let base_path = project_root(state.as_ref())?;
    let mut results = Vec::new();
    let limit = query.limit.unwrap_or(100);
    let match_directories = query.file_type.as_deref() != Some("file");
    let match_files = query.file_type.as_deref() != Some("directory");

    fn find_recursive(
        path: &FsPath,
        root: &FsPath,
        query: &str,
        results: &mut Vec<String>,
        limit: usize,
        match_directories: bool,
        match_files: bool,
    ) {
        if results.len() >= limit {
            return;
        }
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let path_buf = entry.path();
                    if !is_within_root(&path_buf, root) {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    let should_match = (path_buf.is_dir() && match_directories)
                        || (path_buf.is_file() && match_files);
                    if should_match && name.contains(query) {
                        results.push(path_buf.to_string_lossy().to_string());
                    }
                    if path_buf.is_dir() && results.len() < limit {
                        find_recursive(
                            &path_buf,
                            root,
                            query,
                            results,
                            limit,
                            match_directories,
                            match_files,
                        );
                    }
                }
            }
        }
    }

    find_recursive(
        &base_path,
        &base_path,
        &query.query,
        &mut results,
        limit,
        match_directories,
        match_files,
    );
    Ok(Json(results))
}

#[derive(Debug, Deserialize)]
pub struct FindSymbolsQuery {
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line: usize,
}

async fn find_symbols(
    State(_state): State<Arc<ServerState>>,
    Query(query): Query<FindSymbolsQuery>,
) -> Result<Json<Vec<SymbolInfo>>> {
    let _ = query.query.as_str();
    Ok(Json(Vec::new()))
}
