//! Context stack with proper lifecycle management.
//!
//! Provides scoped context values that are automatically cleaned up
//! when their provider fiber unmounts. This implements React-like context
//! semantics where nested providers shadow parent values.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::context_stack::{push_context, get_context, pop_context_for_fiber};
//! use reratui_fiber::FiberId;
//!
//! // Provider pushes a value
//! push_context(FiberId(1), "theme-dark".to_string());
//!
//! // Consumer gets the value
//! let theme = get_context::<String>().unwrap();
//! assert_eq!(theme, "theme-dark");
//!
//! // Nested provider shadows the parent
//! push_context(FiberId(2), "theme-light".to_string());
//! let theme = get_context::<String>().unwrap();
//! assert_eq!(theme, "theme-light");
//!
//! // When inner provider unmounts, outer value is restored
//! pop_context_for_fiber(FiberId(2));
//! let theme = get_context::<String>().unwrap();
//! assert_eq!(theme, "theme-dark");
//! ```

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::fiber::FiberId;

thread_local! {
    /// Thread-local context stack
    static CONTEXT_STACK: RefCell<ContextStack> = RefCell::new(ContextStack::new());
}

/// Type alias for the provider stack to reduce complexity
type ProviderStack = Vec<(FiberId, usize, Box<dyn Any + Send + Sync>)>;

/// Context stack with proper lifecycle management.
///
/// Each context type has its own stack of values, where each value is
/// associated with the fiber that provided it. When a fiber unmounts,
/// all its context values are automatically removed.
pub struct ContextStack {
    /// Stack of values per type, with fiber ownership
    providers: HashMap<TypeId, ProviderStack>,
}

impl ContextStack {
    /// Create a new empty context stack
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Push a context value (called during render).
    ///
    /// The value is associated with the given fiber and will be automatically
    /// removed when `pop_for_fiber` is called for that fiber.
    pub fn push<T: Send + Sync + 'static>(&mut self, fiber_id: FiberId, value: T) {
        self.push_with_slot(fiber_id, usize::MAX, value);
    }

    pub fn push_with_slot<T: Send + Sync + 'static>(
        &mut self,
        fiber_id: FiberId,
        provider_slot: usize,
        value: T,
    ) {
        let type_id = TypeId::of::<T>();
        let stack = self.providers.entry(type_id).or_default();
        stack.retain(|(id, slot, _)| !(*id == fiber_id && *slot == provider_slot));
        stack.push((fiber_id, provider_slot, Box::new(value)));
    }

    /// Pop all context values for a fiber (called on unmount).
    ///
    /// This removes all context values that were pushed by the given fiber,
    /// restoring any shadowed values from parent providers.
    pub fn pop_for_fiber(&mut self, fiber_id: FiberId) {
        for stack in self.providers.values_mut() {
            stack.retain(|(id, _, _)| *id != fiber_id);
        }
    }

    /// Get the nearest context value of type T.
    ///
    /// Returns the most recently pushed value of type T, which corresponds
    /// to the nearest ancestor provider in the component tree.
    pub fn get<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        let type_id = TypeId::of::<T>();
        self.providers
            .get(&type_id)?
            .last()
            .and_then(|(_, _, value)| value.downcast_ref::<T>())
            .cloned()
    }

    /// Check if a context of type T exists.
    pub fn has<T: 'static>(&self) -> bool {
        let type_id = TypeId::of::<T>();
        self.providers
            .get(&type_id)
            .map(|stack| !stack.is_empty())
            .unwrap_or(false)
    }

    /// Clear all context values.
    pub fn clear(&mut self) {
        self.providers.clear();
    }

    /// Get the number of providers for a given type.
    #[cfg(test)]
    pub fn provider_count<T: 'static>(&self) -> usize {
        let type_id = TypeId::of::<T>();
        self.providers
            .get(&type_id)
            .map(|stack| stack.len())
            .unwrap_or(0)
    }
}

impl Default for ContextStack {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Thread-local API functions
// ============================================================================

/// Push a context value to the thread-local stack.
///
/// The value is associated with the given fiber and will be automatically
/// removed when `pop_context_for_fiber` is called for that fiber.
pub fn push_context<T: Send + Sync + 'static>(fiber_id: FiberId, value: T) {
    push_context_with_slot(fiber_id, usize::MAX, value);
}

/// Push a context value with a stable provider slot.
///
/// Re-renders of the same fiber/provider slot replace the previous value instead
/// of growing the stack with stale snapshots.
pub fn push_context_with_slot<T: Send + Sync + 'static>(
    fiber_id: FiberId,
    provider_slot: usize,
    value: T,
) {
    CONTEXT_STACK.with(|stack| {
        stack.borrow_mut().push_with_slot(fiber_id, provider_slot, value);
    });
}

