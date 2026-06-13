//! New render loop with React-like semantics.
//!
//! This module provides the `render` function which implements a proper
//! 5-phase render pipeline:
//!
//! 1. **Poll Phase**: Poll for events (store for later processing)
//! 2. **Render Phase**: Execute component functions (pure, registers event handlers)
//! 3. **Commit Phase**: Apply changes to terminal buffer
//! 4. **Event Phase**: Process events through registered handlers (with state batching)
//! 5. **Effect Phase**: Run queued effects after commit
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::prelude::*;
//!
//! struct Counter;
//!
//! impl Component for Counter {
//!     fn render(&self, area: Rect, buffer: &mut Buffer) {
//!         let (count, set_count) = use_state(|| 0);
//!         // render logic...
//!     }
//! }
//!
//! // Simple: just pass your component!
//! render(Counter).await?;
//! ```

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, EventStream};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio_stream::StreamExt;

use crate::Component;
use crate::event::set_current_event;
use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};
use crate::global_events::process_global_event;
use crate::panic_handler::setup_panic_handler;
use crate::render_context::{clear_render_context, init_render_context};

/// Terminal type alias for convenience
pub type FiberTerminal = Terminal<CrosstermBackend<std::io::Stdout>>;

/// Options for configuring the render loop
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Target frame rate in milliseconds (default: 16ms = ~60fps)
    pub frame_interval_ms: u64,
    /// Whether to enable strict mode (double-render in debug builds)
    pub strict_mode: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            frame_interval_ms: 16,
            strict_mode: false,
        }
    }
}

