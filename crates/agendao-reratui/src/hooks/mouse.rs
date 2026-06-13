//! Mouse event hooks for handling mouse input in components.
//!
//! This module provides fiber-based mouse hooks that integrate with the
//! fiber event system for proper React-like semantics.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::{use_mouse, use_mouse_click, use_mouse_hover};
//! use crossterm::event::MouseButton;
//! use ratatui::layout::Rect;
//!
//! #[component]
//! fn MyComponent() -> Element {
//!     // Handle all mouse events
//!     use_mouse(|mouse_event| {
//!         println!("Mouse at: ({}, {})", mouse_event.column, mouse_event.row);
//!     });
//!
//!     // Handle only click events
//!     use_mouse_click(|button, x, y| {
//!         println!("Clicked {:?} at ({}, {})", button, x, y);
//!     });
//!
//!     // Track hover state over an area
//!     let button_area = Rect::new(10, 5, 20, 3);
//!     let is_hovering = use_mouse_hover(button_area);
//!
//!     rsx! { <Text text={format!("Hovering: {}", is_hovering)} /> }
//! }
//! ```

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

use super::effect_event::use_effect_event;
use super::event::use_event;
use super::r#ref::use_ref;
use super::state::use_state;

/// A hook that handles mouse events with a stable callback.
///
/// This hook uses `use_effect_event` internally to ensure the callback always
/// sees the latest captured values while maintaining a stable identity.
///
/// # Type Parameters
///
/// * `F` - A function that takes a `MouseEvent` and returns nothing
///
/// # Arguments
///
/// * `handler` - A callback function that will be invoked when a mouse event occurs
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_mouse;
/// use reratui_fiber::hooks::use_state;
/// use crossterm::event::{MouseEventKind, MouseButton};
///
/// // Track mouse clicks
/// let (click_count, set_click_count) = use_state(|| 0);
///
/// use_mouse(move |mouse_event| {
///     if matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Left)) {
///         println!("Mouse clicked at: ({}, {})", mouse_event.column, mouse_event.row);
///         set_click_count.update(|c| c + 1);
///     }
/// });
/// ```
///
/// # Note
///
/// - The callback always sees the latest state values (via effect event pattern)
/// - Each mouse event is only processed once per component
/// - The callback has a stable identity across renders
/// - Only mouse events trigger the callback (keyboard, resize, etc. are ignored)
/// - Mouse capture must be enabled in the terminal
pub fn use_mouse<F>(handler: F)
where
    F: Fn(MouseEvent) + Send + Sync + 'static,
{
    // Create a stable callback using effect event pattern
    let stable_handler = use_effect_event(move |mouse_event: MouseEvent| {
        handler(mouse_event);
    });

    // Check for mouse events
    if let Some(Event::Mouse(mouse_event)) = use_event() {
        // Emit the event to the stable handler
        stable_handler.call(mouse_event);
    }
}

/// A hook that handles mouse click events only (filters out movement and drag).
///
/// This is a convenience wrapper around `use_mouse` that only triggers the callback
/// when a mouse button is clicked (pressed down), ignoring movement, drag, and scroll events.
///
/// # Type Parameters
///
/// * `F` - A function that takes `(MouseButton, u16, u16)` (button, column, row) and returns nothing
///
/// # Arguments
///
/// * `handler` - A callback function that will be invoked when a mouse button is clicked
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_mouse_click;
/// use crossterm::event::MouseButton;
///
/// // Track left clicks only
/// use_mouse_click(move |button, x, y| {
///     if button == MouseButton::Left {
///         println!("Left click at ({}, {})", x, y);
///     }
/// });
/// ```
///
/// # Note
///
/// - Only triggers on `MouseEventKind::Down` events
/// - Filters out movement, drag, scroll, and button release events
/// - The callback always sees the latest state values (via effect event pattern)
/// - The callback has a stable identity across renders
pub fn use_mouse_click<F>(handler: F)
where
    F: Fn(MouseButton, u16, u16) + Send + Sync + 'static,
{
    use_mouse(move |mouse_event| {
        // Only handle click (down) events
        if let MouseEventKind::Down(button) = mouse_event.kind {
            handler(button, mouse_event.column, mouse_event.row);
        }
    });
}