/// Get the nearest context value from the thread-local stack.
///
/// Returns the most recently pushed value of type T, which corresponds
/// to the nearest ancestor provider in the component tree.
pub fn get_context<T: Clone + Send + Sync + 'static>() -> Option<T> {
    CONTEXT_STACK.with(|stack| stack.borrow().get::<T>())
}

/// Pop all context values for a fiber from the thread-local stack.
///
/// This removes all context values that were pushed by the given fiber,
/// restoring any shadowed values from parent providers.
pub fn pop_context_for_fiber(fiber_id: FiberId) {
    CONTEXT_STACK.with(|stack| {
        stack.borrow_mut().pop_for_fiber(fiber_id);
    });
}

/// Check if a context of type T exists in the thread-local stack.
pub fn has_context<T: 'static>() -> bool {
    CONTEXT_STACK.with(|stack| stack.borrow().has::<T>())
}

/// Clear all context values from the thread-local stack.
pub fn clear_context_stack() {
    CONTEXT_STACK.with(|stack| {
        stack.borrow_mut().clear();
    });
}

/// Execute a closure with the thread-local context stack.
pub fn with_context_stack<R, F: FnOnce(&ContextStack) -> R>(f: F) -> R {
    CONTEXT_STACK.with(|stack| f(&stack.borrow()))
}

