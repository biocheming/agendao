//! Layout hooks for accessing component area, frame info, and terminal dimensions.
//!
//! This module provides fiber-based layout hooks that integrate with the
//! fiber context system for proper React-like semantics.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::{use_area, use_frame, use_resize, use_media_query};
//! use ratatui::layout::Rect;
//!
//! #[component]
//! fn MyComponent() -> Element {
//!     // Access the component's render area
//!     let area = use_area();
//!     
//!     // Access frame timing information
//!     let frame_info = use_frame();
//!     
//!     // Track terminal dimensions
//!     let (width, height) = use_resize();
//!     
//!     // Responsive breakpoints
//!     let is_narrow = use_media_query(|(w, _)| w < 80);
//!
//!     rsx! { <Text text={format!("Size: {}x{}", area.width, area.height)} /> }
//! }
//! ```

use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use std::ops::Deref;
use std::time::{Duration, Instant};

use super::context::{try_use_context, use_context};
use super::effect_event::use_effect_event;
use super::event::use_event;
use super::state::use_state;

// ============================================================================
// Component Area
// ============================================================================

/// Context type for component render area.
///
/// This is provided by the renderer and consumed by components via `use_area()`.
///
/// Implements `Deref<Target = Rect>` so you can access Rect methods directly:
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_area;
/// use ratatui::layout::Margin;
///
/// let area = use_area();
/// let width = area.width;  // Direct access to Rect fields
/// let height = area.height;
/// let inner = area.inner(Margin::new(1, 1));  // Call Rect methods
/// ```
#[derive(Clone, Default, Copy, Debug, PartialEq, Eq)]
pub struct ComponentArea(pub Rect);

impl Deref for ComponentArea {
    type Target = Rect;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Hook to access the current component's render area.
///
/// This hook retrieves the component's render area from the context provided
/// by the renderer. The area represents the Rect that the component is being
/// rendered into.
///
/// # Panics
///
/// Panics if called outside of a component render context where the area
/// has been provided by the renderer.
///
/// # Use Cases
///
/// - **Layout calculations**: Determine available space for child components
/// - **Responsive design**: Adjust rendering based on available space
/// - **Positioning**: Calculate absolute positions within the component
/// - **Scroll handling**: Track viewport dimensions for scrollable content
/// - **Mouse events**: Determine if mouse events are within component bounds
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_area;
///
/// #[component]
/// fn MyComponent() -> Element {
///     let area = use_area();
///     
///     // Direct access to Rect fields via Deref
///     println!("Width: {}, Height: {}", area.width, area.height);
///     
///     rsx! {
///         <Block title="My Component">
///             <Paragraph>
///                 {format!("Size: {}x{}", area.width, area.height)}
///             </Paragraph>
///         </Block>
///     }
/// }
/// ```
pub fn use_area() -> ComponentArea {
    use_context::<ComponentArea>()
}

/// Hook to try to access the current component's render area.
///
/// Returns `Some(ComponentArea)` if the area context is available,
/// or `None` if not provided.
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::try_use_area;
///
/// let area = try_use_area().unwrap_or_default();
/// ```
pub fn try_use_area() -> Option<ComponentArea> {
    try_use_context::<ComponentArea>()
}

// ============================================================================
// Frame Context
// ============================================================================

/// Frame information (without the Frame pointer).
///
/// This struct contains information about the current frame being rendered.
#[derive(Clone, Copy, Debug)]
pub struct FrameInfo {
    /// The current frame number (starts at 0)
    pub count: u64,
    /// Time elapsed since the last frame
    pub delta: Duration,
    /// Timestamp when this frame started rendering
    pub timestamp: Instant,
}

impl Default for FrameInfo {
    fn default() -> Self {
        Self {
            count: 0,
            delta: Duration::ZERO,
            timestamp: Instant::now(),
        }
    }
}

impl FrameInfo {
    /// Create a new FrameInfo.
    pub fn new(count: u64, delta: Duration, timestamp: Instant) -> Self {
        Self {
            count,
            delta,
            timestamp,
        }
    }

    /// Calculate the current FPS based on delta time.
    pub fn fps(&self) -> f64 {
        if self.delta.as_secs_f64() > 0.0 {
            1.0 / self.delta.as_secs_f64()
        } else {
            0.0
        }
    }

    /// Get delta time in seconds as f64.
    pub fn delta_secs(&self) -> f64 {
        self.delta.as_secs_f64()
    }

    /// Get delta time in milliseconds.
    pub fn delta_millis(&self) -> u128 {
        self.delta.as_millis()
    }

