//! Fiber tree management for tracking all mounted component instances.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::fiber::{Fiber, FiberId};

thread_local! {
    /// Thread-local fiber tree for the current render context
    static FIBER_TREE: RefCell<Option<FiberTree>> = const { RefCell::new(None) };
}

/// Global fiber tree that tracks all mounted component instances
pub struct FiberTree {
    /// All mounted fibers by ID
    pub(crate) fibers: HashMap<FiberId, Fiber>,
    /// Root fiber ID
    pub(crate) root: Option<FiberId>,
    /// Stack of currently rendering fibers (for nested renders)
    pub(crate) render_stack: Vec<FiberId>,
    /// Next available fiber ID
    next_id: AtomicU64,
    /// Fibers scheduled for unmount
    pub(crate) pending_unmount: Vec<FiberId>,
    /// Maps component IDs to their fiber IDs for Component tracking
    component_id_to_fiber: HashMap<u64, FiberId>,
    /// Maps fiber IDs back to component IDs for cleanup
    fiber_to_component_id: HashMap<FiberId, u64>,
    /// Fibers seen during the current render pass (for unmount detection)
    seen_this_render: HashSet<FiberId>,
}

impl FiberTree {
    /// Create a new empty fiber tree
    pub fn new() -> Self {
        Self {
            fibers: HashMap::new(),
            root: None,
            render_stack: Vec::new(),
            next_id: AtomicU64::new(1),
            pending_unmount: Vec::new(),
            component_id_to_fiber: HashMap::new(),
            fiber_to_component_id: HashMap::new(),
            seen_this_render: HashSet::new(),
        }
    }

    /// Mount a new fiber, returning its ID
    pub fn mount(&mut self, parent: Option<FiberId>, key: Option<String>) -> FiberId {
        let id = FiberId(self.next_id.fetch_add(1, Ordering::SeqCst));
        let fiber = Fiber::new(id, parent, key);
        self.fibers.insert(id, fiber);

        // Add to parent's children
        if let Some(parent_id) = parent
            && let Some(parent_fiber) = self.fibers.get_mut(&parent_id)
        {
            parent_fiber.children.push(id);
        }

        // Set as root if no parent
        if parent.is_none() && self.root.is_none() {
            self.root = Some(id);
        }

        id
    }

    /// Get or create a fiber for a Component by its component ID.
    ///
    /// If a fiber already exists for this component ID, returns the existing fiber ID.
    /// Otherwise, creates a new fiber and associates it with the component ID.
    /// The fiber is marked as seen for this render pass.
    pub fn get_or_create_fiber_by_component_id(&mut self, component_id: u64) -> FiberId {
        if let Some(&fiber_id) = self.component_id_to_fiber.get(&component_id) {
            // Existing fiber - mark as seen
            self.seen_this_render.insert(fiber_id);
            fiber_id
        } else {
            // Create new fiber
            let parent = self.current_fiber();
            let fiber_id = self.mount(parent, None);

            // Associate with component ID
            self.component_id_to_fiber.insert(component_id, fiber_id);
            self.fiber_to_component_id.insert(fiber_id, component_id);

            // Mark as seen
            self.seen_this_render.insert(fiber_id);

            fiber_id
        }
    }

    /// Mark fibers not seen this render for unmount and clear the seen set.
    ///
    /// This should be called after the render phase completes. Any fibers that
    /// were associated with Component instances but not rendered this pass
    /// will be scheduled for unmount.
    pub fn mark_unseen_for_unmount(&mut self) {
        // Find fibers that have component IDs but weren't seen this render
        let unseen: Vec<FiberId> = self
            .fiber_to_component_id
            .keys()
            .filter(|fiber_id| !self.seen_this_render.contains(fiber_id))
            .copied()
            .collect();

        // Schedule them for unmount
        for fiber_id in unseen {
            self.schedule_unmount(fiber_id);
        }

        // Clear the seen set for the next render
        self.seen_this_render.clear();
    }

    /// Clean up component ID mappings when a fiber is removed.
    ///
    /// This should be called when a fiber is unmounted to ensure the
    /// component ID mappings are properly cleaned up.
    pub fn cleanup_component_id_mapping(&mut self, fiber_id: FiberId) {
        if let Some(component_id) = self.fiber_to_component_id.remove(&fiber_id) {
            self.component_id_to_fiber.remove(&component_id);
        }
    }

