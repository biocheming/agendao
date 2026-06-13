//! Fiber node representing a mounted component instance.
//!
//! Each fiber maintains its own hook state, pending effects, and cleanup functions.

use std::any::Any;
use std::any::TypeId;
use std::collections::HashMap;

/// Unique identifier for a component instance (like React's Fiber)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FiberId(pub u64);

use std::future::Future;
use std::pin::Pin;

/// Type alias for cleanup functions returned by effects
pub type CleanupFn = Box<dyn FnOnce() + Send + 'static>;

/// Type alias for async cleanup functions returned by async effects
pub type AsyncCleanupFn =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static>;

/// Type alias for the future returned by async effects
pub type AsyncEffectFuture = Pin<Box<dyn Future<Output = Option<AsyncCleanupFn>> + Send>>;

/// Type alias for async effect functions
pub type AsyncEffectFn = Box<dyn FnOnce() -> AsyncEffectFuture + Send + 'static>;

/// A pending effect to be executed after commit
pub struct PendingEffect {
    /// The effect function to execute
    pub effect: Box<dyn FnOnce() -> Option<CleanupFn> + 'static>,
    /// Index of this effect within the fiber's hooks
    pub hook_index: usize,
}

/// A pending async effect to be executed after commit
///
/// This type supports async effect functions that return async cleanup functions.
/// Used by `use_async_effect` for effects that need to perform async operations.
pub struct AsyncPendingEffect {
    /// The async effect function to execute
    /// Returns a future that resolves to an optional async cleanup function
    pub effect: AsyncEffectFn,
    /// Index of this effect within the fiber's hooks
    pub hook_index: usize,
}

/// A Fiber represents a mounted component instance with its own hook state.
/// Named after React's Fiber architecture.
pub struct Fiber {
    /// Unique identifier for this instance
    pub id: FiberId,
    /// Component-scoped hook states (replaces global index)
    pub hooks: Vec<Box<dyn Any + Send>>,
    /// Current hook index during render (reset per render)
    pub hook_index: usize,
    /// Queued effects to run after commit
    pub pending_effects: Vec<PendingEffect>,
    /// Active cleanup functions from previous effects, indexed by hook_index
    pub cleanups: Vec<CleanupFn>,
    /// Cleanup functions indexed by hook_index for proper cleanup ordering
    pub cleanup_by_hook: HashMap<usize, CleanupFn>,
    /// Async cleanup functions indexed by hook_index for proper cleanup ordering
    pub async_cleanup_by_hook: HashMap<usize, AsyncCleanupFn>,
    /// Context values provided by this component
    pub provided_contexts: Vec<TypeId>,
    /// Parent fiber (for tree traversal)
    pub parent: Option<FiberId>,
    /// Child fibers
    pub children: Vec<FiberId>,
    /// Whether this component needs re-render
    pub dirty: bool,
    /// Component key for reconciliation
    pub key: Option<String>,
    /// Hook types from previous render (for development mode warnings)
    #[cfg(debug_assertions)]
    pub previous_hook_types: Vec<&'static str>,
    /// Hook types from current render (for development mode warnings)
    #[cfg(debug_assertions)]
    pub current_hook_types: Vec<&'static str>,
}

impl Fiber {
    /// Create a new fiber with the given ID
    pub fn new(id: FiberId, parent: Option<FiberId>, key: Option<String>) -> Self {
        Self {
            id,
            hooks: Vec::new(),
            hook_index: 0,
            pending_effects: Vec::new(),
            cleanups: Vec::new(),
            cleanup_by_hook: HashMap::new(),
            async_cleanup_by_hook: HashMap::new(),
            provided_contexts: Vec::new(),
            parent,
            children: Vec::new(),
            dirty: true,
            key,
            #[cfg(debug_assertions)]
            previous_hook_types: Vec::new(),
            #[cfg(debug_assertions)]
            current_hook_types: Vec::new(),
        }
    }

    /// Get the next hook index and increment the counter
    pub fn next_hook_index(&mut self) -> usize {
        let index = self.hook_index;
        self.hook_index += 1;
        index
    }

    /// Reset hook index for a new render pass
    pub fn reset_hook_index(&mut self) {
        self.hook_index = 0;
        #[cfg(debug_assertions)]
        {
            // Move current hook types to previous for comparison
            self.previous_hook_types = std::mem::take(&mut self.current_hook_types);
        }
    }

    /// Track a hook call during render (development mode only)
    #[cfg(debug_assertions)]
    pub fn track_hook_call(&mut self, hook_name: &'static str) {
        self.current_hook_types.push(hook_name);
    }

    /// Track a hook call during render (no-op in release mode)
    #[cfg(not(debug_assertions))]
    #[inline(always)]
    pub fn track_hook_call(&mut self, _hook_name: &'static str) {
        // No-op in release mode
    }

