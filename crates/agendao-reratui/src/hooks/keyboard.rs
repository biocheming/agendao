//! Keyboard event hooks for handling keyboard input in components.
//!
//! This module provides fiber-based keyboard hooks that integrate with the
//! fiber event system for proper React-like semantics.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::{use_keyboard, use_keyboard_press, use_keyboard_shortcut};
//! use crossterm::event::{KeyCode, KeyModifiers};
//!
//! #[component]
//! fn MyComponent() -> Element {
//!     // Handle all keyboard events
//!     use_keyboard(|key_event| {
//!         println!("Key: {:?}", key_event.code);
//!     });
//!
//!     // Handle only key press events (not release/repeat)
//!     use_keyboard_press(|key_event| {
//!         println!("Key pressed: {:?}", key_event.code);
//!     });
//!
//!     // Handle specific keyboard shortcuts
//!     use_keyboard_shortcut(KeyCode::Char('s'), KeyModifiers::CONTROL, || {
//!         println!("Ctrl+S pressed - Save!");
//!     });
//!
//!     rsx! { <Text text="Press keys..." /> }
//! }
//! ```

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::effect_event::use_effect_event;
use super::event::use_event;

/// A hook that handles keyboard events with a stable callback.
///
/// This hook uses `use_effect_event` internally to ensure the callback always
/// sees the latest captured values while maintaining a stable identity.
///
/// # Type Parameters
///
/// * `F` - A function that takes a `KeyEvent` and returns nothing
///
/// # Arguments
///
/// * `handler` - A callback function that will be invoked when a key event occurs
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_keyboard;
/// use reratui_fiber::hooks::use_state;
///
/// // Track key press count
/// let (count, set_count) = use_state(|| 0);
///
/// use_keyboard(move |key_event| {
///     println!("Key pressed: {:?}", key_event);
///     set_count.update(|c| c + 1);
/// });
/// ```
///
/// # Note
///
/// - The callback always sees the latest state values (via effect event pattern)
/// - Each key event is only processed once per component
/// - The callback has a stable identity across renders
/// - Only keyboard events trigger the callback (mouse, resize, etc. are ignored)
pub fn use_keyboard<F>(handler: F)
where
    F: Fn(KeyEvent) + Send + Sync + 'static,
{
    // Create a stable callback using effect event pattern
    let stable_handler = use_effect_event(move |key_event: KeyEvent| {
        handler(key_event);
    });

    // Check for keyboard events
    if let Some(Event::Key(key_event)) = use_event() {
        // Emit the event to the stable handler
        stable_handler.call(key_event);
    }
}

/// A hook that handles keyboard press events only (filters out release events).
///
/// This is a convenience wrapper around `use_keyboard` that only triggers the callback
/// when a key is pressed down, ignoring key release and repeat events.
///
/// # Type Parameters
///
/// * `F` - A function that takes a `KeyEvent` and returns nothing
///
/// # Arguments
///
/// * `handler` - A callback function that will be invoked when a key is pressed
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_keyboard_press;
/// use reratui_fiber::hooks::use_state;
///
/// // Only track actual key presses, not releases
/// let (count, set_count) = use_state(|| 0);
///
/// use_keyboard_press(move |key_event| {
///     println!("Key pressed: {:?}", key_event.code);
///     set_count.update(|c| c + 1);
/// });
/// ```
///
/// # Note
///
/// - Only triggers on `KeyEventKind::Press` events
/// - Filters out `KeyEventKind::Release` and `KeyEventKind::Repeat`
/// - The callback always sees the latest state values (via effect event pattern)
/// - The callback has a stable identity across renders
pub fn use_keyboard_press<F>(handler: F)
where
    F: Fn(KeyEvent) + Send + Sync + 'static,
{
    use_keyboard(move |key_event| {
        // Only handle press events, ignore release and repeat
        if key_event.is_press() {
            handler(key_event);
        }
    });
}

