//! Strict mode for development-time checks.
//!
//! Strict mode helps detect impure renders and side effects during development
//! by double-rendering components and double-executing effects on mount.
//!
//! # Features
//!
//! - **Double Render**: Components are rendered twice to detect impure renders
//! - **Effect Double-Execution**: Effects run twice on mount to detect cleanup issues
//! - **Render Diff Warning**: Warns if two renders produce different results
//!
//! # Usage
//!
//! Strict mode is automatically disabled in release builds for performance.
//!
//! ```rust,ignore
//! use reratui_fiber::prelude::*;
//!
//! // Enable strict mode via render options
//! render_with_options(
//!     || rsx! { <App /> },
//!     RenderOptions {
//!         strict_mode: true,
//!         ..Default::default()
//!     }
//! ).await?;
//! ```

use std::cell::RefCell;

use crate::fiber::FiberId;

thread_local! {
    /// Thread-local strict mode configuration
    static STRICT_MODE_ENABLED: RefCell<bool> = const { RefCell::new(false) };
}

/// Check if strict mode is enabled
pub fn is_strict_mode_enabled() -> bool {
    #[cfg(debug_assertions)]
    {
        STRICT_MODE_ENABLED.with(|enabled| *enabled.borrow())
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// Set strict mode enabled state
pub fn set_strict_mode_enabled(enabled: bool) {
    #[cfg(debug_assertions)]
    {
        STRICT_MODE_ENABLED.with(|e| {
            *e.borrow_mut() = enabled;
        });
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = enabled; // Suppress unused warning
    }
}

/// Strict mode configuration for development
#[cfg(debug_assertions)]
pub struct StrictMode {
    /// Whether strict mode is enabled
    pub enabled: bool,
}

#[cfg(debug_assertions)]
impl StrictMode {
    /// Create a new strict mode instance
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Wrap a render function with strict mode checks.
    ///
    /// When strict mode is enabled, this will:
    /// 1. Render the component once
    /// 2. Reset the fiber's hook index
    /// 3. Render the component again (this result is kept)
    /// 4. Warn if the two renders produce different results (when comparable)
    ///
    /// This helps detect impure renders that depend on external state.
    pub fn wrap_render<F, R>(&self, fiber_id: FiberId, component_fn: F) -> R
    where
        F: Fn() -> R,
    {
        if self.enabled {
            // First render (discarded)
            let _result1 = component_fn();

            // Reset fiber's hook index for second render
            crate::fiber_tree::with_fiber_tree_mut(|tree| {
                if let Some(fiber) = tree.get_mut(fiber_id) {
                    fiber.reset_hook_index();
                }
            });

            // Second render (this is the one we keep)
            component_fn()
        } else {
            component_fn()
        }
    }

    /// Wrap a render function with strict mode checks and result comparison.
    ///
    /// This variant compares the two render results and logs a warning
    /// if they differ, indicating an impure render.
    pub fn wrap_render_with_diff<F, R>(&self, fiber_id: FiberId, component_fn: F) -> R
    where
        F: Fn() -> R,
        R: PartialEq + std::fmt::Debug,
    {
        if self.enabled {
            // First render
            let result1 = component_fn();

            // Reset fiber's hook index for second render
            crate::fiber_tree::with_fiber_tree_mut(|tree| {
                if let Some(fiber) = tree.get_mut(fiber_id) {
                    fiber.reset_hook_index();
                }
            });

            // Second render
            let result2 = component_fn();

            // Compare results and warn if different
            if result1 != result2 {
                tracing::warn!(
                    "Strict mode: Component rendered different results! \
                     This indicates an impure render. \
                     First: {:?}, Second: {:?}",
                    result1,
                    result2
                );
            }

            result2
        } else {
            component_fn()
        }
    }

    /// Execute effects with strict mode double-execution on mount.
    ///
    /// When strict mode is enabled and this is a mount (first render),
    /// effects will be executed twice:
    /// 1. Run effect
    /// 2. Run cleanup (if any)
    /// 3. Run effect again
    ///
    /// This helps detect effects that don't properly clean up.
    pub fn wrap_effect_on_mount<F, C>(&self, is_mount: bool, effect: F) -> Option<C>
    where
        F: Fn() -> Option<C>,
        C: FnOnce(),
    {
        if self.enabled && is_mount {
            // First execution
            let cleanup1 = effect();

            // Run cleanup if provided
            if let Some(cleanup) = cleanup1 {
                cleanup();
            }

            // Second execution (this cleanup is kept)
            effect()
        } else {
            effect()
        }
    }
}

#[cfg(debug_assertions)]
impl Default for StrictMode {
    fn default() -> Self {
        Self::new(false)
    }
}

/// No-op strict mode for release builds
#[cfg(not(debug_assertions))]
pub struct StrictMode;

#[cfg(not(debug_assertions))]
impl StrictMode {
    /// Create a new strict mode instance (no-op in release)
    pub fn new(_enabled: bool) -> Self {
        Self
    }

    /// Wrap a render function (no-op in release)
    pub fn wrap_render<F, R>(&self, _fiber_id: FiberId, component_fn: F) -> R
    where
        F: Fn() -> R,
    {
        component_fn()
    }

    /// Wrap a render function with diff checking (no-op in release)
    pub fn wrap_render_with_diff<F, R>(&self, _fiber_id: FiberId, component_fn: F) -> R
    where
        F: Fn() -> R,
        R: PartialEq + std::fmt::Debug,
    {
        component_fn()
    }

    /// Execute effects with strict mode (no-op in release)
    pub fn wrap_effect_on_mount<F, C>(&self, _is_mount: bool, effect: F) -> Option<C>
    where
        F: Fn() -> Option<C>,
        C: FnOnce(),
    {
        effect()
    }
}

#[cfg(not(debug_assertions))]
impl Default for StrictMode {
    fn default() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
    fn test_strict_mode_creation() {
        let strict_mode = StrictMode::new(true);
        #[cfg(debug_assertions)]
        assert!(strict_mode.enabled);

        let strict_mode_disabled = StrictMode::new(false);
        #[cfg(debug_assertions)]
        assert!(!strict_mode_disabled.enabled);
    }

    #[test]
    fn test_strict_mode_default() {
        let strict_mode = StrictMode::default();
        #[cfg(debug_assertions)]
        assert!(!strict_mode.enabled);
    }

    #[test]
    fn test_wrap_render_disabled() {
        let fiber_id = setup_test_fiber();
        let strict_mode = StrictMode::new(false);

        let render_count = Arc::new(AtomicUsize::new(0));
        let render_count_clone = render_count.clone();

        let result = strict_mode.wrap_render(fiber_id, || {
            render_count_clone.fetch_add(1, Ordering::SeqCst);
            42
        });

        assert_eq!(result, 42);
        assert_eq!(render_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_wrap_render_enabled_double_renders() {
        let fiber_id = setup_test_fiber();
        let strict_mode = StrictMode::new(true);

        let render_count = Arc::new(AtomicUsize::new(0));
        let render_count_clone = render_count.clone();

        let result = strict_mode.wrap_render(fiber_id, || {
            render_count_clone.fetch_add(1, Ordering::SeqCst);
            42
        });

        assert_eq!(result, 42);
        // Should render twice in strict mode
        assert_eq!(render_count.load(Ordering::SeqCst), 2);

        cleanup_test();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_wrap_render_resets_hook_index() {
        let fiber_id = setup_test_fiber();
        let strict_mode = StrictMode::new(true);

        let hook_indices = Arc::new(std::sync::Mutex::new(Vec::new()));
        let hook_indices_clone = hook_indices.clone();

        strict_mode.wrap_render(fiber_id, || {
            // Record the hook index at start of each render
            let index = crate::fiber_tree::with_fiber_tree_mut(|tree| {
                tree.get(fiber_id).map(|f| f.hook_index).unwrap_or(999)
            })
            .unwrap_or(999);
            hook_indices_clone.lock().unwrap().push(index);
        });

        let indices = hook_indices.lock().unwrap();
        // Both renders should start with hook_index = 0
        assert_eq!(indices.len(), 2);
        assert_eq!(indices[0], 0);
        assert_eq!(indices[1], 0);

        cleanup_test();
    }

    #[test]
    fn test_wrap_render_with_diff_disabled() {
        let fiber_id = setup_test_fiber();
        let strict_mode = StrictMode::new(false);

        let render_count = Arc::new(AtomicUsize::new(0));
        let render_count_clone = render_count.clone();

        let result = strict_mode.wrap_render_with_diff(fiber_id, || {
            render_count_clone.fetch_add(1, Ordering::SeqCst);
            42
        });

        assert_eq!(result, 42);
        assert_eq!(render_count.load(Ordering::SeqCst), 1);

        cleanup_test();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_wrap_render_with_diff_same_results() {
        let fiber_id = setup_test_fiber();
        let strict_mode = StrictMode::new(true);

        // Pure render - always returns same value
        let result = strict_mode.wrap_render_with_diff(fiber_id, || 42);

        assert_eq!(result, 42);
        // No warning should be logged (can't easily test this without mocking tracing)

        cleanup_test();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_wrap_render_with_diff_different_results() {
        let fiber_id = setup_test_fiber();
        let strict_mode = StrictMode::new(true);

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        // Impure render - returns different value each time
        let result = strict_mode
            .wrap_render_with_diff(fiber_id, || counter_clone.fetch_add(1, Ordering::SeqCst));

        // Should return the second render's result
        assert_eq!(result, 1);
        // Warning should be logged (can't easily test this without mocking tracing)

        cleanup_test();
    }

    #[test]
    fn test_wrap_effect_on_mount_disabled() {
        let strict_mode = StrictMode::new(false);

        let effect_count = Arc::new(AtomicUsize::new(0));
        let cleanup_count = Arc::new(AtomicUsize::new(0));

        let effect_count_clone = effect_count.clone();
        let cleanup_count_clone = cleanup_count.clone();

        let cleanup = strict_mode.wrap_effect_on_mount(true, || {
            effect_count_clone.fetch_add(1, Ordering::SeqCst);
            let cleanup_count = cleanup_count_clone.clone();
            Some(move || {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
            })
        });

        // Effect should run once
        assert_eq!(effect_count.load(Ordering::SeqCst), 1);
        // Cleanup should not have run yet
        assert_eq!(cleanup_count.load(Ordering::SeqCst), 0);
        // Cleanup should be returned
        assert!(cleanup.is_some());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_wrap_effect_on_mount_enabled() {
        let strict_mode = StrictMode::new(true);

        let effect_count = Arc::new(AtomicUsize::new(0));
        let cleanup_count = Arc::new(AtomicUsize::new(0));

        let effect_count_clone = effect_count.clone();
        let cleanup_count_clone = cleanup_count.clone();

        let cleanup = strict_mode.wrap_effect_on_mount(true, || {
            effect_count_clone.fetch_add(1, Ordering::SeqCst);
            let cleanup_count = cleanup_count_clone.clone();
            Some(move || {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
            })
        });

        // Effect should run twice (mount double-execution)
        assert_eq!(effect_count.load(Ordering::SeqCst), 2);
        // Cleanup should have run once (between the two effect executions)
        assert_eq!(cleanup_count.load(Ordering::SeqCst), 1);
        // Second cleanup should be returned
        assert!(cleanup.is_some());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_wrap_effect_on_mount_not_mount() {
        let strict_mode = StrictMode::new(true);

        let effect_count = Arc::new(AtomicUsize::new(0));
        let effect_count_clone = effect_count.clone();

        // Not a mount - should only run once even in strict mode
        let _cleanup = strict_mode.wrap_effect_on_mount(false, || {
            effect_count_clone.fetch_add(1, Ordering::SeqCst);
            Option::<fn()>::None
        });

        assert_eq!(effect_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_strict_mode_global_flag() {
        // Initially disabled
        assert!(!is_strict_mode_enabled());

        // Enable
        set_strict_mode_enabled(true);
        #[cfg(debug_assertions)]
        assert!(is_strict_mode_enabled());

        // Disable
        set_strict_mode_enabled(false);
        assert!(!is_strict_mode_enabled());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_wrap_effect_no_cleanup() {
        let strict_mode = StrictMode::new(true);

        let effect_count = Arc::new(AtomicUsize::new(0));
        let effect_count_clone = effect_count.clone();

        // Effect with no cleanup
        let cleanup = strict_mode.wrap_effect_on_mount(true, || {
            effect_count_clone.fetch_add(1, Ordering::SeqCst);
            Option::<fn()>::None
        });

        // Effect should still run twice
        assert_eq!(effect_count.load(Ordering::SeqCst), 2);
        // No cleanup returned
        assert!(cleanup.is_none());
    }
}
