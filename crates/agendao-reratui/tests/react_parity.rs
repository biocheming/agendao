//! Integration tests for React parity in reratui.
//!
//! These tests verify that the hooks and runtime behave like React:
//! - Effects run after commit, not during render
//! - State updates are batched
//! - Context providers have proper lifecycle
//! - Hook state is isolated between fibers
//! - Cleanup functions run in correct order

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reratui::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree, with_fiber_tree_mut};
use reratui::hooks::{use_context, use_context_provider, use_effect, use_effect_once, use_state};
use reratui::scheduler::batch::{begin_batch, clear_state_batch, end_batch_with_tree};
use reratui::scheduler::effect_queue::{clear_effect_queue, flush_effects_with_tree};
use reratui::{FiberId, context_stack::clear_context_stack};

/// Helper to set up a test fiber tree with a single fiber
fn setup_single_fiber() -> FiberId {
    clear_effect_queue();
    clear_state_batch();
    clear_context_stack();
    let mut tree = FiberTree::new();
    let fiber_id = tree.mount(None, None);
    tree.begin_render(fiber_id);
    set_fiber_tree(tree);
    fiber_id
}

/// Helper to set up a test fiber tree with parent and child fibers
fn setup_parent_child_fibers() -> (FiberId, FiberId) {
    clear_effect_queue();
    clear_state_batch();
    clear_context_stack();
    let mut tree = FiberTree::new();
    let parent_id = tree.mount(None, None);
    let child_id = tree.mount(Some(parent_id), None);
    set_fiber_tree(tree);
    (parent_id, child_id)
}

/// Helper to clean up after tests
fn cleanup() {
    clear_fiber_tree();
    clear_effect_queue();
    clear_state_batch();
    clear_context_stack();
}

// =============================================================================
// Effect Timing Tests (React Parity: effects run after commit)
// =============================================================================

#[test]
fn test_effect_does_not_run_during_render() {
    let _fiber_id = setup_single_fiber();

    let effect_ran = Arc::new(AtomicUsize::new(0));
    let effect_ran_clone = effect_ran.clone();

    // Queue an effect during "render"
    use_effect_once(move || {
        effect_ran_clone.fetch_add(1, Ordering::SeqCst);
        Option::<fn()>::None
    });

    // Effect should NOT have run yet (still in render phase)
    assert_eq!(
        effect_ran.load(Ordering::SeqCst),
        0,
        "Effect should not run during render"
    );

    cleanup();
}

#[test]
fn test_effect_runs_after_commit_phase() {
    let _fiber_id = setup_single_fiber();

    let effect_ran = Arc::new(AtomicUsize::new(0));
    let effect_ran_clone = effect_ran.clone();

    use_effect_once(move || {
        effect_ran_clone.fetch_add(1, Ordering::SeqCst);
        Option::<fn()>::None
    });

    // Simulate commit phase by ending render and flushing effects
    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    // Effect should have run after commit
    assert_eq!(
        effect_ran.load(Ordering::SeqCst),
        1,
        "Effect should run after commit phase"
    );

    cleanup();
}

#[test]
fn test_cleanup_runs_before_new_effect() {
    let fiber_id = setup_single_fiber();

    let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));

    // First render with effect that has cleanup
    {
        let order = execution_order.clone();
        use_effect(
            move || {
                order.lock().unwrap().push("effect1");
                let order_cleanup = order.clone();
                Some(move || {
                    order_cleanup.lock().unwrap().push("cleanup1");
                })
            },
            None::<()>, // Run every render
        );
    }

    // Flush first effect
    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    // Second render - should trigger cleanup then new effect
    with_fiber_tree_mut(|tree| {
        tree.begin_render(fiber_id);
    });

    {
        let order = execution_order.clone();
        use_effect(
            move || {
                order.lock().unwrap().push("effect2");
                Option::<fn()>::None
            },
            None::<()>,
        );
    }

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    let order = execution_order.lock().unwrap();
    assert_eq!(
        *order,
        vec!["effect1", "cleanup1", "effect2"],
        "Cleanup should run before new effect"
    );

    cleanup();
}

// =============================================================================
// Hook State Isolation Tests
// =============================================================================

#[test]
fn test_hook_state_isolated_between_fibers() {
    let (parent_id, child_id) = setup_parent_child_fibers();

    // Parent fiber state
    with_fiber_tree_mut(|tree| {
        tree.begin_render(parent_id);
    });
    let (parent_count, _) = use_state(|| 100);
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    // Child fiber state
    with_fiber_tree_mut(|tree| {
        tree.begin_render(child_id);
    });
    let (child_count, _) = use_state(|| 200);
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    // States should be independent
    assert_eq!(parent_count, 100, "Parent should have its own state");
    assert_eq!(child_count, 200, "Child should have its own state");

    cleanup();
}