/// Information about a drag operation
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DragInfo {
    /// The mouse button being used for dragging
    pub button: Option<MouseButton>,
    /// Starting position (column, row)
    pub start: (u16, u16),
    /// Current position (column, row)
    pub current: (u16, u16),
    /// Whether the drag is currently active
    pub is_dragging: bool,
    /// Whether the drag just started
    pub is_start: bool,
    /// Whether the drag just ended
    pub is_end: bool,
}

/// Hook for tracking mouse drag operations.
///
/// Returns a tuple containing the current drag state and a reset function.
/// The reset function can be used to clear the drag state and reset tracking.
///
/// This hook automatically updates the drag state based on mouse events from the current event context.
///
/// # Returns
///
/// A tuple `(DragInfo, impl Fn())` where:
/// - First element is the current drag information
/// - Second element is a reset function to clear the drag state
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_mouse_drag;
///
/// let (drag_info, reset_drag) = use_mouse_drag();
///
/// if drag_info.is_start {
///     println!("Drag started at {:?}", drag_info.start);
/// } else if drag_info.is_dragging {
///     println!("Dragging from {:?} to {:?}", drag_info.start, drag_info.current);
/// } else if drag_info.is_end {
///     println!("Drag ended at {:?}", drag_info.current);
/// }
///
/// // Reset drag state if needed
/// if some_condition {
///     reset_drag();
/// }
/// ```
///
/// # Note
///
/// - Tracks drag start (button down), drag movement, and drag end (button up)
/// - The drag state persists across renders until the drag ends or is reset
/// - `is_dragging` is `true` during the entire drag operation
/// - `is_start` is only `true` on the first frame of the drag
/// - `is_end` is only `true` on the last frame of the drag
pub fn use_mouse_drag() -> (DragInfo, impl Fn() + Clone) {
    let (drag_info, set_drag_info) = use_state(DragInfo::default);
    let drag_state = use_ref(|| None::<(MouseButton, u16, u16)>);

    let set_info_clone = set_drag_info;
    let state_clone = drag_state.clone();

    use_mouse(move |mouse_event| {
        match mouse_event.kind {
            MouseEventKind::Down(button) => {
                // Start drag
                state_clone.set(Some((button, mouse_event.column, mouse_event.row)));
                set_info_clone.set(DragInfo {
                    button: Some(button),
                    start: (mouse_event.column, mouse_event.row),
                    current: (mouse_event.column, mouse_event.row),
                    is_dragging: true,
                    is_start: true,
                    is_end: false,
                });
            }
            MouseEventKind::Drag(button) => {
                // Continue drag
                if let Some((drag_button, start_x, start_y)) = state_clone.get() {
                    if button == drag_button {
                        set_info_clone.set(DragInfo {
                            button: Some(button),
                            start: (start_x, start_y),
                            current: (mouse_event.column, mouse_event.row),
                            is_dragging: true,
                            is_start: false,
                            is_end: false,
                        });
                    }
                }
            }
            MouseEventKind::Up(button) => {
                // End drag
                if let Some((drag_button, start_x, start_y)) = state_clone.get() {
                    if button == drag_button {
                        set_info_clone.set(DragInfo {
                            button: Some(button),
                            start: (start_x, start_y),
                            current: (mouse_event.column, mouse_event.row),
                            is_dragging: false,
                            is_start: false,
                            is_end: true,
                        });
                        state_clone.set(None);
                    }
                }
            }
            _ => {}
        }
    });

    let reset = {
        let set_info = set_drag_info;
        let state = drag_state.clone();
        move || {
            set_info.set(DragInfo::default());
            state.set(None);
        }
    };

    (drag_info, reset)
}