/// A hook that handles keyboard shortcuts with specific key and modifier combinations.
///
/// This is a high-level convenience hook for detecting keyboard shortcuts like Ctrl+S,
/// Alt+F4, etc. It only triggers on key press events.
///
/// # Type Parameters
///
/// * `F` - A function that takes no arguments and returns nothing
///
/// # Arguments
///
/// * `key_code` - The key code to match (e.g., `KeyCode::Char('s')`)
/// * `modifiers` - The required modifiers (e.g., `KeyModifiers::CONTROL`)
/// * `handler` - A callback function that will be invoked when the shortcut is pressed
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_keyboard_shortcut;
/// use reratui_fiber::hooks::use_state;
/// use crossterm::event::{KeyCode, KeyModifiers};
///
/// // Ctrl+S to save
/// let (saved, set_saved) = use_state(|| false);
/// use_keyboard_shortcut(KeyCode::Char('s'), KeyModifiers::CONTROL, {
///     let set_saved = set_saved.clone();
///     move || {
///         println!("Save triggered!");
///         set_saved.set(true);
///     }
/// });
///
/// // Alt+Q to quit
/// use_keyboard_shortcut(KeyCode::Char('q'), KeyModifiers::ALT, || {
///     println!("Quit triggered!");
/// });
///
/// // No modifiers - just Enter
/// use_keyboard_shortcut(KeyCode::Enter, KeyModifiers::NONE, || {
///     println!("Enter pressed!");
/// });
///
/// // Ctrl+Shift+P for command palette
/// use_keyboard_shortcut(
///     KeyCode::Char('p'),
///     KeyModifiers::CONTROL | KeyModifiers::SHIFT,
///     || {
///         println!("Command palette opened!");
///     }
/// );
/// ```
///
/// # Note
///
/// - Only triggers on exact matches of key code AND modifiers
/// - Uses `use_keyboard_press` internally (only press events, no release/repeat)
/// - The callback always sees the latest state values (via effect event pattern)
/// - The callback has a stable identity across renders
/// - For multiple modifiers, use bitwise OR: `KeyModifiers::CONTROL | KeyModifiers::SHIFT`
pub fn use_keyboard_shortcut<F>(key_code: KeyCode, modifiers: KeyModifiers, handler: F)
where
    F: Fn() + Send + Sync + 'static,
{
    use_keyboard_press(move |key_event| {
        // Check if both key code and modifiers match
        if key_event.code == key_code && key_event.modifiers == modifiers {
            handler();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{clear_current_event, set_current_event};
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree, with_fiber_tree_mut};
    use crossterm::event::{KeyEventKind, KeyEventState};
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// Test mutex to ensure tests run sequentially since they share global state
    static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn setup_test_fiber() -> crate::fiber::FiberId {
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn cleanup_test() {
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });
        clear_fiber_tree();
        clear_current_event();
    }

    fn create_key_event(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            state: KeyEventState::NONE,
        })
    }

    fn create_press_event(code: KeyCode, modifiers: KeyModifiers) -> Event {
        create_key_event(code, modifiers, KeyEventKind::Press)
    }

    #[test]
    fn test_use_keyboard_receives_key_event() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up a key event
        let event = create_press_event(KeyCode::Char('a'), KeyModifiers::NONE);
        set_current_event(Some(Arc::new(event)));

        // Use the keyboard hook
        use_keyboard(move |key_event| {
            assert_eq!(key_event.code, KeyCode::Char('a'));
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_use_keyboard_ignores_non_key_events() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up a mouse event (not a key event)
        let event = Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        set_current_event(Some(Arc::new(event)));

        // Use the keyboard hook
        use_keyboard(move |_key_event| {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Should not be called for mouse events
        assert_eq!(call_count.load(Ordering::SeqCst), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_keyboard_press_only_handles_press() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));

        // Test with Press event
        let press_event =
            create_key_event(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Press);
        set_current_event(Some(Arc::new(press_event)));

        let call_count_clone = call_count.clone();
        use_keyboard_press(move |_| {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Simulate re-render for release event
        with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });
        clear_current_event();

        // Test with Release event
        let release_event = create_key_event(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        set_current_event(Some(Arc::new(release_event)));

        let call_count_clone2 = call_count.clone();
        use_keyboard_press(move |_| {
            call_count_clone2.fetch_add(1, Ordering::SeqCst);
        });

        // Should still be 1 (release event ignored)
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_use_keyboard_shortcut_matches_exact() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up Ctrl+S event
        let event = create_press_event(KeyCode::Char('s'), KeyModifiers::CONTROL);
        set_current_event(Some(Arc::new(event)));

        // Use the shortcut hook
        use_keyboard_shortcut(KeyCode::Char('s'), KeyModifiers::CONTROL, move || {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_use_keyboard_shortcut_ignores_wrong_modifier() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up Alt+S event (not Ctrl+S)
        let event = create_press_event(KeyCode::Char('s'), KeyModifiers::ALT);
        set_current_event(Some(Arc::new(event)));

        // Use the shortcut hook expecting Ctrl+S
        use_keyboard_shortcut(KeyCode::Char('s'), KeyModifiers::CONTROL, move || {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Should not be called
        assert_eq!(call_count.load(Ordering::SeqCst), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_keyboard_shortcut_ignores_wrong_key() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up Ctrl+A event (not Ctrl+S)
        let event = create_press_event(KeyCode::Char('a'), KeyModifiers::CONTROL);
        set_current_event(Some(Arc::new(event)));

        // Use the shortcut hook expecting Ctrl+S
        use_keyboard_shortcut(KeyCode::Char('s'), KeyModifiers::CONTROL, move || {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Should not be called
        assert_eq!(call_count.load(Ordering::SeqCst), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_keyboard_no_event() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // No event set
        clear_current_event();

        use_keyboard(move |_| {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Should not be called
        assert_eq!(call_count.load(Ordering::SeqCst), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_keyboard_shortcut_with_combined_modifiers() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up Ctrl+Shift+P event
        let event = create_press_event(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        set_current_event(Some(Arc::new(event)));

        // Use the shortcut hook
        use_keyboard_shortcut(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            move || {
                call_count_clone.fetch_add(1, Ordering::SeqCst);
            },
        );

        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }
}
