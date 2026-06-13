//! Scheduler for state batching, effect execution, and reconciliation.

pub mod batch;
pub mod effect_queue;
pub mod reconciler;

pub use batch::{
    EqualityCheckFn, StateBatch, StateUpdate, StateUpdateKind, StateUpdaterFn, begin_batch,
    clear_state_batch, end_batch, is_batching, queue_update, with_state_batch,
    with_state_batch_mut,
};
pub use effect_queue::{
    EffectHookState, EffectQueue, clear_effect_queue, flush_effects, has_pending_effects,
    queue_cleanup, queue_effect, with_effect_queue,
};
