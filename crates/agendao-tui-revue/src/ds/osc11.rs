//! OSC11 终端背景色探测 → dark/light 判定（水→土：回流探测反哺主题）。
//!
//! 真实探测需 raw mode 读写终端转义序列，环境不一定支持
//! （CI / 非 TTY / 被禁用终端）。故把"判定"抽成纯函数测，
//! 探测 IO 部分（[`detect_bg`]）尽力而为 + fallback dark。
//! 颜色真值权威仍是 `styles/base.css` 的 `:root`；此处只决定
//! 运行时 `ThemeVariant`（dark/light）切换。

/// 解析 OSC11 响应为 `(r, g, b)`。
///
/// 兼容 `rgb:HHHH/HHHH/HHHH`（16-bit 截到 8-bit，取前 2 位 hex）与
/// `rgb:HH/HH/HH`。响应形如 `\x1b]11;rgb:RRRR/GGGG/BBBB\x07`（或
/// 以 `\x1b\\` ST 结尾）。失败返回 `None`。
pub fn parse_osc11_response(resp: &str) -> Option<(u8, u8, u8)> {
    let body = resp.split("11;").nth(1)?;
    let body = body.trim_end_matches(|c: char| c == '\x07' || c == '\x1b' || c == '\\');
    let rgb = body.strip_prefix("rgb:")?;
    let parts: Vec<&str> = rgb.split('/').collect();
    if parts.len() != 3 { return None; }
    let parse = |s: &str| -> Option<u8> {
        let s = s.trim();
        // 16-bit（≥4 hex）取前 2 位截到 8-bit；8-bit（≥2 hex）整体用。
        let h = if s.len() >= 4 { &s[..2] } else if s.len() >= 2 { s } else { return None };
        u8::from_str_radix(h, 16).ok()
    };
    Some((parse(parts[0])?, parse(parts[1])?, parse(parts[2])?))
}

/// 相对亮度（ITU-R BT.601 加权）→ 是否亮背景。阈值 128。
pub fn is_light_bg(r: u8, g: u8, b: u8) -> bool {
    let lum = 0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64);
    lum > 128.0
}

/// 探测终端背景色。尽力而为：写 OSC11 query 并尝试读响应；
/// 失败 / 超时 / 非 TTY → 返回 `None`（调用方 fallback dark）。
///
/// 当前保守返回 `None`：真实 IO 接线（raw mode + 超时读）环境敏感，
/// 暂不在启动期执行，保证不 panic、不阻塞主线。纯函数
/// [`parse_osc11_response`] / [`is_light_bg`] 已单测覆盖，IO 接通后
/// 此函数只需填入探测结果即可生效。
pub fn detect_bg() -> Option<(u8, u8, u8)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_16bit_rgb() {
        let r = parse_osc11_response("\x1b]11;rgb:1a1b/2626/3838\x1b\\");
        assert_eq!(r, Some((0x1a, 0x26, 0x38))); // 截到 8-bit
    }

    #[test]
    fn parse_8bit_rgb() {
        let r = parse_osc11_response("\x1b]11;rgb:1a/1b/38\x07");
        assert_eq!(r, Some((0x1a, 0x1b, 0x38)));
    }

    #[test]
    fn parse_bel_terminated() {
        // 以 BEL (\x07) 结尾而非 ST (\x1b\\)
        let r = parse_osc11_response("\x1b]11;rgb:d4/d8/f0\x07");
        assert_eq!(r, Some((0xd4, 0xd8, 0xf0)));
    }

    #[test]
    fn parse_rejects_malformed() {
        assert_eq!(parse_osc11_response("garbage"), None);
        assert_eq!(parse_osc11_response("\x1b]11;rgb:12/34\x07"), None); // 只 2 段
        assert_eq!(parse_osc11_response("\x1b]11;#1a1b26\x07"), None); // 非 rgb: 前缀
    }

    #[test]
    fn dark_bg_not_light() {
        assert!(!is_light_bg(0x1a, 0x1b, 0x38)); // Tokyo Night bg
    }

    #[test]
    fn light_bg_is_light() {
        assert!(is_light_bg(0xd4, 0xd8, 0xf0));
    }

    #[test]
    fn detect_bg_fallback_none() {
        assert_eq!(detect_bg(), None); // 默认保守
    }
}
