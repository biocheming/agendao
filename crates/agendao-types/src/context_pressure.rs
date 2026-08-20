//! Context pressure thresholds are a cross-frontend contract:
//! apps/agendao-web/scripts/generate-context-pressure.mjs reads these
//! constants at build time to generate the Web-side thresholds and labels.

pub const CONTEXT_PRESSURE_WARNING_PERCENT: u64 = 80;
pub const CONTEXT_PRESSURE_AUTO_COMPACT_SOON_PERCENT: u64 = 90;
pub const CONTEXT_PRESSURE_CRITICAL_PERCENT: u64 = 95;

pub const CONTEXT_PRESSURE_WARNING_LABEL: &str = "warning";
pub const CONTEXT_PRESSURE_AUTO_COMPACT_SOON_LABEL: &str = "auto-compact soon";
pub const CONTEXT_PRESSURE_CRITICAL_LABEL: &str = "compact now";

// Compile-time guard: thresholds must stay ordered for the generated
// Web-side `contextPressureLabel` cascading checks to make sense.
const _: () = {
    assert!(CONTEXT_PRESSURE_WARNING_PERCENT < CONTEXT_PRESSURE_AUTO_COMPACT_SOON_PERCENT);
    assert!(CONTEXT_PRESSURE_AUTO_COMPACT_SOON_PERCENT < CONTEXT_PRESSURE_CRITICAL_PERCENT);
};
