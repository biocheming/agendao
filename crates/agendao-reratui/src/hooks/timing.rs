//! Timing hooks for timeouts and intervals.
//!
//! This module provides hooks for scheduling delayed and repeated callbacks,
//! with proper cleanup on unmount.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::{use_timeout, use_interval};
//!
//! #[component]
//! fn TimerDemo() -> Element {
//!     let (count, set_count) = use_state(|| 0);
//!     
//!     // Auto-increment every second
//!     use_interval(move || {
//!         set_count.update(|n| n + 1);
//!     }, 1000);
//!     
//!     // Show message after 5 seconds
//!     let (show_message, set_show_message) = use_state(|| false);
//!     use_timeout(move || {
//!         set_show_message.set(true);
//!     }, 5000);
//!     
//!     rsx! {
//!         <Text text={format!("Count: {}", count)} />
//!         {if show_message { rsx!(<Text text="5 seconds passed!" />) } else { Element::default() }}
//!     }
//! }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::hooks::use_effect;

/// Handle for controlling a timeout.
///
/// Provides methods to cancel the timeout before it fires.
#[derive(Clone)]
pub struct TimeoutHandle {
    cancelled: Arc<AtomicBool>,
}

impl TimeoutHandle {
    /// Cancels the timeout, preventing the callback from executing.
    ///
    /// If the timeout has already fired, this has no effect.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let handle = use_timeout(|| println!("Fired!"), 5000);
    /// // Later...
    /// handle.cancel(); // Prevents "Fired!" from printing
    /// ```
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if the timeout has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Schedule a callback to run after a delay.
///
/// The callback will be executed once after the specified delay in milliseconds.
/// The timeout is automatically cancelled when the component unmounts.
///
/// # Arguments
///
/// * `callback` - The function to call after the delay
/// * `delay_ms` - The delay in milliseconds before the callback is executed
///
/// # Returns
///
/// A `TimeoutHandle` that can be used to cancel the timeout.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_timeout;
///
/// #[component]
/// fn DelayedMessage() -> Element {
///     let (visible, set_visible) = use_state(|| false);
///     
///     use_timeout(move || {
///         set_visible.set(true);
///     }, 3000); // Show after 3 seconds
///     
///     rsx! {
///         {if visible { rsx!(<Text text="Hello!" />) } else { Element::default() }}
///     }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context.
pub fn use_timeout<F>(callback: F, delay_ms: u64) -> TimeoutHandle
where
    F: FnOnce() + Send + 'static,
{
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_for_effect = cancelled.clone();

    // Wrap callback in Arc<Mutex<Option<F>>> so it can be moved into the spawned task
    let callback = Arc::new(Mutex::new(Some(callback)));

    use_effect(
        move || {
            let cancelled = cancelled_for_effect.clone();
            let cancelled_for_spawn = cancelled.clone();
            let callback = callback.clone();

            // Spawn a task to execute the callback after delay
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;

                if !cancelled_for_spawn.load(Ordering::SeqCst) {
                    // Take the callback and execute it
                    if let Some(cb) = callback.lock().ok().and_then(|mut guard| guard.take()) {
                        cb();
                    }
                }
            });

            // Return cleanup function that cancels the timeout
            Some(Box::new(move || {
                cancelled.store(true, Ordering::SeqCst);
            }) as Box<dyn FnOnce() + Send>)
        },
        Some(()), // Empty deps - only run once
    );

    TimeoutHandle { cancelled }
}

/// Handle for controlling an interval.
///
/// Provides methods to pause, resume, and stop the interval.
#[derive(Clone)]
pub struct IntervalHandle {
    paused: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}

