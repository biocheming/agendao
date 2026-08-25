//! Non-Unix terminal boundary stubs.
//!
//! Windows and other non-Unix hosts must compile terminal consumers, but no
//! host PTY may be substituted for a contained backend.  These types exist
//! only to preserve the boundary signature; `start_pty` rejects the launch.

use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyDimensions {
    pub rows: u16,
    pub cols: u16,
}

pub struct PtyMaster;

impl PtyMaster {
    pub fn try_clone_reader(&self) -> io::Result<std::io::Cursor<Vec<u8>>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "PTY is unsupported on this platform",
        ))
    }

    pub fn try_clone_writer(&self) -> io::Result<std::io::Cursor<Vec<u8>>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "PTY is unsupported on this platform",
        ))
    }

    pub fn resize(&self, _dims: PtyDimensions) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "PTY is unsupported on this platform",
        ))
    }
}

pub struct PtySlave;
