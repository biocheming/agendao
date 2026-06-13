//! Global event handling for TUI applications
//!
//! This module provides a system for registering and processing global keyboard
//! event handlers. Global handlers are useful for application-wide shortcuts
//! like 'q' to quit, regardless of which component is focused.
//!
//! # Example
//!
//! ```rust,ignore
//! use crossterm::event::KeyCode;
//! use reratui_fiber::global_events::{on_global_event, process_global_event};
//!
//! // Register a handler for the 'q' key
//! on_global_event(KeyCode::Char('q'), || {
//!     println!("Quit requested");
//!     true // Stop event propagation
//! });
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

/// Type alias for event handler functions.
/// Handlers return `true` to stop propagation, `false` to continue.
type EventHandler = dyn Fn() -> bool + Send + Sync + 'static;

/// Global storage for event handlers, keyed by KeyCode.
/// Each KeyCode can have multiple handlers that are called in registration order.
static GLOBAL_EVENT_HANDLERS: Lazy<Mutex<HashMap<KeyCode, Vec<Arc<EventHandler>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Register a global event handler for a specific key code.
///
/// Handlers are called in registration order when the corresponding key is pressed.
/// A handler can stop propagation by returning `true`, which prevents subsequent
/// handlers from being called.
///
/// # Arguments
///
/// * `key` - The key code to listen for
/// * `handler` - A closure that will be called when the key is pressed.
///   Return `true` to indicate the event was handled and stop propagation,
///   or `false` to allow other handlers to process the event.
///
/// # Example
///
/// ```rust,ignore
/// use crossterm::event::KeyCode;
/// use reratui_fiber::global_events::on_global_event;
///
/// on_global_event(KeyCode::Char('q'), || {
///     println!("Quit requested");
///     true // Stop event propagation
/// });
/// ```
pub fn on_global_event<F>(key: KeyCode, handler: F)
where
    F: Fn() -> bool + Send + Sync + 'static,
{
    let mut handlers = GLOBAL_EVENT_HANDLERS.lock();
    let handlers_for_key = handlers.entry(key).or_default();
    handlers_for_key.push(Arc::new(handler));
}

/// Process a key event through all registered global handlers.
///
/// Handlers are called in registration order. If a handler returns `true`,
/// propagation stops and no further handlers are called.
///
/// Only key press events are processed; key release and repeat events are ignored.
///
/// # Arguments
///
/// * `event` - The key event to process
///
/// # Returns
///
/// `true` if the event was handled by any handler (a handler returned `true`),
/// `false` otherwise.
///
/// # Example
///
/// ```rust,ignore
/// use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
/// use reratui_fiber::global_events::process_global_event;
///
/// let event = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
/// if process_global_event(&event) {
///     println!("Event was handled by a global handler");
/// }
/// ```
pub fn process_global_event(event: &KeyEvent) -> bool {
    // Only process key press events, not release or repeat
    if event.kind != KeyEventKind::Press {
        return false;
    }

    let handlers = GLOBAL_EVENT_HANDLERS.lock();
    if let Some(handlers_for_key) = handlers.get(&event.code) {
        for handler in handlers_for_key {
            if handler() {
                return true; // Handler returned true, stop propagation
            }
        }
    }
    false
}

