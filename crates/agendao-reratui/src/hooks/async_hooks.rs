//! Async hooks for managing asynchronous operations.
//!
//! This module provides fiber-based async hooks:
//! - `use_future` - Spawn and track async tasks with loading/error/data states
//! - `use_query` - Data fetching with caching and stale-while-revalidate
//! - `use_mutation` - Mutation state tracking with optimistic updates and retry

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::fiber::FiberId;
use crate::fiber_tree::with_current_fiber;
use crate::hooks::use_effect;
use crate::scheduler::batch::{StateUpdate, StateUpdateKind, queue_update};

// ============================================================================
// FutureState and FutureHandle for use_future
// ============================================================================

/// Represents the current state of a future operation
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FutureState<T, E = String> {
    /// The future has not been started yet
    #[default]
    Idle,
    /// The future is currently pending (actively running)
    Pending,
    /// The future has resolved successfully with a value
    Resolved(T),
    /// The future has failed with an error
    Error(E),
}

impl<T, E> FutureState<T, E> {
    /// Returns true if the future has not been started yet
    pub fn is_idle(&self) -> bool {
        matches!(self, FutureState::Idle)
    }

    /// Returns true if the future is currently pending
    pub fn is_pending(&self) -> bool {
        matches!(self, FutureState::Pending)
    }

    /// Returns true if the future has resolved successfully
    pub fn is_resolved(&self) -> bool {
        matches!(self, FutureState::Resolved(_))
    }

    /// Returns true if the future has failed with an error
    pub fn is_error(&self) -> bool {
        matches!(self, FutureState::Error(_))
    }

    /// Returns the resolved value if available
    pub fn value(&self) -> Option<&T> {
        match self {
            FutureState::Resolved(v) => Some(v),
            _ => None,
        }
    }

    /// Returns the error if available
    pub fn error(&self) -> Option<&E> {
        match self {
            FutureState::Error(e) => Some(e),
            _ => None,
        }
    }

    /// Maps the resolved value to a new type
    pub fn map<U, F>(self, f: F) -> FutureState<U, E>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            FutureState::Idle => FutureState::Idle,
            FutureState::Pending => FutureState::Pending,
            FutureState::Resolved(v) => FutureState::Resolved(f(v)),
            FutureState::Error(e) => FutureState::Error(e),
        }
    }

    /// Maps the error to a new type
    pub fn map_err<F, G>(self, f: F) -> FutureState<T, G>
    where
        F: FnOnce(E) -> G,
    {
        match self {
            FutureState::Idle => FutureState::Idle,
            FutureState::Pending => FutureState::Pending,
            FutureState::Resolved(v) => FutureState::Resolved(v),
            FutureState::Error(e) => FutureState::Error(f(e)),
        }
    }
}

impl<T, E> From<Result<T, E>> for FutureState<T, E> {
    fn from(result: Result<T, E>) -> Self {
        match result {
            Ok(v) => FutureState::Resolved(v),
            Err(e) => FutureState::Error(e),
        }
    }
}

/// A handle to a future operation that provides access to its current state
#[derive(Clone)]
pub struct FutureHandle<T, E = String>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    /// Shared state of the future
    state: Arc<RwLock<FutureState<T, E>>>,
    /// Handle to the running task for cancellation
    task_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Flag to indicate if cancelled
    cancelled: Arc<AtomicBool>,
}

impl<T, E> FutureHandle<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(FutureState::Idle)),
            task_handle: Arc::new(Mutex::new(None)),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Get the current state of the future
    pub fn state(&self) -> FutureState<T, E> {
        self.state.read().clone()
    }

    /// Returns true if the future is idle
    pub fn is_idle(&self) -> bool {
        matches!(&*self.state.read(), FutureState::Idle)
    }

    /// Returns true if the future is pending
    pub fn is_pending(&self) -> bool {
        matches!(&*self.state.read(), FutureState::Pending)
    }

    /// Returns true if the future is resolved
    pub fn is_resolved(&self) -> bool {
        matches!(&*self.state.read(), FutureState::Resolved(_))
    }

    /// Returns true if the future has an error
    pub fn is_error(&self) -> bool {
        matches!(&*self.state.read(), FutureState::Error(_))
    }

    /// Returns the resolved value if available
    pub fn value(&self) -> Option<T> {
        match &*self.state.read() {
            FutureState::Resolved(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Returns the error if available
    pub fn error(&self) -> Option<E> {
        match &*self.state.read() {
            FutureState::Error(e) => Some(e.clone()),
            _ => None,
        }
    }

    /// Cancel the running future
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(handle) = self.task_handle.lock().take() {
            handle.abort();
        }
    }

    fn set_state(&self, new_state: FutureState<T, E>) {
        *self.state.write() = new_state;
    }

    fn set_task_handle(&self, handle: JoinHandle<()>) {
        *self.task_handle.lock() = Some(handle);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl<T, E> fmt::Debug for FutureHandle<T, E>
where
    T: Clone + Send + Sync + fmt::Debug + 'static,
    E: Clone + Send + Sync + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FutureHandle")
            .field("state", &*self.state.read())
            .field("cancelled", &self.cancelled.load(Ordering::SeqCst))
            .finish()
    }
}

/// Internal storage for future hook state
struct FutureHookStorage<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    handle: FutureHandle<T, E>,
    generation: u64,
}

