//! React-like hooks with proper semantics.
//!
//! This module provides hooks that implement React-like behavior:
//! - Effects run after commit, not during render
//! - State updates are batched
//! - Context providers have proper lifecycle

mod async_hooks;
mod context;
mod effect;
mod effect_event;
mod event;
mod form;
mod history;
mod id;
mod keyboard;
mod layout;
mod memo;
mod mouse;
mod reducer;
mod r#ref;
mod scroll;
mod state;
mod timing;

pub use async_hooks::{
    FutureHandle, FutureState, MutationHandle, MutationOptions, MutationState, MutationStatus,
    QueryOptions, QueryResult, QueryStatus, clear_query_cache, use_future, use_future_once,
    use_mutation, use_query,
};
pub use context::{try_use_context, use_context, use_context_provider};
pub use effect::{use_async_effect, use_async_effect_once, use_effect, use_effect_once};
pub use effect_event::{EffectEvent, use_effect_event};
pub use event::{peek_event, stop_propagation, use_event};
pub use form::{
    FieldRegistration, FormConfig, FormConfigBuilder, FormHandle, FormState, Validator,
    try_use_form_context, use_form, use_form_context, use_watch, use_watch_all, use_watch_multiple,
};
pub use history::{HistoryHandle, use_history};
pub use id::use_id;
pub use keyboard::{use_keyboard, use_keyboard_press, use_keyboard_shortcut};
pub use layout::{
    ComponentArea, FrameContext, FrameInfo, try_use_area, try_use_frame, use_area, use_frame,
    use_frame_info, use_media_query, use_on_resize, use_resize,
};
pub use memo::{use_callback, use_memo};
pub use mouse::{
    DragInfo, use_double_click, use_mouse, use_mouse_click, use_mouse_drag, use_mouse_hover,
    use_mouse_position,
};
pub use reducer::{Dispatch, use_reducer};
pub use r#ref::{Ref, use_ref};
pub use scroll::{ScrollHandle, use_scroll, use_scroll_keyboard};
pub use state::{StateSetter, use_state};
pub use timing::{IntervalHandle, TimeoutHandle, use_interval, use_timeout};