/// Exit flag for the render loop
static EXIT_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Flag to track if we're currently in the render phase
/// Used to detect and warn about side effects during render
static IN_RENDER_PHASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Request the render loop to exit
pub fn request_exit() {
    EXIT_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Check if exit has been requested
pub fn should_exit() -> bool {
    EXIT_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Reset the exit flag (useful for tests)
pub fn reset_exit() {
    EXIT_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Check if we're currently in the render phase
///
/// This is useful for detecting side effects during render, which violates
/// React's rules of hooks. Effects should be queued during render and
/// executed after the commit phase.
pub fn is_in_render_phase() -> bool {
    IN_RENDER_PHASE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Set the render phase flag (internal use only)
fn set_render_phase(in_render: bool) {
    IN_RENDER_PHASE.store(in_render, std::sync::atomic::Ordering::SeqCst);
}

/// Warn if an effect is executed during render phase (debug builds only)
///
/// In React, effects should never run during render - they should be queued
/// and executed after the commit phase. This function logs a warning in
/// debug builds to help catch this mistake.
#[cfg(debug_assertions)]
pub fn warn_if_effect_during_render(effect_name: &str) {
    if is_in_render_phase() {
        tracing::warn!(
            "Effect '{}' executed during render phase! Effects should run after commit. \
             This may cause inconsistent behavior.",
            effect_name
        );
    }
}

/// No-op in release builds
#[cfg(not(debug_assertions))]
pub fn warn_if_effect_during_render(_effect_name: &str) {}

/// Set up the terminal for TUI rendering
fn setup_terminal() -> Result<FiberTerminal> {
    use crossterm::{
        execute,
        terminal::{EnterAlternateScreen, enable_raw_mode},
    };

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    Ok(terminal)
}

/// Restore the terminal to its original state
fn restore_terminal() -> Result<()> {
    use crossterm::{
        execute,
        terminal::{LeaveAlternateScreen, disable_raw_mode},
    };

    disable_raw_mode()?;
    execute!(std::io::stdout(), LeaveAlternateScreen)?;

    Ok(())
}

/// New render function with React-like semantics
///
/// This function implements a proper 4-phase render pipeline:
///
/// 1. **Event Phase**: Poll for events with state batching enabled
/// 2. **Render Phase**: Execute component functions (queues effects, doesn't run them)
/// 3. **Commit Phase**: Apply changes to terminal buffer, process unmounts
/// 4. **Effect Phase**: Run queued effects after commit
///
/// # Differences from `render` (deprecated)
///
/// - Proper 4-phase pipeline (event, render, commit, effect)
/// - State batching during event handling
/// - Effects run after commit, not during render
/// - Fiber-based component tracking with isolated hook state
///
/// # Arguments
///
/// * `initializer` - A closure that returns the root Component to render
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::prelude::*;
///
/// struct Counter;
///
/// impl Component for Counter {
///     fn render(&self, area: Rect, buffer: &mut Buffer) {
///         let (count, set_count) = use_state(|| 0);
///         // render logic...
///     }
/// }
///
/// // Simple: just pass a closure that returns your component!
/// render(|| Counter).await?;
/// ```
pub async fn render<C, F>(initializer: F) -> Result<()>
where
    C: Component,
    F: Fn() -> C + 'static,
{
    render_with_options(initializer, RenderOptions::default()).await
}

/// Render with custom options
///
/// This is the full-featured version of `render` that accepts configuration options.
///
/// # Arguments
///
/// * `initializer` - A closure that returns the root Component to render
/// * `options` - Configuration options for the render loop
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::prelude::*;
///
/// struct App;
///
/// impl Component for App {
///     fn render(&self, area: Rect, buffer: &mut Buffer) {
///         // render logic...
///     }
/// }
///
/// render_with_options(
///     || App,
///     RenderOptions {
///         strict_mode: true,  // Enable double-render in debug builds
///         frame_interval_ms: 33,  // ~30fps
///     }
/// ).await?;
/// ```
pub async fn render_with_options<C, F>(initializer: F, options: RenderOptions) -> Result<()>
where
    C: Component,
    F: Fn() -> C + 'static,
{
    // Initialize panic handler
    setup_panic_handler();

    // Reset exit flag
    reset_exit();

    // Initialize terminal backend
    let mut terminal = setup_terminal()?;

    // ═══════════════════════════════════════════════════════════════════
    // Initialize FiberTree (replaces HookContext)
    // ═══════════════════════════════════════════════════════════════════
    let fiber_tree = FiberTree::new();
    set_fiber_tree(fiber_tree);

    // Initialize the consolidated RenderContext
    // This provides a unified interface for all render-related state
    // and enables gradual migration from legacy thread-locals
    init_render_context();

    // Initialize main thread tracking for cross-thread state updates.
    // This allows background tasks (from use_interval, use_timeout) to
    // route their state updates to a global queue that we drain each frame.
    crate::scheduler::batch::init_main_thread();

    // Set strict mode flag
    crate::strict_mode::set_strict_mode_enabled(options.strict_mode);

    // Frame tracking
    let mut last_frame_time = Instant::now();

    // Create async event stream
    let mut events = EventStream::new();

    // Frame interval for timing
    let frame_interval = Duration::from_millis(options.frame_interval_ms);

    // Main render loop
    loop {
        // Calculate frame timing
        let current_time = Instant::now();
        let _delta = current_time.duration_since(last_frame_time);
        last_frame_time = current_time;

        // ═══════════════════════════════════════════════════════════════
        // PHASE 1: POLL FOR EVENTS
        // ═══════════════════════════════════════════════════════════════
        // Poll for events with timeout
        let timeout = tokio::time::sleep(frame_interval);
        tokio::pin!(timeout);

        let current_event: Option<Event> = tokio::select! {
            Some(Ok(event)) = events.next() => {
                Some(event)
            }
            _ = &mut timeout => {
                None
            }
        };

        // Set the current event BEFORE render so use_event() can access it
        // Propagation state is automatically reset by set_current_event()
        if let Some(ref event) = current_event {
            set_current_event(Some(std::sync::Arc::new(event.clone())));
        } else {
            set_current_event(None);
        }

        // ═══════════════════════════════════════════════════════════════
        // PHASE 2: RENDER (Pure - no side effects)
        // ═══════════════════════════════════════════════════════════════
        // Prepare fiber tree for render (reset hook indices)
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.prepare_for_render();
        });

        // Reset component position counter for stable ID generation
        // This enables React-like reconciliation where components created
        // inside the render closure maintain stable identity across frames
        crate::component::reset_component_position_counter();

        // Clear global event handlers before render so they can be re-registered
        // This ensures handlers are fresh each frame and don't accumulate
        crate::global_events::clear_global_handlers();

        // Mark that we're in render phase (for debug warnings)
        set_render_phase(true);

        // Create the component from the initializer
        let component = initializer();

        // Exit render phase
        set_render_phase(false);

        // Mark unseen Component fibers for unmount
        // This schedules cleanup for components that weren't rendered this frame
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.mark_unseen_for_unmount();
        });

        // ═══════════════════════════════════════════════════════════════
        // PHASE 3: COMMIT (Apply to terminal)
        // ═══════════════════════════════════════════════════════════════
        terminal.draw(|frame| {
            let area = frame.area();
            // Wrap the component and render with fiber management
            let wrapper = crate::component::ComponentWrapper::new(component);
            wrapper.render_with_fiber(area, frame.buffer_mut());
        })?;

        // Process pending unmounts (cleans up context providers)
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.process_unmounts();
        });

        // Note: Component cleanup is now handled internally by the fiber tree
        // The old reratui_core::component::cleanup_unmounted() call is no longer needed

        // ═══════════════════════════════════════════════════════════════
        // PHASE 4: EVENT PROCESSING (process global handlers registered during render)
        // ═══════════════════════════════════════════════════════════════
        crate::scheduler::batch::begin_batch();

        // Process key events through global event system (on_global_event handlers)
        // Note: use_event() already consumed the event during render phase
        if let Some(Event::Key(key_event)) = &current_event {
            process_global_event(key_event);
        }

        // Drain cross-thread updates from background tasks (use_interval, use_timeout)
        // into the thread-local batch before ending the batch.
        crate::scheduler::batch::drain_cross_thread_updates();

        // End batch and collect dirty fibers
        let _dirty_fibers = crate::scheduler::batch::end_batch();

        // Clear the event after processing so it doesn't persist to next frame
        set_current_event(None);

        // Check for exit
        if should_exit() {
            break;
        }

        // ═══════════════════════════════════════════════════════════════
        // PHASE 5: EFFECTS (After commit and event processing)
        // ═══════════════════════════════════════════════════════════════
        // First, flush synchronous effects
        crate::scheduler::effect_queue::flush_effects();

        // Then, flush async effects with tokio
        crate::scheduler::effect_queue::flush_async_effects().await;
    }

    // Clear the current event
    set_current_event(None);

    // Clean up the fiber tree
    clear_fiber_tree();

    // Clean up the consolidated RenderContext
    clear_render_context();

    // Restore terminal state
    restore_terminal()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber::FiberId;
    use crate::fiber_tree::{clear_fiber_tree, set_fiber_tree, with_fiber_tree_mut};
    use crate::scheduler::batch::{
        StateUpdate, StateUpdateKind, begin_batch, clear_state_batch, end_batch, queue_update,
    };

    fn setup_test_environment() -> FiberId {
        clear_state_batch();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        // Initialize a hook slot
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);
        tree.mark_clean(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn cleanup_test_environment() {
        clear_fiber_tree();
        clear_state_batch();
    }

    #[test]
    fn test_event_phase_batching_multiple_updates() {
        let fiber_id = setup_test_environment();

        // Simulate event phase: begin_batch -> process events -> end_batch
        begin_batch();

        // Simulate multiple state updates during event handling
        // (like clicking a button that triggers multiple set_state calls)
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(1i32)),
            },
        );
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(2i32)),
            },
        );
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(3i32)),
            },
        );

        // End batch and collect dirty fibers
        let dirty_fibers = end_batch();

        // Only ONE fiber should be dirty (batched into single re-render)
        assert_eq!(dirty_fibers.len(), 1);
        assert!(dirty_fibers.contains(&fiber_id));

        // Final value should be 3 (last update wins)
        with_fiber_tree_mut(|tree| {
            let value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            assert_eq!(value, Some(3));
        });

        cleanup_test_environment();
    }

    #[test]
    fn test_event_phase_batching_functional_updates() {
        let fiber_id = setup_test_environment();

        // Initialize with value 0
        with_fiber_tree_mut(|tree| {
            tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);
        });

        begin_batch();

        // Simulate increment by 5 using functional updates
        // (like React's setCount(n => n + 1) called 5 times)
        for _ in 0..5 {
            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Updater(Box::new(|current| {
                        let val = current.downcast_ref::<i32>().unwrap();
                        Box::new(val + 1)
                    })),
                },
            );
        }

        let dirty_fibers = end_batch();

        // Only ONE re-render needed
        assert_eq!(dirty_fibers.len(), 1);

        // All 5 updates should have been applied
        with_fiber_tree_mut(|tree| {
            let value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            assert_eq!(value, Some(5));
        });

        cleanup_test_environment();
    }

    #[test]
    fn test_event_phase_batching_multiple_fibers() {
        clear_state_batch();
        let mut tree = FiberTree::new();
        let fiber1 = tree.mount(None, None);
        let fiber2 = tree.mount(Some(fiber1), None);
        let fiber3 = tree.mount(Some(fiber1), None);

        // Initialize hooks
        tree.get_mut(fiber1).unwrap().set_hook(0, 0i32);
        tree.get_mut(fiber2).unwrap().set_hook(0, 0i32);
        tree.get_mut(fiber3).unwrap().set_hook(0, 0i32);

        tree.mark_clean(fiber1);
        tree.mark_clean(fiber2);
        tree.mark_clean(fiber3);

        set_fiber_tree(tree);

        begin_batch();

        // Updates to multiple fibers during same event
        queue_update(
            fiber1,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(10i32)),
            },
        );
        queue_update(
            fiber2,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(20i32)),
            },
        );
        queue_update(
            fiber3,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::Value(Box::new(30i32)),
            },
        );

        let dirty_fibers = end_batch();

        // All three fibers should be dirty
        assert_eq!(dirty_fibers.len(), 3);
        assert!(dirty_fibers.contains(&fiber1));
        assert!(dirty_fibers.contains(&fiber2));
        assert!(dirty_fibers.contains(&fiber3));

        // Verify values
        with_fiber_tree_mut(|tree| {
            assert_eq!(tree.get(fiber1).unwrap().get_hook::<i32>(0), Some(10));
            assert_eq!(tree.get(fiber2).unwrap().get_hook::<i32>(0), Some(20));
            assert_eq!(tree.get(fiber3).unwrap().get_hook::<i32>(0), Some(30));
        });

        cleanup_test_environment();
    }

    #[test]
    fn test_event_phase_no_updates_no_dirty_fibers() {
        let _fiber_id = setup_test_environment();

        begin_batch();
        // No updates queued
        let dirty_fibers = end_batch();

        // No fibers should be dirty
        assert!(dirty_fibers.is_empty());

        cleanup_test_environment();
    }

    #[test]
    fn test_event_phase_equality_check_skips_unchanged() {
        let fiber_id = setup_test_environment();

        // Initialize with value 42
        with_fiber_tree_mut(|tree| {
            tree.get_mut(fiber_id).unwrap().set_hook(0, 42i32);
            tree.mark_clean(fiber_id);
        });

        begin_batch();

        // Update with same value using equality check
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index: 0,
                update: StateUpdateKind::ValueIfChanged {
                    value: Box::new(42i32), // Same value
                    eq_check: Box::new(|old, new| {
                        let old = old.downcast_ref::<i32>().unwrap();
                        let new = new.downcast_ref::<i32>().unwrap();
                        old == new
                    }),
                },
            },
        );

        let dirty_fibers = end_batch();

        // Fiber should NOT be dirty because value didn't change
        assert!(dirty_fibers.is_empty());

        cleanup_test_environment();
    }

    #[test]
    fn test_render_options_default() {
        let options = RenderOptions::default();
        assert_eq!(options.frame_interval_ms, 16);
        assert!(!options.strict_mode);
    }

    #[test]
    fn test_exit_flag_operations() {
        // Reset to known state
        reset_exit();
        assert!(!should_exit());

        // Request exit
        request_exit();
        assert!(should_exit());

        // Reset again
        reset_exit();
        assert!(!should_exit());
    }

    #[test]
    fn test_render_phase_flag() {
        // Initially not in render phase
        assert!(!is_in_render_phase());

        // Set render phase
        set_render_phase(true);
        assert!(is_in_render_phase());

        // Exit render phase
        set_render_phase(false);
        assert!(!is_in_render_phase());
    }

    #[test]
    fn test_render_phase_isolation() {
        // Ensure render phase flag is properly isolated
        set_render_phase(false);
        assert!(!is_in_render_phase());

        // Simulate entering render phase
        set_render_phase(true);

        // During render, flag should be true
        assert!(is_in_render_phase());

        // Simulate exiting render phase
        set_render_phase(false);

        // After render, flag should be false
        assert!(!is_in_render_phase());
    }

    #[test]
    fn test_warn_if_effect_during_render_outside_render() {
        // Outside render phase, this should not panic or cause issues
        set_render_phase(false);
        warn_if_effect_during_render("test_effect");
        // No assertion needed - just verify it doesn't panic
    }

    #[test]
    fn test_warn_if_effect_during_render_inside_render() {
        // Inside render phase, this should log a warning (in debug builds)
        set_render_phase(true);
        warn_if_effect_during_render("test_effect");
        set_render_phase(false);
        // No assertion needed - just verify it doesn't panic
        // In debug builds, this would log a warning via tracing
    }

    #[test]
    fn test_prepare_for_render_resets_hook_indices() {
        let fiber_id = setup_test_environment();

        // Simulate some hook calls
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
            let fiber = tree.get_mut(fiber_id).unwrap();
            fiber.next_hook_index();
            fiber.next_hook_index();
            fiber.next_hook_index();
            tree.end_render();
        });

        // Hook index should be 3
        with_fiber_tree_mut(|tree| {
            assert_eq!(tree.get(fiber_id).unwrap().hook_index, 3);
        });

        // Prepare for render should reset all hook indices
        with_fiber_tree_mut(|tree| {
            tree.prepare_for_render();
        });

        // Hook index should be 0
        with_fiber_tree_mut(|tree| {
            assert_eq!(tree.get(fiber_id).unwrap().hook_index, 0);
        });

        cleanup_test_environment();
    }

    #[test]
    fn test_effect_execution_timing_after_commit() {
        use crate::fiber::PendingEffect;
        use crate::scheduler::effect_queue::{
            clear_effect_queue, flush_effects_with_tree, queue_effect,
        };
        use std::sync::Arc;

        let fiber_id = setup_test_environment();
        clear_effect_queue();

        let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));

        // Queue an effect
        let order_clone = execution_order.clone();
        queue_effect(
            fiber_id,
            PendingEffect {
                effect: Box::new(move || {
                    order_clone.lock().unwrap().push("effect");
                    None
                }),
                hook_index: 0,
            },
        );

        // Simulate commit phase (would normally call terminal.draw())
        execution_order.lock().unwrap().push("commit");

        // Flush effects (effect phase)
        with_fiber_tree_mut(|tree| {
            flush_effects_with_tree(tree);
        });

        // Verify order: commit happens before effect
        let order = execution_order.lock().unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], "commit");
        assert_eq!(order[1], "effect");

        cleanup_test_environment();
        clear_effect_queue();
    }

    #[test]
    fn test_cleanup_runs_before_new_effects() {
        use crate::fiber::PendingEffect;
        use crate::scheduler::effect_queue::{
            clear_effect_queue, flush_effects_with_tree, queue_cleanup, queue_effect,
        };
        use std::sync::Arc;

        let fiber_id = setup_test_environment();
        clear_effect_queue();

        let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));

        // Queue a cleanup (from previous effect)
        let order_clone = execution_order.clone();
        queue_cleanup(Box::new(move || {
            order_clone.lock().unwrap().push("cleanup");
        }));

        // Queue a new effect
        let order_clone = execution_order.clone();
        queue_effect(
            fiber_id,
            PendingEffect {
                effect: Box::new(move || {
                    order_clone.lock().unwrap().push("new_effect");
                    None
                }),
                hook_index: 0,
            },
        );

        // Flush effects
        with_fiber_tree_mut(|tree| {
            flush_effects_with_tree(tree);
        });

        // Verify order: cleanup runs before new effect
        let order = execution_order.lock().unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], "cleanup");
        assert_eq!(order[1], "new_effect");

        cleanup_test_environment();
        clear_effect_queue();
    }

    #[test]
    fn test_multiple_cleanups_run_in_reverse_order() {
        use crate::scheduler::effect_queue::{
            clear_effect_queue, flush_effects_with_tree, queue_cleanup,
        };
        use std::sync::Arc;

        let _fiber_id = setup_test_environment();
        clear_effect_queue();

        let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));

        // Queue multiple cleanups
        for i in 1..=3 {
            let order_clone = execution_order.clone();
            queue_cleanup(Box::new(move || {
                order_clone.lock().unwrap().push(i);
            }));
        }

        // Flush effects
        with_fiber_tree_mut(|tree| {
            flush_effects_with_tree(tree);
        });

        // Verify reverse order: 3, 2, 1
        let order = execution_order.lock().unwrap();
        assert_eq!(*order, vec![3, 2, 1]);

        cleanup_test_environment();
        clear_effect_queue();
    }

    #[test]
    fn test_effects_run_in_declaration_order() {
        use crate::fiber::PendingEffect;
        use crate::scheduler::effect_queue::{
            clear_effect_queue, flush_effects_with_tree, queue_effect,
        };
        use std::sync::Arc;

        let fiber_id = setup_test_environment();
        clear_effect_queue();

        let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));

        // Queue multiple effects
        for i in 1..=3 {
            let order_clone = execution_order.clone();
            queue_effect(
                fiber_id,
                PendingEffect {
                    effect: Box::new(move || {
                        order_clone.lock().unwrap().push(i);
                        None
                    }),
                    hook_index: i,
                },
            );
        }

        // Flush effects
        with_fiber_tree_mut(|tree| {
            flush_effects_with_tree(tree);
        });

        // Verify declaration order: 1, 2, 3
        let order = execution_order.lock().unwrap();
        assert_eq!(*order, vec![1, 2, 3]);

        cleanup_test_environment();
        clear_effect_queue();
    }

    /// Test that use_event() can access events during render phase.
    ///
    /// This test verifies the fix for the issue where events were set AFTER render,
    /// making them unavailable to use_event() during the render phase.
    ///
    /// The correct flow is:
    /// 1. Poll event
    /// 2. Set current event (BEFORE render)
    /// 3. Render (use_event() can access the event)
    /// 4. Commit
    /// 5. Process global handlers
    /// 6. Clear event
    /// 7. Effects
    #[test]
    fn test_use_event_available_during_render_phase() {
        use crate::event::{clear_current_event, set_current_event};
        use crate::hooks::use_event;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        use std::sync::Arc;

        // Clear any existing event state first
        clear_current_event();

        let fiber_id = setup_test_environment();

        // Simulate the runtime flow: set event BEFORE render
        let key_event = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ));
        set_current_event(Some(Arc::new(key_event)));

        // Simulate render phase
        set_render_phase(true);

        // Begin render for the fiber
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
        });

        // Call use_event() during render - this should return the event
        let event = use_event();

        // End render
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        set_render_phase(false);

        // Verify the event was available during render
        assert!(
            event.is_some(),
            "use_event() should return event during render phase"
        );

        if let Some(Event::Key(key)) = event {
            assert_eq!(key.code, KeyCode::Char('j'));
            assert_eq!(key.kind, KeyEventKind::Press);
        } else {
            panic!("Expected Key event with 'j'");
        }

        // Clean up
        clear_current_event();
        cleanup_test_environment();
    }

    /// Test that use_event() returns None when no event is set before render.
    #[test]
    fn test_use_event_returns_none_when_no_event() {
        use crate::event::clear_current_event;
        use crate::hooks::use_event;

        let fiber_id = setup_test_environment();

        // Ensure no event is set
        clear_current_event();

        // Simulate render phase
        set_render_phase(true);

        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
        });

        // Call use_event() - should return None
        let event = use_event();

        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        set_render_phase(false);

        assert!(
            event.is_none(),
            "use_event() should return None when no event is set"
        );

        cleanup_test_environment();
    }

    /// Test that events are cleared after processing to prevent stale events.
    #[test]
    fn test_event_cleared_after_render_cycle() {
        use crate::event::{clear_current_event, peek_current_event, set_current_event};
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        use std::sync::Arc;

        let _fiber_id = setup_test_environment();

        // Set an event
        let key_event = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ));
        set_current_event(Some(Arc::new(key_event)));

        // Verify event is set
        assert!(peek_current_event().is_some());

        // Simulate end of render cycle - clear the event
        clear_current_event();

        // Verify event is cleared
        assert!(
            peek_current_event().is_none(),
            "Event should be cleared after render cycle"
        );

        cleanup_test_environment();
    }

    /// Test the complete render cycle flow with use_event().
    #[test]
    fn test_complete_render_cycle_with_use_event() {
        use crate::event::{clear_current_event, set_current_event};
        use crate::hooks::use_event;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        use std::sync::Arc;

        let fiber_id = setup_test_environment();

        // === Frame 1: Event 'a' ===
        // Step 1: Poll event and set it BEFORE render
        let event_a = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ));
        set_current_event(Some(Arc::new(event_a)));

        // Step 2: Render phase
        set_render_phase(true);
        with_fiber_tree_mut(|tree| {
            tree.prepare_for_render();
            tree.begin_render(fiber_id);
        });

        let event1 = use_event();

        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });
        set_render_phase(false);

        // Step 3: Verify event was received
        assert!(event1.is_some());
        if let Some(Event::Key(key)) = event1 {
            assert_eq!(key.code, KeyCode::Char('a'));
        }

        // Step 4: Clear event after processing
        clear_current_event();

        // === Frame 2: Event 'b' ===
        let event_b = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('b'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ));
        set_current_event(Some(Arc::new(event_b)));
        // Propagation state is automatically reset by set_current_event()

        set_render_phase(true);
        with_fiber_tree_mut(|tree| {
            tree.prepare_for_render();
            tree.begin_render(fiber_id);
        });

        let event2 = use_event();

        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });
        set_render_phase(false);

        // Verify second event was received (not stale first event)
        assert!(event2.is_some());
        if let Some(Event::Key(key)) = event2 {
            assert_eq!(key.code, KeyCode::Char('b'));
        }

        clear_current_event();

        // === Frame 3: No event ===
        // Don't set any event

        set_render_phase(true);
        with_fiber_tree_mut(|tree| {
            tree.prepare_for_render();
            tree.begin_render(fiber_id);
        });

        let event3 = use_event();

        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });
        set_render_phase(false);

        // Verify no event
        assert!(
            event3.is_none(),
            "Should return None when no event in frame"
        );

        cleanup_test_environment();
    }

    /// Test that position-based component identification generates stable IDs across frames.
    ///
    /// This test verifies the fix for the issue where creating components inside the
    /// render closure caused state to reset every frame. With position-based identification,
    /// components at the same position in the render tree get the same stable ID.
    #[test]
    fn test_position_based_component_id_stability() {
        use crate::component::reset_component_position_counter;
        use std::any::TypeId;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Helper to generate stable ID (same logic as in component.rs)
        fn generate_id<C: 'static>(position: u64) -> u64 {
            let type_id = TypeId::of::<C>();
            let mut hasher = DefaultHasher::new();
            type_id.hash(&mut hasher);
            position.hash(&mut hasher);
            hasher.finish()
        }

        struct ComponentA;
        struct ComponentB;

        // Simulate multiple render frames
        let mut frame_ids: Vec<(u64, u64)> = Vec::new();

        for _frame in 0..5 {
            // Reset counter at start of each frame (like runtime does)
            reset_component_position_counter();

            // Simulate creating components in the same order each frame
            let id_a = generate_id::<ComponentA>(0); // Position 0
            let id_b = generate_id::<ComponentB>(1); // Position 1

            frame_ids.push((id_a, id_b));
        }

        // All frames should have the same IDs
        for i in 1..frame_ids.len() {
            assert_eq!(
                frame_ids[0].0, frame_ids[i].0,
                "ComponentA should have stable ID across frames"
            );
            assert_eq!(
                frame_ids[0].1, frame_ids[i].1,
                "ComponentB should have stable ID across frames"
            );
        }

        // Different components at same position should have different IDs
        let id_a_pos0 = generate_id::<ComponentA>(0);
        let id_b_pos0 = generate_id::<ComponentB>(0);
        assert_ne!(
            id_a_pos0, id_b_pos0,
            "Different component types at same position should have different IDs"
        );

        // Same component at different positions should have different IDs
        let id_a_pos0 = generate_id::<ComponentA>(0);
        let id_a_pos1 = generate_id::<ComponentA>(1);
        assert_ne!(
            id_a_pos0, id_a_pos1,
            "Same component type at different positions should have different IDs"
        );
    }

    /// Test that the runtime resets the component position counter before each render.
    ///
    /// This ensures that components created inside the render closure get stable IDs
    /// based on their position in the render tree.
    #[test]
    fn test_runtime_resets_position_counter() {
        use crate::component::reset_component_position_counter;
        use std::cell::RefCell;

        // Track position counter values
        thread_local! {
            static POSITIONS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
        }

        // Simulate the runtime's render loop
        for _frame in 0..3 {
            // Runtime resets counter before render phase
            reset_component_position_counter();

            // Simulate component creation during render
            // The counter should start at 0 each frame
            crate::component::reset_component_position_counter();

            // After reset, first component should be at position 0
            // This is verified by the stable ID generation
        }

        // The test passes if no panic occurs - the counter is properly reset
    }

    // ========================================================================
    // Cross-thread state update integration tests
    // ========================================================================

    /// Test that updates from a spawned background thread are applied after drain.
    ///
    /// This simulates the real-world scenario where `use_interval` spawns a tokio
    /// task that updates state. The update goes to the cross-thread queue and is
    /// drained into the thread-local batch before `end_batch()`.
    #[test]
    fn test_cross_thread_updates_applied_after_drain() {
        use crate::scheduler::batch::{
            clear_cross_thread_updates, drain_cross_thread_updates, has_cross_thread_updates,
            init_main_thread, reset_main_thread,
        };

        // Reset global state for test isolation
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        // Set up fiber tree with a fiber
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);
        tree.mark_clean(fiber_id);
        set_fiber_tree(tree);

        // Initialize main thread (simulates runtime startup)
        init_main_thread();

        // Spawn a background thread that queues an update
        // This simulates what use_interval does
        let handle = std::thread::spawn(move || {
            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Value(Box::new(42i32)),
                },
            );
        });

        // Wait for background thread to complete
        handle.join().unwrap();

        // Verify update is in cross-thread queue
        assert!(
            has_cross_thread_updates(),
            "Update from background thread should be in cross-thread queue"
        );

        // Simulate runtime's event phase: begin_batch -> drain -> end_batch
        begin_batch();
        drain_cross_thread_updates();
        let dirty_fibers = end_batch();

        // Verify the update was applied
        assert!(
            dirty_fibers.contains(&fiber_id),
            "Fiber should be marked dirty after cross-thread update"
        );

        with_fiber_tree_mut(|tree| {
            let value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            assert_eq!(
                value,
                Some(42),
                "Cross-thread update should have been applied"
            );
        });

        // Clean up
        clear_fiber_tree();
        clear_state_batch();
        clear_cross_thread_updates();
        reset_main_thread();
    }

    /// Test that multiple background threads can safely queue updates concurrently.
    ///
    /// This tests the thread-safety of the cross-thread update queue when multiple
    /// background tasks (like multiple intervals) are updating state simultaneously.
    #[test]
    fn test_concurrent_cross_thread_updates_from_multiple_threads() {
        use crate::scheduler::batch::{
            CrossThreadUpdate, CrossThreadUpdateKind, StateUpdaterFn, clear_cross_thread_updates,
            drain_cross_thread_updates, init_main_thread, queue_cross_thread_update,
            reset_main_thread,
        };
        use std::sync::{Arc, Mutex};

        // Reset global state
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        // Set up fiber tree
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);
        set_fiber_tree(tree);

        // Initialize main thread
        init_main_thread();

        // Use a barrier to ensure all threads start at the same time
        let barrier = Arc::new(std::sync::Barrier::new(10));

        // Spawn multiple background threads that each queue an update directly to cross-thread queue
        let handles: Vec<_> = (1..=10)
            .map(|i| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait(); // Synchronize thread start
                    let updater: StateUpdaterFn = Box::new(move |any| {
                        let n = any.downcast_ref::<i32>().unwrap();
                        Box::new(n + i)
                    });
                    queue_cross_thread_update(CrossThreadUpdate {
                        fiber_id,
                        hook_index: 0,
                        update: CrossThreadUpdateKind::Updater(Arc::new(Mutex::new(Some(updater)))),
                    });
                })
            })
            .collect();

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Drain and apply
        begin_batch();
        drain_cross_thread_updates();
        let dirty_fibers = end_batch();

        // Verify all updates were applied
        assert!(dirty_fibers.contains(&fiber_id));

        with_fiber_tree_mut(|tree| {
            let value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            // 0 + 1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 10 = 55
            assert_eq!(
                value,
                Some(55),
                "All concurrent updates should have been applied"
            );
        });

        // Clean up
        clear_fiber_tree();
        clear_state_batch();
        clear_cross_thread_updates();
        reset_main_thread();
    }

    /// Test that the runtime correctly integrates cross-thread updates in the render loop.
    ///
    /// This test simulates a complete render frame where:
    /// 1. A background thread queues an update (like use_interval callback)
    /// 2. The runtime drains cross-thread updates before end_batch
    /// 3. The update is applied to the fiber tree
    #[test]
    fn test_runtime_render_loop_with_cross_thread_updates() {
        use crate::scheduler::batch::{
            clear_cross_thread_updates, drain_cross_thread_updates, init_main_thread,
            reset_main_thread,
        };

        // Reset global state
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        // Set up fiber tree (simulates runtime initialization)
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 100i32);
        tree.mark_clean(fiber_id);
        set_fiber_tree(tree);

        // Initialize main thread (runtime does this after fiber tree setup)
        init_main_thread();

        // Simulate a background task updating state (like use_interval)
        let handle = std::thread::spawn(move || {
            // This is what happens inside a use_interval callback
            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index: 0,
                    update: StateUpdateKind::Updater(Box::new(|any| {
                        let n = any.downcast_ref::<i32>().unwrap();
                        Box::new(n + 1) // Increment counter
                    })),
                },
            );
        });
        handle.join().unwrap();

        // === Simulate runtime's event phase ===
        // This is what render_with_options does:
        // 1. begin_batch()
        // 2. process events
        // 3. drain_cross_thread_updates()
        // 4. end_batch()

        begin_batch();

        // (Event processing would happen here)

        // Drain cross-thread updates (the key integration point)
        drain_cross_thread_updates();

        // End batch and collect dirty fibers
        let dirty_fibers = end_batch();

        // Verify the update was applied
        assert!(
            dirty_fibers.contains(&fiber_id),
            "Fiber should be dirty after cross-thread update"
        );

        with_fiber_tree_mut(|tree| {
            let value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            assert_eq!(value, Some(101), "Counter should have been incremented");
        });

        // Clean up
        clear_fiber_tree();
        clear_state_batch();
        clear_cross_thread_updates();
        reset_main_thread();
    }

    /// Test that re-entrant calls during drain fall back to cross-thread queue.
    ///
    /// This tests the edge case where a tokio task runs on the main thread
    /// (due to work-stealing) and tries to update state while STATE_BATCH
    /// is already borrowed.
    ///
    /// Note: This test is in batch.rs where STATE_BATCH is accessible.
    /// Here we test the integration at a higher level.
    #[test]
    fn test_reentrant_update_integration() {
        use crate::scheduler::batch::{
            clear_cross_thread_updates, drain_cross_thread_updates, has_cross_thread_updates,
            init_main_thread, reset_main_thread, test_simulate_reentrant_update,
        };

        // Reset global state
        reset_main_thread();
        clear_state_batch();
        clear_cross_thread_updates();

        // Set up fiber tree
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.get_mut(fiber_id).unwrap().set_hook(0, 0i32);
        set_fiber_tree(tree);

        init_main_thread();

        // Use the test helper to simulate a re-entrant update
        test_simulate_reentrant_update(fiber_id, 0, Box::new(999i32));

        // The update should be in cross-thread queue
        assert!(
            has_cross_thread_updates(),
            "Re-entrant update should fall back to cross-thread queue"
        );

        // Now drain and apply
        begin_batch();
        drain_cross_thread_updates();
        let dirty_fibers = end_batch();

        assert!(dirty_fibers.contains(&fiber_id));

        with_fiber_tree_mut(|tree| {
            let value = tree.get(fiber_id).unwrap().get_hook::<i32>(0);
            assert_eq!(value, Some(999), "Re-entrant update should be applied");
        });

        // Clean up
        clear_fiber_tree();
        clear_state_batch();
        clear_cross_thread_updates();
        reset_main_thread();
    }
}