#[test]
fn test_multiple_hooks_in_same_fiber() {
    let _fiber_id = setup_single_fiber();

    let (count, _) = use_state(|| 1);
    let (name, _) = use_state(|| "Alice".to_string());
    let (active, _) = use_state(|| true);

    assert_eq!(count, 1);
    assert_eq!(name, "Alice");
    assert!(active);

    cleanup();
}

// =============================================================================
// State Batching Tests
// =============================================================================

#[test]
fn test_multiple_state_updates_are_batched() {
    let fiber_id = setup_single_fiber();

    let (_, set_count) = use_state(|| 0);

    // Begin batch (simulating event handler)
    begin_batch();

    // Multiple updates should be batched
    set_count.update(|n| n + 1);
    set_count.update(|n| n + 1);
    set_count.update(|n| n + 1);

    // End batch and apply updates
    with_fiber_tree_mut(|tree| {
        tree.end_render();
        let dirty = end_batch_with_tree(tree);

        // Only one fiber should be dirty (not three separate re-renders)
        assert_eq!(
            dirty.len(),
            1,
            "Updates should be batched into one dirty fiber"
        );
        assert!(dirty.contains(&fiber_id));

        // Final value should reflect all updates: 0 + 1 + 1 + 1 = 3
        let fiber = tree.get(fiber_id).unwrap();
        let final_value = fiber.get_hook::<i32>(0);
        assert_eq!(
            final_value,
            Some(3),
            "All batched updates should be applied"
        );
    });

    cleanup();
}

#[test]
fn test_functional_updates_receive_latest_state() {
    let fiber_id = setup_single_fiber();

    let (_, set_count) = use_state(|| 0);

    begin_batch();

    // Each functional update should receive the result of the previous
    set_count.update(|n| n + 10); // 0 -> 10
    set_count.update(|n| n * 2); // 10 -> 20
    set_count.update(|n| n + 5); // 20 -> 25

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        end_batch_with_tree(tree);

        let fiber = tree.get(fiber_id).unwrap();
        let final_value = fiber.get_hook::<i32>(0);
        assert_eq!(
            final_value,
            Some(25),
            "Functional updates should chain correctly"
        );
    });

    cleanup();
}

#[test]
fn test_set_if_changed_skips_equal_values() {
    let fiber_id = setup_single_fiber();

    let (_, set_count) = use_state(|| 42);

    // End initial render and mark clean
    with_fiber_tree_mut(|tree| {
        tree.end_render();
        tree.mark_clean(fiber_id);
        tree.begin_render(fiber_id);
    });
    clear_state_batch();

    // Set to same value - should NOT mark dirty
    set_count.set_if_changed(42);

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        let dirty = end_batch_with_tree(tree);

        assert!(
            !dirty.contains(&fiber_id),
            "Fiber should not be dirty when value unchanged"
        );
    });

    cleanup();
}

// =============================================================================
// Context Provider Scoping Tests
// =============================================================================

#[test]
fn test_context_provider_scoping() {
    let (parent_id, child_id) = setup_parent_child_fibers();

    // Parent provides context
    with_fiber_tree_mut(|tree| {
        tree.begin_render(parent_id);
    });
    use_context_provider(|| "parent-value".to_string());
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    // Child can consume parent's context
    with_fiber_tree_mut(|tree| {
        tree.begin_render(child_id);
    });
    let value = use_context::<String>();
    assert_eq!(value, "parent-value", "Child should see parent's context");
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    cleanup();
}

#[test]
fn test_nested_context_providers_shadow() {
    let (parent_id, child_id) = setup_parent_child_fibers();

    // Parent provides context
    with_fiber_tree_mut(|tree| {
        tree.begin_render(parent_id);
    });
    use_context_provider(|| "outer".to_string());
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    // Child provides same type - should shadow parent
    with_fiber_tree_mut(|tree| {
        tree.begin_render(child_id);
    });
    use_context_provider(|| "inner".to_string());
    let value = use_context::<String>();
    assert_eq!(value, "inner", "Inner provider should shadow outer");
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    cleanup();
}

