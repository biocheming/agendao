//! 金 — OSC52 剪贴板写入。
//!
//! 终端剪贴板的事实标准：发送 `OSC 52 ; c ; <base64> ST` 转义序列。
//! 这是**非显示序列**——只设置剪贴板选区，不改动任何屏幕 cell。因此从
//! revue 事件回调里直接写 `io::stdout()`、与帧刷新交错也是安全的：即便
//! 序列插在两帧输出之间，终端处理它时不会破坏画面（金律：成形不漂移）。
//!
//! 覆盖范围：现代本机终端（iTerm2/WezTerm/kitty/Alacritty/Windows Terminal）
//! 以及 `set-clipboard on` 的 tmux。不支持 OSC52 的终端会静默忽略，不影响
//! 其余功能（fallback 留作后续：可写临时文件 + toast 路径）。

use std::io::{self, Write};
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
}