impl<T, E> Clone for FutureHookStorage<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            generation: self.generation,
        }
    }
}

impl<T, E> FutureHookStorage<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn new() -> Self {
        Self {
            handle: FutureHandle::new(),
            generation: 0,
        }
    }
}

/// React-style useFuture hook for managing async operations
///
/// This hook spawns and tracks async tasks with loading/error/data states.
/// The future is automatically cancelled when the component unmounts or
/// when dependencies change.
///
/// # Arguments
/// - `future_factory`: A function that returns a Future
/// - `deps`: Dependencies that determine when to re-run the future
///   - `Some(())`: Run only once on mount
///   - `None`: Run after every render (not recommended)
///   - `Some((a, b, ...))`: Run when any dependency changes
///
/// # Example
/// ```ignore
/// // Fetch data once on mount
/// let handle = use_future(|| async {
///     fetch_user_data().await
/// }, Some(()));
///
/// match handle.state() {
///     FutureState::Idle => println!("Not started"),
///     FutureState::Pending => println!("Loading..."),
///     FutureState::Resolved(data) => println!("Data: {:?}", data),
///     FutureState::Error(err) => println!("Error: {}", err),
/// }
///
/// // Re-fetch when user_id changes
/// let handle = use_future(move || async move {
///     fetch_user(user_id).await
/// }, Some((user_id,)));
/// ```
pub fn use_future<Deps, F, Fut, T, E>(future_factory: F, deps: Option<Deps>) -> FutureHandle<T, E>
where
    Deps: PartialEq + Clone + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + ToString + 'static,
{
    // Get fiber context for state management
    let (_fiber_id, _hook_index, handle, _generation) = with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();

        // Get or create storage
        let storage: FutureHookStorage<T, E> =
            fiber.get_or_init_hook(hook_index, FutureHookStorage::new);

        (
            fiber.id,
            hook_index,
            storage.handle.clone(),
            storage.generation,
        )
    })
    .expect("use_future must be called within a component render context");

    // Use effect to manage the future lifecycle
    let handle_for_effect = handle.clone();

    use_effect(
        move || {
            // Cancel any previous future
            handle_for_effect.cancel();

            // Reset cancelled flag for new execution
            handle_for_effect.cancelled.store(false, Ordering::SeqCst);

            // Set state to pending
            handle_for_effect.set_state(FutureState::Pending);

            // Clone handle for async task
            let handle_for_task = handle_for_effect.clone();
            let handle_for_cleanup = handle_for_effect.clone();

            // Spawn the future
            let task_handle = tokio::spawn(async move {
                // Check if cancelled before starting
                if handle_for_task.is_cancelled() {
                    return;
                }

                let result = future_factory().await;

                // Check if cancelled after completion
                if handle_for_task.is_cancelled() {
                    return;
                }

                // Update state based on result
                match result {
                    Ok(value) => {
                        handle_for_task.set_state(FutureState::Resolved(value));
                    }
                    Err(error) => {
                        handle_for_task.set_state(FutureState::Error(error));
                    }
                }
            });

            handle_for_effect.set_task_handle(task_handle);

            // Return cleanup function
            Some(move || {
                handle_for_cleanup.cancel();
            })
        },
        deps,
    );

    handle
}

/// Convenience function for futures that run only once on mount
pub fn use_future_once<F, Fut, T, E>(future_factory: F) -> FutureHandle<T, E>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + ToString + 'static,
{
    use_future(future_factory, Some(()))
}

// ============================================================================
// QueryState and QueryResult for use_query
// ============================================================================

/// Status of a query operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueryStatus {
    /// The query has not started yet
    #[default]
    Idle,
    /// The query is loading (no cached data)
    Loading,
    /// The query is refreshing (has cached data)
    Refreshing,
    /// The query completed successfully
    Success,
    /// The query failed
    Error,
}

/// Configuration options for a query
#[derive(Clone)]
pub struct QueryOptions {
    /// Whether the query is enabled
    pub enabled: bool,
    /// How long data is considered fresh (won't refetch)
    pub stale_time: Duration,
    /// How long to keep data in cache
    pub cache_time: Duration,
    /// Whether to retry on failure
    pub retry: bool,
    /// Number of retry attempts
    pub retry_attempts: u32,
    /// Delay between retries
    pub retry_delay: Duration,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            stale_time: Duration::from_secs(0),
            cache_time: Duration::from_secs(300), // 5 minutes
            retry: true,
            retry_attempts: 3,
            retry_delay: Duration::from_secs(1),
        }
    }
}

/// Result of a query operation
#[derive(Clone)]
pub struct QueryResult<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    /// Current status of the query
    pub status: QueryStatus,
    /// The fetched data, if available
    pub data: Option<T>,
    /// The error, if any occurred
    pub error: Option<E>,
    /// Whether the data is stale
    pub is_stale: bool,
    /// Whether the query is currently fetching
    pub is_fetching: bool,
    /// Function to manually refetch
    refetch_fn: Arc<dyn Fn() + Send + Sync>,
    /// Function to invalidate cache
    invalidate_fn: Arc<dyn Fn() + Send + Sync>,
}