    /// Called when a component starts rendering
    pub fn begin_render(&mut self, id: FiberId) {
        self.render_stack.push(id);
        if let Some(fiber) = self.fibers.get_mut(&id) {
            fiber.hook_index = 0; // Reset hook index for this component
        }
    }

    /// Called when a component finishes rendering
    pub fn end_render(&mut self) {
        if let Some(fiber_id) = self.render_stack.last().copied() {
            #[cfg(debug_assertions)]
            if let Some(fiber) = self.fibers.get(&fiber_id) {
                fiber.check_hook_order();
            }
        }
        self.render_stack.pop();
    }

    /// Get the currently rendering fiber's ID
    pub fn current_fiber(&self) -> Option<FiberId> {
        self.render_stack.last().copied()
    }

    /// Schedule a fiber for unmount (cleanup runs in effect phase)
    pub fn schedule_unmount(&mut self, id: FiberId) {
        self.pending_unmount.push(id);
    }

    /// Prepare all fibers for a new render pass (reset hook indices)
    pub fn prepare_for_render(&mut self) {
        for fiber in self.fibers.values_mut() {
            fiber.reset_hook_index();
        }
    }

    /// Get a reference to a fiber by ID
    pub fn get(&self, id: FiberId) -> Option<&Fiber> {
        self.fibers.get(&id)
    }

    /// Get a mutable reference to a fiber by ID
    pub fn get_mut(&mut self, id: FiberId) -> Option<&mut Fiber> {
        self.fibers.get_mut(&id)
    }

    /// Remove a fiber from the tree
    pub fn remove(&mut self, id: FiberId) -> Option<Fiber> {
        // Remove from parent's children list
        if let Some(fiber) = self.fibers.get(&id)
            && let Some(parent_id) = fiber.parent
            && let Some(parent) = self.fibers.get_mut(&parent_id)
        {
            parent.children.retain(|&child_id| child_id != id);
        }

        // Clear root if this was the root
        if self.root == Some(id) {
            self.root = None;
        }

        // Clean up component ID mappings
        self.cleanup_component_id_mapping(id);

        self.fibers.remove(&id)
    }

