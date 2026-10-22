//! `?` 弹出的键位表。

use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use ratatui::Frame;

use crate::format::{fit, widest};
use crate::i18n::Strings;
use crate::theme::Theme;

/// 键名一列与弹窗宽度的下限; 键名或说明更宽时按 [`key_col`] / [`width`] 放宽, 说明文字不被截断。
const KEY_COL: usize = 18;
const WIDTH: u16 = 48;
/// 左右边框各 1 + `Padding::new(2, 2, 1, 1)` 的左右内距各 2。
const BORDER_PAD_H: u16 = 6;

fn all_rows<'a>(s: &'a Strings, page_rows: &'a [(&'a str, &'a str)]) -> impl Iterator<Item = &'a (&'a str, &'a str)> {
    s.help_rows.iter().chain(page_rows)
}

/// 键名一列: 最宽的键名 + 2 格间隔, 不低于 [`KEY_COL`]。
fn key_col(s: &Strings, page_rows: &[(&str, &str)]) -> usize {
    (widest(all_rows(s, page_rows).map(|(key, _)| *key)) + 2).max(KEY_COL)
}

/// 弹窗宽度: 放得下最宽的说明, 不低于 [`WIDTH`]; 不超过屏幕宽减 4。
fn width(screen: Rect, s: &Strings, page_rows: &[(&str, &str)]) -> u16 {
    let desc = widest(all_rows(s, page_rows).map(|(_, desc)| *desc)) as u16;
    (BORDER_PAD_H + key_col(s, page_rows) as u16 + desc).max(WIDTH).min(screen.width.saturating_sub(4))
}

/// 帮助弹窗的总行数: 全局键 + (非空时) 一行空行分隔 + 页面自己的键位。
fn total_rows(s: &Strings, page_rows: &[(&str, &str)]) -> usize {
    s.help_rows.len() + if page_rows.is_empty() { 0 } else { 1 + page_rows.len() }
}

pub fn area(screen: Rect, s: &Strings, page_rows: &[(&str, &str)]) -> Rect {
    // 行数 + 上下边框 + 上下内距
    let height = total_rows(s, page_rows) as u16 + 4;
    screen.centered(Constraint::Length(width(screen, s, page_rows)), Constraint::Length(height))
}

fn row_line<'a>(key: &'a str, desc: &'a str, key_col: usize, theme: &Theme) -> Line<'a> {
    Line::from(vec![Span::styled(fit(key, key_col), theme.accent_bold()), Span::raw(desc)])
}

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, s: &Strings, page_rows: &[(&str, &str)]) {
    let key_col = key_col(s, page_rows);
    let mut lines: Vec<Line> = s.help_rows.iter().map(|(key, desc)| row_line(key, desc, key_col, theme)).collect();
    if !page_rows.is_empty() {
        lines.push(Line::raw(""));
        lines.extend(page_rows.iter().map(|(key, desc)| row_line(key, desc, key_col, theme)));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .title_top(format!(" {} ", s.help_title))
        .title_bottom(Line::from(format!(" Esc {} ", s.key_close)).right_aligned())
        .padding(Padding::new(2, 2, 1, 1));
    super::clear_popup_area(frame, area);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::ZH;

    /// Fix round D: `area()` 本身不会因为屏幕太小而 panic
    /// (`Rect::centered` 内部的 `Layout` 约束求解器会把结果夹到父矩形自己的宽度以内); 这里补一条
    /// 回归测试锁住这个事实, 呼应 `confirm::area` / `picker::area` 的同类检查。
    #[test]
    fn area_does_not_panic_on_a_tiny_screen() {
        let tiny = Rect::new(0, 0, 20, 10);
        let a = area(tiny, &ZH, &[]);
        assert!(a.width <= tiny.width, "不该比屏幕本身更宽, 实际 {}", a.width);
    }
}
