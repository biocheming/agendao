//! 土 — Shared visual theme: runtime-switchable color palette authority.
//!
//! 颜色真值的单一权威（土律归一）：全部语义色由各主题 `Palette` 承载，
//! 运行时经 `set_current_palette` 切换、`colors::*()` 访问器读取。
//! 渲染每帧重新构造 widget → 每帧重读访问器 → 换色板 + redraw 即全屏生效。
//!
//! 历史上这里是一组 `pub const`（Tokyo Night 硬编码），切换主题无从谈起；
//! 现改为 Palette + 访问器，调用点仅差一对括号。CSS 侧（styles/base.css
//! 的 `:root` 变量）由 `ds::theme::root_css_vars` 程序化同步，不再是手抄副本。

use revue::prelude::Color;

/// 一套主题的完整色板。字段即原 `colors::*` 常量（snake_case 化）。
///
/// 语义分层（与 `ds::color::Semantic` 对应）：
/// - 木（用户输入）→ `e_teal`；火（工具执行）→ `e_amber`
/// - 金（助手输出）→ `fg_primary`；土/水（系统/遥测）→ `fg_muted`
/// - 状态 → `status_*()`（aliases of accent_green/yellow/red/cyan）
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    // ── Backgrounds ──
    pub bg_primary: Color,
    /// 深井（Trench）：比 bg_primary 更暗的下沉容器背景（用于 ToolResult
    /// 整体缩进深井）。亮色主题中此值比 bg_primary 更深一档，保持 JND 可辨。
    pub bg_deep: Color,
    pub bg_secondary: Color,
    pub bg_surface: Color,
    /// 无色相提亮层（预合成 alpha 的储备面，当前无消费者）。
    pub surface_raised: Color,
    /// selected dialog row 高亮（各主题的主角色淡染玻璃面）。
    pub surface_selected: Color,
    // ── Foregrounds ──
    pub fg_primary: Color,
    pub fg_secondary: Color,
    pub fg_muted: Color,
    /// 背景流 Trace（井内列表项/计数），压到「刚好看清」的极限暗度。
    pub fg_trace: Color,
    /// sidebar↔主区之间那根「极暗淡」垂直分隔线的前景色。
    pub sidebar_divider: Color,
    // ── Accents ──
    pub accent_cyan: Color,
    pub accent_green: Color,
    pub accent_yellow: Color,
    pub accent_red: Color,
    pub accent_purple: Color,
    pub accent_blue: Color,
    pub accent_orange: Color,
    /// 工具调用刚性算子 ⚒ 的专用色。
    pub nord_orange: Color,
    // ── Signature accents（木/火角色）──
    /// 木：用户气泡、selected、success badges。
    pub e_teal: Color,
    /// 火：tool chips、group headers、send button。
    pub e_amber: Color,
    // ── Borders ──
    pub border: Color,
}

impl Palette {
    /// Tokyo Night Dark — agendao 默认主题（原硬编码常量原样保留）。
    pub const fn tokyo_night() -> Self {
        Self {
            bg_primary: Color::rgb(26, 27, 38), // #1a1b26
            // 明度差 ≈ 9 阶 > JND 阈值 8，终端可辨（深川·流白「井底」之于深渊）。
            bg_deep: Color::rgb(16, 17, 26),      // #10111a
            bg_secondary: Color::rgb(36, 40, 59), // #24283b
            bg_surface: Color::rgb(47, 51, 70),   // #2f3346
            // rgba(255,255,255,0.025) pre-composited on bg_primary。
            surface_raised: Color::rgb(32, 33, 44),
            // rgba(60,184,162,0.18) pre-composited — selected row 青调玻璃面。
            surface_selected: Color::rgb(45, 70, 65),
            fg_primary: Color::rgb(192, 202, 245),    // #c0caf5
            fg_secondary: Color::rgb(169, 177, 214),  // #a9b1d6
            fg_muted: Color::rgb(86, 95, 137),        // #565f89
            fg_trace: Color::rgb(59, 66, 82),         // #3b4252
            sidebar_divider: Color::rgb(46, 52, 64),  // #2e3440 (Nord polar night 2)
            accent_cyan: Color::rgb(125, 207, 255),   // #7dcfff
            accent_green: Color::rgb(158, 206, 106),  // #9ece6a
            accent_yellow: Color::rgb(224, 175, 104), // #e0af68
            accent_red: Color::rgb(247, 118, 142),    // #f7768e
            accent_purple: Color::rgb(187, 154, 247), // #bb9af7
            accent_blue: Color::rgb(122, 162, 247),   // #7aa2f7
            accent_orange: Color::rgb(255, 184, 108), // #ffb86c
            nord_orange: Color::rgb(208, 135, 112),   // #d08770 (Nord orange)
            e_teal: Color::rgb(60, 184, 162),         // #3cb8a2
            e_amber: Color::rgb(240, 168, 82),        // #f0a852
            border: Color::rgb(59, 66, 97),           // #3b4261
        }
    }

