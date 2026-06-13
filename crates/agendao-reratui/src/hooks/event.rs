//! Event hook for accessing terminal events in components.
//!
//! This module provides hooks for accessing terminal events (keyboard, mouse, resize)
//! during rendering. Events are available to ALL components during a render frame,
//! matching React's event semantics.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui::hooks::{use_event, stop_propagation};
//! use crossterm::event::{Event, KeyCode};
//!
//! fn my_component() {
//!     // Multiple components can read the same event
//!     if let Some(Event::Key(key)) = use_event() {
//!         if key.code == KeyCode::Char('q') {
//!             // Handle quit
//!             stop_propagation(); // Prevent other components from seeing this event
//!         }
//!     }
//! }
//! ```

use crossterm::event::Event;

use crate::event::{get_current_event, peek_current_event as peek_event_internal};

/// Hook that returns the current terminal event.
///
/// Unlike older implementations, this hook returns the same event to ALL callers
/// during a render frame, matching React's event semantics. Multiple components
/// and multiple hooks within the same component can all read the same event.
///
/// Use `stop_propagation()` to explicitly prevent other components from
/// receiving the event.
///
/// # Returns
///
/// * `Some(Event)` - The current event if available and propagation not stopped
/// * `None` - If no event or propagation was stopped by another fiber
///
/// # Example
///
/// ```rust,ignore
/// use reratui::hooks::use_event;
/// use crossterm::event::{Event, KeyCode, KeyEvent};
///
/// fn handle_input() {
///     // First call
///     if let Some(event) = use_event() {
///         match event {
///             Event::Key(KeyEvent { code: KeyCode::Char('q'), .. }) => {
///                 // Quit the application
///             }
///             _ => {}
///         }
///     }
///
///     // Second call - ALSO receives the event (React-like behavior)
///     if let Some(event) = use_event() {
///         // Can process the same event again
///     }
/// }
/// ```
pub fn use_event() -> Option<Event> {
    get_current_event().map(|arc_event| (*arc_event).clone())
}

/// Stops event propagation for the current event.
///
/// After calling this, `use_event()` will return `None` for all other fibers,
/// but the fiber that called `stop_propagation()` can still read the event.
///
/// This matches React's `event.stopPropagation()` behavior.
///
/// # Example
///
/// ```rust,ignore
/// use reratui::hooks::{use_event, stop_propagation};
/// use crossterm::event::{Event, KeyCode};
///
/// fn child_component() {
///     if let Some(Event::Key(key)) = use_event() {
///         if key.code == KeyCode::Enter {
///             // Handle enter - don't let parent see it
///             stop_propagation();
///         }
///     }
/// }
/// ```
pub fn stop_propagation() {
    crate::event::stop_event_propagation();
}

/// Peeks at the current event without affecting propagation.
///
/// This is useful for checking what event is available without
/// participating in the propagation system. Works even after
/// `stop_propagation()` was called.
///
/// # Returns
///
/// * `Some(Event)` - The current event if available (ignores propagation state)
/// * `None` - If no event is available
///
/// # Example
///
/// ```rust,ignore
/// use reratui::hooks::peek_event;
///
/// fn debug_component() {
///     // Check what event is available without affecting anything
///     if let Some(event) = peek_event() {
///         println!("Current event: {:?}", event);
///     }
/// }
/// ```
pub fn peek_event() -> Option<Event> {
    peek_event_internal().map(|arc_event| (*arc_event).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{clear_current_event, set_current_event};
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree, with_fiber_tree_mut};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use std::sync::{Arc, Mutex};

    // Mutex to ensure tests run sequentially (they share global state)
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn create_test_key_event(c: char) -> Event {
        Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char(c),
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))
    }

    fn setup_test_fiber_tree() {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
    }

    fn teardown_test_fiber_tree() {
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });
        clear_fiber_tree();
    }

    #[test]
    fn test_use_event_returns_event() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_current_event();
        setup_test_fiber_tree();

        let event = create_test_key_event('a');
        set_current_event(Some(Arc::new(event.clone())));

        let result = use_event();
        assert!(result.is_some());

        if let Some(Event::Key(key)) = result {
            assert_eq!(key.code, KeyCode::Char('a'));
        } else {
            panic!("Expected Key event");
        }

        teardown_test_fiber_tree();
        clear_current_event();
    }

    #[test]
    fn test_use_event_returns_none_when_no_event() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_current_event();
        setup_test_fiber_tree();

        let result = use_event();
        assert!(result.is_none());

        teardown_test_fiber_tree();
    }

    #[test]
    fn test_use_event_available_multiple_times() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_current_event();
        setup_test_fiber_tree();

        let event = create_test_key_event('b');
        set_current_event(Some(Arc::new(event)));

        // First call should return the event
        let first = use_event();
        assert!(first.is_some());

        // Second call should ALSO return the event (React-like behavior)
        let second = use_event();
        assert!(second.is_some());

        // Third call should ALSO return the event
        let third = use_event();
        assert!(third.is_some());

        teardown_test_fiber_tree();
        clear_current_event();
    }

    #[test]
    fn test_stop_propagation_blocks_other_fibers() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_current_event();

        let mut tree = FiberTree::new();
        let fiber1 = tree.mount(None, None);
        let fiber2 = tree.mount(None, None);
        set_fiber_tree(tree);

        let event = create_test_key_event('c');
        set_current_event(Some(Arc::new(event)));

        // Fiber 1 reads and stops propagation
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber1);
        });
        let result1 = use_event();
        assert!(result1.is_some());
        stop_propagation();
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        // Fiber 2 should NOT be able to read the event
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber2);
        });
        let result2 = use_event();
        assert!(result2.is_none());
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });

        clear_fiber_tree();
        clear_current_event();
    }

    #[test]
    fn test_stop_propagation_allows_same_fiber() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_current_event();
        setup_test_fiber_tree();

        let event = create_test_key_event('d');
        set_current_event(Some(Arc::new(event)));

        // Read event
        let result1 = use_event();
        assert!(result1.is_some());

        // Stop propagation
        stop_propagation();

        // Same fiber can still read the event
        let result2 = use_event();
        assert!(result2.is_some());

        teardown_test_fiber_tree();
        clear_current_event();
    }

    #[test]
    fn test_peek_event_returns_event() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_current_event();
        setup_test_fiber_tree();

        let event = create_test_key_event('e');
        set_current_event(Some(Arc::new(event)));

        let result = peek_event();
        assert!(result.is_some());

        if let Some(Event::Key(key)) = result {
            assert_eq!(key.code, KeyCode::Char('e'));
        } else {
            panic!("Expected Key event");
        }

        teardown_test_fiber_tree();
        clear_current_event();
    }

    #[test]
    fn test_peek_event_works_after_stop_propagation() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_current_event();
        setup_test_fiber_tree();

        let event = create_test_key_event('f');
        set_current_event(Some(Arc::new(event)));

        // Stop propagation
        stop_propagation();

        // Peek should still work
        let result = peek_event();
        assert!(result.is_some());

        teardown_test_fiber_tree();
        clear_current_event();
    }

    #[test]
    fn test_use_event_clones_event() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_current_event();
        setup_test_fiber_tree();

        let event = create_test_key_event('g');
        set_current_event(Some(Arc::new(event.clone())));

        let result = use_event();
        assert!(result.is_some());

        // Verify the returned event is a clone, not the Arc
        if let Some(Event::Key(key)) = result {
            assert_eq!(key.code, KeyCode::Char('g'));
        }

        teardown_test_fiber_tree();
        clear_current_event();
    }
}
