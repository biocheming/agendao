//! Spinner — 可插拔 glyph 集 + 平台感知。
//!
//! 替代 app/mod.rs:843 附近的硬编码 10 帧 braille。提供 Braille/Dots 两套：
//! Linux 默认用 Dots（claudecode 风格的 `·✢✳✶✻✽`），其它平台用 Braille。
//! 调用方负责降速（如 `tick/3`）与 running 判定；本模块只管帧序列。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpinnerGlyph {
    Braille, // ⠋⠙⠹...（10 帧）
    Dots,    // ·✢✳✶✻✽（6 帧，claudecode 风格）
}

impl SpinnerGlyph {
    pub fn frames(&self) -> &'static [&'static str] {
        match self {
            Self::Braille => &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            Self::Dots    => &["·", "✢", "✳", "✶", "✻", "✽"],
        }
    }
}

/// 平台默认 glyph：Linux 用 Dots，其它用 Braille。
pub fn platform_default() -> SpinnerGlyph {
    if cfg!(target_os = "linux") { SpinnerGlyph::Dots } else { SpinnerGlyph::Braille }
}

/// 按 tick 取当前帧。tick 是单调递增的帧计数（调用方可先 `/3` 降速）。
pub fn frame(glyph: SpinnerGlyph, tick: u64) -> &'static str {
    let frames = glyph.frames();
    frames[(tick as usize) % frames.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braille_has_10_frames() {
        assert_eq!(SpinnerGlyph::Braille.frames().len(), 10);
    }

    #[test]
    fn dots_has_6_frames() {
        assert_eq!(SpinnerGlyph::Dots.frames().len(), 6);
    }

    #[test]
    fn frame_wraps_around() {
        // Dots 6 帧：tick 0 与 tick 6 同帧（6 % 6 == 0）
        let f0 = frame(SpinnerGlyph::Dots, 0);
        let f6 = frame(SpinnerGlyph::Dots, 6);
        assert_eq!(f0, f6);
    }

    #[test]
    fn platform_default_is_dots_on_linux() {
        #[cfg(target_os = "linux")]
        assert_eq!(platform_default(), SpinnerGlyph::Dots);
    }
}
