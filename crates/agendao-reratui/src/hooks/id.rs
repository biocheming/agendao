//! Unique ID generation hook.
//!
//! This module provides a hook for generating stable unique identifiers
//! that persist across renders and are unique across all component instances.
//!
//! # Example
//!
//! ```rust,ignore
//! use reratui_fiber::hooks::use_id;
//!
//! #[component]
//! fn AccessibleInput(label: String) -> Element {
//!     let id = use_id();
//!     
//!     rsx! {
//!         <label for={id.clone()}>{label}</label>
//!         <input id={id} />
//!     }
//! }
//! ```

use crate::fiber_tree::with_current_fiber;

/// Internal storage for the generated ID.
#[derive(Clone)]
struct IdStorage {
    id: String,
}

/// React-style useId for generating stable unique identifiers.
///
/// Returns a unique identifier that:
/// - Is stable across renders for the same component instance
/// - Is unique across all component instances
/// - Can be used for accessibility attributes (id, for, aria-labelledby, etc.)
///
/// # How It Works
///
/// 1. On first render, generates a unique ID using fiber_id and hook_index
/// 2. On subsequent renders, returns the same cached ID
/// 3. The ID format is `:r{fiber_id}h{hook_index}:` to ensure uniqueness
///
/// # Returns
///
/// A `String` containing the unique identifier.
///
/// # Example
///
/// ```rust,ignore
/// use reratui_fiber::hooks::use_id;
///
/// #[component]
/// fn FormField(label: String) -> Element {
///     let id = use_id();
///     let error_id = format!("{}-error", id);
///     
///     rsx! {
///         <label for={id.clone()}>{label}</label>
///         <input id={id} aria-describedby={error_id.clone()} />
///         <span id={error_id}>Error message</span>
///     }
/// }
/// ```
///
/// # Panics
///
/// Panics if called outside of a component render context (no current fiber).
pub fn use_id() -> String {
    with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();
        let fiber_id = fiber.id;

        // Check if ID already exists
        let existing_storage: Option<IdStorage> = fiber.get_hook(hook_index);

        if let Some(storage) = existing_storage {
            // Return cached ID
            storage.id
        } else {
            // Generate new unique ID using fiber_id and hook_index
            // Format: :r{fiber_id}h{hook_index}: (similar to React's format)
            let id = format!(":r{}h{}:", fiber_id.0, hook_index);

            // Store for future renders
            let storage = IdStorage { id: id.clone() };
            fiber.set_hook(hook_index, storage);

            id
        }
    })
    .expect("use_id must be called within a component render context")
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
    fn test_use_id_basic() {
        let _fiber_id = setup_test_fiber();

        let id = use_id();

        // ID should be non-empty and follow the format
        assert!(!id.is_empty());
        assert!(id.starts_with(":r"));
        assert!(id.ends_with(':'));

        cleanup_test();
    }

    #[test]
    fn test_use_id_stable_across_renders() {
        let fiber_id = setup_test_fiber();

        // First render
        let id1 = use_id();

        // Simulate re-render
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            tree.begin_render(fiber_id);
        });

        // Second render
        let id2 = use_id();

        // Should be the same ID
        assert_eq!(id1, id2, "ID should be stable across renders");

        cleanup_test();
    }

    #[test]
    fn test_use_id_unique_per_hook() {
        let _fiber_id = setup_test_fiber();

        // Multiple IDs in same component
        let id1 = use_id();
        let id2 = use_id();
        let id3 = use_id();

        // All should be different
        assert_ne!(id1, id2, "Different hooks should have different IDs");
        assert_ne!(id2, id3, "Different hooks should have different IDs");
        assert_ne!(id1, id3, "Different hooks should have different IDs");

        cleanup_test();
    }

    #[test]
    fn test_use_id_unique_per_fiber() {
        // Create first fiber
        let mut tree = FiberTree::new();
        let fiber_id1 = tree.mount(None, None);
        tree.begin_render(fiber_id1);
        set_fiber_tree(tree);

        let id1 = use_id();

        // End first fiber render and create second fiber
        crate::fiber_tree::with_fiber_tree_mut(|tree| {
            tree.end_render();
            let fiber_id2 = tree.mount(None, None);
            tree.begin_render(fiber_id2);
        });

        let id2 = use_id();

        // IDs from different fibers should be different
        assert_ne!(id1, id2, "Different fibers should have different IDs");

        cleanup_test();
    }

    #[test]
    fn test_use_id_format() {
        let fiber_id = setup_test_fiber();

        let id = use_id();

        // Should contain fiber_id and hook_index
        let expected_prefix = format!(":r{}h", fiber_id.0);
        assert!(
            id.starts_with(&expected_prefix),
            "ID should start with :r{{fiber_id}}h"
        );

        cleanup_test();
    }

    #[test]
    #[should_panic(expected = "use_id must be called within a component render context")]
    fn test_use_id_panics_outside_render() {
        clear_fiber_tree();
        let _ = use_id();
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use crate::fiber::FiberId;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};
    use proptest::prelude::*;
    use std::collections::HashSet;

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

    // Property 14: ID stability and uniqueness
    //
    // For any component using use_id, the returned ID SHALL be identical
    // across all renders of that component instance, AND different from IDs
    // of other component instances.
    //
    // Validates: Requirements 7.1, 7.2, 7.3
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_id_stability_across_renders(num_renders in 2usize..20) {
            let fiber_id = setup_test_fiber();

            // Get ID on first render
            let first_id = use_id();

            // Verify ID is stable across multiple re-renders
            for render_num in 1..num_renders {
                // Simulate re-render
                crate::fiber_tree::with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                let current_id = use_id();
                prop_assert_eq!(
                    &first_id, &current_id,
                    "ID should be stable across renders (render {})", render_num
                );
            }

            cleanup_test();
        }

        #[test]
        fn prop_id_uniqueness_across_hooks(num_hooks in 2usize..20) {
            let _fiber_id = setup_test_fiber();

            // Generate multiple IDs in the same component
            let mut ids = HashSet::new();
            for _ in 0..num_hooks {
                let id = use_id();
                prop_assert!(
                    ids.insert(id.clone()),
                    "Each hook should generate a unique ID"
                );
            }

            // All IDs should be unique
            prop_assert_eq!(ids.len(), num_hooks, "All IDs should be unique");

            cleanup_test();
        }

        #[test]
        fn prop_id_uniqueness_across_fibers(num_fibers in 2usize..10) {
            // Create a single tree with multiple fibers
            let mut tree = FiberTree::new();
            let mut all_ids = HashSet::new();
            let mut fiber_ids = Vec::new();

            // Mount all fibers first
            for _ in 0..num_fibers {
                let fiber_id = tree.mount(None, None);
                fiber_ids.push(fiber_id);
            }

            set_fiber_tree(tree);

            // Generate an ID from each fiber
            for (fiber_num, &fiber_id) in fiber_ids.iter().enumerate() {
                crate::fiber_tree::with_fiber_tree_mut(|tree| {
                    tree.begin_render(fiber_id);
                });

                let id = use_id();
                prop_assert!(
                    all_ids.insert(id.clone()),
                    "Fiber {} should have unique ID", fiber_num
                );

                crate::fiber_tree::with_fiber_tree_mut(|tree| {
                    tree.end_render();
                });
            }

            // All IDs from different fibers should be unique
            prop_assert_eq!(all_ids.len(), num_fibers, "All fiber IDs should be unique");

            cleanup_test();
        }

        #[test]
        fn prop_id_stability_with_multiple_hooks(num_hooks in 2usize..10, num_renders in 2usize..10) {
            let fiber_id = setup_test_fiber();

            // Get IDs on first render
            let mut first_render_ids = Vec::new();
            for _ in 0..num_hooks {
                first_render_ids.push(use_id());
            }

            // Verify all IDs are stable across multiple re-renders
            for render_num in 1..num_renders {
                // Simulate re-render
                crate::fiber_tree::with_fiber_tree_mut(|tree| {
                    tree.end_render();
                    tree.begin_render(fiber_id);
                });

                // Get IDs again
                for (hook_idx, first_id) in first_render_ids.iter().enumerate() {
                    let current_id = use_id();
                    prop_assert_eq!(
                        first_id, &current_id,
                        "Hook {} ID should be stable across renders (render {})",
                        hook_idx, render_num
                    );
                }
            }

            cleanup_test();
        }

        #[test]
        fn prop_id_format_valid(num_hooks in 1usize..10) {
            let _fiber_id = setup_test_fiber();

            for _ in 0..num_hooks {
                let id = use_id();

                // Verify format: :r{fiber_id}h{hook_index}:
                prop_assert!(id.starts_with(":r"), "ID should start with :r");
                prop_assert!(id.ends_with(':'), "ID should end with :");
                prop_assert!(id.contains('h'), "ID should contain 'h' separator");

                // Verify it can be parsed (contains valid numbers)
                let inner = &id[2..id.len()-1]; // Remove :r and :
                let parts: Vec<&str> = inner.split('h').collect();
                prop_assert_eq!(parts.len(), 2, "ID should have exactly one 'h' separator");
                prop_assert!(parts[0].parse::<u64>().is_ok(), "Fiber ID part should be numeric");
                prop_assert!(parts[1].parse::<usize>().is_ok(), "Hook index part should be numeric");
            }

            cleanup_test();
        }
    }
}
