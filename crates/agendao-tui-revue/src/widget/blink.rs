//! Blink 原语：600ms 周期闪烁（圆↔空格），仿 claudecode useBlink(BLINK_INTERVAL_MS=600)。
//!
//! tick 是单调递增帧计数（与 app 层 spinner_tick 同源，未经 `/3` 降速）；
//! @50ms/帧时 12 帧 = 600ms。驱动 ToolCall 执行中的 `⏺` 闪烁。

/// 600ms 周期内是否"可见"（前半周可见，后半周空白）。
pub fn blink_visible(tick: u64) -> bool {
    (tick / 12) % 2 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_half_visible_second_half_hidden() {
        assert!(blink_visible(0));    // 0-11 帧：可见
        assert!(blink_visible(11));
        assert!(!blink_visible(12));  // 12-23 帧：空白
        assert!(!blink_visible(23));
        assert!(blink_visible(24));   // 24+ 周期重复
    }
}
