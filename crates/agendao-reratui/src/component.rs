//! Component trait for fiber-based component architecture.
//!
//! This module provides the `Component` trait that integrates with the fiber system,
//! enabling components to have isolated hook state and proper lifecycle management.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::any::TypeId;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

thread_local! {
    /// Position counter for generating stable component IDs within a render frame.
    /// This counter is reset at the start of each render frame.
    static COMPONENT_POSITION_COUNTER: RefCell<u64> = const { RefCell::new(0) };
}

/// Resets the component position counter at the start of a render frame.
/// This should be called by the runtime before each render phase.
pub fn reset_component_position_counter() {
    COMPONENT_POSITION_COUNTER.with(|counter| {
        *counter.borrow_mut() = 0;
    });
}

/// Generates a stable component ID based on TypeId and position in the render tree.
/// This allows components created inside the render closure to maintain stable identity
/// across frames, similar to React's reconciliation algorithm.
fn generate_stable_component_id<C: 'static>() -> u64 {
    let type_id = TypeId::of::<C>();
    let position = COMPONENT_POSITION_COUNTER.with(|counter| {
        let mut c = counter.borrow_mut();
        let pos = *c;
        *c += 1;
        pos
    });

    // Combine TypeId and position into a stable hash
    let mut hasher = DefaultHasher::new();
    type_id.hash(&mut hasher);
    position.hash(&mut hasher);
    hasher.finish()
}

/// Context value providing the render area to child components.
///
/// This is automatically provided when a `Component` renders, allowing
/// child components to access their render area via `use_context`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentArea(pub Rect);

impl ComponentArea {
    /// Returns the render area.
    pub fn area(&self) -> Rect {
        self.0
    }
}

/// A component trait that integrates with the fiber-based architecture.
///
/// Users only need to implement `render()` - no Clone required!
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::{Component, Element};
/// use ratatui::{buffer::Buffer, layout::Rect};
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
/// // Both patterns work - state persists across renders!
///
/// // Option 1: Create inside closure (like React)
/// render(|| Element::component(Counter)).await?;
///
/// // Option 2: Create outside and clone (also works)
/// let app = Element::component(Counter);
/// render(move || app.clone()).await?;
/// ```
pub trait Component: 'static {
    /// Renders the component to the given buffer within the specified area.
    ///
    /// This method is called during the render phase. Hooks can be used within
    /// this method as the fiber context is automatically set up.
    fn render(&self, area: Rect, buffer: &mut Buffer);
}

/// Internal wrapper that handles fiber management when rendering a Component.
///
/// This wrapper is responsible for:
/// - Getting or creating a fiber for the component based on its stable ID
/// - Setting up the fiber context before rendering
/// - Providing ComponentArea context to child components
/// - Restoring the previous fiber context after rendering
///
/// Uses Rc internally so components don't need to implement Clone.
///
/// # Stable Component Identity
///
/// Component IDs are generated based on TypeId + position in the render tree,
/// similar to React's reconciliation algorithm. This means you can create
/// components inside the render closure and state will persist:
///
/// ```rust,ignore
/// // BOTH work correctly - state persists across renders!
///
/// // Option 1: Create inside closure (like React)
/// render(|| Element::component(Counter)).await?;
///
/// // Option 2: Create outside and clone (also works)
/// let app = Element::component(Counter);
/// render(move || app.clone()).await?;
/// ```
#[doc(hidden)]
pub struct ComponentWrapper<C: Component> {
    component: Rc<C>,
}

impl<C: Component> ComponentWrapper<C> {
    /// Create a new wrapper for a Component.
    pub fn new(component: C) -> Self {
        Self {
            component: Rc::new(component),
        }
    }

