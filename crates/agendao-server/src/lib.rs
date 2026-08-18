#![allow(ambiguous_glob_reexports)]

pub mod error;
mod live_snapshot;
#[cfg(feature = "mcp")]
pub mod mcp_oauth;
pub mod oauth;
pub mod openapi;
pub(crate) mod recovery;
pub(crate) mod request_options;
pub mod routes;
pub mod scheduler_backends;
mod scheduler_cache;
pub mod scheduler_capabilities;
pub(crate) mod scheduler_runner;
pub mod server;
pub(crate) mod session_runtime;
#[cfg(unix)]
pub mod unix_socket; // Phase 5: Unix Socket 传输层
pub mod web;
pub mod worktree;

pub use agendao_server_core::runtime_control;
pub use agendao_server_core::runtime_state;
pub use error::*;
#[cfg(feature = "mcp")]
pub use mcp_oauth::*;
pub use oauth::*;
pub use openapi::*;
pub use routes::*;
pub use server::*;
pub use session_runtime::local_frontend::{
    spawn_local_frontend_events, spawn_local_session_events,
};
#[cfg(unix)]
pub use unix_socket::*;
pub use web::*;
pub use worktree::*;

#[cfg(test)]
pub(crate) fn isolate_test_config_home() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let path =
            std::env::temp_dir().join(format!("agendao-server-test-home-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("server test config home should be created");
        std::env::set_var("AGENDAO_HOME", path);
    });
}

#[cfg(test)]
pub(crate) mod test_alloc {
    //! 测试专用统计分配器：仅统计"当前线程在 `AllocGuard` 存活期间"的分配
    //! 字节数（realloc 按新尺寸计），用于断言流式快照归并的分配量级
    //! 从 O(n²) 降下来。线程本地开关为 const 初始化且无 Drop，
    //! 不会在 TLS 析构期访问，对并行运行的其他测试零干扰。
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
        static BYTES: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) struct CountingAllocator;

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if ENABLED.with(|flag| flag.get()) {
                BYTES.with(|bytes| bytes.set(bytes.get().saturating_add(layout.size())));
            }
            System.alloc(layout)
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout)
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            if ENABLED.with(|flag| flag.get()) {
                BYTES.with(|bytes| bytes.set(bytes.get().saturating_add(new_size)));
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
                start: BYTES.with(Cell::get),
            }
        }

        /// 自保活以来当前线程累计分配的字节数。
        pub(crate) fn bytes(&self) -> usize {
            BYTES.with(Cell::get) - self.start
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