    /// Tokyo Night Light — 官方亮色系真值（修复历史上 light 携带 dark
    /// 色值的伪权威：`Theme::custom` 从 `Theme::dark()` 克隆所致）。
    pub const fn tokyo_night_light() -> Self {
        Self {
            bg_primary: Color::rgb(213, 214, 219),   // #d5d6db
            bg_deep: Color::rgb(195, 197, 205),      // #c3c5cd
            bg_secondary: Color::rgb(203, 204, 209), // #cbccd1
            bg_surface: Color::rgb(191, 193, 203),   // #bfc1cb
            surface_raised: Color::rgb(224, 225, 230),
            surface_selected: Color::rgb(185, 213, 207), // 青调淡染
            fg_primary: Color::rgb(52, 59, 88),          // #343b58
            fg_secondary: Color::rgb(86, 90, 110),       // #565a6e
            fg_muted: Color::rgb(150, 153, 163),         // #9699a3
            fg_trace: Color::rgb(180, 182, 192),
            sidebar_divider: Color::rgb(176, 178, 190),
            accent_cyan: Color::rgb(0, 113, 151),    // #007197
            accent_green: Color::rgb(88, 117, 57),   // #587539
            accent_yellow: Color::rgb(143, 94, 21),  // #8f5e15
            accent_red: Color::rgb(245, 42, 101),    // #f52a65
            accent_purple: Color::rgb(120, 71, 189), // #7847bd
            accent_blue: Color::rgb(46, 125, 233),   // #2e7de9
            accent_orange: Color::rgb(177, 92, 0),   // #b15c00
            nord_orange: Color::rgb(177, 92, 0),
            e_teal: Color::rgb(0, 113, 151), // 木 = 青
            e_amber: Color::rgb(177, 92, 0), // 火 = 橙
            border: Color::rgb(168, 171, 189),
        }
    }

    /// 天青·汝窑 — 宋代美学亮主题。
    ///
    /// 北宋汝窑天青釉「雨过天青云破处，这般颜色做将来」：宣纸暖白为底、
    /// 松烟墨为字、天青为木（用户）、朱砂印泥为火（工具），状态色取
    /// 松绿/藤黄/绛红/黛蓝。宋式审美在「简、淡、雅」——全部角色色压
    /// 低饱和度，不取鲜亮刺激色。
    pub const fn tianqing() -> Self {
        Self {
            bg_primary: Color::rgb(242, 239, 230),   // #f2efe6 宣纸暖白
            bg_deep: Color::rgb(233, 229, 216),      // #e9e5d8 纸纹深处
            bg_secondary: Color::rgb(230, 226, 212), // #e6e2d4
            bg_surface: Color::rgb(220, 214, 196),   // #dcd6c4
            surface_raised: Color::rgb(237, 234, 221),
            surface_selected: Color::rgb(207, 220, 210), // 天青淡染
            fg_primary: Color::rgb(46, 50, 56),          // #2e3238 墨色（非纯黑）
            fg_secondary: Color::rgb(77, 83, 91),        // #4d535b
            fg_muted: Color::rgb(139, 143, 136),         // #8b8f88 烟墨淡
            fg_trace: Color::rgb(184, 180, 166),
            sidebar_divider: Color::rgb(200, 195, 178),
            accent_cyan: Color::rgb(107, 158, 147), // #6b9e93 天青
            accent_green: Color::rgb(84, 128, 94),  // #54805e 松绿
            accent_yellow: Color::rgb(168, 134, 42), // #a8862a 藤黄
            accent_red: Color::rgb(158, 61, 58),    // #9e3d3a 绛红
            accent_purple: Color::rgb(125, 107, 143), // #7d6b8f 藕荷
            accent_blue: Color::rgb(79, 107, 138),  // #4f6b8a 黛蓝
            accent_orange: Color::rgb(156, 106, 58), // #9c6a3a 赭石
            nord_orange: Color::rgb(156, 106, 58),
            e_teal: Color::rgb(95, 145, 132), // 木 = 天青 #5f9184
            e_amber: Color::rgb(181, 80, 60), // 火 = 朱砂 #b5503c
            border: Color::rgb(201, 194, 176),
        }
    }

