//! 金 — 系统剪贴板写入，OSC52 作为非确认式回退。
//!
//! 终端剪贴板的事实标准：发送 `OSC 52 ; c ; <base64> ST` 转义序列。
//! 这是**非显示序列**——只设置剪贴板选区，不改动任何屏幕 cell。因此从
//! revue 事件回调里直接写 `io::stdout()`、与帧刷新交错也是安全的：即便
//! 序列插在两帧输出之间，终端处理它时不会破坏画面（金律：成形不漂移）。
//!
//! 优先使用桌面环境的剪贴板命令，因为它们的退出状态可以确认写入结果。
//! OSC52 只表示序列成功写到终端，终端仍可能因策略而忽略它，因此该路径
//! 总是同时写临时文件并由调用方显示路径，不能误报成“已复制”。

use base64::Engine;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_clipboard_command(program: &str, args: &[&str], text: &str) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "clipboard stdin unavailable"))?
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{program} exited with status {status}"
        )))
    }
}

fn try_native_clipboard(text: &str) -> io::Result<()> {
    let mut candidates: Vec<(&str, &[&str])> = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        candidates.push(("wl-copy", &[]));
    }
    if std::env::var_os("DISPLAY").is_some() {
        candidates.push(("xclip", &["-selection", "clipboard"]));
        candidates.push(("xsel", &["--clipboard", "--input"]));
    }
    if cfg!(target_os = "macos") {
        candidates.push(("pbcopy", &[]));
    }
    if cfg!(target_os = "windows") {
        candidates.push(("clip.exe", &[]));
    }

    let mut last_error = None;
    for (program, args) in candidates {
        match run_clipboard_command(program, args, text) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no supported native clipboard command is available",
        )
    }))
}

/// Probe whether a native clipboard command is discoverable in this environment.
///
/// This is intentionally an explicit, observable seam for integration tests:
/// `AGENDAO_CLIPBOARD_TEST_MODE=skip` forces an unavailable result. A failed
/// probe means tests should skip GUI-dependent assertions and exercise
/// the fallback/mock path instead; it does not modify the user's clipboard.
pub fn native_clipboard_available() -> bool {
    if clipboard_test_mode_skips(std::env::var_os("AGENDAO_CLIPBOARD_TEST_MODE")) {
        return false;
    }
    let mut candidates = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        candidates.push("wl-copy");
    }
    if std::env::var_os("DISPLAY").is_some() {
        candidates.extend(["xclip", "xsel"]);
    }
    if cfg!(target_os = "macos") {
        candidates.push("pbcopy");
    }
    if cfg!(target_os = "windows") {
        candidates.push("clip.exe");
    }
    candidates.into_iter().any(command_on_path)
}

fn command_on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join(program))
        .any(|path| path.is_file())
}

fn clipboard_test_mode_skips(mode: Option<std::ffi::OsString>) -> bool {
    mode.as_deref()
        .is_some_and(|mode| mode.to_string_lossy().eq_ignore_ascii_case("skip"))
}

/// Write to a native system clipboard and only return success after the
/// clipboard command confirms the operation.
pub fn copy(text: &str) -> io::Result<()> {
    if clipboard_test_mode_skips(std::env::var_os("AGENDAO_CLIPBOARD_TEST_MODE")) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "native clipboard disabled by AGENDAO_CLIPBOARD_TEST_MODE=skip",
        ));
    }
    try_native_clipboard(text)
}

fn send_osc52(text: &str) -> io::Result<()> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mut out = io::stdout().lock();
    // OSC52 = ESC ] 52 ; <选区=c=clipboard> ; <base64> BEL
    write!(out, "\x1b]52;c;{}\x07", b64)?;
    out.flush()
}

/// `copy_with_fallback` 的回流判别（U18①）。
pub enum CopyOutcome {
    /// A native clipboard command confirmed the write.
    Clipboard,
    /// Native clipboard confirmation was unavailable. An OSC52 request may
    /// also have been emitted, but the file is the recoverable source of truth.
    FileFallback(PathBuf),
}

