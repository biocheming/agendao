//! Terminal spinner for CLI progress indicators.
//!
//! Provides an animated spinner that runs in a background tokio task,
//! displaying a progress message with rotating frames.
//!
//! The spinner automatically pauses its output while other streams (stdout)
//! are writing, preventing visual trampling in the terminal.

use crate::cli_style::CliStyle;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// Spinner frames for terminal animation.
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Interval between spinner frame updates (milliseconds).
const SPINNER_INTERVAL_MS: u64 = 80;

/// Spinner state values.
const STATE_RUNNING: u8 = 0;
const STATE_PAUSED: u8 = 1;
const STATE_STOPPED: u8 = 2;

/// A terminal spinner that animates in a background task.
///
/// Supports pause/resume to prevent output trampling when other
/// content is being written to the terminal.
pub struct Spinner {
    state: Arc<AtomicU8>,
    message: Arc<Mutex<String>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

/// Lightweight, cloneable handle for pausing/resuming a [`Spinner`] from
/// other contexts (e.g. permission prompts, question callbacks).
#[derive(Clone)]
pub struct SpinnerGuard {
    state: Arc<AtomicU8>,
}

impl SpinnerGuard {
    /// Pause the spinner and clear its stderr line.
    pub fn pause(&self) {
        self.state.store(STATE_PAUSED, Ordering::Relaxed);
        clear_stderr_line();
    }

    /// Resume the spinner if it was paused.
    pub fn resume(&self) {
        let current = self.state.load(Ordering::Relaxed);
        if current == STATE_PAUSED {
            self.state.store(STATE_RUNNING, Ordering::Relaxed);
        }
    }

    /// Create a no-op guard that does nothing (for contexts without a spinner).
    pub fn noop() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(STATE_STOPPED)),
        }
    }
}

impl Spinner {
    /// Get a lightweight, cloneable guard for pausing/resuming this spinner.
    pub fn guard(&self) -> SpinnerGuard {
        SpinnerGuard {
            state: self.state.clone(),
        }
    }

    /// Start a new spinner with the given message.
    pub fn start(message: impl Into<String>, style: &CliStyle) -> Self {
        let message_str = message.into();
        let state = Arc::new(AtomicU8::new(STATE_RUNNING));
        let message = Arc::new(Mutex::new(message_str));
        let state_clone = state.clone();
        let message_clone = message.clone();
        let color = style.color;

        let handle = tokio::spawn(async move {
            let mut frame_idx = 0usize;
            let mut was_visible = false;

            loop {
                let current = state_clone.load(Ordering::Relaxed);
                if current == STATE_STOPPED {
                    break;
                }

                if current == STATE_PAUSED {
                    // When paused, clear any visible spinner and wait.
                    if was_visible {
                        clear_stderr_line();
                        was_visible = false;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(SPINNER_INTERVAL_MS))
                        .await;
                    continue;
                }

                let frame = SPINNER_FRAMES[frame_idx % SPINNER_FRAMES.len()];
                let msg = message_clone
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or_default();
                let line = if color {
                    format!("\r\x1b[2K\x1b[36m{}\x1b[0m \x1b[2m{}\x1b[0m", frame, msg)
                } else {
                    format!("\r\x1b[2K{} {}", frame, msg)
                };
                let _ = write!(io::stderr(), "{}", line);
                let _ = io::stderr().flush();
                was_visible = true;
                frame_idx += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(SPINNER_INTERVAL_MS)).await;
            }

            // Clear the spinner line on stop.
            if was_visible {
                clear_stderr_line();
            }
        });