    /// Check if this is the first frame.
    pub fn is_first_frame(&self) -> bool {
        self.count == 0
    }
}

/// Frame context that holds both the Frame pointer and frame information.
///
/// This is provided by the renderer via context and consumed by components.
///
/// # Safety
///
/// The frame_ptr is only valid during the current render cycle.
#[derive(Clone, Copy)]
pub struct FrameContext {
    /// Pointer to the current Frame (with 'static lifetime for context storage)
    frame_ptr: *mut Frame<'static>,
    /// Frame information
    pub info: FrameInfo,
}

// Safety: FrameContext is only used within a single thread during rendering
unsafe impl Send for FrameContext {}
unsafe impl Sync for FrameContext {}

impl FrameContext {
    /// Create a new FrameContext.
    ///
    /// # Safety
    ///
    /// The frame pointer must be valid for the duration of the render cycle.
    pub unsafe fn new(frame: &mut Frame, count: u64, delta: Duration, timestamp: Instant) -> Self {
        let frame_ptr = std::ptr::from_mut(frame).cast::<Frame<'static>>();
        Self {
            frame_ptr,
            info: FrameInfo::new(count, delta, timestamp),
        }
    }

    /// Create a new FrameContext from a raw pointer.
    ///
    /// # Safety
    ///
    /// The frame pointer must be valid for the duration of the render cycle.
    pub unsafe fn from_raw_ptr(
        frame_ptr: *mut Frame<'static>,
        count: u64,
        delta: Duration,
        timestamp: Instant,
    ) -> Self {
        Self {
            frame_ptr,
            info: FrameInfo::new(count, delta, timestamp),
        }
    }

    /// Get a reference to the Frame.
    ///
    /// # Safety
    ///
    /// This is safe as long as the FrameContext is only used during the render cycle.
    pub fn frame(&self) -> &Frame<'static> {
        unsafe { &*self.frame_ptr }
    }

    /// Get a mutable reference to the Frame.
    ///
    /// # Safety
    ///
    /// This is safe as long as the FrameContext is only used during the render cycle
    /// and no other mutable references exist.
    #[allow(clippy::mut_from_ref)]
    pub fn frame_mut(&mut self) -> &mut Frame<'static> {
        unsafe { &mut *self.frame_ptr }
    }

    /// Get the frame count.
    pub fn count(&self) -> u64 {
        self.info.count
    }

    /// Get the delta time.
    pub fn delta(&self) -> Duration {
        self.info.delta
    }

    /// Get the timestamp.
    pub fn timestamp(&self) -> Instant {
        self.info.timestamp
    }

    /// Calculate the current FPS based on delta time.
    pub fn fps(&self) -> f64 {
        self.info.fps()
    }

    /// Get delta time in seconds as f64.
    pub fn delta_secs(&self) -> f64 {
        self.info.delta_secs()
    }

    /// Get delta time in milliseconds.
    pub fn delta_millis(&self) -> u128 {
        self.info.delta_millis()
    }

    /// Check if this is the first frame.
    pub fn is_first_frame(&self) -> bool {
        self.info.is_first_frame()
    }

    /// Get FrameInfo (without the Frame pointer).
    pub fn frame_info(&self) -> FrameInfo {
        self.info
    }
}

/// Hook to access the current frame context.
///
/// Returns a `FrameContext` which provides access to:
/// - The current ratatui Frame (via `.frame()` or `.frame_mut()`)
/// - Frame count, delta time, timestamp
///
/// # Panics
///
/// Panics if called outside of a component render context where the frame context
/// has been provided by the renderer.
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_frame;
///
/// let frame_ctx = use_frame();
///
/// // Access frame info
/// let count = frame_ctx.count();
/// let fps = frame_ctx.fps();
///
/// // Access the Frame
/// let frame = frame_ctx.frame();
/// let area = frame.area();
/// ```
pub fn use_frame() -> FrameContext {
    use_context::<FrameContext>()
}

/// Hook to try to access the current frame context.
///
/// Returns `Some(FrameContext)` if the frame context is available,
/// or `None` if not provided.
pub fn try_use_frame() -> Option<FrameContext> {
    try_use_context::<FrameContext>()
}

/// Hook to access only the frame information (without the Frame pointer).
///
/// This is useful when you only need frame timing information and don't
/// need access to the actual Frame.
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_frame_info;
///
/// let info = use_frame_info();
/// println!("Frame: {} @ {:.1} FPS", info.count, info.fps());
/// ```
pub fn use_frame_info() -> FrameInfo {
    try_use_context::<FrameContext>()
        .map(|ctx| ctx.frame_info())
        .unwrap_or_default()
}

// ============================================================================
// Resize Hooks
// ============================================================================

