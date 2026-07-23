pub mod app;
pub mod bridge;
pub mod config;
pub mod dialog;
pub mod input;
pub mod markdown;
pub mod screen;
pub mod store;
pub mod telemetry;
pub mod theme;
pub mod transport;
pub mod widget;
pub mod ds;

pub use app::{run_app, run_app_with_config};
pub use config::AppConfig;

#[cfg(test)]
pub(crate) mod test_alloc {
    //! 测试专用统计分配器：仅统计"当前线程在 `AllocGuard` 存活期间"的分配
    //! 字节数（realloc 按新尺寸计），用于断言快照归并的分配量级从 O(n²)
    //! 降下来。线程本地开关为 const 初始化且无 Drop，不会在 TLS 析构期
    //! 访问，对并行运行的其他测试零干扰。
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
    }
    static BYTES: AtomicUsize = AtomicUsize::new(0);

    pub(crate) struct CountingAllocator;

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if ENABLED.with(|flag| flag.get()) {
                BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            }
            System.alloc(layout)
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout)
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            if ENABLED.with(|flag| flag.get()) {
                BYTES.fetch_add(new_size, Ordering::Relaxed);
            }
            System.realloc(ptr, layout, new_size)
        }
    }

    /// 构造时开启当前线程的分配统计，Drop 时关闭。
    pub(crate) struct AllocGuard {
        start: usize,
    }

    impl AllocGuard {
        pub(crate) fn start() -> Self {
            ENABLED.with(|flag| flag.set(true));
            Self {
                start: BYTES.load(Ordering::Relaxed),
            }
        }

        /// 自保活以来当前线程累计分配的字节数。
        pub(crate) fn bytes(&self) -> usize {
            BYTES.load(Ordering::Relaxed) - self.start
        }
    }

    impl Drop for AllocGuard {
        fn drop(&mut self) {
            ENABLED.with(|flag| flag.set(false));
        }
    }
}

#[cfg(test)]
#[global_allocator]
static TEST_COUNTING_ALLOCATOR: test_alloc::CountingAllocator = test_alloc::CountingAllocator;