/// A hook that detects double-click events with configurable timing.
///
/// This hook detects when a mouse button is clicked twice within a specified
/// time window (default 500ms).
///
/// # Type Parameters
///
/// * `F` - A function that takes `(MouseButton, u16, u16)` (button, column, row) and returns nothing
///
/// # Arguments
///
/// * `max_delay` - Maximum time between clicks to be considered a double-click
/// * `handler` - A callback function that will be invoked when a double-click is detected
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_double_click;
/// use std::time::Duration;
///
/// // Detect double-clicks with 500ms window
/// use_double_click(Duration::from_millis(500), move |button, x, y| {
///     println!("Double-click at ({}, {})", x, y);
/// });
/// ```
///
/// # Note
///
/// - Default timing window is 500ms (typical for most UIs)
/// - Only triggers on the second click of a double-click
/// - Uses `use_ref` internally to track click timing without re-renders
/// - The callback always sees the latest state values (via effect event pattern)
/// - The callback has a stable identity across renders
pub fn use_double_click<F>(max_delay: Duration, handler: F)
where
    F: Fn(MouseButton, u16, u16) + Send + Sync + 'static,
{
    // Track last click: Option<(button, x, y, time)>
    let last_click = use_ref(|| None::<(MouseButton, u16, u16, Instant)>);

    use_mouse(move |mouse_event| {
        if let MouseEventKind::Down(button) = mouse_event.kind {
            let now = Instant::now();
            let current_pos = (mouse_event.column, mouse_event.row);

            if let Some((last_button, last_x, last_y, last_time)) = last_click.get() {
                // Check if this is a double-click
                let time_diff = now.duration_since(last_time);
                let same_button = button == last_button;
                let same_position = current_pos == (last_x, last_y);

                if same_button && same_position && time_diff <= max_delay {
                    // Double-click detected!
                    handler(button, mouse_event.column, mouse_event.row);
                    // Reset to prevent triple-click from triggering another double-click
                    last_click.set(None);
                    return;
                }
            }

            // Store this click for potential double-click
            last_click.set(Some((button, mouse_event.column, mouse_event.row, now)));
        }
    });
}

/// A hook that tracks the current mouse position.
///
/// Returns a tuple `(x, y)` representing the current mouse coordinates.
/// The position is updated whenever any mouse event occurs (move, click, scroll, etc.).
///
/// # Returns
///
/// A tuple `(u16, u16)` where:
/// - First element is the column (x-coordinate)
/// - Second element is the row (y-coordinate)
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_mouse_position;
///
/// let (x, y) = use_mouse_position();
/// println!("Mouse is at position: ({}, {})", x, y);
/// ```
///
/// # Note
///
/// - The position starts at (0, 0) until the first mouse event
/// - Mouse capture must be enabled in the terminal
/// - The hook updates on any mouse event, including movement, clicks, and scrolling
pub fn use_mouse_position() -> (u16, u16) {
    let (position, set_position) = use_state(|| (0u16, 0u16));

    use_mouse({
        move |mouse_event| {
            let new_pos = (mouse_event.column, mouse_event.row);
            if new_pos != position {
                set_position.set(new_pos);
            }
        }
    });

    position
}