/// Clear all registered global event handlers.
///
/// This is useful for cleanup during testing or when resetting application state.
pub fn clear_global_handlers() {
    GLOBAL_EVENT_HANDLERS.lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventState, KeyModifiers};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Test mutex to ensure tests run sequentially since they share global state
    /// This is shared between unit tests and property tests
    pub(super) static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    /// Helper to create a key press event
    fn key_press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Helper to create a key release event
    fn key_release(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn test_handler_registration_and_invocation() {
        let _lock = TEST_MUTEX.lock();
        clear_global_handlers();

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        on_global_event(KeyCode::Char('a'), move || {
            called_clone.store(true, Ordering::SeqCst);
            true
        });

        let event = key_press(KeyCode::Char('a'));
        let result = process_global_event(&event);

        assert!(called.load(Ordering::SeqCst), "Handler should be called");
        assert!(result, "Event should be marked as handled");
    }

    #[test]
    fn test_propagation_stops_on_true() {
        let _lock = TEST_MUTEX.lock();
        clear_global_handlers();

        let first_called = Arc::new(AtomicBool::new(false));
        let second_called = Arc::new(AtomicBool::new(false));

        let first = first_called.clone();
        on_global_event(KeyCode::Char('b'), move || {
            first.store(true, Ordering::SeqCst);
            true // Stop propagation
        });

        let second = second_called.clone();
        on_global_event(KeyCode::Char('b'), move || {
            second.store(true, Ordering::SeqCst);
            true
        });

        let event = key_press(KeyCode::Char('b'));
        let result = process_global_event(&event);

        assert!(
            first_called.load(Ordering::SeqCst),
            "First handler should be called"
        );
        assert!(
            !second_called.load(Ordering::SeqCst),
            "Second handler should NOT be called"
        );
        assert!(result, "Event should be marked as handled");
    }

    #[test]
    fn test_propagation_continues_on_false() {
        let _lock = TEST_MUTEX.lock();
        clear_global_handlers();

        let call_count = Arc::new(AtomicUsize::new(0));

        let count1 = call_count.clone();
        on_global_event(KeyCode::Char('c'), move || {
            count1.fetch_add(1, Ordering::SeqCst);
            false // Continue propagation
        });

        let count2 = call_count.clone();
        on_global_event(KeyCode::Char('c'), move || {
            count2.fetch_add(1, Ordering::SeqCst);
            false // Continue propagation
        });

        let event = key_press(KeyCode::Char('c'));
        let result = process_global_event(&event);

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            2,
            "Both handlers should be called"
        );
        assert!(
            !result,
            "Event should NOT be marked as handled when all return false"
        );
    }

    #[test]
    fn test_no_handlers_for_key() {
        let _lock = TEST_MUTEX.lock();
        clear_global_handlers();

        let event = key_press(KeyCode::Char('x'));
        let result = process_global_event(&event);

        assert!(!result, "Should return false when no handlers registered");
    }

    #[test]
    fn test_only_press_events_processed() {
        let _lock = TEST_MUTEX.lock();
        clear_global_handlers();

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        on_global_event(KeyCode::Char('d'), move || {
            called_clone.store(true, Ordering::SeqCst);
            true
        });

        // Release event should not trigger handler
        let release_event = key_release(KeyCode::Char('d'));
        let result = process_global_event(&release_event);

        assert!(
            !called.load(Ordering::SeqCst),
            "Handler should NOT be called for release"
        );
        assert!(!result, "Release event should not be handled");

        // Press event should trigger handler
        let press_event = key_press(KeyCode::Char('d'));
        let result = process_global_event(&press_event);

        assert!(
            called.load(Ordering::SeqCst),
            "Handler should be called for press"
        );
        assert!(result, "Press event should be handled");
    }

    #[test]
    fn test_clear_handlers() {
        let _lock = TEST_MUTEX.lock();
        clear_global_handlers();

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        on_global_event(KeyCode::Char('e'), move || {
            called_clone.store(true, Ordering::SeqCst);
            true
        });

        clear_global_handlers();

        let event = key_press(KeyCode::Char('e'));
        let result = process_global_event(&event);

        assert!(
            !called.load(Ordering::SeqCst),
            "Handler should NOT be called after clear"
        );
        assert!(!result, "Should return false after handlers cleared");
    }

    #[test]
    fn test_handlers_called_in_registration_order() {
        let _lock = TEST_MUTEX.lock();
        clear_global_handlers();

        let order = Arc::new(Mutex::new(Vec::new()));

        let order1 = order.clone();
        on_global_event(KeyCode::Char('f'), move || {
            order1.lock().push(1);
            false
        });

        let order2 = order.clone();
        on_global_event(KeyCode::Char('f'), move || {
            order2.lock().push(2);
            false
        });

        let order3 = order.clone();
        on_global_event(KeyCode::Char('f'), move || {
            order3.lock().push(3);
            true // Stop here
        });

        let event = key_press(KeyCode::Char('f'));
        process_global_event(&event);

        let recorded_order = order.lock().clone();
        assert_eq!(
            recorded_order,
            vec![1, 2, 3],
            "Handlers should be called in registration order"
        );
    }
}

#[cfg(test)]
mod property_tests {
    use super::tests::TEST_MUTEX;
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Strategy to generate valid KeyCode values
    fn key_code_strategy() -> impl Strategy<Value = KeyCode> {
        prop_oneof![
            // Character keys
            any::<char>().prop_filter_map("printable char", |c| {
                if c.is_ascii_alphanumeric() || c.is_ascii_punctuation() {
                    Some(KeyCode::Char(c))
                } else {
                    None
                }
            }),
            // Function keys
            (1u8..=12).prop_map(KeyCode::F),
            // Special keys
            Just(KeyCode::Enter),
            Just(KeyCode::Tab),
            Just(KeyCode::Backspace),
            Just(KeyCode::Esc),
            Just(KeyCode::Left),
            Just(KeyCode::Right),
            Just(KeyCode::Up),
            Just(KeyCode::Down),
            Just(KeyCode::Home),
            Just(KeyCode::End),
            Just(KeyCode::PageUp),
            Just(KeyCode::PageDown),
            Just(KeyCode::Delete),
            Just(KeyCode::Insert),
        ]
    }

