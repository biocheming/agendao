//! # agendao-watcher (EXPERIMENTAL — parked)
//!
//! This crate is parked as of 2026-06-02. It has no active consumer in the
//! production path (TUI, CLI, server, web). The `WatcherConfig` schema entry
//! in `agendao-config` is dormant.
//!
//! It is retained in the workspace as experimental infrastructure for future
//! file-watching needs, but is **not guaranteed compatibility** and should
//! not be treated as a production-maintained crate until a concrete consumer
//! is integrated.
//!
//! See: `docs/agendao-33-crates-layered-cleanup-table-2026-06-02.md` § P0.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashSet;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// File change event types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileEvent {
    Add,
    Change,
    Unlink,
}

/// A file watcher event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherEvent {
    pub file: PathBuf,
    pub event: FileEvent,
}

/// File watcher errors
#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("Failed to create watcher: {0}")]
    CreateError(#[from] notify::Error),

    #[error("Failed to watch path: {0}")]
    WatchError(String),

    #[error("Path does not exist: {0}")]
    PathNotFound(PathBuf),

    #[error("Already watching: {0}")]
    AlreadyWatching(PathBuf),
}

/// Configuration for the file watcher
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// Paths to ignore (glob patterns)
    pub ignore_patterns: Vec<String>,
    /// Per-path minimum interval between emitted events (leading-edge
    /// throttle): events for a file that arrive within this window after the
    /// last emitted event for the same file are dropped.
    pub debounce_ms: u64,
    /// Whether to watch recursively
    pub recursive: bool,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            ignore_patterns: vec![
                "**/node_modules/**".to_string(),
                "**/.git/**".to_string(),
                "**/target/**".to_string(),
                "**/.DS_Store".to_string(),
                "**/*.swp".to_string(),
                "**/*.swo".to_string(),
                "**/*~".to_string(),
            ],
            debounce_ms: 100,
            recursive: true,
        }
    }
}

/// File system watcher
pub struct FileWatcher {
    watcher: RwLock<Option<RecommendedWatcher>>,
    watched_paths: DashSet<PathBuf>,
    ignore_patterns: Vec<glob::Pattern>,
    event_tx: broadcast::Sender<WatcherEvent>,
    debounce_ms: u64,
    recursive: bool,
    last_events: Arc<dashmap::DashMap<PathBuf, std::time::Instant>>,
}

impl FileWatcher {
    /// Create a new file watcher
    pub fn new(config: WatcherConfig) -> Result<Self, WatcherError> {
        let (event_tx, _) = broadcast::channel(1024);

        let ignore_patterns = config
            .ignore_patterns
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();

        Ok(Self {
            watcher: RwLock::new(None),
            watched_paths: DashSet::new(),
            ignore_patterns,
            event_tx,
            debounce_ms: config.debounce_ms,
            recursive: config.recursive,
            last_events: Arc::new(dashmap::DashMap::new()),
        })
    }

    /// Subscribe to file watcher events
    pub fn subscribe(&self) -> broadcast::Receiver<WatcherEvent> {
        self.event_tx.subscribe()
    }

    fn build_watcher(&self) -> Result<RecommendedWatcher, WatcherError> {
        let event_tx = self.event_tx.clone();
        let ignore_patterns = self.ignore_patterns.clone();
        let last_events = Arc::clone(&self.last_events);
        let debounce = Duration::from_millis(self.debounce_ms);

        let watcher = notify::recommended_watcher(
            move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    let file_event = match event.kind {
                        EventKind::Create(_) => FileEvent::Add,
                        EventKind::Modify(_) => FileEvent::Change,
                        EventKind::Remove(_) => FileEvent::Unlink,
                        _ => return,
                    };

                    for path in &event.paths {
                        let path_str = path.to_string_lossy();
                        let should_ignore = ignore_patterns.iter().any(|p| p.matches(&path_str));

                        if should_ignore {
                            debug!(path = ?path, "Ignoring file event");
                            continue;
                        }

                        // Leading-edge per-path throttle: drop events that arrive
                        // within the debounce window after the last emitted event.
                        if debounce > Duration::ZERO {
                            let now = std::time::Instant::now();
                            if let Some(last) = last_events.get(path) {
                                if now.duration_since(*last) < debounce {
                                    continue;
                                }
                            }
                            last_events.insert(path.clone(), now);
                        }

                        let watcher_event = WatcherEvent {
                            file: path.clone(),
                            event: file_event,
                        };

                        if let Err(e) = event_tx.send(watcher_event) {
                            warn!(error = %e, "Failed to send watcher event");
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Watcher error");
                }
            },
        )?;

        Ok(watcher)
    }

