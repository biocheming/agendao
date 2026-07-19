//! Spinner — 可插拔 glyph 集 + 平台感知。
//!
//! 替代 app/mod.rs:843 附近的硬编码 10 帧 braille。提供 Braille/Dots 两套：
//! Linux 默认用 Dots（`·✢✳✶✻✽` 点阵风格），其它平台用 Braille。
//! 调用方负责降速（如 `tick/3`）与 running 判定；本模块只管帧序列。

use revue::prelude::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpinnerGlyph {
    Braille, // ⠋⠙⠹...（10 帧）
    Dots,    // ·✢✳✶✻✽（6 帧）
    Claude,  // ·✢✳✶✻✽ 正放+倒放（10 帧来回，点阵往返风格）
    Wuxing,  // 木→火→土→金→水（5 帧相生流转，各相位取五行语义色）
}

impl SpinnerGlyph {
    pub fn frames(&self) -> &'static [&'static str] {
        match self {
            Self::Braille => &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            Self::Dots    => &["·", "✢", "✳", "✶", "✻", "✽"],
            // 正放 + 倒放构成完整往返周期（SpinnerGlyph 往返点阵）
            Self::Claude  => &["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"],
            // 相生序：木生火、火生土、土生金、金生水、水生木——
            // 运行中的一回合就是一次完整的五行流转（土律·阴阳闭环）。
            Self::Wuxing  => &["木", "火", "土", "金", "水"],
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

/// 潮汐帧：▁▂▃▄▅▆▇█▇▆▅▄▃▂▁ 涨落循环（14 帧一次完整潮汐）。
///
/// 设计取舍（深川·流白）：形状动、色不动——单水位起伏传递"水·回流"语义,
/// 单色雾灰克制不抢戏（宋式简淡）；不做整字 CJK 切换（全宽字符逐帧跳变
/// 视觉上过重过闪,违背"状态指示应是小动作、连续感"）。
pub fn tide_frame(tick: u64) -> &'static str {
    const TIDE: [&str; 14] = [
        "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂",
    ];
    TIDE[(tick as usize) % TIDE.len()]
}

/// 潮汐配色：雾灰单点（与 hint 文字同色系,形状自成一体不依赖色相）。
pub fn tide_color() -> Color {
    crate::theme::colors::FG_MUTED()
}

/// 墨晕帧：·∘○◉●◉○∘·（8 帧一次晕开收拢）。
///
/// 墨滴入水,晕开再收——宋式「墨分五色」里最安静的一笔。运行时逐帧晕染,
/// 静止时落定一点 ◉（见 [`INK_REST`]）。与潮汐同原则：形状动、色不动。
pub fn ink_frame(tick: u64) -> &'static str {
    const INK: [&str; 8] = ["·", "∘", "○", "◉", "●", "◉", "○", "∘"];
    INK[(tick as usize) % INK.len()]
}

/// 静止时的墨点（不转动画也要有个安静的在场）。
pub const INK_REST: &str = "◉";

/// 墨韵配色：雾灰单点（同 hint 文字,简淡）。
pub fn ink_color() -> Color {
    crate::theme::colors::FG_MUTED()
}

/// stall 时颜色向红插值：3 秒无新输出后转红（interpolateColor 简化）。
/// secs_since_last 是距上次输出的秒数；<3 返回 base，>=3 返回 ACCENT_RED。
pub fn stall_color(base: Color, secs_since_last: u64) -> Color {
    use crate::theme::colors;
    if secs_since_last < 3 { base } else { colors::ACCENT_RED() }
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
    fn claude_frames_bounce() {
        // 点阵来回帧：正放 6 + 倒放 4 = 10。中心 ✽ 为峰值，两侧对称。
        let f = SpinnerGlyph::Claude.frames();
        assert_eq!(f.len(), 10);
        assert_eq!(f[0], "·");      // 起点
        assert_eq!(f[5], "✽");      // 峰值（正放到底）
        assert_eq!(f[1], f[9]);     // ✢ ... ✢（倒放止于 ✢，省略重复的 ·）
        assert_eq!(f[4], f[6]);     // ✻ ... ✻
    }

    #[test]
    fn ink_blooms_and_contracts() {
        // 8 帧墨晕：晕开（·→●）收拢（●→∘），首尾回环。
        let frames: Vec<_> = (0..8).map(crate::widget::spinner::ink_frame).collect();
        assert_eq!(frames[0], "·");
        assert_eq!(frames[4], "●", "晕开顶点居中");
        assert_eq!(frames[1], frames[7], "晕形首尾对称（∘ 回环）");
        assert_eq!(crate::widget::spinner::ink_frame(8), crate::widget::spinner::ink_frame(0));
        assert_eq!(crate::widget::spinner::INK_REST, "◉");
    }

    #[test]
    fn tide_rises_and_falls() {
        // 14 帧完整潮汐：先涨（▁→█）后落（█→▂），首尾不接同帧重复。
        let frames: Vec<_> = (0..14).map(crate::widget::spinner::tide_frame).collect();
        assert_eq!(frames[0], "▁");
        assert_eq!(frames[7], "█", "涨潮顶点居中");
        assert_eq!(frames[1], frames[13], "潮形首尾对称（▂ 回环）");
        assert_eq!(crate::widget::spinner::tide_frame(14), crate::widget::spinner::tide_frame(0));
    }

    #[test]
    fn stall_color_red_after_3s() {
        use crate::theme::colors;
        assert_eq!(stall_color(colors::E_AMBER(), 0), colors::E_AMBER());
        assert_eq!(stall_color(colors::E_AMBER(), 2), colors::E_AMBER());
        assert_eq!(stall_color(colors::E_AMBER(), 3), colors::ACCENT_RED());
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