    /// Check if hook order has changed and warn if so (development mode only)
    #[cfg(debug_assertions)]
    pub fn check_hook_order(&self) {
        if self.previous_hook_types.is_empty() {
            // First render, nothing to compare
            return;
        }

        if self.current_hook_types.len() != self.previous_hook_types.len() {
            tracing::warn!(
                fiber_id = ?self.id,
                previous_count = self.previous_hook_types.len(),
                current_count = self.current_hook_types.len(),
                "Hook count changed between renders. This may indicate conditional hook calls, which can lead to bugs."
            );
            return;
        }

        for (index, (current, previous)) in self
            .current_hook_types
            .iter()
            .zip(self.previous_hook_types.iter())
            .enumerate()
        {
            if current != previous {
                tracing::warn!(
                    fiber_id = ?self.id,
                    hook_index = index,
                    previous_hook = previous,
                    current_hook = current,
                    "Hook order changed between renders. Hooks must be called in the same order on every render."
                );
            }
        }
    }

    /// Get a hook value at the given index
    pub fn get_hook<T: Clone + 'static>(&self, index: usize) -> Option<T> {
        self.hooks
            .get(index)
            .and_then(|h| h.downcast_ref::<T>())
            .cloned()
    }

    /// Set a hook value at the given index
    pub fn set_hook<T: Send + 'static>(&mut self, index: usize, value: T) {
        if index >= self.hooks.len() {
            self.hooks.resize_with(index + 1, || Box::new(()));
        }
        self.hooks[index] = Box::new(value);
    }

    /// Get or initialize a hook value
    pub fn get_or_init_hook<T, F>(&mut self, index: usize, initializer: F) -> T
    where
        T: Clone + Send + 'static,
        F: FnOnce() -> T,
    {
        if index >= self.hooks.len() {
            self.hooks.resize_with(index + 1, || Box::new(()));
        }

        if self.hooks[index].downcast_ref::<T>().is_none() {
            self.hooks[index] = Box::new(initializer());
        }

        self.hooks[index]
            .downcast_ref::<T>()
            .cloned()
            .expect("Hook type mismatch")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fiber_creation() {
        let fiber = Fiber::new(FiberId(1), None, None);
        assert_eq!(fiber.id, FiberId(1));
        assert_eq!(fiber.hook_index, 0);
        assert!(fiber.hooks.is_empty());
        assert!(fiber.parent.is_none());
        assert!(fiber.children.is_empty());
        assert!(fiber.dirty);
    }

    #[test]
    fn test_fiber_with_parent_and_key() {
        let fiber = Fiber::new(FiberId(2), Some(FiberId(1)), Some("my-key".to_string()));
        assert_eq!(fiber.id, FiberId(2));
        assert_eq!(fiber.parent, Some(FiberId(1)));
        assert_eq!(fiber.key, Some("my-key".to_string()));
    }

    #[test]
    fn test_next_hook_index() {
        let mut fiber = Fiber::new(FiberId(1), None, None);
        assert_eq!(fiber.next_hook_index(), 0);
        assert_eq!(fiber.next_hook_index(), 1);
        assert_eq!(fiber.next_hook_index(), 2);
        assert_eq!(fiber.hook_index, 3);
    }

    #[test]
    fn test_reset_hook_index() {
        let mut fiber = Fiber::new(FiberId(1), None, None);
        fiber.next_hook_index();
        fiber.next_hook_index();
        assert_eq!(fiber.hook_index, 2);

        fiber.reset_hook_index();
        assert_eq!(fiber.hook_index, 0);
    }

    #[test]
    fn test_get_or_init_hook() {
        let mut fiber = Fiber::new(FiberId(1), None, None);

        // First call initializes
        let value: i32 = fiber.get_or_init_hook(0, || 42);
        assert_eq!(value, 42);

        // Second call returns existing value
        let value: i32 = fiber.get_or_init_hook(0, || 100);
        assert_eq!(value, 42);
    }

    #[test]
    fn test_set_and_get_hook() {
        let mut fiber = Fiber::new(FiberId(1), None, None);

        fiber.set_hook(0, 42i32);
        assert_eq!(fiber.get_hook::<i32>(0), Some(42));

        fiber.set_hook(0, 100i32);
        assert_eq!(fiber.get_hook::<i32>(0), Some(100));
    }

    #[test]
    fn test_fiber_id_traits() {
        let id1 = FiberId(1);
        let id2 = FiberId(1);
        let id3 = FiberId(2);

        // Clone
        let id1_clone = id1;
        assert_eq!(id1, id1_clone);

        // PartialEq
        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        // Debug
        let debug_str = format!("{:?}", id1);
        assert!(debug_str.contains("FiberId"));
    }

    #[test]
    fn test_pending_effect_creation() {
        let effect = PendingEffect {
            effect: Box::new(|| None),
            hook_index: 0,
        };
        assert_eq!(effect.hook_index, 0);
    }

    #[test]
    fn test_pending_effect_with_cleanup() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let cleanup_called = Arc::new(AtomicBool::new(false));
        let cleanup_called_clone = cleanup_called.clone();

        let effect = PendingEffect {
            effect: Box::new(move || {
                Some(Box::new(move || {
                    cleanup_called_clone.store(true, Ordering::SeqCst);
                }) as CleanupFn)
            }),
            hook_index: 1,
        };

        // Execute the effect and get the cleanup
        let cleanup = (effect.effect)();
        assert!(cleanup.is_some());

        // Execute the cleanup
        cleanup.unwrap()();
        assert!(cleanup_called.load(Ordering::SeqCst));
    }

    #[test]
    fn test_async_pending_effect_creation() {
        let effect = AsyncPendingEffect {
            effect: Box::new(|| Box::pin(async { None })),
            hook_index: 0,
        };
        assert_eq!(effect.hook_index, 0);
    }

    #[tokio::test]
    async fn test_async_pending_effect_execution() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let effect_ran = Arc::new(AtomicBool::new(false));
        let effect_ran_clone = effect_ran.clone();

        let effect = AsyncPendingEffect {
            effect: Box::new(move || {
                let effect_ran = effect_ran_clone.clone();
                Box::pin(async move {
                    effect_ran.store(true, Ordering::SeqCst);
                    None
                })
            }),
            hook_index: 0,
        };

        // Execute the async effect
        let cleanup = (effect.effect)().await;
        assert!(effect_ran.load(Ordering::SeqCst));
        assert!(cleanup.is_none());
    }

    #[tokio::test]
    async fn test_async_pending_effect_with_async_cleanup() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let effect_ran = Arc::new(AtomicBool::new(false));
        let cleanup_ran = Arc::new(AtomicBool::new(false));
        let effect_ran_clone = effect_ran.clone();
        let cleanup_ran_clone = cleanup_ran.clone();

        let effect = AsyncPendingEffect {
            effect: Box::new(move || {
                let effect_ran = effect_ran_clone.clone();
                let cleanup_ran = cleanup_ran_clone.clone();
                Box::pin(async move {
                    effect_ran.store(true, Ordering::SeqCst);
                    Some(Box::new(move || {
                        let cleanup_ran = cleanup_ran.clone();
                        Box::pin(async move {
                            cleanup_ran.store(true, Ordering::SeqCst);
                        }) as Pin<Box<dyn Future<Output = ()> + Send>>
                    }) as AsyncCleanupFn)
                })
            }),
            hook_index: 1,
        };

        // Execute the async effect
        let cleanup = (effect.effect)().await;
        assert!(effect_ran.load(Ordering::SeqCst));
        assert!(cleanup.is_some());

        // Execute the async cleanup
        cleanup.unwrap()().await;
        assert!(cleanup_ran.load(Ordering::SeqCst));
    }
}

