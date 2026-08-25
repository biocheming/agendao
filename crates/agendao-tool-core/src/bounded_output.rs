//! Bounded, concurrent stdout/stderr collection for model-reachable tools.
//!
//! Readers intentionally keep draining after either the aggregate or a
//! per-stream limit is reached.  Stopping a pipe reader at the limit can make
//! a child block forever on its next write, turning an output limit into a
//! process-lifecycle leak.

use tokio::io::{AsyncRead, AsyncReadExt};

/// Maximum retained bytes for a complete tool result.  The limit applies to
/// the sum of retained stdout and stderr bytes.
pub const MAX_CAPTURED_OUTPUT_BYTES: usize = 50 * 1024;

/// Maximum retained bytes for either individual stream.  This prevents one
/// noisy stream from starving the other stream's diagnostic output.
pub const MAX_CAPTURED_STREAM_BYTES: usize = 50 * 1024;

const READ_CHUNK_BYTES: usize = 8 * 1024;

/// The finite output retained from two concurrently drained child pipes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BoundedOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

impl BoundedOutput {
    pub fn truncated(&self) -> bool {
        self.stdout_truncated || self.stderr_truncated
    }

    fn append_stdout(&mut self, bytes: &[u8]) {
        append_bounded(
            &mut self.stdout,
            &mut self.stdout_truncated,
            bytes,
            self.stderr.len(),
        );
    }

    fn append_stderr(&mut self, bytes: &[u8]) {
        append_bounded(
            &mut self.stderr,
            &mut self.stderr_truncated,
            bytes,
            self.stdout.len(),
        );
    }
}

fn append_bounded(target: &mut Vec<u8>, truncated: &mut bool, bytes: &[u8], other_len: usize) {
    let stream_remaining = MAX_CAPTURED_STREAM_BYTES.saturating_sub(target.len());
    let total_remaining = MAX_CAPTURED_OUTPUT_BYTES.saturating_sub(target.len() + other_len);
    let retain = bytes.len().min(stream_remaining).min(total_remaining);
    target.extend_from_slice(&bytes[..retain]);
    if retain != bytes.len() {
        *truncated = true;
    }
}

/// Drain stdout and stderr until *both* reach EOF, retaining only bounded
/// prefixes. Raw reads rather than `lines()` are deliberate: an unbroken
/// multi-megabyte line must never become an unbounded allocation.
pub async fn drain_piped_output<Stdout, Stderr>(
    mut stdout: Stdout,
    mut stderr: Stderr,
) -> std::io::Result<BoundedOutput>
where
    Stdout: AsyncRead + Unpin,
    Stderr: AsyncRead + Unpin,
{
    let mut output = BoundedOutput::default();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut stdout_buf = [0u8; READ_CHUNK_BYTES];
    let mut stderr_buf = [0u8; READ_CHUNK_BYTES];

    while stdout_open || stderr_open {
        tokio::select! {
            result = stdout.read(&mut stdout_buf), if stdout_open => {
                match result? {
                    0 => stdout_open = false,
                    count => output.append_stdout(&stdout_buf[..count]),
                }
            }
            result = stderr.read(&mut stderr_buf), if stderr_open => {
                match result? {
                    0 => stderr_open = false,
                    count => output.append_stderr(&stderr_buf[..count]),
                }
            }
        }
    }

    Ok(output)
}