#[test]
fn test_context_cleanup_on_unmount() {
    let (parent_id, child_id) = setup_parent_child_fibers();

    // Parent provides outer context
    with_fiber_tree_mut(|tree| {
        tree.begin_render(parent_id);
    });
    use_context_provider(|| "outer".to_string());
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    // Child provides inner context
    with_fiber_tree_mut(|tree| {
        tree.begin_render(child_id);
    });
    use_context_provider(|| "inner".to_string());
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    // Verify inner is visible
    assert_eq!(
        reratui::context_stack::get_context::<String>(),
        Some("inner".to_string())
    );

    // Unmount child
    with_fiber_tree_mut(|tree| {
        tree.schedule_unmount(child_id);
        tree.process_unmounts();
    });

    // After unmount, should see outer context again
    assert_eq!(
        reratui::context_stack::get_context::<String>(),
        Some("outer".to_string()),
        "After child unmount, parent context should be visible"
    );

    cleanup();
}

// =============================================================================
// Component Unmount Cleanup Order Tests
// =============================================================================

#[test]
fn test_unmount_cleanup_order() {
    let (parent_id, child_id) = setup_parent_child_fibers();

    // Both fibers provide context
    with_fiber_tree_mut(|tree| {
        tree.begin_render(parent_id);
    });
    use_context_provider(|| 1i32);
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    with_fiber_tree_mut(|tree| {
        tree.begin_render(child_id);
    });
    use_context_provider(|| 2i32);
    with_fiber_tree_mut(|tree| {
        tree.end_render();
    });

    // Schedule both for unmount
    with_fiber_tree_mut(|tree| {
        tree.schedule_unmount(child_id);
        tree.schedule_unmount(parent_id);

        let unmounted = tree.process_unmounts();

        // Both should be unmounted
        assert_eq!(unmounted.len(), 2);
        // Use public API to check if fibers exist
        assert!(tree.get(child_id).is_none());
        assert!(tree.get(parent_id).is_none());
    });

    // Context should be fully cleaned up
    assert_eq!(
        reratui::context_stack::get_context::<i32>(),
        None,
        "All context should be cleaned up after unmount"
    );

    cleanup();
}

// =============================================================================
// Effect Dependency Tests
// =============================================================================

#[test]
fn test_effect_with_empty_deps_runs_once() {
    let fiber_id = setup_single_fiber();

    let run_count = Arc::new(AtomicUsize::new(0));

    // First render
    {
        let count = run_count.clone();
        use_effect(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            },
            Some(()), // Empty deps = run once
        );
    }

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    assert_eq!(run_count.load(Ordering::SeqCst), 1);

    // Second render - should NOT run again
    with_fiber_tree_mut(|tree| {
        tree.begin_render(fiber_id);
    });

    {
        let count = run_count.clone();
        use_effect(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            },
            Some(()),
        );
    }

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    assert_eq!(
        run_count.load(Ordering::SeqCst),
        1,
        "Effect with empty deps should only run once"
    );

    cleanup();
}

#[test]
fn test_effect_with_changing_deps_reruns() {
    let fiber_id = setup_single_fiber();

    let run_count = Arc::new(AtomicUsize::new(0));

    // First render with deps = (1,)
    {
        let count = run_count.clone();
        use_effect(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            },
            Some((1i32,)),
        );
    }

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    assert_eq!(run_count.load(Ordering::SeqCst), 1);

    // Second render with different deps = (2,) - should run
    with_fiber_tree_mut(|tree| {
        tree.begin_render(fiber_id);
    });

    {
        let count = run_count.clone();
        use_effect(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            },
            Some((2i32,)), // Different deps
        );
    }

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    assert_eq!(
        run_count.load(Ordering::SeqCst),
        2,
        "Effect should rerun when deps change"
    );

    cleanup();
}

#[test]
fn test_effect_with_same_deps_does_not_rerun() {
    let fiber_id = setup_single_fiber();

    let run_count = Arc::new(AtomicUsize::new(0));

    // First render with deps = (42,)
    {
        let count = run_count.clone();
        use_effect(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            },
            Some((42i32,)),
        );
    }

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    assert_eq!(run_count.load(Ordering::SeqCst), 1);

    // Second render with same deps = (42,) - should NOT run
    with_fiber_tree_mut(|tree| {
        tree.begin_render(fiber_id);
    });

    {
        let count = run_count.clone();
        use_effect(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Option::<fn()>::None
            },
            Some((42i32,)), // Same deps
        );
    }

    with_fiber_tree_mut(|tree| {
        tree.end_render();
        flush_effects_with_tree(tree);
    });

    assert_eq!(
        run_count.load(Ordering::SeqCst),
        1,
        "Effect should not rerun when deps are unchanged"
    );

    cleanup();
}