    /// 千里江山 — 宋代美学暗主题。
    ///
    /// 王希孟《千里江山图》（北宋）青绿山水：绢本墨青为底、月白为字、
    /// 石绿为木（用户）、泥金为火（工具），石青/雌黄/珊瑚朱/紫檀矿彩
    /// 点染。矿物颜料的沉稳厚润对应暗底上的中低明度角色色。
    pub const fn qianli() -> Self {
        Self {
            bg_primary: Color::rgb(18, 26, 28),   // #121a1c 绢底墨青
            bg_deep: Color::rgb(11, 17, 19),      // #0b1113
            bg_secondary: Color::rgb(26, 38, 41), // #1a2629
            bg_surface: Color::rgb(34, 48, 51),   // #223033
            surface_raised: Color::rgb(27, 36, 39),
            surface_selected: Color::rgb(30, 58, 52), // 石绿淡染
            fg_primary: Color::rgb(216, 223, 211),    // #d8dfd3 绢上月白
            fg_secondary: Color::rgb(169, 184, 172),  // #a9b8ac
            fg_muted: Color::rgb(93, 111, 106),       // #5d6f6a
            fg_trace: Color::rgb(58, 71, 68),
            sidebar_divider: Color::rgb(40, 52, 58),
            accent_cyan: Color::rgb(111, 159, 208), // #6f9fd0 石青
            accent_green: Color::rgb(106, 174, 124), // #6aae7c 三绿
            accent_yellow: Color::rgb(211, 174, 78), // #d3ae4e 雌黄
            accent_red: Color::rgb(207, 107, 85),   // #cf6b55 珊瑚朱
            accent_purple: Color::rgb(154, 138, 184), // #9a8ab8 紫檀
            accent_blue: Color::rgb(125, 168, 216), // #7da8d8 石青亮
            accent_orange: Color::rgb(192, 131, 84), // #c08354 赭石
            nord_orange: Color::rgb(192, 131, 84),
            e_teal: Color::rgb(76, 174, 138),  // 木 = 石绿 #4cae8a
            e_amber: Color::rgb(201, 160, 90), // 火 = 泥金 #c9a05a
            border: Color::rgb(44, 62, 66),
        }
    }
}

thread_local! {
    /// 当前生效色板（阴面记账）。TUI 渲染为单线程事件循环，thread_local
    /// Cell 零锁读取；每帧重读，切换后下一帧即全屏生效。
    static CURRENT: std::cell::Cell<Palette> = const { std::cell::Cell::new(Palette::tokyo_night()) };
}

/// 读当前色板（每帧渲染路径调用，Copy 零成本）。
pub fn current_palette() -> Palette {
    CURRENT.with(|c| c.get())
}

/// 切换当前色板。唯一写入口是 `ds::theme::apply_theme`（土律归一）。
pub fn set_current_palette(p: Palette) {
    CURRENT.with(|c| c.set(p));
}

/// 语义色访问器——原 `colors::X` 常量的运行时化替身。
///
/// 调用点写作 `colors::FG_PRIMARY()`（比常量只多一对括号）。
/// 非蛇形命名是有意为之：保持与历史常量同名，codemod 成本最低。
#[allow(non_snake_case)]
pub mod colors {
    use super::{current_palette, Color};

