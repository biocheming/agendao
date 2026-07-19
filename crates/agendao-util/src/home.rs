//! agendao_home — 用户级目录单一权威（土律归一）。
//!
//! 一切用户级持久化（数据库/凭证/日志/缓存/全局状态/全局 skills）统一收在
//! `~/.agendao`（可用 `AGENDAO_HOME` 环境变量覆盖），与 codex / claude code /
//! zcode 的 `~/.codex`、`~/.claude`、`~/.zcode` 同约定。
//!
//! 历史上这些文件散在 XDG 三处（`~/.local/share/agendao`、`~/.cache/agendao`、
//! `~/.config/agendao`）。首次解析 home 时自动迁移：`rename` 优先、跨设备退回
//! 复制；目标已存在则跳过（以新为准）。项目级 `.agendao/` 不在此列（本就该
//! 留在工作区）。

use std::path::{Path, PathBuf};
use std::sync::Once;

static MIGRATE_ONCE: Once = Once::new();

/// 用户级目录唯一入口。所有需要用户级路径的 crate 一律经此解析，
/// 不得各自拼 XDG 路径（土律·第四条·单点权威）。
pub fn agendao_home() -> PathBuf {
    MIGRATE_ONCE.call_once(migrate_legacy_home);
    raw_agendao_home()
}

