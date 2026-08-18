use agendao_orchestrator::{
    ArtifactRequest, CapabilityBackend, CheckpointHandle, CheckpointRequest, RestoreRequest,
    RunDisposition, WorkspaceLimits,
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct WorkspaceCapabilityHost {
    allowed_root: PathBuf,
    created_checkpoints: Mutex<Vec<CreatedCheckpoint>>,
    created_artifacts: Mutex<Vec<PathBuf>>,
    in_flight: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

#[derive(Clone)]
struct CreatedCheckpoint {
    handle: CheckpointHandle,
    path: PathBuf,
}

impl WorkspaceCapabilityHost {
    pub fn new(allowed_root: PathBuf) -> Result<Self, String> {
        Ok(Self {
            allowed_root: allowed_root
                .canonicalize()
                .map_err(|error| format!("invalid workspace authority: {error}"))?,
            created_checkpoints: Mutex::new(Vec::new()),
            created_artifacts: Mutex::new(Vec::new()),
            in_flight: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl CapabilityBackend for WorkspaceCapabilityHost {
    async fn checkpoint(&self, request: &CheckpointRequest) -> Result<CheckpointHandle, String> {
        let request = request.clone();
        let handle = CheckpointHandle {
            capability: request.capability.clone(),
            workspace_root: request.workspace_root.clone(),
            id: checkpoint_id(&request),
            iteration: request.iteration,
        };
        let allowed_root = self.allowed_root.clone();
        let (workspace, checkpoint) = checkpoint_paths(&allowed_root, &request)?;
        let created = CreatedCheckpoint {
            handle: handle.clone(),
            path: checkpoint.clone(),
        };
        match self.created_checkpoints.lock() {
            Ok(mut checkpoints) => checkpoints.push(created),
            Err(_) => {
                let _ = fs::remove_dir_all(&created.path);
                return Err("checkpoint cleanup registry is poisoned".to_string());
            }
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let operation = tokio::task::spawn_blocking(move || {
            let result = checkpoint_blocking(&workspace, &checkpoint, &request);
            let _ = sender.send(result);
        });
        self.register_operation(operation)?;
        receiver
            .await
            .map_err(|_| "checkpoint worker stopped before reporting a result".to_string())??;
        Ok(handle)
    }

    async fn restore(&self, request: &RestoreRequest) -> Result<(), String> {
        let checkpoint = self
            .created_checkpoints
            .lock()
            .map_err(|_| "checkpoint cleanup registry is poisoned".to_string())?
            .iter()
            .find(|created| created.handle == request.checkpoint)
            .cloned()
            .ok_or_else(|| "checkpoint handle is not owned by this capability host".to_string())?;
        let request = request.clone();
        let allowed_root = self.allowed_root.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let operation = tokio::task::spawn_blocking(move || {
            let result = restore_blocking(&allowed_root, &checkpoint.path, &request);
            let _ = sender.send(result);
        });
        self.register_operation(operation)?;
        receiver.await.map_err(|_| {
            "checkpoint restore worker stopped before reporting a result".to_string()
        })?
    }

    async fn store_artifact(&self, request: &ArtifactRequest) -> Result<String, String> {
        let workspace = self.allowed_workspace(&request.workspace_root)?;
        if request.content.len() as u64 > request.limits.max_total_bytes {
            return Err("artifact exceeds resource limit".to_string());
        }
        let name = sanitize_name(&request.name)?;
        let relative = PathBuf::from(".agendao")
            .join("artifacts")
            .join(request.capability.as_str())
            .join(name);
        let target = workspace.join(&relative);
        let write_target = target.clone();
        let content = request.content.clone();
        let limits = request.limits.clone();
        match self.created_artifacts.lock() {
            Ok(mut artifacts) => artifacts.push(target.clone()),
            Err(_) => {
                let _ = fs::remove_file(target);
                return Err("artifact cleanup registry is poisoned".to_string());
            }
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let operation = tokio::task::spawn_blocking(move || {
            let result = store_artifact_blocking(&workspace, &write_target, &content, &limits);
            let _ = sender.send(result);
        });
        self.register_operation(operation)?;
        receiver
            .await
            .map_err(|_| "artifact worker stopped before reporting a result".to_string())??;
        Ok(relative.display().to_string())
    }

    async fn finalize(&self, disposition: RunDisposition) -> Result<(), String> {
        let operations = {
            let mut in_flight = self
                .in_flight
                .lock()
                .map_err(|_| "capability operation registry is poisoned".to_string())?;
            std::mem::take(&mut *in_flight)
        };
        let mut operation_errors = Vec::new();
        for operation in operations {
            if let Err(error) = operation.await {
                operation_errors.push(format!("capability worker failed: {error}"));
            }
        }
        let (checkpoints, artifacts) = {
            let mut checkpoint_registry = self
                .created_checkpoints
                .lock()
                .map_err(|_| "checkpoint cleanup registry is poisoned".to_string())?;
            let mut artifact_registry = self
                .created_artifacts
                .lock()
                .map_err(|_| "artifact cleanup registry is poisoned".to_string())?;
            (
                std::mem::take(&mut *checkpoint_registry),
                std::mem::take(&mut *artifact_registry),
            )
        };
        tokio::task::spawn_blocking(move || {
            let checkpoint_result = cleanup_checkpoints(checkpoints);
            let artifact_result = match disposition {
                RunDisposition::Commit => Ok(()),
                RunDisposition::Rollback => cleanup_artifacts(artifacts),
            };
            combine_cleanup_results(checkpoint_result, artifact_result)
        })
        .await
        .map_err(|error| format!("capability finalizer worker failed: {error}"))??;
        if operation_errors.is_empty() {
            Ok(())
        } else {
            Err(operation_errors.join("; "))
        }
    }
}

impl WorkspaceCapabilityHost {
    fn allowed_workspace(&self, workspace_root: &str) -> Result<PathBuf, String> {
        let workspace = PathBuf::from(workspace_root)
            .canonicalize()
            .map_err(|error| format!("invalid workspace root: {error}"))?;
        if !workspace.starts_with(&self.allowed_root) {
            return Err("workspace root is outside capability authority".to_string());
        }
        Ok(workspace)
    }

    fn register_operation(&self, operation: tokio::task::JoinHandle<()>) -> Result<(), String> {
        self.in_flight
            .lock()
            .map_err(|_| "capability operation registry is poisoned".to_string())?
            .push(operation);
        Ok(())
    }
}

impl Drop for WorkspaceCapabilityHost {
    fn drop(&mut self) {
        let checkpoints = self
            .created_checkpoints
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        let _ = cleanup_checkpoints(std::mem::take(checkpoints));
        let artifacts = self
            .created_artifacts
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        let _ = cleanup_artifacts(std::mem::take(artifacts));
    }
}

fn combine_cleanup_results(
    first: Result<(), String>,
    second: Result<(), String>,
) -> Result<(), String> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(second)) => Err(format!("{first}; {second}")),
    }
}

fn cleanup_checkpoints(checkpoints: Vec<CreatedCheckpoint>) -> Result<(), String> {
    let mut errors = Vec::new();
    for checkpoint in checkpoints.into_iter().rev() {
        if let Err(error) = fs::remove_dir_all(&checkpoint.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("{}: {error}", checkpoint.path.display()));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn cleanup_artifacts(artifacts: Vec<PathBuf>) -> Result<(), String> {
    let mut errors = Vec::new();
    for artifact in artifacts.into_iter().rev() {
        if let Err(error) = fs::remove_file(&artifact) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("{}: {error}", artifact.display()));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn restore_blocking(
    allowed_root: &Path,
    checkpoint: &Path,
    request: &RestoreRequest,
) -> Result<(), String> {
    let started = Instant::now();
    let deadline = Duration::from_millis(request.limits.operation_timeout_ms);
    let workspace = PathBuf::from(&request.checkpoint.workspace_root)
        .canonicalize()
        .map_err(|error| format!("invalid workspace root: {error}"))?;
    if !workspace.starts_with(allowed_root) {
        return Err("workspace root is outside capability authority".to_string());
    }
    let checkpoint = checkpoint
        .canonicalize()
        .map_err(|error| format!("checkpoint is unavailable: {error}"))?;
    let checkpoint_root = workspace.join(".agendao").join("checkpoints");
    if !checkpoint.starts_with(&checkpoint_root) {
        return Err("checkpoint path is outside workspace checkpoint authority".to_string());
    }
    let scan_request = CheckpointRequest {
        capability: request.checkpoint.capability.clone(),
        workspace_root: request.checkpoint.workspace_root.clone(),
        scope: request.checkpoint.id.clone(),
        iteration: request.checkpoint.iteration,
        limits: request.limits.clone(),
    };
    let entries = collect_entries(&checkpoint, started, deadline, &scan_request)?;
    check_deadline(started, deadline)?;

    // Once restoration starts, it must finish. The preflight scan enforces the
    // file and byte bounds before any workspace entry is removed.
    clear_workspace_entries(&workspace)?;
    copy_entries(&checkpoint, &workspace, &entries, || Ok(()))
}

fn clear_workspace_entries(workspace: &Path) -> Result<(), String> {
    for item in fs::read_dir(workspace).map_err(|error| error.to_string())? {
        let item = item.map_err(|error| error.to_string())?;
        if item.file_name() == ".agendao" {
            continue;
        }
        let file_type = item.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() && !file_type.is_symlink() {
            fs::remove_dir_all(item.path()).map_err(|error| error.to_string())?;
        } else {
            fs::remove_file(item.path()).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn store_artifact_blocking(
    workspace: &Path,
    target: &Path,
    content: &str,
    limits: &WorkspaceLimits,
) -> Result<(), String> {
    let started = Instant::now();
    let deadline = Duration::from_millis(limits.operation_timeout_ms);
    check_deadline(started, deadline)?;
    let required = content.len() as u64;
    let available = available_disk_bytes(workspace)?;
    if available < limits.min_free_disk_bytes.saturating_add(required) {
        return Err(format!(
            "insufficient disk for artifact: {available} bytes available, {required} bytes required"
        ));
    }

    let parent = target
        .parent()
        .ok_or_else(|| "artifact path has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    if target.exists() {
        return Err("artifact destination already exists".to_string());
    }
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "artifact target has no valid file name".to_string())?;
    let partial = parent.join(format!(".{file_name}.partial"));
    let write_result = (|| -> Result<(), String> {
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial)
            .map_err(|error| error.to_string())?;
        for chunk in content.as_bytes().chunks(64 * 1024) {
            check_deadline(started, deadline)?;
            output.write_all(chunk).map_err(|error| error.to_string())?;
        }
        check_deadline(started, deadline)?;
        output.flush().map_err(|error| error.to_string())?;
        fs::rename(&partial, target).map_err(|error| error.to_string())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    write_result
}

fn checkpoint_blocking(
    workspace: &Path,
    checkpoint_root: &Path,
    request: &CheckpointRequest,
) -> Result<(), String> {
    let started = Instant::now();
    let deadline = Duration::from_millis(request.limits.operation_timeout_ms);
    let entries = collect_entries(workspace, started, deadline, request)?;
    let required = entries.iter().map(|entry| entry.bytes).sum::<u64>();
    let available = available_disk_bytes(workspace)?;
    if available < request.limits.min_free_disk_bytes.saturating_add(required) {
        return Err(format!(
            "insufficient disk for checkpoint: {available} bytes available, {required} bytes required"
        ));
    }

    if checkpoint_root.exists() {
        return Err("checkpoint destination already exists".to_string());
    }
    if let Err(error) = copy_entries(workspace, checkpoint_root, &entries, || {
        check_deadline(started, deadline)
    }) {
        let _ = fs::remove_dir_all(checkpoint_root);
        return Err(error);
    }
    Ok(())
}

fn checkpoint_id(request: &CheckpointRequest) -> String {
    let scope = format!("{:x}", Sha256::digest(request.scope.as_bytes()));
    format!(
        "{}:{scope}:{}",
        request.capability.as_str(),
        request.iteration
    )
}

fn checkpoint_paths(
    allowed_root: &Path,
    request: &CheckpointRequest,
) -> Result<(PathBuf, PathBuf), String> {
    let workspace = PathBuf::from(&request.workspace_root)
        .canonicalize()
        .map_err(|error| format!("invalid workspace root: {error}"))?;
    if !workspace.starts_with(allowed_root) {
        return Err("workspace root is outside capability authority".to_string());
    }
    let scope = format!("{:x}", Sha256::digest(request.scope.as_bytes()));
    let checkpoint = workspace
        .join(".agendao")
        .join("checkpoints")
        .join(request.capability.as_str())
        .join(scope)
        .join(format!("iteration-{}", request.iteration));
    Ok((workspace, checkpoint))
}

struct FileEntry {
    relative: PathBuf,
    bytes: u64,
}

fn collect_entries(
    workspace: &Path,
    started: Instant,
    deadline: Duration,
    request: &CheckpointRequest,
) -> Result<Vec<FileEntry>, String> {
    let mut pending = vec![workspace.to_path_buf()];
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    while let Some(directory) = pending.pop() {
        check_deadline(started, deadline)?;
        for item in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let item = item.map_err(|error| error.to_string())?;
            if directory == workspace && item.file_name() == ".agendao" {
                continue;
            }
            let file_type = item.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(item.path());
            } else if file_type.is_file() {
                let bytes = item.metadata().map_err(|error| error.to_string())?.len();
                total_bytes = total_bytes.saturating_add(bytes);
                files.push(FileEntry {
                    relative: item
                        .path()
                        .strip_prefix(workspace)
                        .map_err(|error| error.to_string())?
                        .to_path_buf(),
                    bytes,
                });
                if files.len() > request.limits.max_files as usize
                    || total_bytes > request.limits.max_total_bytes
                {
                    return Err("workspace exceeds checkpoint resource limits".to_string());
                }
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

fn copy_entries(
    workspace: &Path,
    destination: &Path,
    entries: &[FileEntry],
    mut check_operation: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let mut buffer = vec![0u8; 64 * 1024];
    for entry in entries {
        check_operation()?;
        let target = destination.join(&entry.relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut source =
            fs::File::open(workspace.join(&entry.relative)).map_err(|error| error.to_string())?;
        let mut output = fs::File::create(&target).map_err(|error| error.to_string())?;
        let mut copied = 0u64;
        loop {
            check_operation()?;
            let read = source
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            copied = copied.saturating_add(read as u64);
        }
        if copied != entry.bytes {
            return Err("workspace file changed while checkpointing".to_string());
        }
    }
    Ok(())
}

fn check_deadline(started: Instant, deadline: Duration) -> Result<(), String> {
    if started.elapsed() >= deadline {
        Err("checkpoint deadline exceeded".to_string())
    } else {
        Ok(())
    }
}

fn sanitize_name(name: &str) -> Result<&str, String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.as_bytes().contains(&0)
    {
        Err("artifact name must be a single safe path segment".to_string())
    } else {
        Ok(name)
    }
}

#[cfg(unix)]
fn available_disk_bytes(path: &Path) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "workspace path contains a NUL byte".to_string())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let stats = unsafe { stats.assume_init() };
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn available_disk_bytes(_path: &Path) -> Result<u64, String> {
    Err("checkpoint disk accounting is unsupported on this platform".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agendao_orchestrator::{CapabilityId, WorkspaceLimits};

    fn request(root: &Path) -> CheckpointRequest {
        CheckpointRequest {
            capability: CapabilityId::from("workspace"),
            workspace_root: root.display().to_string(),
            scope: "root/test-loop".to_string(),
            iteration: 1,
            limits: WorkspaceLimits {
                max_files: 4,
                max_total_bytes: 1024,
                min_free_disk_bytes: 0,
                operation_timeout_ms: 5_000,
            },
        }
    }

    #[tokio::test]
    async fn creates_bounded_checkpoint_and_rejects_oversized_workspace() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source.txt"), b"content").unwrap();
        let host = WorkspaceCapabilityHost::new(root.path().to_path_buf()).unwrap();
        let checkpoint_request = request(root.path());
        let (_, checkpoint_path) = checkpoint_paths(root.path(), &checkpoint_request).unwrap();
        let checkpoint = host.checkpoint(&checkpoint_request).await.unwrap();
        assert_eq!(
            fs::read(checkpoint_path.join("source.txt")).unwrap(),
            b"content"
        );
        fs::write(root.path().join("source.txt"), b"mutated").unwrap();
        fs::write(root.path().join("new.txt"), b"temporary").unwrap();
        host.restore(&RestoreRequest {
            checkpoint: checkpoint.clone(),
            limits: request(root.path()).limits,
        })
        .await
        .unwrap();
        assert_eq!(
            fs::read(root.path().join("source.txt")).unwrap(),
            b"content"
        );
        assert!(!root.path().join("new.txt").exists());

        fs::write(root.path().join("source.txt"), b"keep-on-timeout").unwrap();
        let mut expired_limits = request(root.path()).limits;
        expired_limits.operation_timeout_ms = 0;
        assert!(host
            .restore(&RestoreRequest {
                checkpoint,
                limits: expired_limits,
            })
            .await
            .is_err());
        assert_eq!(
            fs::read(root.path().join("source.txt")).unwrap(),
            b"keep-on-timeout"
        );
        host.finalize(RunDisposition::Commit).await.unwrap();
        drop(host);
        assert!(!checkpoint_path.exists());

        let second = tempfile::tempdir().unwrap();
        fs::write(second.path().join("large.bin"), vec![0; 32]).unwrap();
        let host = WorkspaceCapabilityHost::new(second.path().to_path_buf()).unwrap();
        let mut limited = request(second.path());
        limited.limits.max_total_bytes = 8;
        assert!(host.checkpoint(&limited).await.is_err());
        assert!(!second.path().join(".agendao/checkpoints").exists());
    }

    #[tokio::test]
    async fn isolates_checkpoints_by_scope_at_the_same_iteration() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("source.txt"), b"content").unwrap();
        let host = WorkspaceCapabilityHost::new(root.path().to_path_buf()).unwrap();
        let first = request(root.path());
        let mut second = first.clone();
        second.scope = "root/parallel-loop".to_string();
        let (_, first_path) = checkpoint_paths(root.path(), &first).unwrap();
        let (_, second_path) = checkpoint_paths(root.path(), &second).unwrap();

        let first_handle = host.checkpoint(&first).await.unwrap();
        let second_handle = host.checkpoint(&second).await.unwrap();

        assert_ne!(first_handle.id, second_handle.id);
        assert_ne!(first_path, second_path);
        assert_eq!(fs::read(first_path.join("source.txt")).unwrap(), b"content");
        assert_eq!(
            fs::read(second_path.join("source.txt")).unwrap(),
            b"content"
        );
        host.finalize(RunDisposition::Commit).await.unwrap();
        assert!(!first_path.exists());
        assert!(!second_path.exists());
    }

    #[tokio::test]
    async fn finalization_waits_for_registered_work_after_its_caller_is_dropped() {
        let root = tempfile::tempdir().unwrap();
        let artifact = root.path().join(".agendao/artifacts/test/pending.txt");
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::write(&artifact, b"pending").unwrap();
        let host =
            std::sync::Arc::new(WorkspaceCapabilityHost::new(root.path().to_path_buf()).unwrap());
        host.created_artifacts
            .lock()
            .unwrap()
            .push(artifact.clone());
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let (registered_sender, registered_receiver) = tokio::sync::oneshot::channel();
        let caller_host = host.clone();
        let caller = tokio::spawn(async move {
            let operation = tokio::task::spawn_blocking(move || {
                release_receiver.recv().unwrap();
            });
            caller_host.register_operation(operation).unwrap();
            registered_sender.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        registered_receiver.await.unwrap();
        caller.abort();

        let finalizer_host = host.clone();
        let finalizer =
            tokio::spawn(async move { finalizer_host.finalize(RunDisposition::Rollback).await });
        tokio::task::yield_now().await;
        assert!(!finalizer.is_finished());
        assert!(artifact.exists());

        release_sender.send(()).unwrap();
        finalizer.await.unwrap().unwrap();
        assert!(!artifact.exists());
    }

    #[tokio::test]
    async fn stores_artifact_inside_authority_and_rejects_path_traversal() {
        let root = tempfile::tempdir().unwrap();
        let host = WorkspaceCapabilityHost::new(root.path().to_path_buf()).unwrap();
        let limits = request(root.path()).limits;
        let stored = host
            .store_artifact(&ArtifactRequest {
                capability: CapabilityId::from("artifacts"),
                workspace_root: root.path().display().to_string(),
                name: "result.txt".to_string(),
                content: "answer".to_string(),
                limits: limits.clone(),
            })
            .await
            .unwrap();
        assert_eq!(stored, ".agendao/artifacts/artifacts/result.txt");
        assert_eq!(
            fs::read_to_string(root.path().join(stored)).unwrap(),
            "answer"
        );
        assert!(host
            .store_artifact(&ArtifactRequest {
                capability: CapabilityId::from("artifacts"),
                workspace_root: root.path().display().to_string(),
                name: "../escape".to_string(),
                content: "bad".to_string(),
                limits,
            })
            .await
            .is_err());

        let mut pressure_limits = request(root.path()).limits;
        pressure_limits.min_free_disk_bytes = u64::MAX;
        assert!(host
            .store_artifact(&ArtifactRequest {
                capability: CapabilityId::from("artifacts"),
                workspace_root: root.path().display().to_string(),
                name: "disk-pressure.txt".to_string(),
                content: "blocked".to_string(),
                limits: pressure_limits,
            })
            .await
            .is_err());
        assert!(!root
            .path()
            .join(".agendao/artifacts/artifacts/disk-pressure.txt")
            .exists());
    }

    #[tokio::test]
    async fn commits_successful_artifacts_and_removes_rolled_back_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let committed = root
            .path()
            .join(".agendao/artifacts/artifacts/committed.txt");
        let host = WorkspaceCapabilityHost::new(root.path().to_path_buf()).unwrap();
        host.store_artifact(&ArtifactRequest {
            capability: CapabilityId::from("artifacts"),
            workspace_root: root.path().display().to_string(),
            name: "committed.txt".to_string(),
            content: "keep".to_string(),
            limits: request(root.path()).limits,
        })
        .await
        .unwrap();
        host.finalize(RunDisposition::Commit).await.unwrap();
        drop(host);
        assert_eq!(fs::read_to_string(&committed).unwrap(), "keep");

        let rolled_back = root
            .path()
            .join(".agendao/artifacts/artifacts/rolled-back.txt");
        let host = WorkspaceCapabilityHost::new(root.path().to_path_buf()).unwrap();
        host.store_artifact(&ArtifactRequest {
            capability: CapabilityId::from("artifacts"),
            workspace_root: root.path().display().to_string(),
            name: "rolled-back.txt".to_string(),
            content: "discard".to_string(),
            limits: request(root.path()).limits,
        })
        .await
        .unwrap();
        host.finalize(RunDisposition::Rollback).await.unwrap();
        assert!(!rolled_back.exists());
    }
}