/// 兜底文件路径（temp_dir 下按进程隔离；同进程复用同一文件、逐次覆盖——
/// 兜底只要"最近一次复制可取回"，不累积垃圾文件）。
fn fallback_path() -> PathBuf {
    std::env::temp_dir().join(format!("agendao-copy-{}.txt", std::process::id()))
}

/// 写兜底文件（独立成函数以便测试——OSC52 成败依赖终端，测试不可控；
/// 文件兜底是可控的那一半）。
fn write_fallback(text: &str) -> io::Result<PathBuf> {
    let path = fallback_path();
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Prefer a confirmed native clipboard write. If that is unavailable, emit
/// OSC52 for terminals that accept it and always persist a file fallback.
pub fn copy_with_fallback(text: &str) -> io::Result<CopyOutcome> {
    copy_with_fallback_using(text, copy, send_osc52, write_fallback)
}

/// Dependency-injected variant used by deterministic tests. Production callers
/// should use [`copy_with_fallback`], which supplies the real clipboard/OSC52/
/// filesystem implementations.
pub(crate) fn copy_with_fallback_using(
    text: &str,
    native: fn(&str) -> io::Result<()>,
    osc52: fn(&str) -> io::Result<()>,
    fallback: fn(&str) -> io::Result<PathBuf>,
) -> io::Result<CopyOutcome> {
    match native(text) {
        Ok(()) => Ok(CopyOutcome::Clipboard),
        Err(native_err) => {
            let osc_error = osc52(text).err();
            match fallback(text) {
                Ok(path) => Ok(CopyOutcome::FileFallback(path)),
                Err(fs_err) => Err(io::Error::new(
                    fs_err.kind(),
                    format!(
                        "native clipboard failed ({native_err}); OSC52 {}; file fallback failed ({fs_err})",
                        osc_error
                            .map(|error| format!("failed ({error})"))
                            .unwrap_or_else(|| "was sent without confirmation".to_string())
                    ),
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_plain_ascii() {
        // 仅验证 base64 编码正确性（OSC52 语义本身由终端解释，这里只断言
        // 我们拼出的载荷是合规 base64，避免手写编码器引入静默错码）。
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello");
        assert_eq!(b64, "aGVsbG8=");
    }

    #[test]
    fn encodes_utf8() {
        let b64 = base64::engine::general_purpose::STANDARD.encode("你好".as_bytes());
        // 已知正确值：UTF-8 编码后 base64。
        assert_eq!(b64, "5L2g5aW9");
    }

    /// U18①：兜底文件写入可控路径——内容逐字节可取回。
    #[test]
    fn write_fallback_roundtrip() {
        let path = write_fallback("fallback 内容").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fallback 内容");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_native_command_is_reported() {
        let error =
            run_clipboard_command("agendao-definitely-missing-clipboard-command", &[], "hello")
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn injected_native_success_is_reported_as_clipboard() {
        fn native(_: &str) -> io::Result<()> {
            Ok(())
        }
        fn osc(_: &str) -> io::Result<()> {
            panic!("OSC52 must not run after native success")
        }
        fn fallback(_: &str) -> io::Result<PathBuf> {
            panic!("file fallback must not run after native success")
        }
        assert!(matches!(
            copy_with_fallback_using("hello", native, osc, fallback).unwrap(),
            CopyOutcome::Clipboard
        ));
    }

    #[test]
    fn injected_native_failure_uses_file_fallback() {
        fn native(_: &str) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::NotFound, "mock unavailable"))
        }
        fn osc(_: &str) -> io::Result<()> {
            Ok(())
        }
        fn fallback(_: &str) -> io::Result<PathBuf> {
            Ok(PathBuf::from("/tmp/mock-clipboard.txt"))
        }
        assert!(matches!(
            copy_with_fallback_using("hello", native, osc, fallback).unwrap(),
            CopyOutcome::FileFallback(path)
                if path.as_path() == std::path::Path::new("/tmp/mock-clipboard.txt")
        ));
    }

    #[test]
    fn skip_mode_is_detected_without_mutating_process_environment() {
        assert!(clipboard_test_mode_skips(Some("skip".into())));
        assert!(clipboard_test_mode_skips(Some("SKIP".into())));
        assert!(!clipboard_test_mode_skips(Some("native".into())));
        assert!(!clipboard_test_mode_skips(None));
    }
}