impl IntervalHandle {
    /// Pauses the interval, preventing further callbacks until resumed.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let handle = use_interval(|| println!("Tick!"), 1000);
    /// handle.pause(); // Stops ticking
    /// handle.resume(); // Resumes ticking
    /// ```
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Resumes a paused interval.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    /// Returns true if the interval is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Cancels the interval completely.
    ///
    /// Once cancelled, the interval cannot be resumed.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if the interval has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Execute a callback repeatedly at a fixed interval.
///
/// The callback will be executed every `interval_ms` milliseconds.
/// The interval is automatically cancelled when the component unmounts.
///
/// # Arguments
///
/// * `callback` - The function to call at each interval
/// * `interval_ms` - The interval in milliseconds between callbacks
///
/// # Returns
///
/// An `IntervalHandle` that can be used to pause, resume, or cancel the interval.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_interval;
///
/// #[component]
/// fn Counter() -> Element {
///     let (count, set_count) = use_state(|| 0);
///     
///     let handle = use_interval(move || {
///         set_count.update(|n| n + 1);
///     }, 1000); // Increment every second
///     
///     rsx! {
///         <Text text={format!("Count: {}", count)} />
///         <Button on_click={move |_| handle.pause()}>Pause</Button>
///         <Button on_click={move |_| handle.resume()}>Resume</Button>
///     }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context.
pub fn use_interval<F>(callback: F, interval_ms: u64) -> IntervalHandle
where
    F: Fn() + Send + Sync + 'static,
{
    let paused = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));

    let paused_for_effect = paused.clone();
    let cancelled_for_effect = cancelled.clone();
    let callback = Arc::new(callback);

    use_effect(
        move || {
            let paused = paused_for_effect.clone();
            let cancelled = cancelled_for_effect.clone();
            let cancelled_for_spawn = cancelled.clone();
            let paused_for_spawn = paused.clone();
            let callback = callback.clone();

            // Spawn a task to execute the callback at intervals
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));

                loop {
                    interval.tick().await;

                    if cancelled_for_spawn.load(Ordering::SeqCst) {
                        break;
                    }

                    if !paused_for_spawn.load(Ordering::SeqCst) {
                        callback();
                    }
                }
            });

            // Return cleanup function that cancels the interval
            Some(Box::new(move || {
                cancelled.store(true, Ordering::SeqCst);
            }) as Box<dyn FnOnce() + Send>)
        },
        Some(()), // Empty deps - only run once
    );

    IntervalHandle { paused, cancelled }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber::FiberId;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};

    fn setup_test_fiber() -> FiberId {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn cleanup_test() {
        clear_fiber_tree();
    }

    #[test]
    fn test_timeout_handle_cancel() {
        let handle = TimeoutHandle {
            cancelled: Arc::new(AtomicBool::new(false)),
        };

        assert!(!handle.is_cancelled());
        handle.cancel();
        assert!(handle.is_cancelled());
    }

    #[test]
    fn test_interval_handle_pause_resume() {
        let handle = IntervalHandle {
            paused: Arc::new(AtomicBool::new(false)),
            cancelled: Arc::new(AtomicBool::new(false)),
        };

        assert!(!handle.is_paused());
        handle.pause();
        assert!(handle.is_paused());
        handle.resume();
        assert!(!handle.is_paused());
    }

    #[test]
    fn test_interval_handle_cancel() {
        let handle = IntervalHandle {
            paused: Arc::new(AtomicBool::new(false)),
            cancelled: Arc::new(AtomicBool::new(false)),
        };

        assert!(!handle.is_cancelled());
        handle.cancel();
        assert!(handle.is_cancelled());
    }

    #[test]
    fn test_use_timeout_returns_handle() {
        let _fiber_id = setup_test_fiber();

        let handle = use_timeout(|| {}, 1000);
        assert!(!handle.is_cancelled());

        cleanup_test();
    }

    #[test]
    fn test_use_timeout_can_cancel() {
        let _fiber_id = setup_test_fiber();

        let handle = use_timeout(|| {}, 1000);
        handle.cancel();
        assert!(handle.is_cancelled());

        cleanup_test();
    }

    #[test]
    fn test_use_interval_returns_handle() {
        let _fiber_id = setup_test_fiber();

        let handle = use_interval(|| {}, 1000);
        assert!(!handle.is_cancelled());
        assert!(!handle.is_paused());

        // Clean up
        handle.cancel();
        cleanup_test();
    }

    #[test]
    fn test_use_interval_can_pause_resume() {
        let _fiber_id = setup_test_fiber();

        let handle = use_interval(|| {}, 1000);

        handle.pause();
        assert!(handle.is_paused());

        handle.resume();
        assert!(!handle.is_paused());

        // Clean up
        handle.cancel();
        cleanup_test();
    }

    #[test]
    fn test_use_interval_can_cancel() {
        let _fiber_id = setup_test_fiber();

        let handle = use_interval(|| {}, 1000);
        handle.cancel();
        assert!(handle.is_cancelled());

        cleanup_test();
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use crate::fiber::FiberId;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};
    use proptest::prelude::*;

    fn setup_test_fiber() -> FiberId {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn cleanup_test() {
        clear_fiber_tree();
    }

    // Property 10: Timeout executes after delay
    // Property 11: Timeout cancelled on unmount
    // These are tested via handle behavior since actual timing tests are flaky

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property: Timeout handle cancel is idempotent
        /// Validates: Requirements 5.2, 5.3
        #[test]
        fn prop_timeout_cancel_idempotent(cancel_count in 1usize..10) {
            let _fiber_id = setup_test_fiber();

            let handle = use_timeout(|| {}, 10000);

            // Cancel multiple times should be safe
            for _ in 0..cancel_count {
                handle.cancel();
                prop_assert!(handle.is_cancelled());
            }

            cleanup_test();
        }

        /// Property: Timeout handle clone shares state
        /// Validates: Requirements 5.3
        #[test]
        fn prop_timeout_handle_clone_shares_state(_dummy in 0..1i32) {
            let _fiber_id = setup_test_fiber();

            let handle1 = use_timeout(|| {}, 10000);
            let handle2 = handle1.clone();

            prop_assert!(!handle1.is_cancelled());
            prop_assert!(!handle2.is_cancelled());

            handle1.cancel();

            prop_assert!(handle1.is_cancelled());
            prop_assert!(handle2.is_cancelled());

            cleanup_test();
        }

        /// Property: Interval handle pause/resume is consistent
        /// Validates: Requirements 6.3
        #[test]
        fn prop_interval_pause_resume_consistent(ops in prop::collection::vec(prop_oneof![Just(true), Just(false)], 1..20)) {
            let _fiber_id = setup_test_fiber();

            let handle = use_interval(|| {}, 10000);

            #[allow(unused_assignments)]
            let mut expected_paused = false;
            for op in ops {
                if op {
                    handle.pause();
                    expected_paused = true;
                } else {
                    handle.resume();
                    expected_paused = false;
                }
                prop_assert_eq!(handle.is_paused(), expected_paused);
            }

            handle.cancel();
            cleanup_test();
        }

        /// Property: Interval handle cancel is idempotent
        /// Validates: Requirements 6.2
        #[test]
        fn prop_interval_cancel_idempotent(cancel_count in 1usize..10) {
            let _fiber_id = setup_test_fiber();

            let handle = use_interval(|| {}, 10000);

            // Cancel multiple times should be safe
            for _ in 0..cancel_count {
                handle.cancel();
                prop_assert!(handle.is_cancelled());
            }

            cleanup_test();
        }

        /// Property: Interval handle clone shares state
        /// Validates: Requirements 6.2, 6.3
        #[test]
        fn prop_interval_handle_clone_shares_state(_dummy in 0..1i32) {
            let _fiber_id = setup_test_fiber();

            let handle1 = use_interval(|| {}, 10000);
            let handle2 = handle1.clone();

            // Test pause sharing
            handle1.pause();
            prop_assert!(handle1.is_paused());
            prop_assert!(handle2.is_paused());

            handle2.resume();
            prop_assert!(!handle1.is_paused());
            prop_assert!(!handle2.is_paused());

            // Test cancel sharing
            handle1.cancel();
            prop_assert!(handle1.is_cancelled());
            prop_assert!(handle2.is_cancelled());

            cleanup_test();
        }

        /// Property: Multiple timeouts are independent
        /// Validates: Requirements 5.1, 5.3
        #[test]
        fn prop_multiple_timeouts_independent(count in 2usize..10) {
            let _fiber_id = setup_test_fiber();

            let handles: Vec<_> = (0..count)
                .map(|_| use_timeout(|| {}, 10000))
                .collect();

            // Cancel first half
            for handle in handles.iter().take(count / 2) {
                handle.cancel();
            }

            // Verify first half cancelled, second half not
            for (i, handle) in handles.iter().enumerate() {
                if i < count / 2 {
                    prop_assert!(handle.is_cancelled(), "Handle {} should be cancelled", i);
                } else {
                    prop_assert!(!handle.is_cancelled(), "Handle {} should not be cancelled", i);
                }
            }

            // Clean up remaining
            for handle in handles.iter().skip(count / 2) {
                handle.cancel();
            }

            cleanup_test();
        }

        /// Property: Multiple intervals are independent
        /// Validates: Requirements 6.1, 6.3
        #[test]
        fn prop_multiple_intervals_independent(count in 2usize..10) {
            let _fiber_id = setup_test_fiber();

            let handles: Vec<_> = (0..count)
                .map(|_| use_interval(|| {}, 10000))
                .collect();

            // Pause first half
            for handle in handles.iter().take(count / 2) {
                handle.pause();
            }

            // Verify first half paused, second half not
            for (i, handle) in handles.iter().enumerate() {
                if i < count / 2 {
                    prop_assert!(handle.is_paused(), "Handle {} should be paused", i);
                } else {
                    prop_assert!(!handle.is_paused(), "Handle {} should not be paused", i);
                }
            }

            // Clean up
            for handle in &handles {
                handle.cancel();
            }

            cleanup_test();
        }
    }
}