    // ── Backgrounds ──
    pub fn BG_PRIMARY() -> Color {
        current_palette().bg_primary
    }
    pub fn BG_DEEP() -> Color {
        current_palette().bg_deep
    }
    pub fn BG_SECONDARY() -> Color {
        current_palette().bg_secondary
    }
    pub fn BG_SURFACE() -> Color {
        current_palette().bg_surface
    }
    pub fn SURFACE_SELECTED() -> Color {
        current_palette().surface_selected
    }
    /// selected row 高亮（surface_selected 的别名，保持历史命名）。
    pub fn BG_HIGHLIGHT() -> Color {
        current_palette().surface_selected
    }

    // ── Foregrounds ──
    pub fn FG_PRIMARY() -> Color {
        current_palette().fg_primary
    }
    pub fn FG_SECONDARY() -> Color {
        current_palette().fg_secondary
    }
    pub fn FG_MUTED() -> Color {
        current_palette().fg_muted
    }
    pub fn FG_TRACE() -> Color {
        current_palette().fg_trace
    }
    pub fn SIDEBAR_DIVIDER() -> Color {
        current_palette().sidebar_divider
    }

    // ── Accents ──
    pub fn ACCENT_CYAN() -> Color {
        current_palette().accent_cyan
    }
    pub fn ACCENT_GREEN() -> Color {
        current_palette().accent_green
    }
    pub fn ACCENT_YELLOW() -> Color {
        current_palette().accent_yellow
    }
    pub fn ACCENT_RED() -> Color {
        current_palette().accent_red
    }
    pub fn ACCENT_PURPLE() -> Color {
        current_palette().accent_purple
    }
    pub fn ACCENT_BLUE() -> Color {
        current_palette().accent_blue
    }
    pub fn NORD_ORANGE() -> Color {
        current_palette().nord_orange
    }

    // ── Signature accents ──
    pub fn E_TEAL() -> Color {
        current_palette().e_teal
    }
    pub fn E_AMBER() -> Color {
        current_palette().e_amber
    }

    // ── Borders ──
    pub fn BORDER() -> Color {
        current_palette().border
    }

    // ── Semantic status aliases ──
    pub fn STATUS_OK() -> Color {
        current_palette().accent_green
    }
    pub fn STATUS_WARN() -> Color {
        current_palette().accent_yellow
    }
    pub fn STATUS_ERROR() -> Color {
        current_palette().accent_red
    }
    pub fn STATUS_INFO() -> Color {
        current_palette().accent_cyan
    }
}

/// Helper: format token counts with K suffix (e.g. "1.2k", "456").
pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// Helper: format cost with appropriate precision.
pub fn fmt_cost(cost: f64) -> String {
    if cost < 0.001 {
        format!("${:.4}", cost)
    } else if cost < 1.0 {
        format!("${:.3}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_is_tokyo_night() {
        assert_eq!(current_palette(), Palette::tokyo_night());
    }

    #[test]
    fn set_current_palette_switches_reads() {
        set_current_palette(Palette::tianqing());
        assert_eq!(colors::BG_PRIMARY(), Palette::tianqing().bg_primary);
        assert_eq!(colors::E_TEAL(), Palette::tianqing().e_teal);
        // 还原，避免污染同线程其他测试。
        set_current_palette(Palette::tokyo_night());
        assert_eq!(colors::BG_PRIMARY(), Palette::tokyo_night().bg_primary);
    }

    #[test]
    fn four_themes_have_distinct_signature_colors() {
        let palettes = [
            Palette::tokyo_night(),
            Palette::tokyo_night_light(),
            Palette::tianqing(),
            Palette::qianli(),
        ];
        for (i, a) in palettes.iter().enumerate() {
            for b in palettes.iter().skip(i + 1) {
                assert_ne!(a.bg_primary, b.bg_primary, "bg_primary 撞车");
                assert_ne!(a.e_teal, b.e_teal, "e_teal（木）撞车");
                assert_ne!(a.e_amber, b.e_amber, "e_amber（火）撞车");
            }
        }
    }

    #[test]
    fn status_aliases_track_accents() {
        let p = Palette::qianli();
        set_current_palette(p);
        assert_eq!(colors::STATUS_OK(), p.accent_green);
        assert_eq!(colors::STATUS_ERROR(), p.accent_red);
        assert_eq!(colors::BG_HIGHLIGHT(), p.surface_selected);
        set_current_palette(Palette::tokyo_night());
    }
}
