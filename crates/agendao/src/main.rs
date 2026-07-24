mod host;
mod product_cli;

/// 全局分配器换 mimalloc：glibc 默认分配器在多线程 arena 下会把
/// 流式期高频深拷贝 churn 碎片化驻留成高水位 RSS（实测单会话 4.9GB
/// 而会话文本仅 ~2MB），mimalloc 的碎片回收到 OS 明显更积极。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    agendao::init_logging();
    agendao_cli::spawn_process_reaper();
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if product_cli::dispatch_if_product_command(args.clone()).await? {
        return Ok(());
    }
    agendao_cli::run_cli_with_context(args, host::cli_runtime_context()).await
}
