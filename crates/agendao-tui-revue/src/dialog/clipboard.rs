//! 金 — OSC52 剪贴板写入。
//!
//! 终端剪贴板的事实标准：发送 `OSC 52 ; c ; <base64> ST` 转义序列。
//! 这是**非显示序列**——只设置剪贴板选区，不改动任何屏幕 cell。因此从
//! revue 事件回调里直接写 `io::stdout()`、与帧刷新交错也是安全的：即便
//! 序列插在两帧输出之间，终端处理它时不会破坏画面（金律：成形不漂移）。
//!
//! 覆盖范围：现代本机终端（iTerm2/WezTerm/kitty/Alacritty/Windows Terminal）
//! 以及 `set-clipboard on` 的 tmux。不支持 OSC52 的终端会静默忽略，不影响
//! 其余功能；OSC52 写入本身失败时走 `copy_with_fallback` 的临时文件兜底
//! （U18①：写临时文件 + toast 给路径，复制意图不无声丢失）。

use std::io::{self, Write};
use std::path::PathBuf;
use base64::Engine;

/// 把 `text` 写入终端剪贴板（OSC52，剪贴板选区 `c`）。
///
/// 返回 `io::Result` 以便调用方在写入失败时 toast 报错，而非静默吞错
/// （道纪：回流写入路径必须有可观测出口）。
pub fn copy(text: &str) -> io::Result<()> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mut out = io::stdout().lock();
    // OSC52 = ESC ] 52 ; <选区=c=clipboard> ; <base64> BEL
    write!(out, "\x1b]52;c;{}\x07", b64)?;
    out.flush()
}

/// `copy_with_fallback` 的回流判别（U18①）。
pub enum CopyOutcome {
    /// OSC52 写入成功——文本已进终端剪贴板。
    Clipboard,
    /// OSC52 写失败（stdout 断/重定向等）——已兜底写临时文件，路径给
    /// toast，用户仍可取回本次复制内容（复制意图不无声丢失）。
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

/// OSC52 优先，失败兜底写临时文件（U18①）。
///
/// 双层失败（OSC52 写失败 + 文件也写失败）才返回 Err，错误消息携带两层
/// 原因；单层失败返回 `FileFallback`，调用方 toast 路径（Warning 而非
/// Error——复制目的以另一种形式达成了）。
pub fn copy_with_fallback(text: &str) -> io::Result<CopyOutcome> {
    match copy(text) {
        Ok(()) => Ok(CopyOutcome::Clipboard),
        Err(osc_err) => match write_fallback(text) {
            Ok(path) => Ok(CopyOutcome::FileFallback(path)),
            Err(fs_err) => Err(io::Error::new(
                fs_err.kind(),
                format!("OSC52 write failed ({osc_err}); file fallback failed ({fs_err})"),
            )),
        },
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

    /// U18①：正常环境 OSC52 写入成功 → Clipboard 臂（测试环境 stdout
    /// 可写，走不到兜底；兜底臂由 write_fallback_roundtrip 覆盖）。
    #[test]
    fn copy_with_fallback_ok_is_clipboard() {
        assert!(matches!(
            copy_with_fallback("hello").unwrap(),
            CopyOutcome::Clipboard
        ));
    }
}