    /// Render the component with proper fiber management.
    pub fn render_with_fiber(&self, area: Rect, buffer: &mut Buffer) {
        use crate::context_stack::push_context;
        use crate::fiber_tree::with_fiber_tree_mut;

        // Generate stable ID based on TypeId + position (like React's reconciliation)
        let stable_id = generate_stable_component_id::<C>();

        // Get or create fiber for this component
        let fiber_id =
            with_fiber_tree_mut(|tree| tree.get_or_create_fiber_by_component_id(stable_id))
                .expect("render_with_fiber must be called within a render context");

        // Begin render for this fiber
        with_fiber_tree_mut(|tree| {
            tree.begin_render(fiber_id);
        });

        // Push ComponentArea context
        push_context(fiber_id, ComponentArea(area));

        // Call the component's render method
        self.component.render(area, buffer);

        // End render for this fiber
        with_fiber_tree_mut(|tree| {
            tree.end_render();
        });
    }
}

impl<C: Component> Clone for ComponentWrapper<C> {
    fn clone(&self) -> Self {
        Self {
            component: Rc::clone(&self.component),
        }
    }
}

// Implement RenderableComponent trait from internal element module
impl<C: Component> crate::element::RenderableComponent for ComponentWrapper<C> {
    fn render_with_fiber(&self, area: Rect, buffer: &mut Buffer) {
        ComponentWrapper::render_with_fiber(self, area, buffer)
    }

    fn clone_box(&self) -> Box<dyn crate::element::RenderableComponent> {
        Box::new(self.clone())
    }

    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentWrapper")
            .field("component_type", &std::any::type_name::<C>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestComponent;

    impl Component for TestComponent {
        fn render(&self, _area: Rect, _buffer: &mut Buffer) {}
    }

    struct AnotherComponent;

    impl Component for AnotherComponent {
        fn render(&self, _area: Rect, _buffer: &mut Buffer) {}
    }

    #[test]
    fn test_position_counter_reset() {
        // Reset counter
        reset_component_position_counter();

        // Generate some IDs
        let id1 = generate_stable_component_id::<TestComponent>();
        let id2 = generate_stable_component_id::<TestComponent>();

        // IDs should be different (different positions)
        assert_ne!(id1, id2);

        // Reset counter
        reset_component_position_counter();

        // Generate IDs again - should match the first batch
        let id1_again = generate_stable_component_id::<TestComponent>();
        let id2_again = generate_stable_component_id::<TestComponent>();

        assert_eq!(
            id1, id1_again,
            "Same type at same position should have same ID"
        );
        assert_eq!(
            id2, id2_again,
            "Same type at same position should have same ID"
        );
    }

    #[test]
    fn test_different_types_different_ids() {
        reset_component_position_counter();

        let id1 = generate_stable_component_id::<TestComponent>();

        reset_component_position_counter();

        let id2 = generate_stable_component_id::<AnotherComponent>();

        // Different types at same position should have different IDs
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_same_type_different_positions() {
        reset_component_position_counter();

        let id1 = generate_stable_component_id::<TestComponent>();
        let id2 = generate_stable_component_id::<TestComponent>();
        let id3 = generate_stable_component_id::<TestComponent>();

        // Same type at different positions should have different IDs
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_simulated_render_frames() {
        // Simulate multiple render frames where component is created inside closure
        let mut ids_per_frame: Vec<Vec<u64>> = Vec::new();

        for _ in 0..5 {
            // Reset at start of each frame (like runtime does)
            reset_component_position_counter();

            // Simulate creating components inside render closure
            let id1 = generate_stable_component_id::<TestComponent>();
            let id2 = generate_stable_component_id::<AnotherComponent>();

            ids_per_frame.push(vec![id1, id2]);
        }

        // All frames should have the same IDs (stable across renders)
        for i in 1..ids_per_frame.len() {
            assert_eq!(
                ids_per_frame[0], ids_per_frame[i],
                "Component IDs should be stable across render frames"
            );
        }
    }

    #[test]
    fn test_wrapper_clone() {
        let wrapper1 = ComponentWrapper::new(TestComponent);
        let wrapper2 = wrapper1.clone();

        // Both should share the same Rc
        assert!(Rc::ptr_eq(&wrapper1.component, &wrapper2.component));
    }
}