impl<T, E> QueryResult<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    /// Manually trigger a refetch
    pub fn refetch(&self) {
        (self.refetch_fn)();
    }

    /// Invalidate the query cache
    pub fn invalidate(&self) {
        (self.invalidate_fn)();
    }

    /// Returns true if the query is loading (no data yet)
    pub fn is_loading(&self) -> bool {
        self.status == QueryStatus::Loading
    }

    /// Returns true if the query completed successfully
    pub fn is_success(&self) -> bool {
        self.status == QueryStatus::Success
    }

    /// Returns true if the query failed
    pub fn is_error(&self) -> bool {
        self.status == QueryStatus::Error
    }
}

impl<T, E> fmt::Debug for QueryResult<T, E>
where
    T: Clone + Send + Sync + fmt::Debug + 'static,
    E: Clone + Send + Sync + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryResult")
            .field("status", &self.status)
            .field("data", &self.data)
            .field("error", &self.error)
            .field("is_stale", &self.is_stale)
            .field("is_fetching", &self.is_fetching)
            .finish()
    }
}

/// Internal query cache entry
struct QueryCacheEntry<T> {
    data: T,
    fetched_at: Instant,
}

/// Type alias for the query cache map
type QueryCacheMap =
    Arc<Mutex<std::collections::HashMap<String, Box<dyn std::any::Any + Send + Sync>>>>;

/// Global query cache
static QUERY_CACHE: once_cell::sync::Lazy<QueryCacheMap> =
    once_cell::sync::Lazy::new(|| Arc::new(Mutex::new(std::collections::HashMap::new())));

/// Internal state for query hook
struct QueryHookState<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    status: QueryStatus,
    data: Option<T>,
    error: Option<E>,
    is_stale: bool,
    is_fetching: bool,
    refetch_trigger: Arc<AtomicU64>,
}

impl<T, E> Clone for QueryHookState<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            status: self.status,
            data: self.data.clone(),
            error: self.error.clone(),
            is_stale: self.is_stale,
            is_fetching: self.is_fetching,
            refetch_trigger: self.refetch_trigger.clone(),
        }
    }
}

impl<T, E> Default for QueryHookState<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            status: QueryStatus::Idle,
            data: None,
            error: None,
            is_stale: false,
            is_fetching: false,
            refetch_trigger: Arc::new(AtomicU64::new(0)),
        }
    }
}

