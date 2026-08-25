//! macOS platform backend: Seatbelt profile enforcement.
//!
//! The SBPL profile construction is pure and compiles on every host so
//! its contract tests run everywhere (Phase 7 quality gate: CI runs at
//! least the pure contract tests); only the `sandbox-exec` execution
//! shell is `cfg(target_os = "macos")`.

pub mod seatbelt;