    /// Get all dirty fibers that need re-rendering
    pub fn dirty_fibers(&self) -> HashSet<FiberId> {
        self.fibers
            .iter()
            .filter(|(_, fiber)| fiber.dirty)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Mark a fiber as dirty (needs re-render)
    pub fn mark_dirty(&mut self, id: FiberId) {
        if let Some(fiber) = self.fibers.get_mut(&id) {
            fiber.dirty = true;
        }
    }

    /// Mark a fiber as clean (rendered)
    pub fn mark_clean(&mut self, id: FiberId) {
        if let Some(fiber) = self.fibers.get_mut(&id) {
            fiber.dirty = false;
        }
    }

    /// Process all pending unmounts, cleaning up context and removing fibers.
    ///
    /// This should be called during the commit phase after rendering is complete.
    /// It performs the following for each pending unmount:
    /// 1. Queues all sync and async cleanups from the fiber
    /// 2. Pops all context values provided by the fiber
    /// 3. Removes the fiber from the tree
    ///
    /// Returns the list of fiber IDs that were unmounted.
    pub fn process_unmounts(&mut self) -> Vec<FiberId> {
        use crate::context_stack::pop_context_for_fiber;
        use crate::scheduler::effect_queue::{queue_async_cleanup, queue_cleanup};

        let unmounted: Vec<FiberId> = self.pending_unmount.drain(..).collect();

        for fiber_id in &unmounted {
            // Queue all cleanups from the fiber before removing it
            if let Some(fiber) = self.fibers.get_mut(fiber_id) {
                // Queue sync cleanups in reverse order (LIFO)
                let mut sync_cleanups: Vec<_> = fiber.cleanup_by_hook.drain().collect();
                sync_cleanups.sort_by(|a, b| b.0.cmp(&a.0)); // Sort by hook_index descending
                for (_, cleanup) in sync_cleanups {
                    queue_cleanup(cleanup);
                }

                // Queue async cleanups in reverse order (LIFO)
                let mut async_cleanups: Vec<_> = fiber.async_cleanup_by_hook.drain().collect();
                async_cleanups.sort_by(|a, b| b.0.cmp(&a.0)); // Sort by hook_index descending
                for (_, async_cleanup) in async_cleanups {
                    queue_async_cleanup(async_cleanup);
                }
            }

            // Clean up context values provided by this fiber
            pop_context_for_fiber(*fiber_id);

            // Remove the fiber from the tree
            self.remove(*fiber_id);
        }

        unmounted
    }

    /// Check if there are pending unmounts
    pub fn has_pending_unmounts(&self) -> bool {
        !self.pending_unmount.is_empty()
    }

    /// Get the number of pending unmounts
    pub fn pending_unmount_count(&self) -> usize {
        self.pending_unmount.len()
    }
}

impl Default for FiberTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Set the thread-local fiber tree
pub fn set_fiber_tree(tree: FiberTree) {
    FIBER_TREE.with(|t| {
        *t.borrow_mut() = Some(tree);
    });
}

/// Get a reference to the thread-local fiber tree and execute a closure
pub fn with_fiber_tree<R, F: FnOnce(&FiberTree) -> R>(f: F) -> Option<R> {
    FIBER_TREE.with(|t| t.borrow().as_ref().map(f))
}

/// Get a mutable reference to the thread-local fiber tree and execute a closure
pub fn with_fiber_tree_mut<R, F: FnOnce(&mut FiberTree) -> R>(f: F) -> Option<R> {
    FIBER_TREE.with(|t| t.borrow_mut().as_mut().map(f))
}

/// Clear the thread-local fiber tree
pub fn clear_fiber_tree() {
    FIBER_TREE.with(|t| {
        *t.borrow_mut() = None;
    });
}

/// Execute a closure with the current fiber
pub fn with_current_fiber<R, F: FnOnce(&mut Fiber) -> R>(f: F) -> Option<R> {
    FIBER_TREE.with(|t| {
        let mut tree = t.borrow_mut();
        let tree = tree.as_mut()?;
        let current_id = tree.current_fiber()?;
        tree.fibers.get_mut(&current_id).map(f)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fiber_tree_creation() {
        let tree = FiberTree::new();
        assert!(tree.fibers.is_empty());
        assert!(tree.root.is_none());
        assert!(tree.render_stack.is_empty());
    }

    #[test]
    fn test_mount_root_fiber() {
        let mut tree = FiberTree::new();
        let id = tree.mount(None, None);

        assert_eq!(tree.root, Some(id));
        assert!(tree.fibers.contains_key(&id));
    }

    #[test]
    fn test_mount_child_fiber() {
        let mut tree = FiberTree::new();
        let parent_id = tree.mount(None, None);
        let child_id = tree.mount(Some(parent_id), Some("child".to_string()));

        let parent = tree.get(parent_id).unwrap();
        assert!(parent.children.contains(&child_id));

        let child = tree.get(child_id).unwrap();
        assert_eq!(child.parent, Some(parent_id));
        assert_eq!(child.key, Some("child".to_string()));
    }

    #[test]
    fn test_render_stack() {
        let mut tree = FiberTree::new();
        let id1 = tree.mount(None, None);
        let id2 = tree.mount(Some(id1), None);

        assert!(tree.current_fiber().is_none());

        tree.begin_render(id1);
        assert_eq!(tree.current_fiber(), Some(id1));

        tree.begin_render(id2);
        assert_eq!(tree.current_fiber(), Some(id2));

        tree.end_render();
        assert_eq!(tree.current_fiber(), Some(id1));

        tree.end_render();
        assert!(tree.current_fiber().is_none());
    }

    #[test]
    fn test_schedule_unmount() {
        let mut tree = FiberTree::new();
        let id = tree.mount(None, None);

        tree.schedule_unmount(id);
        assert!(tree.pending_unmount.contains(&id));
    }

    #[test]
    fn test_prepare_for_render() {
        let mut tree = FiberTree::new();
        let id = tree.mount(None, None);

        // Simulate some hook calls
        tree.begin_render(id);
        {
            let fiber = tree.get_mut(id).unwrap();
            fiber.next_hook_index();
            fiber.next_hook_index();
        }
        tree.end_render();

        assert_eq!(tree.get(id).unwrap().hook_index, 2);

        tree.prepare_for_render();
        assert_eq!(tree.get(id).unwrap().hook_index, 0);
    }

    #[test]
    fn test_remove_fiber() {
        let mut tree = FiberTree::new();
        let parent_id = tree.mount(None, None);
        let child_id = tree.mount(Some(parent_id), None);

        tree.remove(child_id);

        assert!(!tree.fibers.contains_key(&child_id));
        let parent = tree.get(parent_id).unwrap();
        assert!(!parent.children.contains(&child_id));
    }

    #[test]
    fn test_dirty_fibers() {
        let mut tree = FiberTree::new();
        let id1 = tree.mount(None, None);
        let id2 = tree.mount(Some(id1), None);

        // Both start dirty
        let dirty = tree.dirty_fibers();
        assert!(dirty.contains(&id1));
        assert!(dirty.contains(&id2));

        tree.mark_clean(id1);
        let dirty = tree.dirty_fibers();
        assert!(!dirty.contains(&id1));
        assert!(dirty.contains(&id2));
    }

    #[test]
    fn test_process_unmounts_removes_fibers() {
        let mut tree = FiberTree::new();
        let id1 = tree.mount(None, None);
        let id2 = tree.mount(Some(id1), None);

        tree.schedule_unmount(id2);
        assert!(tree.has_pending_unmounts());
        assert_eq!(tree.pending_unmount_count(), 1);

        let unmounted = tree.process_unmounts();

        assert_eq!(unmounted, vec![id2]);
        assert!(!tree.fibers.contains_key(&id2));
        assert!(!tree.has_pending_unmounts());
    }

    #[test]
    fn test_process_unmounts_cleans_up_context() {
        use crate::context_stack::{clear_context_stack, get_context, push_context};

        clear_context_stack();

        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);

        // Simulate a context provider
        push_context(fiber_id, "test-context".to_string());
        assert_eq!(get_context::<String>(), Some("test-context".to_string()));

        // Schedule and process unmount
        tree.schedule_unmount(fiber_id);
        tree.process_unmounts();

        // Context should be cleaned up
        assert_eq!(get_context::<String>(), None);

        clear_context_stack();
    }

    #[test]
    fn test_process_unmounts_multiple_fibers() {
        let mut tree = FiberTree::new();
        let id1 = tree.mount(None, None);
        let id2 = tree.mount(Some(id1), None);
        let id3 = tree.mount(Some(id1), None);

        tree.schedule_unmount(id2);
        tree.schedule_unmount(id3);

        assert_eq!(tree.pending_unmount_count(), 2);

        let unmounted = tree.process_unmounts();

        assert_eq!(unmounted.len(), 2);
        assert!(unmounted.contains(&id2));
        assert!(unmounted.contains(&id3));
        assert!(!tree.fibers.contains_key(&id2));
        assert!(!tree.fibers.contains_key(&id3));
        assert!(tree.fibers.contains_key(&id1)); // Parent still exists
    }

    #[test]
    fn test_process_unmounts_nested_context_cleanup() {
        use crate::context_stack::{clear_context_stack, get_context, push_context};

        clear_context_stack();

        let mut tree = FiberTree::new();
        let outer_fiber = tree.mount(None, None);
        let inner_fiber = tree.mount(Some(outer_fiber), None);

        // Outer provider
        push_context(outer_fiber, "outer".to_string());
        // Inner provider shadows outer
        push_context(inner_fiber, "inner".to_string());

        assert_eq!(get_context::<String>(), Some("inner".to_string()));

        // Unmount inner fiber
        tree.schedule_unmount(inner_fiber);
        tree.process_unmounts();

        // Should now get outer value
        assert_eq!(get_context::<String>(), Some("outer".to_string()));

        // Unmount outer fiber
        tree.schedule_unmount(outer_fiber);
        tree.process_unmounts();

        // No context left
        assert_eq!(get_context::<String>(), None);

        clear_context_stack();
    }

    #[test]
    fn test_has_pending_unmounts() {
        let mut tree = FiberTree::new();
        let id = tree.mount(None, None);

        assert!(!tree.has_pending_unmounts());

        tree.schedule_unmount(id);
        assert!(tree.has_pending_unmounts());

        tree.process_unmounts();
        assert!(!tree.has_pending_unmounts());
    }

    #[test]
    fn test_get_or_create_fiber_by_component_id_creates_new() {
        let mut tree = FiberTree::new();

        let fiber_id = tree.get_or_create_fiber_by_component_id(42);

        assert!(tree.fibers.contains_key(&fiber_id));
        assert!(tree.seen_this_render.contains(&fiber_id));
        assert_eq!(tree.component_id_to_fiber.get(&42), Some(&fiber_id));
        assert_eq!(tree.fiber_to_component_id.get(&fiber_id), Some(&42));
    }

    #[test]
    fn test_get_or_create_fiber_by_component_id_returns_existing() {
        let mut tree = FiberTree::new();

        let fiber_id1 = tree.get_or_create_fiber_by_component_id(42);
        let fiber_id2 = tree.get_or_create_fiber_by_component_id(42);

        assert_eq!(fiber_id1, fiber_id2);
        assert_eq!(tree.fibers.len(), 1);
    }

    #[test]
    fn test_get_or_create_fiber_by_component_id_different_ids() {
        let mut tree = FiberTree::new();

        let fiber_id1 = tree.get_or_create_fiber_by_component_id(42);
        let fiber_id2 = tree.get_or_create_fiber_by_component_id(43);

        assert_ne!(fiber_id1, fiber_id2);
        assert_eq!(tree.fibers.len(), 2);
    }

    #[test]
    fn test_get_or_create_fiber_by_component_id_with_parent() {
        let mut tree = FiberTree::new();
        let parent_id = tree.mount(None, None);

        tree.begin_render(parent_id);
        let child_fiber_id = tree.get_or_create_fiber_by_component_id(100);
        tree.end_render();

        let child = tree.get(child_fiber_id).unwrap();
        assert_eq!(child.parent, Some(parent_id));
    }

    #[test]
    fn test_mark_unseen_for_unmount_schedules_unseen() {
        let mut tree = FiberTree::new();

        // Create two fibers via component IDs
        let fiber_id1 = tree.get_or_create_fiber_by_component_id(1);
        let fiber_id2 = tree.get_or_create_fiber_by_component_id(2);

        // Clear seen set and only mark fiber_id1 as seen
        tree.seen_this_render.clear();
        tree.seen_this_render.insert(fiber_id1);

        tree.mark_unseen_for_unmount();

        // fiber_id2 should be scheduled for unmount
        assert!(tree.pending_unmount.contains(&fiber_id2));
        assert!(!tree.pending_unmount.contains(&fiber_id1));
        assert!(tree.seen_this_render.is_empty());
    }

    #[test]
    fn test_mark_unseen_for_unmount_clears_seen_set() {
        let mut tree = FiberTree::new();

        tree.get_or_create_fiber_by_component_id(1);
        assert!(!tree.seen_this_render.is_empty());

        tree.mark_unseen_for_unmount();
        assert!(tree.seen_this_render.is_empty());
    }

    #[test]
    fn test_cleanup_component_id_mapping() {
        let mut tree = FiberTree::new();

        let fiber_id = tree.get_or_create_fiber_by_component_id(42);

        assert!(tree.component_id_to_fiber.contains_key(&42));
        assert!(tree.fiber_to_component_id.contains_key(&fiber_id));

        tree.cleanup_component_id_mapping(fiber_id);

        assert!(!tree.component_id_to_fiber.contains_key(&42));
        assert!(!tree.fiber_to_component_id.contains_key(&fiber_id));
    }

    #[test]
    fn test_remove_cleans_up_component_id_mapping() {
        let mut tree = FiberTree::new();

        let fiber_id = tree.get_or_create_fiber_by_component_id(42);

        tree.remove(fiber_id);

        assert!(!tree.component_id_to_fiber.contains_key(&42));
        assert!(!tree.fiber_to_component_id.contains_key(&fiber_id));
    }

    #[test]
    fn test_fiber_lifecycle_full_cycle() {
        let mut tree = FiberTree::new();

        // First render: create fiber
        let fiber_id = tree.get_or_create_fiber_by_component_id(42);
        tree.mark_unseen_for_unmount();

        // Second render: fiber is seen again
        let fiber_id2 = tree.get_or_create_fiber_by_component_id(42);
        assert_eq!(fiber_id, fiber_id2);
        tree.mark_unseen_for_unmount();

        // Third render: fiber is NOT rendered (component removed)
        // Don't call get_or_create_fiber_by_component_id
        tree.mark_unseen_for_unmount();

        // Fiber should be scheduled for unmount
        assert!(tree.pending_unmount.contains(&fiber_id));
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Property 2: Fiber Lifecycle Management**
        /// **Validates: Requirements 1.5, 2.1, 2.2, 2.3**
        ///
        /// For any Component render, a fiber is created/retrieved, set as current
        /// during render, and restored after.
        #[test]
        fn prop_fiber_lifecycle_management(
            component_ids in prop::collection::vec(1u64..10000, 1..20),
            render_passes in 1usize..5
        ) {
            let mut tree = FiberTree::new();
            let mut created_fibers: HashMap<u64, FiberId> = HashMap::new();

            for _pass in 0..render_passes {
                // Simulate rendering each component
                for &component_id in &component_ids {
                    let fiber_id = tree.get_or_create_fiber_by_component_id(component_id);

                    // Property: Same component ID always returns same fiber
                    if let Some(&existing_fiber_id) = created_fibers.get(&component_id) {
                        prop_assert_eq!(fiber_id, existing_fiber_id,
                            "Component ID {} should always map to same fiber", component_id);
                    } else {
                        created_fibers.insert(component_id, fiber_id);
                    }

                    // Property: Fiber exists in tree
                    prop_assert!(tree.fibers.contains_key(&fiber_id),
                        "Fiber {} should exist in tree", fiber_id.0);

                    // Property: Fiber is marked as seen
                    prop_assert!(tree.seen_this_render.contains(&fiber_id),
                        "Fiber {} should be marked as seen", fiber_id.0);

                    // Property: begin_render sets current fiber
                    tree.begin_render(fiber_id);
                    prop_assert_eq!(tree.current_fiber(), Some(fiber_id),
                        "Current fiber should be {} during render", fiber_id.0);

                    // Property: end_render restores previous context
                    tree.end_render();
                    prop_assert!(tree.current_fiber().is_none(),
                        "Current fiber should be None after end_render");
                }

                tree.mark_unseen_for_unmount();
            }
        }

        /// Property: Unseen fibers are scheduled for unmount
        /// **Validates: Requirements 2.5**
        #[test]
        fn prop_unseen_fibers_scheduled_for_unmount(
            initial_ids in prop::collection::vec(1u64..10000, 1..10),
            surviving_ids in prop::collection::vec(1u64..10000, 0..5)
        ) {
            let mut tree = FiberTree::new();

            // Create fibers for all initial IDs
            let mut fiber_map: HashMap<u64, FiberId> = HashMap::new();
            for &id in &initial_ids {
                let fiber_id = tree.get_or_create_fiber_by_component_id(id);
                fiber_map.insert(id, fiber_id);
            }
            tree.mark_unseen_for_unmount();

            // Second render: only surviving_ids are rendered
            for &id in &surviving_ids {
                tree.get_or_create_fiber_by_component_id(id);
            }
            tree.mark_unseen_for_unmount();

            // Property: Fibers not in surviving_ids should be scheduled for unmount
            for (&component_id, &fiber_id) in &fiber_map {
                let should_survive = surviving_ids.contains(&component_id);
                let is_pending_unmount = tree.pending_unmount.contains(&fiber_id);

                if !should_survive {
                    prop_assert!(is_pending_unmount,
                        "Fiber for component {} should be scheduled for unmount", component_id);
                }
            }
        }

        /// Property: Component ID mappings are bidirectional and consistent
        /// **Validates: Requirements 2.4**
        #[test]
        fn prop_component_id_mapping_consistency(
            component_ids in prop::collection::vec(1u64..10000, 1..20)
        ) {
            let mut tree = FiberTree::new();

            for &component_id in &component_ids {
                let fiber_id = tree.get_or_create_fiber_by_component_id(component_id);

                // Property: Bidirectional mapping is consistent
                prop_assert_eq!(
                    tree.component_id_to_fiber.get(&component_id),
                    Some(&fiber_id),
                    "component_id_to_fiber should map {} to {:?}", component_id, fiber_id
                );
                prop_assert_eq!(
                    tree.fiber_to_component_id.get(&fiber_id),
                    Some(&component_id),
                    "fiber_to_component_id should map {:?} to {}", fiber_id, component_id
                );
            }
        }
    }
}