/// React Query-style hook for data fetching with caching
///
/// This hook provides:
/// - Automatic caching with configurable stale time
/// - Background refetching when data becomes stale
/// - Manual refetch and cache invalidation
/// - Retry logic with configurable attempts
///
/// # Arguments
/// - `key`: A unique key for caching the query result
/// - `query_fn`: A function that returns a Future with the query result
/// - `options`: Optional configuration for the query
///
/// # Example
/// ```ignore
/// let result = use_query(
///     "user-123",
///     || async { fetch_user(123).await },
///     Some(QueryOptions {
///         stale_time: Duration::from_secs(60),
///         ..Default::default()
///     }),
/// );
///
/// if result.is_loading() {
///     println!("Loading...");
/// } else if let Some(user) = &result.data {
///     println!("User: {:?}", user);
/// }
///
/// // Manual refetch
/// result.refetch();
/// ```
pub fn use_query<K, F, Fut, T, E>(
    key: K,
    query_fn: F,
    options: Option<QueryOptions>,
) -> QueryResult<T, E>
where
    K: std::hash::Hash + Eq + Clone + Send + Sync + std::fmt::Debug + 'static,
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + ToString + 'static,
{
    let options = options.unwrap_or_default();
    let cache_key = format!("{:?}", key);

    // Get fiber context
    let (fiber_id, hook_index) = with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();
        (fiber.id, hook_index)
    })
    .expect("use_query must be called within a component render context");

    // Get or initialize state
    let state: QueryHookState<T, E> =
        with_current_fiber(|fiber| fiber.get_or_init_hook(hook_index, QueryHookState::default))
            .expect("use_query must be called within a component render context");

    let refetch_trigger = state.refetch_trigger.clone();
    let current_trigger = refetch_trigger.load(Ordering::SeqCst);

    // Create refetch function
    let refetch_fn = {
        let trigger = refetch_trigger.clone();
        Arc::new(move || {
            trigger.fetch_add(1, Ordering::SeqCst);
        }) as Arc<dyn Fn() + Send + Sync>
    };

    // Create invalidate function
    let invalidate_fn = {
        let cache_key = cache_key.clone();
        let trigger = refetch_trigger.clone();
        Arc::new(move || {
            // Remove from cache
            QUERY_CACHE.lock().remove(&cache_key);
            // Trigger refetch
            trigger.fetch_add(1, Ordering::SeqCst);
        }) as Arc<dyn Fn() + Send + Sync>
    };

    // Check cache for existing data
    let cached_data: Option<(T, bool)> = {
        let cache = QUERY_CACHE.lock();
        if let Some(entry) = cache.get(&cache_key) {
            if let Some(cache_entry) = entry.downcast_ref::<QueryCacheEntry<T>>() {
                let is_stale = cache_entry.fetched_at.elapsed() > options.stale_time;
                let is_expired = cache_entry.fetched_at.elapsed() > options.cache_time;
                if !is_expired {
                    Some((cache_entry.data.clone(), is_stale))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    // Determine initial state based on cache
    let (_initial_status, initial_data, initial_is_stale) = match &cached_data {
        Some((data, is_stale)) => (QueryStatus::Success, Some(data.clone()), *is_stale),
        None => (QueryStatus::Idle, None, false),
    };

    // Use effect to manage query lifecycle
    let cache_key_for_effect = cache_key.clone();
    let options_for_effect = options.clone();

    use_effect(
        move || {
            if !options_for_effect.enabled {
                return None;
            }

            // Check if we need to fetch
            let should_fetch = match &cached_data {
                None => true,
                Some((_, is_stale)) => *is_stale,
            };

            if !should_fetch && current_trigger == 0 {
                return None;
            }

            // Update state to loading/refreshing
            let new_status = if cached_data.is_some() {
                QueryStatus::Refreshing
            } else {
                QueryStatus::Loading
            };

            // Queue state update
            queue_update(
                fiber_id,
                StateUpdate {
                    hook_index,
                    update: StateUpdateKind::Value(Box::new(QueryHookState::<T, E> {
                        status: new_status,
                        data: cached_data.as_ref().map(|(d, _)| d.clone()),
                        error: None,
                        is_stale: false,
                        is_fetching: true,
                        refetch_trigger: refetch_trigger.clone(),
                    })),
                },
            );

            // Spawn query task
            let query_fn = query_fn.clone();
            let cache_key = cache_key_for_effect.clone();
            let options = options_for_effect.clone();
            let refetch_trigger = refetch_trigger.clone();

            let cancelled = Arc::new(AtomicBool::new(false));
            let cancelled_for_cleanup = cancelled.clone();

            let task_handle = tokio::spawn(async move {
                let mut attempts = 0;
                let max_attempts = if options.retry {
                    options.retry_attempts
                } else {
                    1
                };

                loop {
                    if cancelled.load(Ordering::SeqCst) {
                        return;
                    }

                    attempts += 1;
                    let result = query_fn().await;

                    if cancelled.load(Ordering::SeqCst) {
                        return;
                    }

                    match result {
                        Ok(data) => {
                            // Update cache
                            {
                                let mut cache = QUERY_CACHE.lock();
                                cache.insert(
                                    cache_key.clone(),
                                    Box::new(QueryCacheEntry {
                                        data: data.clone(),
                                        fetched_at: Instant::now(),
                                    }),
                                );
                            }

                            // Update state
                            queue_update(
                                fiber_id,
                                StateUpdate {
                                    hook_index,
                                    update: StateUpdateKind::Value(Box::new(QueryHookState::<
                                        T,
                                        E,
                                    > {
                                        status: QueryStatus::Success,
                                        data: Some(data),
                                        error: None,
                                        is_stale: false,
                                        is_fetching: false,
                                        refetch_trigger: refetch_trigger.clone(),
                                    })),
                                },
                            );
                            break;
                        }
                        Err(error) => {
                            if attempts >= max_attempts {
                                // Update state with error
                                queue_update(
                                    fiber_id,
                                    StateUpdate {
                                        hook_index,
                                        update: StateUpdateKind::Value(Box::new(QueryHookState::<
                                            T,
                                            E,
                                        > {
                                            status: QueryStatus::Error,
                                            data: cached_data.as_ref().map(|(d, _)| d.clone()),
                                            error: Some(error),
                                            is_stale: false,
                                            is_fetching: false,
                                            refetch_trigger: refetch_trigger.clone(),
                                        })),
                                    },
                                );
                                break;
                            }

                            // Wait before retry
                            tokio::time::sleep(options.retry_delay).await;
                        }
                    }
                }
            });

            Some(move || {
                cancelled_for_cleanup.store(true, Ordering::SeqCst);
                task_handle.abort();
            })
        },
        Some((cache_key.clone(), current_trigger, options.enabled)),
    );

    QueryResult {
        status: state.status,
        data: state.data.or(initial_data),
        error: state.error,
        is_stale: state.is_stale || initial_is_stale,
        is_fetching: state.is_fetching,
        refetch_fn,
        invalidate_fn,
    }
}

/// Clear all query cache entries
pub fn clear_query_cache() {
    QUERY_CACHE.lock().clear();
}

// ============================================================================
// MutationState and MutationHandle for use_mutation
// ============================================================================

/// Status of a mutation operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MutationStatus {
    /// Mutation has not been triggered yet
    #[default]
    Idle,
    /// Mutation is currently executing
    Pending,
    /// Mutation completed successfully
    Success,
    /// Mutation failed with an error
    Error,
}
/// Configuration options for a mutation
#[derive(Clone)]
pub struct MutationOptions {
    /// Whether to retry on failure
    pub retry: bool,
    /// Number of retry attempts
    pub retry_attempts: u32,
    /// Delay between retries
    pub retry_delay: Duration,
    /// Whether to use exponential backoff
    pub retry_exponential_backoff: bool,
    /// Maximum delay for exponential backoff
    pub retry_max_delay: Duration,
}

impl Default for MutationOptions {
    fn default() -> Self {
        Self {
            retry: false,
            retry_attempts: 0,
            retry_delay: Duration::from_secs(1),
            retry_exponential_backoff: false,
            retry_max_delay: Duration::from_secs(30),
        }
    }
}

/// State of a mutation operation
#[derive(Clone)]
pub struct MutationState<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    /// Current status
    pub status: MutationStatus,
    /// Data returned by successful mutation
    pub data: Option<T>,
    /// Error returned by failed mutation
    pub error: Option<E>,
    /// Whether the mutation is pending
    pub is_pending: bool,
    /// Whether the mutation succeeded
    pub is_success: bool,
    /// Whether the mutation failed
    pub is_error: bool,
    /// Whether the mutation is idle
    pub is_idle: bool,
}

impl<T, E> Default for MutationState<T, E>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            status: MutationStatus::Idle,
            data: None,
            error: None,
            is_pending: false,
            is_success: false,
            is_error: false,
            is_idle: true,
        }
    }
}

