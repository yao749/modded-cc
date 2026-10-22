//! 「是 / 否」确认弹窗。`y`/`Y` 是, `n`/`N`/`Esc`/`⏎` 否 (默认 N), 其余按键被吞掉 (键盘路由在
//! `App::handle_key` 里, 这里只画)。
//!
//! **支持多行 prompt** (删除订阅要列出引用它的虚拟模型): 按 `'\n'` 拆行, 宽度取最宽一行 + 8,
//! 高度是 `行数 + 4`——单行时是 5。弹窗宽度有上限 (屏幕宽减 4), 比上限还宽的一行按显示宽度折行,
//! 不被右边框截断。

use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::format::wrap;
use crate::i18n::Strings;
use crate::popup::ConfirmState;
use crate::theme::Theme;

/// 弹窗宽度下限; 比这个还窄的提示也按这个宽度画, 免得太局促。
const MIN_WIDTH: u16 = 30;
/// 弹窗与屏幕两侧边缘至少留的空隙 (合计, 不是单边)。
const SCREEN_MARGIN: u16 = 4;
/// 单行提示的高度 (行数 1 + 4)。
const HEIGHT: u16 = 5;
/// 左右边框各 1 + `Padding::new(2, 2, 1, 1)` 的左右内距各 2。
const BORDER_PAD_H: u16 = 6;

/// 按 `'\n'` 拆行, 超过 `max_width` 列的行再按显示宽度折行。`area` 与 `draw` 用同一个函数, 几何与
/// 内容不会对不上: `area` 按上限折好后定宽, `draw` 按这个宽度再折一次得到同样的行。
fn prompt_lines(prompt: &str, max_width: u16) -> Vec<String> {
    wrap(prompt, usize::from(max_width))
}

/// 最宽一行的显示宽度。
fn widest_line(lines: &[String]) -> u16 {
    lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16
}

/// 弹窗高度 = 行数 + 4, 用 [`HEIGHT`] (单行的既有高度 5) 当基准往上加——单行时加 0, 恰好等于
/// 既有的 5, 不用另外重复一份 "+4"。
fn height_for(lines: &[String]) -> u16 {
    let extra_lines = lines.len().max(1) as u16 - 1;
    HEIGHT + extra_lines
}

/// 居中, 宽 = 最宽一行的显示宽度 + 8, 夹在 `MIN_WIDTH..=screen.width - SCREEN_MARGIN` 之间,
/// 高 = 行数 + 4 (单行时是 5)。
///
/// `screen.width < MIN_WIDTH + SCREEN_MARGIN` (34) 时, `screen.width - SCREEN_MARGIN` 会小于
/// `MIN_WIDTH`——`clamp(MIN_WIDTH, 那个更小的上界)` 违反 `min <= max` 会直接 panic。
/// 用 `.max(MIN_WIDTH)` 兜底上界, 保证任何 `Rect` 传进来都不 panic; 主循环本来就不会在小于
/// `app::MIN_WIDTH`(80)/`MIN_HEIGHT`(24) 的终端上调用这个函数 (`App::draw` 的早退分支挡住了),
/// 这里只是让函数本身对任意输入都是全函数 (total function), 不依赖调用方守规矩。
pub fn area(screen: Rect, prompt: &str) -> Rect {
    let upper = screen.width.saturating_sub(SCREEN_MARGIN).max(MIN_WIDTH);
    let lines = prompt_lines(prompt, upper.saturating_sub(BORDER_PAD_H));
    let width = (widest_line(&lines) + 8).clamp(MIN_WIDTH, upper);
    screen.centered(Constraint::Length(width), Constraint::Length(height_for(&lines)))
}

pub fn draw(frame: &mut Frame, area: Rect, state: &ConfirmState, theme: &Theme, s: &Strings) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .title_top(format!(" {} ", s.confirm_title))
        .title_bottom(Line::from(format!(" {} ", s.confirm_keys)).right_aligned())
        .padding(Padding::new(2, 2, 1, 1));
    super::clear_popup_area(frame, area);
    let lines = prompt_lines(&state.prompt, area.width.saturating_sub(BORDER_PAD_H));
    let text = Text::from(lines.into_iter().map(Line::from).collect::<Vec<_>>());
    frame.render_widget(Paragraph::new(text).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_is_centered_and_clamped() {
        let screen = Rect::new(0, 0, 80, 24);

        // 短提示的宽度应该夹到下限 30, 且整体居中——直接用 ratatui 自己的 `centered` 算出预期值,
        // 不手动重算舍入方向 (`Flex::Center` 奇数余量往哪边多分 1 列是 ratatui 的实现细节)。
        let short = area(screen, "短");
        let expect_short = screen.centered(Constraint::Length(MIN_WIDTH), Constraint::Length(HEIGHT));
        assert_eq!(short, expect_short, "短提示应该夹到下限 30 并居中");

        // 超长提示的宽度应该夹到上限 screen.width - SCREEN_MARGIN, 内容按 76 - 6 = 70 列折成 3 行。
        let long = area(screen, &"x".repeat(200));
        let expect_long = screen.centered(Constraint::Length(screen.width - SCREEN_MARGIN), Constraint::Length(HEIGHT + 2));
        assert_eq!(long, expect_long, "超长提示应该夹到上限 screen-4、折行后居中");

        // 中等长度的提示 (30 + 8 = 38) 落在 30..=76 区间内, 应该正好等于「宽度+8」, 不被夹到任一端。
        let mid = area(screen, &"x".repeat(30));
        assert_eq!(mid.width, 38, "没有触顶或触底时, 宽度应该正好是提示宽度 + 8");
    }

    /// 多行 prompt (删除确认列出引用它的虚拟模型) 应该把高度撑到「行数 + 4」, 宽度按最宽一行算;
    /// 单行仍然是 5。
    #[test]
    fn a_multi_line_prompt_grows_the_popup() {
        let screen = Rect::new(0, 0, 80, 24);

        let one_line = area(screen, "短提示");
        assert_eq!(one_line.height, 5, "单行应该维持既有的高度 5");

        let three_lines = area(screen, "第一行\n第二行更长一些\n第三行");
        assert_eq!(three_lines.height, 7, "3 行应该是 7 = 3 + 4");
        // 宽度按最宽一行算: "第二行更长一些" 比其它两行都宽, 应该由它决定宽度, 不是取最后一行
        // 或者整个字符串 (含换行符) 的宽度。
        let widest_only = area(screen, "第二行更长一些");
        assert_eq!(three_lines.width, widest_only.width, "宽度应该由最宽一行决定");
    }

    /// `screen.width < MIN_WIDTH + SCREEN_MARGIN` (34) 时直接 `clamp` 会 panic (下界 30 > 上界
    /// `screen.width - 4`)。20 列宽的屏幕远小于这个阈值, 任何提示长度都不该
    /// panic——内部算出来的目标宽度会被 `.max(MIN_WIDTH)` 兜到 30, 但 `Rect::centered` 用的
    /// `Layout` 约束求解器会把它进一步夹到父矩形自己的宽度以内, 结果不会比屏幕本身更宽。
    #[test]
    fn area_does_not_panic_on_a_tiny_screen() {
        let tiny = Rect::new(0, 0, 20, 10);
        let short = area(tiny, "短");
        assert!(short.width <= tiny.width, "不该比屏幕本身更宽, 实际 {}", short.width);
        let long = area(tiny, &"x".repeat(200));
        assert!(long.width <= tiny.width, "超长提示也不该比屏幕本身更宽, 实际 {}", long.width);
    }
}