/// 不带迁移的原始解析（迁移逻辑自身用）。
fn raw_agendao_home() -> PathBuf {
    if let Ok(v) = std::env::var("AGENDAO_HOME") {
        let t = v.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agendao")
}

/// 老 XDG 三处根目录（Linux: ~/.local/share / ~/.cache / ~/.config；
/// 经 dirs crate 取平台路径,macOS 同样成立）。
struct LegacyRoots {
    data_local: PathBuf,
    cache: PathBuf,
    config: PathBuf,
}

fn legacy_roots() -> LegacyRoots {
    LegacyRoots {
        data_local: dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(std::env::temp_dir)
            .join("agendao"),
        cache: dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("agendao"),
        config: dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("agendao"),
    }
}

/// 迁移一项（文件或目录）：目标已存在则跳过（以新为准）；
/// rename 优先，跨设备退回递归复制。
fn migrate_one(src: &Path, dst: &Path, label: &str) {
    if !src.exists() || dst.exists() {
        return;
    }
    if let Some(parent) = dst.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(%e, %label, "agendao_home 迁移：创建目标父目录失败");
            return;
        }
    }
    match std::fs::rename(src, dst) {
        Ok(()) => tracing::info!(%label, from = %src.display(), to = %dst.display(), "agendao_home 迁移完成"),
        Err(_) => {
            if let Err(e) = copy_recursive(src, dst) {
                tracing::warn!(%e, %label, "agendao_home 迁移：复制失败");
            } else {
                let _ = std::fs::remove_dir_all(src);
                tracing::info!(%label, from = %src.display(), to = %dst.display(), "agendao_home 迁移完成(复制)");
            }
        }
    }
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_file() {
        std::fs::copy(src, dst)?;
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src).unwrap();
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// 首次启动迁移（`agendao_home` 内经 Once 调用一次）。
/// 幂等：目标已存在即跳过；老位置搬空后仅余空目录，可随时回滚。
fn migrate_legacy_home() {
    let target = raw_agendao_home();
    // AGENDAO_HOME 显式覆盖时不动用户老数据（人家就是要另起一处）。
    if std::env::var("AGENDAO_HOME").is_ok() {
        return;
    }
    let roots = legacy_roots();
    let d = roots.data_local.as_path();
    let c = roots.cache.as_path();
    let g = roots.config.as_path();

    for name in ["agendao.db", "agendao.db-wal", "agendao.db-shm"] {
        migrate_one(&d.join(name), &target.join(name), "database");
    }
    // MCP OAuth 凭证（旧版直接放在 data_local 根下）。
    migrate_one(&d.join("mcp-auth.json"), &target.join("mcp-auth.json"), "mcp-auth");
    migrate_one(&d.join("log"), &target.join("log"), "log");
    // 凭证目录(data/)内容平铺进 home 根（auth.json 等）。
    let data = d.join("data");
    if data.is_dir() {
        for entry in walkdir::WalkDir::new(&data).min_depth(1).max_depth(1).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if let Some(name) = entry.path().file_name() {
                    migrate_one(entry.path(), &target.join(name), "auth");
                }
            }
        }
        let _ = std::fs::remove_dir(&data);
    }
    migrate_one(&c.join("catalog"), &target.join("cache").join("catalog"), "catalog-cache");
    // 全局 UI 状态：state_dir 优先（~/.local/state）,老版本也可能在 cache_dir。
    let state_dir = dirs::state_dir()
        .unwrap_or_else(|| roots.data_local.clone())
        .join("agendao")
        .join("global-state.json");
    migrate_one(&state_dir, &target.join("global-state.json"), "global-state");
    migrate_one(
        &roots.cache.join("global-state.json"),
        &target.join("global-state.json"),
        "global-state",
    );
    migrate_one(&g.join("state.json"), &target.join("state.json"), "state");
    // 全局配置文件（agendao.json/jsonc）。
    for name in ["agendao.json", "agendao.jsonc"] {
        migrate_one(&g.join(name), &target.join(name), "global-config");
    }
    // TUI prompt 历史 / stash。
    for name in ["prompt-history.json", "prompt-stash.json"] {
        migrate_one(&d.join(name), &target.join(name), "prompt-history");
    }
    for name in ["skill", "skills"] {
        migrate_one(&g.join(name), &target.join("skills"), "skills");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_roots() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
        (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        )
    }

    #[test]
    fn migrate_moves_legacy_files_when_target_absent() {
        let (dl, ca, co, home) = temp_roots();
        let roots = LegacyRoots {
            data_local: dl.path().join("agendao"),
            cache: ca.path().join("agendao"),
            config: co.path().join("agendao"),
        };
        std::fs::create_dir_all(&roots.data_local).unwrap();
        std::fs::write(roots.data_local.join("agendao.db"), b"db").unwrap();
        std::fs::create_dir_all(roots.data_local.join("log")).unwrap();
        std::fs::write(roots.data_local.join("log").join("agendao.log"), b"log").unwrap();
        std::fs::create_dir_all(roots.data_local.join("data")).unwrap();
        std::fs::write(roots.data_local.join("data").join("auth.json"), b"{}").unwrap();
        std::fs::create_dir_all(roots.cache.join("catalog")).unwrap();
        std::fs::write(roots.cache.join("catalog").join("models.snapshot.json"), b"[]").unwrap();
        std::fs::create_dir_all(&roots.config).unwrap();
        std::fs::write(roots.config.join("state.json"), b"{}").unwrap();
        std::fs::create_dir_all(roots.config.join("skills")).unwrap();

        let target = home.path().join(".agendao");
        for name in ["agendao.db", "agendao.db-wal", "agendao.db-shm"] {
            migrate_one(&roots.data_local.join(name), &target.join(name), "database");
        }
        migrate_one(&roots.data_local.join("log"), &target.join("log"), "log");
        let data = roots.data_local.join("data");
        for entry in walkdir::WalkDir::new(&data).min_depth(1).max_depth(1).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if let Some(name) = entry.path().file_name() {
                    migrate_one(entry.path(), &target.join(name), "auth");
                }
            }
        }
        migrate_one(&roots.cache.join("catalog"), &target.join("cache").join("catalog"), "catalog-cache");
        migrate_one(&roots.config.join("state.json"), &target.join("state.json"), "state");
        migrate_one(&roots.config.join("skills"), &target.join("skills"), "skills");

        assert_eq!(std::fs::read(target.join("agendao.db")).unwrap(), b"db");
        assert_eq!(std::fs::read(target.join("log").join("agendao.log")).unwrap(), b"log");
        assert_eq!(std::fs::read(target.join("auth.json")).unwrap(), b"{}");
        assert!(target.join("cache").join("catalog").join("models.snapshot.json").exists());
        assert!(target.join("state.json").exists());
        assert!(target.join("skills").exists());
        // rename 后老位置已空。
        assert!(!roots.data_local.join("agendao.db").exists());
        assert!(!roots.data_local.join("log").exists());
    }

    #[test]
    fn migrate_skips_when_target_exists() {
        let (dl, _ca, _co, home) = temp_roots();
        let src = dl.path().join("agendao").join("agendao.db");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, b"old").unwrap();
        let dst = home.path().join(".agendao").join("agendao.db");
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::write(&dst, b"new").unwrap();

        migrate_one(&src, &dst, "database");
        // 以新为准：目标不动,源保留。
        assert_eq!(std::fs::read(&dst).unwrap(), b"new");
        assert!(src.exists());
    }

    #[test]
    fn migrate_noop_when_nothing_exists() {
        let (dl, _ca, _co, home) = temp_roots();
        let src = dl.path().join("agendao").join("agendao.db");
        let dst = home.path().join(".agendao").join("agendao.db");
        migrate_one(&src, &dst, "database");
        assert!(!dst.exists());
    }
}