impl<T, E> fmt::Debug for MutationState<T, E>
where
    T: Clone + Send + Sync + fmt::Debug + 'static,
    E: Clone + Send + Sync + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutationState")
            .field("status", &self.status)
            .field("data", &self.data)
            .field("error", &self.error)
            .field("is_pending", &self.is_pending)
            .field("is_success", &self.is_success)
            .field("is_error", &self.is_error)
            .field("is_idle", &self.is_idle)
            .finish()
    }
}

/// Type alias for mutation function
type MutationFn<TData, TError, TVariables> = Arc<
    dyn Fn(TVariables) -> Pin<Box<dyn Future<Output = Result<TData, TError>> + Send>> + Send + Sync,
>;

/// Handle for triggering and managing mutations
pub struct MutationHandle<TData, TError, TVariables>
where
    TData: Clone + Send + Sync + 'static,
    TError: Clone + Send + Sync + 'static,
    TVariables: Clone + Send + Sync + 'static,
{
    /// Shared state
    state: Arc<RwLock<MutationState<TData, TError>>>,
    /// Mutation function
    mutation_fn: MutationFn<TData, TError, TVariables>,
    /// Options
    options: Arc<MutationOptions>,
    /// Current task handle
    task_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Fiber ID for state updates
    fiber_id: FiberId,
    /// Hook index for state updates
    hook_index: usize,
}

impl<TData, TError, TVariables> Clone for MutationHandle<TData, TError, TVariables>
where
    TData: Clone + Send + Sync + 'static,
    TError: Clone + Send + Sync + 'static,
    TVariables: Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            mutation_fn: self.mutation_fn.clone(),
            options: self.options.clone(),
            task_handle: self.task_handle.clone(),
            fiber_id: self.fiber_id,
            hook_index: self.hook_index,
        }
    }
}