/// A hook that triggers a callback when the terminal is resized.
///
/// This hook monitors resize events and calls the provided callback with the new
/// terminal dimensions (width, height) whenever a resize occurs.
///
/// # Type Parameters
///
/// * `F` - The callback function type that takes `(u16, u16)` as parameters
///
/// # Arguments
///
/// * `callback` - A callback function that will be invoked with `(width, height)` when resize occurs
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_on_resize;
///
/// use_on_resize(|(width, height)| {
///     println!("Terminal resized to: {}x{}", width, height);
/// });
/// ```
pub fn use_on_resize<F>(callback: F)
where
    F: Fn((u16, u16)) + Send + Sync + 'static,
{
    // Create a stable callback using effect event pattern
    let stable_handler = use_effect_event(move |dimensions: (u16, u16)| {
        callback(dimensions);
    });

    // Check for resize events
    if let Some(Event::Resize(width, height)) = use_event() {
        stable_handler.call((width, height));
    }
}

/// A hook that returns the current terminal dimensions as a tuple.
///
/// This is a convenience hook that automatically tracks terminal size and returns
/// the current dimensions directly as a tuple.
///
/// # Returns
///
/// A tuple `(u16, u16)` containing the current terminal dimensions (width, height)
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_resize;
///
/// let (width, height) = use_resize();
/// println!("Terminal: {}x{}", width, height);
/// ```
///
/// # Notes
///
/// - Returns (0, 0) until the first resize event occurs
/// - Automatically updates when the terminal is resized
/// - Re-renders the component when dimensions change
pub fn use_resize() -> (u16, u16) {
    let (size, set_size) = use_state(|| (0u16, 0u16));

    use_on_resize({
        move |(width, height)| {
            set_size.set((width, height));
        }
    });

    size
}