    /// Start watching a directory
    pub fn watch(&self, path: &Path) -> Result<(), WatcherError> {
        if !path.exists() {
            return Err(WatcherError::PathNotFound(path.to_path_buf()));
        }

        if self.watched_paths.contains(path) {
            return Err(WatcherError::AlreadyWatching(path.to_path_buf()));
        }

        let mode = if self.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        // Reuse the existing watcher when present: replacing it would drop the
        // OS watches for every previously registered path while `watched_paths`
        // still lists them.
        let mut guard = self.watcher.write();
        if guard.is_none() {
            *guard = Some(self.build_watcher()?);
        }
        guard.as_mut().expect("watcher just built").watch(path, mode)?;

        info!(path = ?path, "Started watching directory");

        self.watched_paths.insert(path.to_path_buf());

        Ok(())
    }

    pub fn unwatch(&self, path: &Path) -> Result<(), WatcherError> {
        self.watched_paths.remove(path);
        self.last_events.remove(path);

        let mut guard = self.watcher.write();
        if let Some(watcher) = guard.as_mut() {
            // Best-effort: the path may not be registered with the backend.
            let _ = watcher.unwatch(path);
        }

        if self.watched_paths.is_empty() {
            *guard = None;
            self.last_events.clear();
            info!("Stopped all watchers");
        }

        Ok(())
    }

    pub fn stop(&self) {
        self.watched_paths.clear();
        *self.watcher.write() = None;
        self.last_events.clear();
        info!("Stopped all file watchers");
    }

    pub fn watched_paths(&self) -> Vec<PathBuf> {
        self.watched_paths.iter().map(|p| p.clone()).collect()
    }

    pub fn is_watching(&self, path: &Path) -> bool {
        self.watched_paths.contains(path)
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

static FILE_WATCHER: std::sync::OnceLock<Arc<FileWatcher>> = std::sync::OnceLock::new();

pub fn get_watcher() -> Arc<FileWatcher> {
    FILE_WATCHER
        .get_or_init(|| {
            Arc::new(
                FileWatcher::new(WatcherConfig::default()).expect("Failed to create file watcher"),
            )
        })
        .clone()
}

pub fn init_watcher(config: WatcherConfig) -> Arc<FileWatcher> {
    let watcher = Arc::new(FileWatcher::new(config).expect("Failed to create file watcher"));
    FILE_WATCHER.get_or_init(|| watcher.clone()).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::time::sleep;

    #[test]
    fn test_watcher_config_default() {
        let config = WatcherConfig::default();
        assert!(!config.ignore_patterns.is_empty());
        assert!(config.recursive);
    }

    #[test]
    fn test_file_event_serde() {
        let event = FileEvent::Add;
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, "\"add\"");

        let parsed: FileEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, FileEvent::Add);
    }

    #[tokio::test]
    async fn test_watch_directory() {
        let temp_dir = TempDir::new().unwrap();
        let watcher = FileWatcher::new(WatcherConfig::default()).unwrap();

        let mut rx = watcher.subscribe();

        watcher.watch(temp_dir.path()).unwrap();

        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "hello").unwrap();

        sleep(Duration::from_millis(200)).await;

        match rx.try_recv() {
            Ok(event) => {
                assert!(event.file.ends_with("test.txt"));
                assert_eq!(event.event, FileEvent::Add);
            }
            Err(broadcast::error::TryRecvError::Empty) => {}
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    #[test]
    fn test_watch_nonexistent_path() {
        let watcher = FileWatcher::new(WatcherConfig::default()).unwrap();
        let result = watcher.watch(Path::new("/nonexistent/path"));
        assert!(matches!(result, Err(WatcherError::PathNotFound(_))));
    }

    #[test]
    fn test_watch_same_path_twice() {
        let temp_dir = TempDir::new().unwrap();
        let watcher = FileWatcher::new(WatcherConfig::default()).unwrap();

        watcher.watch(temp_dir.path()).unwrap();
        let result = watcher.watch(temp_dir.path());
        assert!(matches!(result, Err(WatcherError::AlreadyWatching(_))));
    }

    #[test]
    fn test_watched_paths() {
        let temp_dir = TempDir::new().unwrap();
        let watcher = FileWatcher::new(WatcherConfig::default()).unwrap();

        assert!(watcher.watched_paths().is_empty());
        assert!(!watcher.is_watching(temp_dir.path()));

        watcher.watch(temp_dir.path()).unwrap();

        assert_eq!(watcher.watched_paths().len(), 1);
        assert!(watcher.is_watching(temp_dir.path()));
    }

    #[tokio::test]
    async fn test_second_watch_does_not_replace_first() {
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();
        let watcher = FileWatcher::new(WatcherConfig {
            debounce_ms: 0,
            ..WatcherConfig::default()
        })
        .unwrap();

        let mut rx = watcher.subscribe();
        watcher.watch(dir_a.path()).unwrap();
        watcher.watch(dir_b.path()).unwrap();
        assert_eq!(watcher.watched_paths().len(), 2);

        // The first path must still produce events after the second watch.
        let file_a = dir_a.path().join("a.txt");
        fs::write(&file_a, "hello").unwrap();

        sleep(Duration::from_millis(300)).await;

        let mut saw_a = false;
        while let Ok(event) = rx.try_recv() {
            if event.file.ends_with("a.txt") {
                saw_a = true;
            }
        }
        assert!(saw_a, "events from the first watched path were lost");
    }
}