/// Execute a closure with mutable access to the thread-local context stack.
pub fn with_context_stack_mut<R, F: FnOnce(&mut ContextStack) -> R>(f: F) -> R {
    CONTEXT_STACK.with(|stack| f(&mut stack.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_stack_creation() {
        let stack = ContextStack::new();
        assert!(stack.providers.is_empty());
    }

    #[test]
    fn test_push_and_get() {
        let mut stack = ContextStack::new();
        let fiber_id = FiberId(1);

        stack.push(fiber_id, 42i32);
        assert_eq!(stack.get::<i32>(), Some(42));
    }

    #[test]
    fn test_nested_providers_shadow() {
        let mut stack = ContextStack::new();
        let fiber1 = FiberId(1);
        let fiber2 = FiberId(2);

        stack.push(fiber1, "outer".to_string());
        assert_eq!(stack.get::<String>(), Some("outer".to_string()));

        stack.push(fiber2, "inner".to_string());
        assert_eq!(stack.get::<String>(), Some("inner".to_string()));
    }

    #[test]
    fn test_pop_for_fiber() {
        let mut stack = ContextStack::new();
        let fiber1 = FiberId(1);
        let fiber2 = FiberId(2);

        stack.push(fiber1, "outer".to_string());
        stack.push(fiber2, "inner".to_string());

        stack.pop_for_fiber(fiber2);
        assert_eq!(stack.get::<String>(), Some("outer".to_string()));

        stack.pop_for_fiber(fiber1);
        assert_eq!(stack.get::<String>(), None);
    }

    #[test]
    fn test_multiple_types() {
        let mut stack = ContextStack::new();
        let fiber_id = FiberId(1);

        stack.push(fiber_id, 42i32);
        stack.push(fiber_id, "hello".to_string());

        assert_eq!(stack.get::<i32>(), Some(42));
        assert_eq!(stack.get::<String>(), Some("hello".to_string()));
    }

    #[test]
    fn test_has_context() {
        let mut stack = ContextStack::new();
        let fiber_id = FiberId(1);

        assert!(!stack.has::<i32>());

        stack.push(fiber_id, 42i32);
        assert!(stack.has::<i32>());
        assert!(!stack.has::<String>());
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let stack = ContextStack::new();
        assert_eq!(stack.get::<i32>(), None);
    }

    #[test]
    fn test_clear() {
        let mut stack = ContextStack::new();
        let fiber_id = FiberId(1);

        stack.push(fiber_id, 42i32);
        stack.push(fiber_id, "hello".to_string());

        assert!(stack.has::<i32>());
        assert!(stack.has::<String>());

        stack.clear();

        assert!(!stack.has::<i32>());
        assert!(!stack.has::<String>());
    }

    #[test]
    fn test_provider_count() {
        let mut stack = ContextStack::new();
        let fiber1 = FiberId(1);
        let fiber2 = FiberId(2);

        assert_eq!(stack.provider_count::<i32>(), 0);

        stack.push(fiber1, 1i32);
        assert_eq!(stack.provider_count::<i32>(), 1);

        stack.push(fiber2, 2i32);
        assert_eq!(stack.provider_count::<i32>(), 2);

        stack.pop_for_fiber(fiber2);
        assert_eq!(stack.provider_count::<i32>(), 1);
    }

    #[test]
    fn test_thread_local_push_and_get() {
        clear_context_stack();

        let fiber_id = FiberId(1);
        push_context(fiber_id, 42i32);

        assert_eq!(get_context::<i32>(), Some(42));
        assert!(has_context::<i32>());

        clear_context_stack();
    }

    #[test]
    fn test_thread_local_pop_for_fiber() {
        clear_context_stack();

        let fiber1 = FiberId(1);
        let fiber2 = FiberId(2);

        push_context(fiber1, "outer".to_string());
        push_context(fiber2, "inner".to_string());

        assert_eq!(get_context::<String>(), Some("inner".to_string()));

        pop_context_for_fiber(fiber2);
        assert_eq!(get_context::<String>(), Some("outer".to_string()));

        clear_context_stack();
    }

    #[test]
    fn test_with_context_stack() {
        clear_context_stack();

        let fiber_id = FiberId(1);
        push_context(fiber_id, 42i32);

        let has_int = with_context_stack(|stack| stack.has::<i32>());
        assert!(has_int);

        clear_context_stack();
    }

    #[test]
    fn test_with_context_stack_mut() {
        clear_context_stack();

        with_context_stack_mut(|stack| {
            stack.push(FiberId(1), 42i32);
        });

        assert!(has_context::<i32>());

        clear_context_stack();
    }

    #[test]
    fn test_default_impl() {
        let stack: ContextStack = Default::default();
        assert!(!stack.has::<i32>());
    }

    #[test]
    fn test_deeply_nested_providers() {
        let mut stack = ContextStack::new();

        // Simulate a deep component tree
        for i in 1..=5 {
            stack.push(FiberId(i), format!("level-{}", i));
        }

        // Should get the innermost value
        assert_eq!(stack.get::<String>(), Some("level-5".to_string()));

        // Pop from innermost to outermost
        for i in (1..=5).rev() {
            assert_eq!(stack.get::<String>(), Some(format!("level-{}", i)));
            stack.pop_for_fiber(FiberId(i));
        }

        assert_eq!(stack.get::<String>(), None);
    }

    #[test]
    fn test_multiple_contexts_same_fiber() {
        let mut stack = ContextStack::new();
        let fiber_id = FiberId(1);

        // A single fiber can provide multiple context types
        stack.push(fiber_id, 42i32);
        stack.push(fiber_id, "theme".to_string());
        stack.push(fiber_id, true);

        assert_eq!(stack.get::<i32>(), Some(42));
        assert_eq!(stack.get::<String>(), Some("theme".to_string()));
        assert_eq!(stack.get::<bool>(), Some(true));

        // Popping the fiber removes all its contexts
        stack.pop_for_fiber(fiber_id);

        assert_eq!(stack.get::<i32>(), None);
        assert_eq!(stack.get::<String>(), None);
        assert_eq!(stack.get::<bool>(), None);
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Property 11: Context Stack Shadowing**
        /// **Validates: Requirements 6.3, 6.5**
        ///
        /// For any nested context providers of the same type, use_context SHALL
        /// return the value from the innermost (most recently pushed) provider.
        #[test]
        fn prop_context_stack_shadowing(
            outer_value in any::<i32>(),
            inner_value in any::<i32>(),
            deepest_value in any::<i32>()
        ) {
            // Ensure values are different for clear testing
            prop_assume!(outer_value != inner_value);
            prop_assume!(inner_value != deepest_value);
            prop_assume!(outer_value != deepest_value);

            clear_context_stack();

            let outer_fiber = FiberId(1);
            let inner_fiber = FiberId(2);
            let deepest_fiber = FiberId(3);

            // Push outer context
            push_context(outer_fiber, outer_value);

            // Property: Should get outer value
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, Some(outer_value),
                "Should get outer value when only outer provider exists");

            // Push inner context (shadows outer)
            push_context(inner_fiber, inner_value);

            // Property: Should get inner value (shadowing outer)
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, Some(inner_value),
                "Should get inner value when inner provider shadows outer");

            // Push deepest context (shadows both)
            push_context(deepest_fiber, deepest_value);

            // Property: Should get deepest value (shadowing both)
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, Some(deepest_value),
                "Should get deepest value when deepest provider shadows all");

            // Pop deepest - should restore inner
            pop_context_for_fiber(deepest_fiber);
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, Some(inner_value),
                "Should restore inner value after deepest unmounts");

            // Pop inner - should restore outer
            pop_context_for_fiber(inner_fiber);
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, Some(outer_value),
                "Should restore outer value after inner unmounts");

            // Pop outer - should have no context
            pop_context_for_fiber(outer_fiber);
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, None,
                "Should have no context after all providers unmount");
        }

        /// **Property 12: Context Cleanup on Unmount**
        /// **Validates: Requirements 6.4**
        ///
        /// For any fiber that provided context values, when the fiber unmounts,
        /// all its context values SHALL be removed from the stack, restoring any
        /// shadowed values.
        #[test]
        fn prop_context_cleanup_on_unmount(
            values in prop::collection::vec(any::<i32>(), 1..10)
        ) {
            clear_context_stack();

            // Create fibers and push contexts
            let fiber_ids: Vec<FiberId> = (0..values.len())
                .map(|i| FiberId(i as u64))
                .collect();

            for (i, &value) in values.iter().enumerate() {
                push_context(fiber_ids[i], value);
            }

            // Property: Should get the last value (most recent provider)
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, Some(*values.last().unwrap()),
                "Should get last value before any unmounts");

            // Unmount fibers in reverse order
            for i in (0..fiber_ids.len()).rev() {
                pop_context_for_fiber(fiber_ids[i]);

                // Property: After unmounting, should get the previous value (if any)
                let expected = if i > 0 {
                    Some(values[i - 1])
                } else {
                    None
                };
                let retrieved = get_context::<i32>();
                prop_assert_eq!(retrieved, expected,
                    "After unmounting fiber {}, should get correct value", i);
            }

            // Property: All contexts should be cleaned up
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, None,
                "All contexts should be cleaned up after all unmounts");
        }

        /// **Property: Multiple context types are independent**
        /// **Validates: Requirement 6.3**
        ///
        /// Different context types should not interfere with each other.
        #[test]
        fn prop_multiple_context_types_independent(
            int_value in any::<i32>(),
            string_value in any::<String>(),
            bool_value in any::<bool>()
        ) {
            clear_context_stack();

            let fiber1 = FiberId(1);
            let fiber2 = FiberId(2);
            let fiber3 = FiberId(3);

            // Push different types
            push_context(fiber1, int_value);
            push_context(fiber2, string_value.clone());
            push_context(fiber3, bool_value);

            // Property: Each type should be retrievable independently
            let retrieved_int = get_context::<i32>();
            prop_assert_eq!(retrieved_int, Some(int_value),
                "Should get correct i32 value");

            let retrieved_string = get_context::<String>();
            prop_assert_eq!(retrieved_string, Some(string_value.clone()),
                "Should get correct String value");

            let retrieved_bool = get_context::<bool>();
            prop_assert_eq!(retrieved_bool, Some(bool_value),
                "Should get correct bool value");

            // Pop one type - others should remain
            pop_context_for_fiber(fiber2);

            let retrieved_int = get_context::<i32>();
            prop_assert_eq!(retrieved_int, Some(int_value),
                "i32 should still be available after String unmount");

            let retrieved_string = get_context::<String>();
            prop_assert_eq!(retrieved_string, None,
                "String should be gone after unmount");

            let retrieved_bool = get_context::<bool>();
            prop_assert_eq!(retrieved_bool, Some(bool_value),
                "bool should still be available after String unmount");
        }

        /// **Property: Fiber can provide multiple context types**
        /// **Validates: Requirement 6.1**
        ///
        /// A single fiber should be able to provide multiple context types,
        /// and all should be cleaned up when the fiber unmounts.
        #[test]
        fn prop_fiber_provides_multiple_types(
            int_value in any::<i32>(),
            string_value in any::<String>()
        ) {
            clear_context_stack();

            let fiber = FiberId(1);

            // Single fiber provides multiple types
            push_context(fiber, int_value);
            push_context(fiber, string_value.clone());

            // Property: Both types should be available
            let retrieved_int = get_context::<i32>();
            prop_assert_eq!(retrieved_int, Some(int_value),
                "i32 should be available");

            let retrieved_string = get_context::<String>();
            prop_assert_eq!(retrieved_string, Some(string_value),
                "String should be available");

            // Unmount the fiber
            pop_context_for_fiber(fiber);

            // Property: Both types should be cleaned up
            let retrieved_int = get_context::<i32>();
            prop_assert_eq!(retrieved_int, None,
                "i32 should be cleaned up after fiber unmount");

            let retrieved_string = get_context::<String>();
            prop_assert_eq!(retrieved_string, None,
                "String should be cleaned up after fiber unmount");
        }

        /// **Property: Context shadowing works with same fiber**
        /// **Validates: Requirement 6.5**
        ///
        /// If a fiber pushes multiple values of the same type, the last one
        /// should be the active one.
        #[test]
        fn prop_same_fiber_multiple_values(
            first_value in any::<i32>(),
            second_value in any::<i32>()
        ) {
            prop_assume!(first_value != second_value);

            clear_context_stack();

            let fiber = FiberId(1);

            // Push first value
            push_context(fiber, first_value);
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, Some(first_value),
                "Should get first value");

            // Push second value from same fiber
            push_context(fiber, second_value);
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, Some(second_value),
                "Should get second value (shadows first)");

            // Unmount fiber - both values should be removed
            pop_context_for_fiber(fiber);
            let retrieved = get_context::<i32>();
            prop_assert_eq!(retrieved, None,
                "Both values should be removed after fiber unmount");
        }
    }
}