        Self {
            state,
            message,
            handle: Some(handle),
        }
    }

    /// Pause the spinner animation and clear its line.
    ///
    /// Call this before writing streaming output to stdout to prevent
    /// the spinner's `\r` from trampling the output.
    pub fn pause(&self) {
        self.state.store(STATE_PAUSED, Ordering::Relaxed);
        clear_stderr_line();
    }

    /// Resume the spinner animation after pausing.
    pub fn resume(&self) {
        let current = self.state.load(Ordering::Relaxed);
        if current == STATE_PAUSED {
            self.state.store(STATE_RUNNING, Ordering::Relaxed);
        }
    }

    /// Check if the spinner is currently paused.
    pub fn is_paused(&self) -> bool {
        self.state.load(Ordering::Relaxed) == STATE_PAUSED
    }

    /// Stop the spinner and clear the line.
    pub async fn stop(mut self) {
        self.state.store(STATE_STOPPED, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            if let Err(error) = handle.await {
                tracing::debug!(
                    error = %error,
                    "Failed to join CLI spinner task during shutdown"
                );
            }
        }
    }

    /// Update the spinner message dynamically.
    pub fn update_message(&self, message: impl Into<String>) {
        if let Ok(mut guard) = self.message.lock() {
            *guard = message.into();
        }
    }
}

fn clear_stderr_line() {
    let _ = write!(io::stderr(), "\r\x1b[2K");
    let _ = io::stderr().flush();
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.state.store(STATE_STOPPED, Ordering::Relaxed);
        // Can't await in Drop, but at least signal stop.
    }
}

/// A simple progress bar for step-based progress.
pub struct ProgressBar {
    total: usize,
    current: usize,
    label: String,
    style_color: bool,
}

impl ProgressBar {
    pub fn new(total: usize, label: impl Into<String>, style: &CliStyle) -> Self {
        Self {
            total,
            current: 0,
            label: label.into(),
            style_color: style.color,
        }
    }

    /// Advance the progress bar by one step and redraw.
    pub fn tick(&mut self) {
        self.current = (self.current + 1).min(self.total);
        self.draw();
    }

    /// Set the current progress and redraw.
    pub fn set(&mut self, current: usize) {
        self.current = current.min(self.total);
        self.draw();
    }

    /// Clear the progress bar line.
    pub fn finish(&self) {
        let _ = write!(io::stderr(), "\r\x1b[2K");
        let _ = io::stderr().flush();
    }

    fn draw(&self) {
        let bar_width = 20;
        let filled = if self.total > 0 {
            (self.current * bar_width) / self.total
        } else {
            0
        };
        let empty = bar_width - filled;
        let bar: String = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
        let pct = if self.total > 0 {
            (self.current * 100) / self.total
        } else {
            0
        };

        let line = if self.style_color {
            format!(
                "\r\x1b[2K\x1b[36m{}\x1b[0m \x1b[2m{} {}/{}  {}%\x1b[0m",
                bar, self.label, self.current, self.total, pct
            )
        } else {
            format!(
                "\r\x1b[2K{} {} {}/{}  {}%",
                bar, self.label, self.current, self.total, pct
            )
        };
        let _ = write!(io::stderr(), "{}", line);
        let _ = io::stderr().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_frames_not_empty() {
        assert!(!SPINNER_FRAMES.is_empty());
    }

    #[test]
    fn progress_bar_calculates_percentage() {
        let style = CliStyle::plain();
        let mut bar = ProgressBar::new(10, "test", &style);
        bar.set(5);
        assert_eq!(bar.current, 5);
        assert_eq!(bar.total, 10);
    }

    #[test]
    fn progress_bar_clamps_overflow() {
        let style = CliStyle::plain();
        let mut bar = ProgressBar::new(10, "test", &style);
        bar.set(20);
        assert_eq!(bar.current, 10);
    }

    #[tokio::test]
    async fn spinner_can_start_and_stop() {
        let style = CliStyle::plain();
        let spinner = Spinner::start("testing...", &style);
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        spinner.stop().await;
        // If we get here without panic, the spinner lifecycle works
    }

    #[tokio::test]
    async fn spinner_pause_and_resume() {
        let style = CliStyle::plain();
        let spinner = Spinner::start("testing...", &style);
        spinner.pause();
        assert!(spinner.is_paused());
        spinner.resume();
        assert!(!spinner.is_paused());
        spinner.stop().await;
    }

    #[tokio::test]
    async fn spinner_update_message() {
        let style = CliStyle::plain();
        let spinner = Spinner::start("initial", &style);
        spinner.update_message("updated message");
        let msg = spinner.message.lock().unwrap().clone();
        assert_eq!(msg, "updated message");
        spinner.stop().await;
    }
}