#[test]
#[cfg(debug_assertions)]
fn test_hook_order_detection() {
    let mut fiber = Fiber::new(FiberId(1), None, None);

    // First render - establish hook order
    fiber.track_hook_call("use_state");
    fiber.track_hook_call("use_effect");
    fiber.track_hook_call("use_state");

    // End first render
    fiber.reset_hook_index();

    // Second render - same order (should not warn)
    fiber.track_hook_call("use_state");
    fiber.track_hook_call("use_effect");
    fiber.track_hook_call("use_state");

    // Check hook order - should not panic or warn
    fiber.check_hook_order();

    // Verify hook types were tracked
    assert_eq!(fiber.current_hook_types.len(), 3);
    assert_eq!(fiber.previous_hook_types.len(), 3);
}

#[test]
#[cfg(debug_assertions)]
fn test_hook_count_change_detection() {
    let mut fiber = Fiber::new(FiberId(1), None, None);

    // First render - 2 hooks
    fiber.track_hook_call("use_state");
    fiber.track_hook_call("use_effect");

    // End first render
    fiber.reset_hook_index();

    // Second render - 3 hooks (different count)
    fiber.track_hook_call("use_state");
    fiber.track_hook_call("use_effect");
    fiber.track_hook_call("use_state");

    // Check hook order - will log warning about count change
    fiber.check_hook_order();

    // Verify counts are different
    assert_eq!(fiber.previous_hook_types.len(), 2);
    assert_eq!(fiber.current_hook_types.len(), 3);
}

#[test]
#[cfg(debug_assertions)]
fn test_hook_order_change_detection() {
    let mut fiber = Fiber::new(FiberId(1), None, None);

    // First render
    fiber.track_hook_call("use_state");
    fiber.track_hook_call("use_effect");

    // End first render
    fiber.reset_hook_index();

    // Second render - different order
    fiber.track_hook_call("use_effect");
    fiber.track_hook_call("use_state");

    // Check hook order - will log warning about order change
    fiber.check_hook_order();

    // Verify order changed
    assert_eq!(fiber.previous_hook_types[0], "use_state");
    assert_eq!(fiber.current_hook_types[0], "use_effect");
}
