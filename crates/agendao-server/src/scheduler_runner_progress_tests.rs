use super::{should_emit_scheduler_progress, SCHEDULER_PROGRESS_EMIT_BYTES};

#[test]
fn scheduler_progress_emits_first_delta_then_coalesces_small_updates() {
    assert!(should_emit_scheduler_progress(0, 1));
    assert!(!should_emit_scheduler_progress(
        1,
        SCHEDULER_PROGRESS_EMIT_BYTES
    ));
    assert!(should_emit_scheduler_progress(
        1,
        SCHEDULER_PROGRESS_EMIT_BYTES + 1
    ));
}

#[test]
fn scheduler_progress_length_accounting_is_saturating() {
    assert!(!should_emit_scheduler_progress(512, 256));
}