impl<TData, TError, TVariables> MutationHandle<TData, TError, TVariables>
where
    TData: Clone + Send + Sync + 'static,
    TError: Clone + Send + Sync + 'static,
    TVariables: Clone + Send + Sync + 'static,
{
    /// Get the current mutation state
    pub fn state(&self) -> MutationState<TData, TError> {
        self.state.read().clone()
    }

    /// Returns true if the mutation is idle
    pub fn is_idle(&self) -> bool {
        self.state.read().is_idle
    }

    /// Returns true if the mutation is pending
    pub fn is_pending(&self) -> bool {
        self.state.read().is_pending
    }

    /// Returns true if the mutation succeeded
    pub fn is_success(&self) -> bool {
        self.state.read().is_success
    }

    /// Returns true if the mutation failed
    pub fn is_error(&self) -> bool {
        self.state.read().is_error
    }

    /// Get the mutation data if available
    pub fn data(&self) -> Option<TData> {
        self.state.read().data.clone()
    }

    /// Get the mutation error if available
    pub fn error(&self) -> Option<TError> {
        self.state.read().error.clone()
    }

    /// Reset the mutation state to idle
    pub fn reset(&self) {
        // Cancel any running task
        if let Some(handle) = self.task_handle.lock().take() {
            handle.abort();
        }

        // Reset state
        *self.state.write() = MutationState::default();
    }

    /// Trigger the mutation (fire and forget)
    pub fn mutate(&self, variables: TVariables) {
        let state = self.state.clone();
        let mutation_fn = self.mutation_fn.clone();
        let options = self.options.clone();
        let task_handle = self.task_handle.clone();
        let fiber_id = self.fiber_id;
        let hook_index = self.hook_index;

        // Cancel any existing task
        if let Some(handle) = task_handle.lock().take() {
            handle.abort();
        }

        // Update state to pending
        {
            let mut s = state.write();
            s.status = MutationStatus::Pending;
            s.is_pending = true;
            s.is_idle = false;
            s.is_success = false;
            s.is_error = false;
            s.error = None;
        }

        // Queue state update for re-render
        queue_update(
            fiber_id,
            StateUpdate {
                hook_index,
                update: StateUpdateKind::Value(Box::new(state.read().clone())),
            },
        );

        // Spawn mutation task
        let handle = tokio::spawn(async move {
            let mut attempts = 0;
            let max_attempts = if options.retry {
                options.retry_attempts + 1
            } else {
                1
            };

            loop {
                attempts += 1;
                let result = mutation_fn(variables.clone()).await;

                match result {
                    Ok(data) => {
                        // Update state to success
                        {
                            let mut s = state.write();
                            s.status = MutationStatus::Success;
                            s.data = Some(data);
                            s.error = None;
                            s.is_pending = false;
                            s.is_success = true;
                            s.is_error = false;
                            s.is_idle = false;
                        }

                        // Queue state update
                        queue_update(
                            fiber_id,
                            StateUpdate {
                                hook_index,
                                update: StateUpdateKind::Value(Box::new(state.read().clone())),
                            },
                        );
                        break;
                    }
                    Err(error) => {
                        if attempts >= max_attempts {
                            // Update state to error
                            {
                                let mut s = state.write();
                                s.status = MutationStatus::Error;
                                s.error = Some(error);
                                s.is_pending = false;
                                s.is_success = false;
                                s.is_error = true;
                                s.is_idle = false;
                            }

                            // Queue state update
                            queue_update(
                                fiber_id,
                                StateUpdate {
                                    hook_index,
                                    update: StateUpdateKind::Value(Box::new(state.read().clone())),
                                },
                            );
                            break;
                        }

                        // Calculate retry delay
                        let delay = if options.retry_exponential_backoff {
                            let exp_delay = options
                                .retry_delay
                                .checked_mul(2_u32.pow(attempts - 1))
                                .unwrap_or(options.retry_max_delay);
                            exp_delay.min(options.retry_max_delay)
                        } else {
                            options.retry_delay
                        };

                        tokio::time::sleep(delay).await;
                    }
                }
            }
        });

        *task_handle.lock() = Some(handle);
    }

    /// Trigger the mutation and wait for result
    pub async fn mutate_async(&self, variables: TVariables) -> Result<TData, TError> {
        let state = self.state.clone();
        let mutation_fn = self.mutation_fn.clone();
        let options = self.options.clone();
        let fiber_id = self.fiber_id;
        let hook_index = self.hook_index;

        // Update state to pending
        {
            let mut s = state.write();
            s.status = MutationStatus::Pending;
            s.is_pending = true;
            s.is_idle = false;
            s.is_success = false;
            s.is_error = false;
            s.error = None;
        }

        queue_update(
            fiber_id,
            StateUpdate {
                hook_index,
                update: StateUpdateKind::Value(Box::new(state.read().clone())),
            },
        );

        let mut attempts = 0;
        let max_attempts = if options.retry {
            options.retry_attempts + 1
        } else {
            1
        };

        loop {
            attempts += 1;
            let result = mutation_fn(variables.clone()).await;

            match result {
                Ok(data) => {
                    {
                        let mut s = state.write();
                        s.status = MutationStatus::Success;
                        s.data = Some(data.clone());
                        s.error = None;
                        s.is_pending = false;
                        s.is_success = true;
                        s.is_error = false;
                        s.is_idle = false;
                    }

                    queue_update(
                        fiber_id,
                        StateUpdate {
                            hook_index,
                            update: StateUpdateKind::Value(Box::new(state.read().clone())),
                        },
                    );

                    return Ok(data);
                }
                Err(error) => {
                    if attempts >= max_attempts {
                        {
                            let mut s = state.write();
                            s.status = MutationStatus::Error;
                            s.error = Some(error.clone());
                            s.is_pending = false;
                            s.is_success = false;
                            s.is_error = true;
                            s.is_idle = false;
                        }

                        queue_update(
                            fiber_id,
                            StateUpdate {
                                hook_index,
                                update: StateUpdateKind::Value(Box::new(state.read().clone())),
                            },
                        );

                        return Err(error);
                    }

                    let delay = if options.retry_exponential_backoff {
                        let exp_delay = options
                            .retry_delay
                            .checked_mul(2_u32.pow(attempts - 1))
                            .unwrap_or(options.retry_max_delay);
                        exp_delay.min(options.retry_max_delay)
                    } else {
                        options.retry_delay
                    };

                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

impl<TData, TError, TVariables> fmt::Debug for MutationHandle<TData, TError, TVariables>
where
    TData: Clone + Send + Sync + fmt::Debug + 'static,
    TError: Clone + Send + Sync + fmt::Debug + 'static,
    TVariables: Clone + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutationHandle")
            .field("state", &*self.state.read())
            .finish()
    }
}

/// React Query-style mutation hook for data mutations
///
/// This hook provides:
/// - Mutation state tracking (idle, pending, success, error)
/// - Retry logic with configurable attempts and exponential backoff
/// - Reset functionality
///
/// # Arguments
/// - `mutation_fn`: A function that takes variables and returns a Future
/// - `options`: Optional configuration for the mutation
///
/// # Example
/// ```ignore
/// let mutation = use_mutation(
///     |user: CreateUserRequest| async move {
///         api::create_user(user).await
///     },
///     None,
/// );
///
/// // Trigger mutation
/// mutation.mutate(CreateUserRequest { name: "John".to_string() });
///
/// // Check state
/// if mutation.is_pending() {
///     println!("Creating user...");
/// } else if mutation.is_success() {
///     println!("User created: {:?}", mutation.data());
/// } else if mutation.is_error() {
///     println!("Error: {:?}", mutation.error());
/// }
///
/// // Reset state
/// mutation.reset();
/// ```
pub fn use_mutation<TData, TError, TVariables, F, Fut>(
    mutation_fn: F,
    options: Option<MutationOptions>,
) -> MutationHandle<TData, TError, TVariables>
where
    TData: Clone + Send + Sync + 'static,
    TError: Clone + Send + Sync + 'static,
    TVariables: Clone + Send + Sync + 'static,
    F: Fn(TVariables) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<TData, TError>> + Send + 'static,
{
    let options = options.unwrap_or_default();

    // Get fiber context
    let (fiber_id, hook_index) = with_current_fiber(|fiber| {
        let hook_index = fiber.next_hook_index();
        (fiber.id, hook_index)
    })
    .expect("use_mutation must be called within a component render context");

    // Get or initialize state
    let state: Arc<RwLock<MutationState<TData, TError>>> = with_current_fiber(|fiber| {
        fiber.get_or_init_hook(hook_index, || {
            Arc::new(RwLock::new(MutationState::default()))
        })
    })
    .expect("use_mutation must be called within a component render context");

    // Wrap mutation function
    let boxed_fn: MutationFn<TData, TError, TVariables> = Arc::new(move |variables: TVariables| {
        Box::pin(mutation_fn(variables))
            as Pin<Box<dyn Future<Output = Result<TData, TError>> + Send>>
    });

    MutationHandle {
        state,
        mutation_fn: boxed_fn,
        options: Arc::new(options),
        task_handle: Arc::new(Mutex::new(None)),
        fiber_id,
        hook_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fiber_tree::{FiberTree, clear_fiber_tree, set_fiber_tree};
    use crate::scheduler::batch::clear_state_batch;
    use crate::scheduler::effect_queue::clear_effect_queue;

    fn setup_test_fiber() -> FiberId {
        clear_effect_queue();
        clear_state_batch();
        let mut tree = FiberTree::new();
        let fiber_id = tree.mount(None, None);
        tree.begin_render(fiber_id);
        set_fiber_tree(tree);
        fiber_id
    }

    fn cleanup_test() {
        clear_fiber_tree();
        clear_effect_queue();
        clear_state_batch();
        clear_query_cache();
    }

    // ========================================================================
    // FutureState tests
    // ========================================================================

    #[test]
    fn test_future_state_default() {
        let state: FutureState<i32, String> = FutureState::default();
        assert!(state.is_idle());
        assert!(!state.is_pending());
        assert!(!state.is_resolved());
        assert!(!state.is_error());
    }

    #[test]
    fn test_future_state_methods() {
        let idle: FutureState<i32, String> = FutureState::Idle;
        assert!(idle.is_idle());
        assert!(idle.value().is_none());
        assert!(idle.error().is_none());

        let pending: FutureState<i32, String> = FutureState::Pending;
        assert!(pending.is_pending());

        let resolved: FutureState<i32, String> = FutureState::Resolved(42);
        assert!(resolved.is_resolved());
        assert_eq!(resolved.value(), Some(&42));

        let error: FutureState<i32, String> = FutureState::Error("error".to_string());
        assert!(error.is_error());
        assert_eq!(error.error(), Some(&"error".to_string()));
    }

    #[test]
    fn test_future_state_map() {
        let resolved: FutureState<i32, String> = FutureState::Resolved(42);
        let mapped = resolved.map(|x| x * 2);
        assert_eq!(mapped, FutureState::Resolved(84));

        let error: FutureState<i32, String> = FutureState::Error("error".to_string());
        let mapped_err = error.map_err(|e| format!("wrapped: {}", e));
        assert_eq!(mapped_err, FutureState::Error("wrapped: error".to_string()));
    }

    #[test]
    fn test_future_state_from_result() {
        let ok: FutureState<i32, String> = Ok(42).into();
        assert_eq!(ok, FutureState::Resolved(42));

        let err: FutureState<i32, String> = Err("error".to_string()).into();
        assert_eq!(err, FutureState::Error("error".to_string()));
    }

    // ========================================================================
    // FutureHandle tests
    // ========================================================================

    #[test]
    fn test_future_handle_new() {
        let handle: FutureHandle<i32, String> = FutureHandle::new();
        assert!(handle.is_idle());
        assert!(!handle.is_pending());
        assert!(!handle.is_resolved());
        assert!(!handle.is_error());
        assert!(handle.value().is_none());
        assert!(handle.error().is_none());
    }

    #[test]
    fn test_future_handle_set_state() {
        let handle: FutureHandle<i32, String> = FutureHandle::new();

        handle.set_state(FutureState::Pending);
        assert!(handle.is_pending());

        handle.set_state(FutureState::Resolved(42));
        assert!(handle.is_resolved());
        assert_eq!(handle.value(), Some(42));

        handle.set_state(FutureState::Error("error".to_string()));
        assert!(handle.is_error());
        assert_eq!(handle.error(), Some("error".to_string()));
    }

    #[test]
    fn test_future_handle_cancel() {
        let handle: FutureHandle<i32, String> = FutureHandle::new();
        assert!(!handle.is_cancelled());

        handle.cancel();
        assert!(handle.is_cancelled());
    }

    // ========================================================================
    // use_future tests
    // ========================================================================

    #[test]
    fn test_use_future_returns_handle() {
        let _fiber_id = setup_test_fiber();

        let handle = use_future(|| async { Ok::<i32, String>(42) }, Some(()));

        // Handle should be returned (state managed by effect)
        assert!(!handle.is_cancelled());

        cleanup_test();
    }

    // ========================================================================
    // QueryStatus tests
    // ========================================================================

    #[test]
    fn test_query_status_default() {
        let status = QueryStatus::default();
        assert_eq!(status, QueryStatus::Idle);
    }

    // ========================================================================
    // QueryOptions tests
    // ========================================================================

    #[test]
    fn test_query_options_default() {
        let options = QueryOptions::default();
        assert!(options.enabled);
        assert_eq!(options.stale_time, Duration::from_secs(0));
        assert_eq!(options.cache_time, Duration::from_secs(300));
        assert!(options.retry);
        assert_eq!(options.retry_attempts, 3);
    }

    // ========================================================================
    // MutationStatus tests
    // ========================================================================

    #[test]
    fn test_mutation_status_default() {
        let status = MutationStatus::default();
        assert_eq!(status, MutationStatus::Idle);
    }

    // ========================================================================
    // MutationOptions tests
    // ========================================================================

    #[test]
    fn test_mutation_options_default() {
        let options = MutationOptions::default();
        assert!(!options.retry);
        assert_eq!(options.retry_attempts, 0);
        assert_eq!(options.retry_delay, Duration::from_secs(1));
        assert!(!options.retry_exponential_backoff);
    }

    // ========================================================================
    // MutationState tests
    // ========================================================================

    #[test]
    fn test_mutation_state_default() {
        let state: MutationState<i32, String> = MutationState::default();
        assert_eq!(state.status, MutationStatus::Idle);
        assert!(state.data.is_none());
        assert!(state.error.is_none());
        assert!(!state.is_pending);
        assert!(!state.is_success);
        assert!(!state.is_error);
        assert!(state.is_idle);
    }

    // ========================================================================
    // use_mutation tests
    // ========================================================================

    #[test]
    fn test_use_mutation_returns_handle() {
        let _fiber_id = setup_test_fiber();

        let mutation = use_mutation(|x: i32| async move { Ok::<i32, String>(x * 2) }, None);

        assert!(mutation.is_idle());
        assert!(!mutation.is_pending());
        assert!(!mutation.is_success());
        assert!(!mutation.is_error());

        cleanup_test();
    }

    #[test]
    fn test_use_mutation_reset() {
        let _fiber_id = setup_test_fiber();

        let mutation = use_mutation(|x: i32| async move { Ok::<i32, String>(x * 2) }, None);

        // Manually set state to test reset
        {
            let mut state = mutation.state.write();
            state.status = MutationStatus::Success;
            state.data = Some(42);
            state.is_success = true;
            state.is_idle = false;
        }

        assert!(mutation.is_success());

        mutation.reset();

        assert!(mutation.is_idle());
        assert!(mutation.data().is_none());

        cleanup_test();
    }

    // ========================================================================
    // use_query tests
    // ========================================================================

    #[test]
    fn test_use_query_returns_result() {
        let _fiber_id = setup_test_fiber();

        let result = use_query("test-key", || async { Ok::<i32, String>(42) }, None);

        // Initial state should be idle or loading
        assert!(result.status == QueryStatus::Idle || result.status == QueryStatus::Loading);

        cleanup_test();
    }

    #[test]
    fn test_use_query_disabled() {
        let _fiber_id = setup_test_fiber();

        let result = use_query(
            "disabled-key",
            || async { Ok::<i32, String>(42) },
            Some(QueryOptions {
                enabled: false,
                ..Default::default()
            }),
        );

        // Should remain idle when disabled
        assert_eq!(result.status, QueryStatus::Idle);

        cleanup_test();
    }

    #[test]
    fn test_clear_query_cache() {
        // Add something to cache
        {
            let mut cache = QUERY_CACHE.lock();
            cache.insert(
                "test".to_string(),
                Box::new(QueryCacheEntry {
                    data: 42i32,
                    fetched_at: Instant::now(),
                }),
            );
        }

        assert!(!QUERY_CACHE.lock().is_empty());

        clear_query_cache();

        assert!(QUERY_CACHE.lock().is_empty());
    }
}