/// A hook that detects if the mouse is hovering over a specific rectangular area.
///
/// Returns `true` if the mouse cursor is currently within the specified area bounds,
/// `false` otherwise. The hover state is updated on any mouse event.
///
/// # Arguments
///
/// * `area` - A `Rect` defining the rectangular area to monitor for hover events.
///   The area is defined by its `x`, `y`, `width`, and `height` properties.
///
/// # Returns
///
/// A boolean indicating whether the mouse is currently hovering over the area.
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_mouse_hover;
/// use ratatui::layout::Rect;
///
/// let button_area = Rect::new(10, 5, 20, 3);
/// let is_hovering = use_mouse_hover(button_area);
///
/// if is_hovering {
///     println!("Mouse is hovering over the button!");
/// }
/// ```
///
/// # Note
///
/// - The hover detection is inclusive of the area boundaries
/// - Mouse position (x, y) is considered inside if:
///   - `x >= area.x && x < area.x + area.width`
///   - `y >= area.y && y < area.y + area.height`
/// - The hook updates on any mouse event (movement, clicks, scrolling)
/// - Mouse capture must be enabled in the terminal
pub fn use_mouse_hover(area: Rect) -> bool {
    let (is_hovering, set_hovering) = use_state(|| false);

    use_mouse({
        move |mouse_event| {
            let is_inside = mouse_event.column >= area.x
                && mouse_event.column < area.x + area.width
                && mouse_event.row >= area.y
                && mouse_event.row < area.y + area.height;

            if is_inside != is_hovering {
                set_hovering.set(is_inside);
            }
        }
    });

    is_hovering
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{clear_current_event, set_current_event};
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree, with_fiber_tree_mut};
    use crossterm::event::KeyModifiers;
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
        crate::scheduler::batch::clear_state_batch();
    }

    fn create_mouse_event(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn test_use_mouse_receives_mouse_event() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up a mouse event
        let event = create_mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 20);
        set_current_event(Some(Arc::new(event)));

        // Use the mouse hook
        use_mouse(move |mouse_event| {
            assert_eq!(mouse_event.column, 10);
            assert_eq!(mouse_event.row, 20);
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_use_mouse_ignores_non_mouse_events() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up a key event (not a mouse event)
        let event = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('a'),
            KeyModifiers::NONE,
        ));
        set_current_event(Some(Arc::new(event)));

        // Use the mouse hook
        use_mouse(move |_mouse_event| {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Should not be called for key events
        assert_eq!(call_count.load(Ordering::SeqCst), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_mouse_click_only_handles_down() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));

        // Test with Down event
        let down_event = create_mouse_event(MouseEventKind::Down(MouseButton::Left), 10, 20);
        set_current_event(Some(Arc::new(down_event)));

        let call_count_clone = call_count.clone();
        use_mouse_click(move |button, x, y| {
            assert_eq!(button, MouseButton::Left);
            assert_eq!(x, 10);
            assert_eq!(y, 20);
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Simulate re-render for move event
        with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });
        clear_current_event();

        // Test with Move event
        let move_event = create_mouse_event(MouseEventKind::Moved, 15, 25);
        set_current_event(Some(Arc::new(move_event)));

        let call_count_clone2 = call_count.clone();
        use_mouse_click(move |_, _, _| {
            call_count_clone2.fetch_add(1, Ordering::SeqCst);
        });

        // Should still be 1 (move event ignored)
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_use_mouse_no_event() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // No event set
        clear_current_event();

        use_mouse(move |_| {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Should not be called
        assert_eq!(call_count.load(Ordering::SeqCst), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_mouse_hover_inside_area() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let fiber_id = setup_test_fiber();

        // Set up a mouse event inside the area
        let event = create_mouse_event(MouseEventKind::Moved, 15, 7);
        set_current_event(Some(Arc::new(event)));

        let area = Rect::new(10, 5, 20, 10);
        // First render - state is initialized to false, event triggers update
        let is_hovering = use_mouse_hover(area);
        // Initial state is false (state updates are batched)
        assert!(!is_hovering);

        // Apply the batch and re-render
        with_fiber_tree_mut(|tree| {
            tree.end_render();
            crate::scheduler::batch::with_state_batch_mut(|batch| {
                batch.end_batch(tree);
            });
            tree.begin_render(fiber_id);
        });
        clear_current_event();

        // Second render - state should now be true
        let is_hovering = use_mouse_hover(area);
        assert!(is_hovering);

        cleanup_test();
    }

    #[test]
    fn test_use_mouse_hover_outside_area() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        // Set up a mouse event outside the area
        let event = create_mouse_event(MouseEventKind::Moved, 5, 3);
        set_current_event(Some(Arc::new(event)));

        let area = Rect::new(10, 5, 20, 10);
        let is_hovering = use_mouse_hover(area);

        // Initial state is false, and event is outside area, so no update queued
        assert!(!is_hovering);

        cleanup_test();
    }

    #[test]
    fn test_use_mouse_position() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let fiber_id = setup_test_fiber();

        // Set up a mouse event
        let event = create_mouse_event(MouseEventKind::Moved, 42, 24);
        set_current_event(Some(Arc::new(event)));

        // First render - state is initialized to (0, 0), event triggers update
        let (x, y) = use_mouse_position();
        // Initial state is (0, 0) (state updates are batched)
        assert_eq!(x, 0);
        assert_eq!(y, 0);

        // Apply the batch and re-render
        with_fiber_tree_mut(|tree| {
            tree.end_render();
            crate::scheduler::batch::with_state_batch_mut(|batch| {
                batch.end_batch(tree);
            });
            tree.begin_render(fiber_id);
        });
        clear_current_event();

        // Second render - state should now be (42, 24)
        let (x, y) = use_mouse_position();
        assert_eq!(x, 42);
        assert_eq!(y, 24);

        cleanup_test();
    }

    #[test]
    fn test_use_mouse_position_default() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        // No event set
        clear_current_event();

        let (x, y) = use_mouse_position();

        // Should return default (0, 0)
        assert_eq!(x, 0);
        assert_eq!(y, 0);

        cleanup_test();
    }

    #[test]
    fn test_drag_info_default() {
        let info = DragInfo::default();
        assert_eq!(info.button, None);
        assert_eq!(info.start, (0, 0));
        assert_eq!(info.current, (0, 0));
        assert!(!info.is_dragging);
        assert!(!info.is_start);
        assert!(!info.is_end);
    }
}
