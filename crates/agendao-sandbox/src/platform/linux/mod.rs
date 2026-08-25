//! Linux backend family. Bubblewrap provides the namespace/mount core;
//! seccomp adds a syscall-filter layer on top (defense in depth against
//! mount or namespace regressions, not the primary isolation).

pub mod bwrap;
pub mod probe;
pub mod seccomp;

pub use bwrap::{build_bwrap_args, bwrap_backend, BwrapBackend};
