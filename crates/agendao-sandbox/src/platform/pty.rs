//! Pty allocation for interactive sandbox launches (unix).
//!
//! The launcher opens the pty pair itself and hands the *master* side
//! back to the host (reader/writer/resize), while the *slave* is
//! consumed by the backend's `spawn_pty`: it becomes the child's
//! stdio and controlling terminal. This keeps every spawn inside a
//! backend — a pty host can never bypass the plan, the seccomp fd
//! hand-off, or the event ladder by spawning the argv itself.

use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

/// Terminal size for a fresh pty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyDimensions {
    pub rows: u16,
    pub cols: u16,
}

/// The host side of an interactive sandbox execution: buffered reads,
/// line writes, and window resizing. Dropping it closes the master fd,
/// which hangs up the session's foreground process group.
pub struct PtyMaster {
    fd: File,
}

impl PtyMaster {
    /// A duplicate of the master fd for a dedicated reader. The pty
    /// master is a single open file description; duplicated handles see
    /// the same byte stream.
    pub fn try_clone_reader(&self) -> io::Result<File> {
        self.fd.try_clone()
    }

    /// A duplicate of the master fd for a dedicated writer.
    pub fn try_clone_writer(&self) -> io::Result<File> {
        self.fd.try_clone()
    }

    /// Resize the pty window (TIOCSWINSZ on the master).
    pub fn resize(&self, dims: PtyDimensions) -> io::Result<()> {
        let winsize = libc::winsize {
            ws_row: dims.rows,
            ws_col: dims.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: ioctl writes into the stack-local winsize; the fd is
        // owned by self and stays valid for the call.
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCSWINSZ, &winsize) };
        if rc == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// The child side, consumed by `SandboxBackend::spawn_pty`.
pub struct PtySlave {
    fd: File,
}

impl PtySlave {
    /// A duplicate of the slave fd, e.g. for the pre_exec
    /// setsid/TIOCSCTTY dance in a backend.
    pub fn try_clone(&self) -> io::Result<File> {
        self.fd.try_clone()
    }
}

impl AsRawFd for PtySlave {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Open a new pty pair at the given size.
pub fn openpty(dims: PtyDimensions) -> io::Result<(PtyMaster, PtySlave)> {
    let mut master_fd: RawFd = -1;
    let mut slave_fd: RawFd = -1;
    #[cfg(target_os = "macos")]
    let mut winsize = libc::winsize {
        ws_row: dims.rows,
        ws_col: dims.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    #[cfg(not(target_os = "macos"))]
    let winsize = libc::winsize {
        ws_row: dims.rows,
        ws_col: dims.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    #[cfg(target_os = "macos")]
    let winsize_ptr = &mut winsize;
    #[cfg(not(target_os = "macos"))]
    let winsize_ptr = &winsize;
    // SAFETY: openpty writes the two fds through the passed pointers;
    // both are immediately wrapped in owned File handles below.
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            winsize_ptr,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fresh descriptors from a successful openpty, immediately
    // owned. A panic between from_raw_fd calls would leak one fd, which
    // is acceptable for a launch-time failure path.
    Ok((
        PtyMaster {
            fd: unsafe { File::from_raw_fd(master_fd) },
        },
        PtySlave {
            fd: unsafe { File::from_raw_fd(slave_fd) },
        },
    ))
}