    /// Strategy to generate key press events
    fn key_press_event_strategy() -> impl Strategy<Value = KeyEvent> {
        key_code_strategy().prop_map(|code| KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    // **Property 3: Global Event Handler Propagation**
    // *For any* sequence of registered global event handlers for a key code, when an event is processed:
    // - Handlers SHALL be called in registration order
    // - If a handler returns `true`, subsequent handlers SHALL NOT be called
    // - If a handler returns `false`, the next handler SHALL be called
    // **Validates: Requirements 6.2, 6.3, 6.4, 6.5**

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 3: Global Event Handler Propagation - handlers called in registration order
        /// and propagation stops when handler returns true
        #[test]
        fn prop_handler_propagation(
            key_event in key_press_event_strategy(),
            handler_count in 1usize..=10,
        ) {
            let _lock = TEST_MUTEX.lock();
            clear_global_handlers();

            // Track call order
            let call_order = Arc::new(Mutex::new(Vec::new()));

            // Register handlers that track their call order
            for i in 0..handler_count {
                let order = call_order.clone();
                on_global_event(key_event.code, move || {
                    order.lock().push(i);
                    false // Continue propagation
                });
            }

            // Process the event
            process_global_event(&key_event);

            // Verify all handlers were called in registration order
            let recorded = call_order.lock().clone();
            let expected: Vec<usize> = (0..handler_count).collect();
            prop_assert_eq!(recorded, expected, "Handlers should be called in registration order");
        }

        /// Property 3: Propagation stops when handler returns true
        #[test]
        fn prop_propagation_stops_on_true(
            key_event in key_press_event_strategy(),
            stop_at in 0usize..10,
            total_handlers in 1usize..=10,
        ) {
            let _lock = TEST_MUTEX.lock();
            clear_global_handlers();

            // Ensure stop_at is within bounds
            let stop_at = stop_at % total_handlers;

            // Track which handlers were called
            let called = Arc::new(Mutex::new(Vec::new()));

            // Register handlers
            for i in 0..total_handlers {
                let called_clone = called.clone();
                let should_stop = i == stop_at;
                on_global_event(key_event.code, move || {
                    called_clone.lock().push(i);
                    should_stop // Stop propagation at stop_at
                });
            }

            // Process the event
            let result = process_global_event(&key_event);

            // Verify propagation behavior
            let called_handlers = called.lock().clone();

            // Handlers up to and including stop_at should be called
            let expected: Vec<usize> = (0..=stop_at).collect();
            prop_assert_eq!(called_handlers, expected, "Only handlers up to stop_at should be called");

            // Result should be true since a handler returned true
            prop_assert!(result, "Event should be marked as handled");
        }

        /// Property 3: All handlers called when none return true
        #[test]
        fn prop_all_handlers_called_when_none_stop(
            key_event in key_press_event_strategy(),
            handler_count in 1usize..=10,
        ) {
            let _lock = TEST_MUTEX.lock();
            clear_global_handlers();

            let call_count = Arc::new(AtomicUsize::new(0));

            // Register handlers that all return false
            for _ in 0..handler_count {
                let count = call_count.clone();
                on_global_event(key_event.code, move || {
                    count.fetch_add(1, Ordering::SeqCst);
                    false // Continue propagation
                });
            }

            // Process the event
            let result = process_global_event(&key_event);

            // All handlers should be called
            prop_assert_eq!(
                call_count.load(Ordering::SeqCst),
                handler_count,
                "All handlers should be called when none return true"
            );

            // Result should be false since no handler returned true
            prop_assert!(!result, "Event should not be marked as handled when all return false");
        }

        /// Property 3: Only press events trigger handlers
        #[test]
        fn prop_only_press_events_processed(
            code in key_code_strategy(),
        ) {
            let _lock = TEST_MUTEX.lock();
            clear_global_handlers();

            let called = Arc::new(AtomicUsize::new(0));
            let called_clone = called.clone();

            on_global_event(code, move || {
                called_clone.fetch_add(1, Ordering::SeqCst);
                true
            });

            // Test release event - should not trigger handler
            let release_event = KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Release,
                state: KeyEventState::NONE,
            };
            let release_result = process_global_event(&release_event);
            prop_assert!(!release_result, "Release event should not be handled");
            prop_assert_eq!(called.load(Ordering::SeqCst), 0, "Handler should not be called for release");

            // Test repeat event - should not trigger handler
            let repeat_event = KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Repeat,
                state: KeyEventState::NONE,
            };
            let repeat_result = process_global_event(&repeat_event);
            prop_assert!(!repeat_result, "Repeat event should not be handled");
            prop_assert_eq!(called.load(Ordering::SeqCst), 0, "Handler should not be called for repeat");

            // Test press event - should trigger handler
            let press_event = KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            };
            let press_result = process_global_event(&press_event);
            prop_assert!(press_result, "Press event should be handled");
            prop_assert_eq!(called.load(Ordering::SeqCst), 1, "Handler should be called for press");
        }
    }
}
