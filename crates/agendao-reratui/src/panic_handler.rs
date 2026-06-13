//! Panic handler for TUI applications
//!
//! This module provides a panic handler that restores terminal state when a panic occurs.
//! This is essential for TUI applications to ensure the terminal is left in a usable state
//! even if the application crashes.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::panic_handler::setup_panic_handler;
//!
//! fn main() {
//!     // Set up panic handler early in your application
//!     setup_panic_handler();
//!     
//!     // ... rest of your TUI application
//! }
//! ```

use std::io::{self, Write};
use std::panic;
use std::sync::Once;

/// Ensures the panic handler is only initialized once.
static INIT: Once = Once::new();

/// Sets up a custom panic hook that restores terminal state before panicking.
///
/// This function configures a panic hook that:
/// 1. Disables raw mode
/// 2. Leaves the alternate screen
/// 3. Disables mouse capture
/// 4. Flushes stdout/stderr
/// 5. Calls the original panic hook
/// 6. Ensures terminal is restored again after panic output
///
/// This function is idempotent - calling it multiple times is safe and will
/// only set up the panic handler once.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::panic_handler::setup_panic_handler;
///
/// // Call early in your application
/// setup_panic_handler();
///
/// // Safe to call multiple times
/// setup_panic_handler();
/// setup_panic_handler();
/// ```
pub fn setup_panic_handler() {
    INIT.call_once(|| {
        // Take the existing panic hook (could be the default or a custom one)
        let original_hook = panic::take_hook();

        panic::set_hook(Box::new(move |panic_info| {
            use crossterm::event::DisableMouseCapture;
            use crossterm::execute;
            use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};

            // Restore terminal before panic output
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
            let _ = io::stdout().flush();

            // Call the original hook to display the panic message
            original_hook(panic_info);

            // Ensure terminal is restored after panic output
            // This handles cases where the original hook might have re-enabled raw mode
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
            let _ = io::stderr().flush();
            let _ = io::stdout().flush();
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    #[test]
    fn test_setup_panic_handler_idempotent() {
        // Test that calling setup_panic_handler multiple times is safe
        setup_panic_handler();
        setup_panic_handler();
        setup_panic_handler();

        // If we get here without panicking, the test passes
    }

    #[test]
    fn test_setup_panic_handler_thread_safety() {
        let handles: Vec<_> = (0..10)
            .map(|_| {
                thread::spawn(|| {
                    setup_panic_handler();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // If all threads complete successfully, the test passes
    }

    #[test]
    fn test_catch_panic_after_setup() {
        setup_panic_handler();

        // Test that catch_unwind works correctly with the panic handler installed
        let result = std::panic::catch_unwind(|| 42);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);

        // Test that catch_unwind catches panics even with our custom hook
        let result = std::panic::catch_unwind(|| {
            panic!("test panic");
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_panic_payload_preserved() {
        setup_panic_handler();

        let result = std::panic::catch_unwind(|| {
            panic!("preserved message");
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.downcast_ref::<&str>();
        assert!(msg.is_some());
        assert_eq!(*msg.unwrap(), "preserved message");
    }

    #[test]
    fn test_multiple_setups_from_different_threads() {
        let call_count = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..5)
            .map(|_| {
                let count = call_count.clone();
                thread::spawn(move || {
                    setup_panic_handler();
                    count.fetch_add(1, Ordering::SeqCst);
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // All threads should have completed
        assert_eq!(call_count.load(Ordering::SeqCst), 5);
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // **Property 6: Panic Handler Idempotence**
    // *For any* number of calls to `setup_panic_handler()`, the function SHALL complete
    // without error and the panic hook SHALL be installed exactly once.
    // **Validates: Requirements 3.5**

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 6: Panic handler idempotence - multiple calls complete without error
        #[test]
        fn prop_setup_panic_handler_idempotent(
            call_count in 1usize..=20,
        ) {
            // Call setup_panic_handler multiple times
            for _ in 0..call_count {
                setup_panic_handler();
            }

            // Verify that catch_unwind still works after multiple setups
            let result = std::panic::catch_unwind(|| 42);
            prop_assert!(result.is_ok(), "catch_unwind should work after setup");
            prop_assert_eq!(result.unwrap(), 42);
        }

        /// Property 6: Panic handler preserves panic payload after multiple setups
        #[test]
        fn prop_panic_payload_preserved_after_multiple_setups(
            call_count in 1usize..=10,
            panic_value in 1i32..1000,
        ) {
            // Call setup_panic_handler multiple times
            for _ in 0..call_count {
                setup_panic_handler();
            }

            // Verify panic payload is preserved
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                std::panic::panic_any(panic_value);
            }));

            prop_assert!(result.is_err(), "Panic should be caught");
            let err = result.unwrap_err();
            let value = err.downcast_ref::<i32>();
            prop_assert!(value.is_some(), "Panic payload should be i32");
            prop_assert_eq!(*value.unwrap(), panic_value, "Panic value should be preserved");
        }

        /// Property 6: Concurrent setup calls complete without error
        #[test]
        fn prop_concurrent_setup_completes(
            thread_count in 1usize..=10,
        ) {
            let completed = Arc::new(AtomicUsize::new(0));

            let handles: Vec<_> = (0..thread_count)
                .map(|_| {
                    let completed = completed.clone();
                    std::thread::spawn(move || {
                        setup_panic_handler();
                        completed.fetch_add(1, Ordering::SeqCst);
                    })
                })
                .collect();

            for handle in handles {
                handle.join().expect("Thread should complete without panic");
            }

            // All threads should have completed
            prop_assert_eq!(
                completed.load(Ordering::SeqCst),
                thread_count,
                "All threads should complete"
            );
        }
    }
}