/// A hook that evaluates a media query predicate against terminal dimensions.
///
/// This hook allows you to create responsive layouts by checking terminal size
/// against custom conditions. The predicate is re-evaluated whenever the terminal
/// is resized.
///
/// # Type Parameters
///
/// * `F` - A function that takes `(u16, u16)` and returns `bool`
///
/// # Arguments
///
/// * `predicate` - A function that receives `(width, height)` and returns whether the condition matches
///
/// # Returns
///
/// `bool` - `true` if the predicate matches the current terminal dimensions, `false` otherwise
///
/// # Examples
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_media_query;
///
/// // Check if terminal is narrow
/// let is_narrow = use_media_query(|(width, _)| width < 80);
///
/// // Responsive breakpoints
/// let is_mobile = use_media_query(|(width, _)| width < 60);
/// let is_tablet = use_media_query(|(width, _)| width >= 60 && width < 120);
/// let is_desktop = use_media_query(|(width, _)| width >= 120);
/// ```
///
/// # Notes
///
/// - Returns `false` until the first resize event occurs (dimensions are 0x0)
/// - Automatically re-evaluates when terminal is resized
/// - Triggers component re-render when the predicate result changes
pub fn use_media_query<F>(predicate: F) -> bool
where
    F: Fn((u16, u16)) -> bool + Send + Sync + 'static,
{
    let (matches, set_matches) = use_state(|| false);

    use_on_resize({
        move |(width, height)| {
            let result = predicate((width, height));
            set_matches.set(result);
        }
    });

    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_stack::{clear_context_stack, push_context};
    use crate::event::{clear_current_event, set_current_event};
    use crate::fiber::FiberId;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree, with_fiber_tree_mut};
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// Test mutex to ensure tests run sequentially since they share global state
    static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn setup_test_fiber() -> FiberId {
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
        clear_context_stack();
        crate::scheduler::batch::clear_state_batch();
    }

    // ========================================================================
    // ComponentArea Tests
    // ========================================================================

    #[test]
    fn test_component_area_deref() {
        let area = ComponentArea(Rect::new(10, 20, 100, 50));
        assert_eq!(area.width, 100);
        assert_eq!(area.height, 50);
        assert_eq!(area.x, 10);
        assert_eq!(area.y, 20);
    }

    #[test]
    fn test_component_area_default() {
        let area = ComponentArea::default();
        assert_eq!(area.width, 0);
        assert_eq!(area.height, 0);
    }

    #[test]
    fn test_use_area_with_context() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let fiber_id = setup_test_fiber();

        // Push area context
        let test_area = ComponentArea(Rect::new(10, 20, 100, 50));
        push_context(fiber_id, test_area);

        let area = use_area();
        assert_eq!(area.width, 100);
        assert_eq!(area.height, 50);

        cleanup_test();
    }

    #[test]
    fn test_try_use_area_without_context() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let area = try_use_area();
        assert!(area.is_none());

        cleanup_test();
    }

    // ========================================================================
    // FrameInfo Tests
    // ========================================================================

    #[test]
    fn test_frame_info_creation() {
        let now = Instant::now();
        let info = FrameInfo::new(42, Duration::from_millis(16), now);

        assert_eq!(info.count, 42);
        assert_eq!(info.delta, Duration::from_millis(16));
        assert_eq!(info.timestamp, now);
    }

    #[test]
    fn test_frame_info_fps() {
        let info = FrameInfo::new(1, Duration::from_millis(16), Instant::now());
        let fps = info.fps();

        // 16ms = ~62.5 FPS
        assert!((fps - 62.5).abs() < 0.1);
    }

    #[test]
    fn test_frame_info_delta_conversions() {
        let info = FrameInfo::new(1, Duration::from_millis(16), Instant::now());

        assert_eq!(info.delta_millis(), 16);
        assert!((info.delta_secs() - 0.016).abs() < 0.001);
    }

    #[test]
    fn test_frame_info_is_first_frame() {
        let frame0 = FrameInfo::new(0, Duration::from_millis(16), Instant::now());
        let frame1 = FrameInfo::new(1, Duration::from_millis(16), Instant::now());

        assert!(frame0.is_first_frame());
        assert!(!frame1.is_first_frame());
    }

    #[test]
    fn test_frame_info_default() {
        let info = FrameInfo::default();
        assert_eq!(info.count, 0);
        assert_eq!(info.delta, Duration::ZERO);
        assert!(info.is_first_frame());
    }

    // ========================================================================
    // Resize Hooks Tests
    // ========================================================================

    #[test]
    fn test_use_on_resize_receives_event() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up a resize event
        let event = Event::Resize(120, 40);
        set_current_event(Some(Arc::new(event)));

        use_on_resize(move |(width, height)| {
            assert_eq!(width, 120);
            assert_eq!(height, 40);
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[test]
    fn test_use_on_resize_ignores_non_resize_events() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        let call_count = Arc::new(AtomicI32::new(0));
        let call_count_clone = call_count.clone();

        // Set up a key event (not a resize event)
        let event = Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::NONE,
        ));
        set_current_event(Some(Arc::new(event)));

        use_on_resize(move |_| {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Should not be called for key events
        assert_eq!(call_count.load(Ordering::SeqCst), 0);

        cleanup_test();
    }

    #[test]
    fn test_use_resize_default() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        // No event set
        clear_current_event();

        let (width, height) = use_resize();

        // Should return default (0, 0)
        assert_eq!(width, 0);
        assert_eq!(height, 0);

        cleanup_test();
    }

    #[test]
    fn test_use_resize_updates_on_event() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let fiber_id = setup_test_fiber();

        // Set up a resize event
        let event = Event::Resize(120, 40);
        set_current_event(Some(Arc::new(event)));

        // First render - state is initialized to (0, 0), event triggers update
        let (width, height) = use_resize();
        assert_eq!(width, 0);
        assert_eq!(height, 0);

        // Apply the batch and re-render
        with_fiber_tree_mut(|tree| {
            tree.end_render();
            crate::scheduler::batch::with_state_batch_mut(|batch| {
                batch.end_batch(tree);
            });
            tree.begin_render(fiber_id);
        });
        clear_current_event();

        // Second render - state should now be (120, 40)
        let (width, height) = use_resize();
        assert_eq!(width, 120);
        assert_eq!(height, 40);

        cleanup_test();
    }

    #[test]
    fn test_use_media_query_default() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let _fiber_id = setup_test_fiber();

        // No event set
        clear_current_event();

        let is_narrow = use_media_query(|(w, _)| w < 80);

        // Should return false (default)
        assert!(!is_narrow);

        cleanup_test();
    }

    #[test]
    fn test_use_media_query_evaluates_predicate() {
        let _lock = TEST_MUTEX.lock();
        cleanup_test();
        let fiber_id = setup_test_fiber();

        // Set up a resize event with narrow width
        let event = Event::Resize(60, 40);
        set_current_event(Some(Arc::new(event)));

        // First render
        let is_narrow = use_media_query(|(w, _)| w < 80);
        assert!(!is_narrow); // Initial state is false

        // Apply the batch and re-render
        with_fiber_tree_mut(|tree| {
            tree.end_render();
            crate::scheduler::batch::with_state_batch_mut(|batch| {
                batch.end_batch(tree);
            });
            tree.begin_render(fiber_id);
        });
        clear_current_event();

        // Second render - predicate should now be true
        let is_narrow = use_media_query(|(w, _)| w < 80);
        assert!(is_narrow);

        cleanup_test();
    }
}
