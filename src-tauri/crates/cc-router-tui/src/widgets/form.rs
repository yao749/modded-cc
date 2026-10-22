//! 表单的**纯渲染**: 调用方每帧把自己的状态映射成一列 [`FormRow`], 这里只管画。它不持有任何
//! 状态, 也不处理按键——文本编辑由调用方自己的 `tui_input::Input` 负责 (与 `widgets::picker`
//! 把输入框状态放在 `PickerState` 里是同一条分工)。
//!
//! 画法细节 (决定渲染字节, 与 `widgets::picker::draw` 保持一致的地方都直接照抄):
//! - 聚焦行前缀 `"▌ "`, 非聚焦行 `"  "` (与列表选中符号一致)。
//! - 标签用 [`crate::format::fit`] 定宽到 [`label_col`]; 聚焦行 `accent_bold`, 否则 `muted`。
//! - 值列宽度用 `Layout` 的 `Constraint::Min(0)` 自动吃掉 "剩余宽度 - hint 宽度 - 1 格间隔",
//!   不手算——超长的值交给 [`crate::format::fit`] 截断 (无光标行) 或 `Paragraph::scroll` 裁切
//!   (有光标行)。
//! - `error` 另起一行, 缩进到值那一列, `⚠ ` 前缀 + warn 色。
//! - `Note` 按显示宽度折行 (`crate::format::wrap`), 最多 3 行, 超出时第 3 行截断收尾补 `…`——
//!   与订阅详情页「最近错误」超长截断同一套先例 (创建失败时后端的报错原文可能很长)。
//! - `Button` 居中画 `"[ 标签 ]"`, 聚焦时**只给这一段 span** 加 `REVERSED`——`Line` 自己的
//!   `style()` 会把整个 `area` 宽度都铺上反色 (`Buffer::set_style` 先垫一层背景再画 span), 焦点
//!   移到按钮上时一整行 (含左右大片空白) 都被反色, 看起来像列表选中条, 方括号失去意义。所以只在
//!   `Span::styled` 上加修饰符, `Line` 本身不设 `style`。`busy` 时标签
//!   前面插一个 throbber 符号 (`to_symbol_span` 自带的尾随空格正好当分隔)。
//! - 光标: **照抄 `widgets::picker::draw` 的算法**——可视宽度先减 1 再算
//!   滚动量, 光标 x 再 `.min(右边界 - 1)`, 否则文本正好填满输入框时光标会画在最后一个字符上面
//!   而不是紧跟其后的空位。`form.rs` 没有 `tui_input::Input` 可以借, 所以 [`visual_scroll`] 是
//!   照同一份 `char` 宽度对齐规则重写的一份, 输入换成 `FormRow::Field::cursor` 那个已经算好的
//!   显示列偏移。`FormView::show_cursor` 为假时 (有弹窗叠在表单上面) 整个关口统一跳过
//!   `set_cursor_position`——调用方不用对每个文本行各自算一遍"弹窗开着就不设光标"。
//! - 步骤条 `area.width < 60` 时不画 (与总览页 logo 同一条让位原则)。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use ratatui::Frame;
use throbber_widgets_tui::{Throbber, BRAILLE_SIX};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::format::{fit, widest, wrap};
use crate::i18n::Strings;
use crate::theme::Theme;
use crate::widgets::spinner_state;

/// `Note` 行最多画几行, 超出截断收尾补 `…` —— 与订阅详情页「最近错误」同一套上限规则。
const NOTE_MAX_ROWS: usize = 3;

/// 标签列宽度下限; 所有字段的值左对齐到同一条竖线。实际宽度见 [`label_col`]。
const LABEL_COL: usize = 12;

/// 聚焦/非聚焦行前缀 (`"▌ "` / `"  "`) 的显示宽度。
const PREFIX_COL: u16 = 2;
/// 值列与右端 hint 之间固定留一格间隔。
const HINT_GAP: u16 = 1;
/// 步骤条低于这个宽度就不画。
const STEP_BAR_MIN_WIDTH: u16 = 60;

pub enum FormRow<'a> {
    /// 一个字段。`value` 是**已经处理好的显示文本**(掩码、占位由调用方决定)。
    Field {
        label: &'a str,
        value: &'a str,
        /// `value` 为空时画的灰字提示。
        placeholder: &'a str,
        /// 右端灰字, 比如「⏎ 选择」「Ctrl+R 显示」。
        hint: Option<&'a str>,
        /// 聚焦且可输入时, 光标在 `value` 里的**显示列**偏移 (`Input::visual_cursor()`)。
        /// `None` = 这一行不是文本输入 (选择行 / 锁定行)。
        cursor: Option<usize>,
        /// 校验失败的原因, 画在这一行正下方 (warn 色, 前缀 `⚠ `)。
        error: Option<&'a str>,
        /// 不可编辑 (锁定的鉴权头): 值画成 muted。
        locked: bool,
    },
    /// 居中的按钮。`busy` 时前面画 throbber。
    Button { label: &'a str, busy: bool },
    /// 整行说明 (muted)。
    Note { text: &'a str },
    Spacer,
}

impl FormRow<'_> {
    /// 这一行占几个终端行: 带错误信息的字段行额外占一行; `Note` 按 `width` 折行, 封顶
    /// `NOTE_MAX_ROWS`——所以行高现在依赖横向宽度, 不是一个纯常量。
    fn height(&self, width: u16) -> u16 {
        match self {
            FormRow::Field { error: Some(_), .. } => 2,
            FormRow::Note { text } => note_lines(text, width).len() as u16,
            _ => 1,
        }
    }
}

/// 一边 push 行、一边声明「这一行是不是焦点」, 焦点下标由它记下——调用方不维护「字段 → 行下标」
/// 映射表, 也不用为前置的说明行 / 空行手算偏移 (手写映射的话, 增删一行漏改映射, 焦点标记就画错行
/// 而编译器不报错)。
#[derive(Default)]
pub struct FormBuilder<'a> {
    rows: Vec<FormRow<'a>>,
    focus: Option<usize>,
}

/// [`FormBuilder::finish`] 的产物: 交给 [`FormView`] 的 `rows` / `focus`。
pub struct FormRows<'a> {
    pub rows: Vec<FormRow<'a>>,
    /// 聚焦行的下标; 没有任何一行声明为焦点时是 0。
    pub focus: usize,
}

impl<'a> FormBuilder<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一行。`focused` 为真时记下它的下标——只认**第一个**声明为焦点的行 (一张表单同一时刻
    /// 只有一个焦点, 调用方按 `字段 == 焦点` 传, 不会出现两个真)。
    pub fn push(&mut self, row: FormRow<'a>, focused: bool) {
        if focused && self.focus.is_none() {
            self.focus = Some(self.rows.len());
        }
        self.rows.push(row);
    }

    pub fn finish(self) -> FormRows<'a> {
        FormRows { focus: self.focus.unwrap_or(0), rows: self.rows }
    }
}

pub struct FormView<'a> {
    pub title: &'a str,
    /// 步骤指示条: `(当前步下标, 全部步骤名)`。`None` = 不画 (自定义单页)。
    pub steps: Option<(usize, &'a [&'a str])>,
    pub rows: &'a [FormRow<'a>],
    /// 聚焦的行在 `rows` 里的下标 (由 [`FormBuilder`] 产出)。
    pub focus: usize,
    /// 画 throbber 用 (与「重连中」同一套 `widgets::spinner_state`)。
    pub tick: u64,
    /// 有弹窗叠在表单上面时调用方传 `false`——统一在这里忽略所有行的 `cursor`(不调
    /// `set_cursor_position`), 不用让每个文本行各自算一遍 `(!popup_open).then(...)`——每加一个文本行
    /// 就得记得抄一遍这个条件, 漏一个就是「弹窗下面光标在闪」。
    pub show_cursor: bool,
}

/// 画在 `area` 里: `Block::bordered()` + `BorderType::Rounded`, `title_top` 左边是 `title`、
/// 右边是步骤条, `Padding::new(2, 2, 1, 1)`。**内容超过可视高度时按焦点行滚动**: 从头画到满就
/// 截断的话, 焦点行 (含它下面的错误行) 完全可能被截在看不见的地方——自定义表单 14 行内容在 80×24
/// 下内容区可用高度只有 16 行, 带一条会折成 2 行的说明时总高度 17 行, 创建失败时焦点常常停在最后
/// 一行的「创建」按钮上。算法: 焦点行底边 (含它
/// 自己的高度, 用 `FormRow::height` 按真实高度算, 不是按行数) ≤ 可用高度就从第 0 行画起; 否则
/// 起点 = 焦点行底边 − 可用高度, 再向上取整到最近的行边界 (只能整行跳过, 不能把一行从中间切开)。
/// 需要滚动时保守地给顶部/底部提示行各留一行 (哪怕最终只有一侧真的截断)——换一次能一步算清楚的
/// 起点, 不用先画一遍猜、猜错了再回头重算。
///
/// **返回聚焦行的矩形**, 聚焦行没画出来时是 `None`, 只有需要滚动时的两种情况:
/// - 内容区不到 3 行 (`inner.height < 3`, 去掉上下两条提示行一行都不剩): 整块内容连同提示行
///   一行都不画, 免得画出内容区之外。
/// - 焦点行自己比可用高度 (`inner.height - 2`) 还高 (表单里最高的可聚焦行是「字段 + 错误」两行):
///   上面的起点算法会把焦点行整个跳过, 从它的下一行画起。起点那一行总是整行画出, 哪怕它也比可用
///   高度高——这时它和底部提示行会压到内边距上。
///
/// 80×24 起两种情况都不会发生 (内容区可用高度至少 16 行)。调用方 (向导) 拿它播
/// `fx::field_err`——几何只有 `draw` 知道, 而校验失败时焦点一定就在出错的那一行, 所以
/// 一个矩形就够。
pub fn draw(frame: &mut Frame, area: Rect, view: &FormView, theme: &Theme, s: &'static Strings) -> Option<Rect> {
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .padding(Padding::new(2, 2, 1, 1))
        .title_top(format!(" {} ", view.title));
    if let Some((current, steps)) = view.steps {
        if area.width >= STEP_BAR_MIN_WIDTH {
            block = block.title_top(step_bar(current, steps, theme).right_aligned());
        }
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let heights: Vec<u16> = view.rows.iter().map(|r| r.height(inner.width)).collect();
    let total_height: u16 = heights.iter().sum();

    let (first_visible, usable) = if total_height <= inner.height {
        // 全部放得下: 不滚动、不留提示行, 从第一行画起——与旧行为完全一致。
        (0usize, inner.height)
    } else {
        let usable = inner.height.saturating_sub(2);
        if usable == 0 {
            return None;
        }
        let focus_idx = view.focus.min(heights.len().saturating_sub(1));
        let focus_bottom: u16 = heights[..=focus_idx].iter().sum();
        let start_height = focus_bottom.saturating_sub(usable);

        let mut skipped = 0u16;
        let mut first = heights.len();
        for (i, h) in heights.iter().enumerate() {
            if skipped >= start_height {
                first = i;
                break;
            }
            skipped += *h;
        }
        (first, usable)
    };

    // 从 `first_visible` 起, 按 `usable` 装得下多少整行——顺带算出下方是否被截断; 上方截没截
    // 由 `first_visible > 0` 直接得出。
    let mut shown = 0u16;
    let mut last_visible = first_visible;
    let mut bottom_cut = false;
    for (i, h) in heights.iter().enumerate().skip(first_visible) {
        if shown + h > usable {
            bottom_cut = true;
            break;
        }
        shown += h;
        last_visible = i;
    }
    let top_cut = first_visible > 0;

    let mut y = inner.y;
    if top_cut {
        frame.render_widget(Line::raw(s.form_more).centered().style(theme.muted_style()), Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }

    // `last_visible` 从 `first_visible` 起步、只增不减 (上面那个 for 循环的初值与推进方式保证),
    // 减法不会下溢。
    let label_col = label_col(view.rows);
    let mut focus_rect = None;
    let visible_count = last_visible + 1 - first_visible;
    for (i, row) in view.rows.iter().enumerate().skip(first_visible).take(visible_count) {
        let needed = heights[i];
        let row_rect = Rect::new(inner.x, y, inner.width, needed);
        draw_row(frame, row_rect, row, label_col, i == view.focus, theme, view.tick, view.show_cursor);
        if i == view.focus {
            focus_rect = Some(row_rect);
        }
        y += needed;
    }

    if bottom_cut {
        frame.render_widget(Line::raw(s.form_more).centered().style(theme.muted_style()), Rect::new(inner.x, y, inner.width, 1));
    }

    focus_rect
}

/// 标签列宽度: 这张表单里最宽的字段标签 + 2 格间隔, 不低于 [`LABEL_COL`]。按表单自己的行算 (不是
/// 全局常量), 英文标签比中文宽时整列一起右移, 值仍然对齐。
fn label_col(rows: &[FormRow]) -> usize {
    let labels = rows.iter().filter_map(|row| match row {
        FormRow::Field { label, .. } => Some(*label),
        FormRow::Button { .. } | FormRow::Note { .. } | FormRow::Spacer => None,
    });
    (widest(labels) + 2).max(LABEL_COL)
}

fn step_bar(current: usize, steps: &[&str], theme: &Theme) -> Line<'static> {
    // 首尾各留一格, 跟左边 `title_top(format!(" {} ", view.title))` 同一条规矩——不留白的话步骤条
    // 会贴着圆角边框的拐角画, 跟左边标题的呼吸感不一致。
    let mut spans = Vec::with_capacity(steps.len() * 2 + 2);
    spans.push(Span::raw(" "));
    for (i, step) in steps.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" ── "));
        }
        let style = if i == current { theme.accent_bold() } else { theme.muted_style() };
        spans.push(Span::styled((*step).to_string(), style));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

#[allow(clippy::too_many_arguments)]
fn draw_row(frame: &mut Frame, area: Rect, row: &FormRow, label_col: usize, focused: bool, theme: &Theme, tick: u64, show_cursor: bool) {
    match row {
        FormRow::Field { label, value, placeholder, hint, cursor, error, locked } => {
            let line_area = Rect { height: 1, ..area };
            let cell = FieldCell { label, label_col, value, placeholder, hint: *hint, cursor: *cursor, locked: *locked };
            draw_field_line(frame, line_area, &cell, focused, theme, show_cursor);
            if let Some(err) = error {
                let error_area = Rect { y: area.y + 1, height: 1, ..area };
                draw_field_error(frame, error_area, err, label_col, theme);
            }
        }
        FormRow::Button { label, busy } => draw_button(frame, area, label, *busy, focused, tick),
        FormRow::Note { text } => draw_note(frame, area, text, theme),
        FormRow::Spacer => {}
    }
}

/// 按显示宽度折行, 最多 `NOTE_MAX_ROWS` 行, 超出时最后一行截断收尾补 `…`。
fn note_lines(text: &str, width: u16) -> Vec<String> {
    let width = width.max(1) as usize;
    let mut lines = wrap(text, width);
    if lines.len() > NOTE_MAX_ROWS {
        lines.truncate(NOTE_MAX_ROWS);
        if let Some(last) = lines.last_mut() {
            *last = ellipsize(last, width);
        }
    }
    lines
}

/// 强制截断收尾补 `…`, 不管 `text` 本身是否已经等于 `width`——跟 [`crate::format::fit`] 的
/// "已经放得下就不截" 不同: 这里调用方已经知道"后面还有更多内容被砍掉了", 必须显式提示,
/// 不能因为这一行凑巧正好填满 `width` 就悄悄放过不加省略号。
fn ellipsize(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

fn draw_note(frame: &mut Frame, area: Rect, text: &str, theme: &Theme) {
    for (i, line) in note_lines(text, area.width).into_iter().enumerate() {
        let line_area = Rect { y: area.y + i as u16, height: 1, ..area };
        frame.render_widget(Line::raw(line).style(theme.muted_style()), line_area);
    }
}

/// 一个字段行要画的内容 (标签列宽由整张表单统一算好传进来)。
struct FieldCell<'a> {
    label: &'a str,
    label_col: usize,
    value: &'a str,
    placeholder: &'a str,
    hint: Option<&'a str>,
    cursor: Option<usize>,
    locked: bool,
}

fn draw_field_line(frame: &mut Frame, area: Rect, cell: &FieldCell, focused: bool, theme: &Theme, show_cursor: bool) {
    let FieldCell { label, label_col, value, placeholder, hint, cursor, locked } = *cell;
    let prefix = if focused { "▌ " } else { "  " };
    let hint_width = hint.map(|h| h.width() as u16).unwrap_or(0);
    let label_style = if focused { theme.accent_bold() } else { theme.muted_style() };

    let [prefix_area, label_area, value_area, _gap_area, hint_area] = Layout::horizontal([
        Constraint::Length(PREFIX_COL),
        Constraint::Length(label_col as u16),
        Constraint::Min(0),
        Constraint::Length(HINT_GAP),
        Constraint::Length(hint_width),
    ])
    .areas(area);

    frame.render_widget(Span::raw(prefix), prefix_area);
    frame.render_widget(Span::styled(fit(label, label_col), label_style), label_area);

    let show_placeholder = value.is_empty();
    let text = if show_placeholder { placeholder } else { value };
    let value_style = if show_placeholder || locked { theme.muted_style() } else { Style::new() };

    if let Some(pos) = cursor {
        // 可视宽度先减 1 再算滚动量, 否则文本正好填满时光标会画在最后一个
        // 字符上面而不是紧跟其后的空位。
        let visual_width = value_area.width.max(1).saturating_sub(1) as usize;
        let scroll = visual_scroll(text, pos, visual_width);
        frame.render_widget(Paragraph::new(text).style(value_style).scroll((0, scroll as u16)), value_area);
        // 弹窗叠在表单上面时 `show_cursor` 为假: 值本身照常显示 (`scroll` 已经按光标位置算好),
        // 只是不去调 `set_cursor_position`——没有任何一行会在这一帧设终端光标, 效果就是光标隐藏。
        if focused && show_cursor {
            let cursor_x = value_area.x + pos.saturating_sub(scroll) as u16;
            frame.set_cursor_position((cursor_x.min(value_area.right().saturating_sub(1)), value_area.y));
        }
    } else {
        frame.render_widget(Paragraph::new(fit(text, value_area.width as usize)).style(value_style), value_area);
    }

    if let Some(hint) = hint {
        frame.render_widget(Span::styled(hint, theme.muted_style()), hint_area);
    }
}

/// 照抄 `tui_input::Input::visual_scroll` 的算法 (逐字符累加宽度直到追上目标滚动量, 保证滚动
/// 永远落在字符边界上, 不会把一个宽字符从中间切开)——`form.rs` 不持有 `Input`, 只有调用方已经
/// 算好的显示列光标 (`pos`), 所以按同一份规则在这里重算一次滚动量。
fn visual_scroll(text: &str, pos: usize, width: usize) -> usize {
    let target = pos.max(width) - width;
    let mut consumed = 0usize;
    let mut chars = text.chars();
    while consumed < target {
        match chars.next() {
            Some(c) => consumed += c.width().unwrap_or(0),
            None => break,
        }
    }
    consumed
}

fn draw_field_error(frame: &mut Frame, area: Rect, error: &str, label_col: usize, theme: &Theme) {
    let indent = PREFIX_COL + label_col as u16;
    let text_area = Rect { x: area.x + indent.min(area.width), width: area.width.saturating_sub(indent), ..area };
    frame.render_widget(Line::raw(format!("⚠ {error}")).style(Style::new().fg(theme.warn)), text_area);
}

fn draw_button(frame: &mut Frame, area: Rect, label: &str, busy: bool, focused: bool, tick: u64) {
    let text = if busy {
        let state = spinner_state(tick);
        // `to_symbol_span` 自带一个尾随空格, 正好当符号与标签之间的分隔, 不用再手动加。
        let spinner = Throbber::default().throbber_set(BRAILLE_SIX).to_symbol_span(&state);
        format!("[ {}{label} ]", spinner.content)
    } else {
        format!("[ {label} ]")
    };
    // REVERSED 只加在这个 span 上, 不能调 `Line::style()`——`Line` 的 `style` 会在渲染时先给
    // 整个 `area` 宽度垫一层背景 (`Buffer::set_style`), 于是聚焦时按钮两侧大片空白也会被反色,
    // 看起来像列表选中条。`Span::styled` 的样式只覆盖它自己的字符, `Line` 本身留默认 `Style`。
    let style = if focused { Style::new().add_modifier(Modifier::REVERSED) } else { Style::new() };
    let line = Line::from(Span::styled(text, style)).centered();
    frame.render_widget(line, area);
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::i18n::ZH;
    use crate::theme::ColorMode;

    fn field<'a>(label: &'a str, value: &'a str) -> FormRow<'a> {
        FormRow::Field { label, value, placeholder: "", hint: None, cursor: None, error: None, locked: false }
    }

    fn render(view: &FormView, width: u16, height: u16) -> String {
        let theme = Theme::new(ColorMode::TrueColor);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| { draw(frame, frame.area(), view, &theme, &ZH); }).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn the_focused_row_is_marked_and_others_are_not() {
        let rows = vec![field("厂商", "智谱"), field("接入点", "国内版")];
        let view = FormView { title: "T", steps: None, rows: &rows, focus: 0, tick: 0, show_cursor: true };
        let out = render(&view, 50, 12);
        let lines: Vec<&str> = out.lines().collect();

        let provider_line = lines.iter().find(|l| l.contains("智谱")).expect(&out);
        assert!(provider_line.contains('▌'), "聚焦行应该带 ▌ 前缀\n{out}");

        let endpoint_line = lines.iter().find(|l| l.contains("国内版")).expect(&out);
        assert!(!endpoint_line.contains('▌'), "非聚焦行不该带 ▌ 前缀\n{out}");
    }

    #[test]
    fn an_empty_value_shows_the_placeholder() {
        let rows = vec![FormRow::Field {
            label: "备注名",
            value: "",
            placeholder: "留空自动生成",
            hint: None,
            cursor: None,
            error: None,
            locked: false,
        }];
        let view = FormView { title: "T", steps: None, rows: &rows, focus: 0, tick: 0, show_cursor: true };
        let out = render(&view, 50, 12);
        assert!(out.contains("留空自动生成"), "{out}");
    }

    #[test]
    fn a_field_error_is_drawn_under_its_row() {
        let rows = vec![FormRow::Field {
            label: "API Key",
            value: "",
            placeholder: "",
            hint: None,
            cursor: None,
            error: Some("API Key 不能为空"),
            locked: false,
        }];
        let view = FormView { title: "T", steps: None, rows: &rows, focus: 0, tick: 0, show_cursor: true };
        let out = render(&view, 50, 12);
        let lines: Vec<&str> = out.lines().collect();

        let label_idx = lines.iter().position(|l| l.contains("API Key")).expect(&out);
        assert!(lines[label_idx + 1].contains("⚠ API Key 不能为空"), "错误应该画在字段行正下方\n{out}");
    }

    #[test]
    fn the_step_bar_disappears_on_a_narrow_area() {
        // 用短 ASCII 步骤名 (不是真实 i18n 文案), 避开宽字符/圆圈数字的显示宽度歧义——这里只关心
        // "够不够 60 列就画不画", 不关心真实文案在边界宽度下会不会被裁切。
        let rows = vec![field("厂商", "智谱")];
        let steps: [&str; 2] = ["STEP1", "STEP2"];
        let view = FormView { title: "新建订阅", steps: Some((0, &steps)), rows: &rows, focus: 0, tick: 0, show_cursor: true };

        let wide = render(&view, 60, 10);
        assert!(wide.contains("STEP1"), "宽度够时应该画步骤条\n{wide}");

        let narrow = render(&view, 59, 10);
        assert!(!narrow.contains("STEP1"), "宽度不够时步骤条应该让位\n{narrow}");
    }

    /// `Note` 太长时按宽度折行、封顶 3 行, 第 3 行截断收尾补 `…`——不能硬切成一行撞在单词中间,
    /// 也不能无限往下长占满整张表单。
    #[test]
    fn a_long_note_wraps_up_to_three_lines_and_the_third_ends_with_an_ellipsis() {
        let text = "A".repeat(300);
        let rows = vec![FormRow::Note { text: &text }];
        let view = FormView { title: "T", steps: None, rows: &rows, focus: 0, tick: 0, show_cursor: true };
        let out = render(&view, 50, 12);
        let a_lines: Vec<&str> = out.lines().filter(|l| l.contains('A')).collect();
        assert_eq!(a_lines.len(), NOTE_MAX_ROWS, "应该最多折 {NOTE_MAX_ROWS} 行\n{out}");
        assert!(a_lines[NOTE_MAX_ROWS - 1].contains('…'), "最后一行应该以省略号收尾\n{out}");
        // 前两行不该被截, 应该是纯 'A' 填满一整行 (折行本身工作正常, 不是每行都强行加省略号)。
        assert!(!a_lines[0].contains('…'), "第一行不该有省略号\n{out}");
    }

    /// 内容超过可视高度时按焦点行滚动, 而不是简单截断。8 行内容塞进只有 5 行可用高度的区域, 焦点
    /// 在最后一行 (下标 7): 应该滚到能看见焦点行, 顶部因此被截 (前几行不可见、顶部出现
    /// `form_more`), 返回值是 `Some`。
    #[test]
    fn content_taller_than_the_area_scrolls_so_the_focus_row_stays_visible() {
        let labels = ["行0", "行1", "行2", "行3", "行4", "行5", "行6", "行7"];
        let rows: Vec<FormRow> = labels.iter().map(|l| field(l, "值")).collect();
        let view = FormView { title: "T", steps: None, rows: &rows, focus: rows.len() - 1, tick: 0, show_cursor: true };

        let theme = Theme::new(ColorMode::TrueColor);
        // area 高 9: 去掉上下边框 (2) 与 padding 上下 (2) 剩 5 行可用, 装不下 8 行。
        let mut terminal = Terminal::new(TestBackend::new(50, 9)).unwrap();
        let mut focus_rect = None;
        terminal
            .draw(|frame| {
                focus_rect = draw(frame, frame.area(), &view, &theme, &ZH);
            })
            .unwrap();
        let out = terminal.backend().to_string();
        assert!(out.contains(ZH.form_more), "顶部被滚出去的内容应该显示提示行\n{out}");
        assert!(out.contains("行7"), "聚焦行 (最后一行) 应该被滚进可视区域\n{out}");
        assert!(!out.contains("行0"), "被滚出去的行不该再出现\n{out}");
        assert!(focus_rect.is_some(), "聚焦行现在应该可见, 应该返回它的矩形\n{out}");
    }

    /// 聚焦第一行时应该从顶部开始画 (不该无谓地把它也滚出视野), 下方装不下的内容
    /// 用 `form_more` 提示, 不该同时出现顶部提示 (那意味着起点算错、平白多滚了一段)。
    #[test]
    fn content_taller_than_the_area_starts_from_the_top_when_focus_is_near_the_top() {
        let labels = ["行0", "行1", "行2", "行3", "行4", "行5", "行6", "行7"];
        let rows: Vec<FormRow> = labels.iter().map(|l| field(l, "值")).collect();
        let view = FormView { title: "T", steps: None, rows: &rows, focus: 0, tick: 0, show_cursor: true };

        let theme = Theme::new(ColorMode::TrueColor);
        let mut terminal = Terminal::new(TestBackend::new(50, 9)).unwrap();
        let mut focus_rect = None;
        terminal
            .draw(|frame| {
                focus_rect = draw(frame, frame.area(), &view, &theme, &ZH);
            })
            .unwrap();
        let out = terminal.backend().to_string();
        assert!(out.contains("行0"), "聚焦第一行时应该从顶部开始画\n{out}");
        assert!(!out.contains("行7"), "下方装不下的行不该出现\n{out}");
        assert!(out.contains(ZH.form_more), "下方装不下的内容应该显示提示\n{out}");
        let rect = focus_rect.expect("聚焦的第一行现在应该可见");
        assert_eq!(rect.y, 2, "第一行应该紧贴内容区顶部 (border 1 + padding-top 1), 不该被顶部提示占位\n{out}");
    }

    /// 最糟场景: 一条真实的长错误 (经 `wiz_create_failed` 格式化) 在
    /// 表单宽度下会折成 2 行, 加上自定义表单固定的 14 行内容, 总高度 (2 说明 + 1 空行 + 14) = 17
    /// 超过 80×24 下自定义表单实际可用的 16 行 (24 − 3 标签栏 − 1 底栏 − 2 边框 − 2 内距); 焦点
    /// 停在最后一行的「创建」按钮——它必须被滚进可视区域, 并返回它的矩形。
    #[test]
    fn scrolling_keeps_a_focused_button_visible_behind_a_long_wrapped_note() {
        let note = (ZH.wiz_create_failed)("network: error sending request for url (https://relay.example.com/v1beta/models)");
        let mut rows: Vec<FormRow> = vec![FormRow::Note { text: &note }, FormRow::Spacer];
        for label in ["协议", "厂商名", "Base URL", "请求路径", "鉴权", "API Key", "备注名"] {
            rows.push(field(label, "值"));
        }
        rows.push(FormRow::Button { label: "获取模型列表", busy: false });
        for label in ["fable", "opus", "sonnet", "haiku", "兜底"] {
            rows.push(field(label, "值"));
        }
        rows.push(FormRow::Button { label: "创建", busy: false });
        let focus = rows.len() - 1;
        let view = FormView { title: "T", steps: None, rows: &rows, focus, tick: 0, show_cursor: true };

        let theme = Theme::new(ColorMode::TrueColor);
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        let mut focus_rect = None;
        terminal
            .draw(|frame| {
                focus_rect = draw(frame, frame.area(), &view, &theme, &ZH);
            })
            .unwrap();
        let out = terminal.backend().to_string();
        assert!(out.contains("[ 创建 ]"), "聚焦的创建按钮应该被滚动进可视区域\n{out}");
        assert!(focus_rect.is_some(), "聚焦行现在应该可见, 应该返回它的矩形\n{out}");
    }

    /// 前置说明行 + 空行之后, `FormBuilder` 记下的焦点下标仍然指向声明为焦点的那一行 (调用方不用
    /// 手加「有说明行就 +2」的偏移), 画出来 `▌` 也落在那一行上。
    #[test]
    fn the_builder_tracks_the_focus_index_past_leading_note_rows() {
        let mut b = FormBuilder::new();
        b.push(FormRow::Note { text: "上一次失败的原因" }, false);
        b.push(FormRow::Spacer, false);
        b.push(field("厂商", "智谱"), false);
        b.push(field("接入点", "国内版"), true);
        b.push(FormRow::Spacer, false);
        b.push(FormRow::Button { label: "下一步", busy: false }, false);
        let built = b.finish();
        assert_eq!(built.focus, 3, "说明行 + 空行占了下标 0/1, 焦点行是下标 3");

        let view = FormView { title: "T", steps: None, rows: &built.rows, focus: built.focus, tick: 0, show_cursor: true };
        let out = render(&view, 50, 14);
        let endpoint_line = out.lines().find(|l| l.contains("国内版")).expect(&out);
        assert!(endpoint_line.contains('▌'), "焦点标记应该画在接入点行\n{out}");
        let provider_line = out.lines().find(|l| l.contains("智谱")).expect(&out);
        assert!(!provider_line.contains('▌'), "{out}");
    }

    /// 只认第一个声明为焦点的行; 一行都没声明时焦点下标是 0。
    #[test]
    fn the_builder_keeps_the_first_focus_and_defaults_to_zero() {
        let mut b = FormBuilder::new();
        b.push(field("a", "1"), false);
        b.push(field("b", "2"), true);
        b.push(field("c", "3"), true);
        assert_eq!(b.finish().focus, 1);

        let mut none = FormBuilder::new();
        none.push(field("a", "1"), false);
        none.push(field("b", "2"), false);
        assert_eq!(none.finish().focus, 0);
    }

    /// 需要滚动、而内容区连「两条提示行 + 一整行」都放不下时 (`inner.height` = 2), 一行内容都不画、
    /// 返回 `None`——不能把可用高度硬抬到 1、连同提示行画出内容区之外。
    #[test]
    fn a_content_area_too_short_to_scroll_draws_nothing_and_returns_none() {
        let labels = ["行0", "行1", "行2", "行3"];
        let rows: Vec<FormRow> = labels.iter().map(|l| field(l, "值")).collect();
        let view = FormView { title: "T", steps: None, rows: &rows, focus: 0, tick: 0, show_cursor: true };
        let theme = Theme::new(ColorMode::TrueColor);
        // 高 6: 上下边框 2 + 上下内距 2, 内容区剩 2 行。
        let mut terminal = Terminal::new(TestBackend::new(50, 6)).unwrap();
        let mut focus_rect = Some(Rect::default());
        terminal
            .draw(|frame| {
                focus_rect = draw(frame, frame.area(), &view, &theme, &ZH);
            })
            .unwrap();
        let out = terminal.backend().to_string();
        assert!(focus_rect.is_none(), "焦点行没画出来, 应该返回 None\n{out}");
        assert!(!out.contains("行0") && !out.contains(ZH.form_more), "内容区放不下时不该画任何行\n{out}");
    }

    /// `show_cursor: false` 时哪怕聚焦行带着 `cursor`, 画完之后终端光标也
    /// 不该可见——`Terminal::draw` 按这一帧有没有被调过 `Frame::set_cursor_position` 决定要不要
    /// 显示/隐藏光标, 一行没设不代表别的行也没设, 必须统一在 `draw()` 这一个关口拦住。
    #[test]
    fn show_cursor_false_hides_the_terminal_cursor_even_when_the_focused_row_has_one() {
        let rows =
            vec![FormRow::Field { label: "API Key", value: "sk-test", placeholder: "", hint: None, cursor: Some(3), error: None, locked: false }];
        let view = FormView { title: "T", steps: None, rows: &rows, focus: 0, tick: 0, show_cursor: false };
        let theme = Theme::new(ColorMode::TrueColor);
        let mut terminal = Terminal::new(TestBackend::new(50, 10)).unwrap();
        terminal.draw(|frame| { draw(frame, frame.area(), &view, &theme, &ZH); }).unwrap();
        assert!(!terminal.backend().cursor_visible(), "show_cursor: false 时终端光标不该可见");
    }

    /// 焦点在按钮行时, `[ 标签 ]` 之外的格子 (左右大片空白) **不该**带 `REVERSED`,
    /// 只有标签本身那几格带——用 `TestBackend` 的 buffer 逐格查 `modifier`, 比只看渲染出来的字符
    /// 更能咬住"整行被反色"这类样式回归 (文字断言看不出颜色/修饰符)。用短 ASCII 标签 (不是真实
    /// i18n 文案), 避开宽字符的"第二格是延续格, `Buffer::set_stringn` 对它调 `reset()` 不保留
    /// 修饰符"这个渲染细节——这里只关心按钮本身的反色范围, 不是宽字符怎么占格。
    #[test]
    fn the_button_reverses_only_its_own_label_not_the_whole_row() {
        let rows = vec![FormRow::Button { label: "SAVE", busy: false }];
        let view = FormView { title: "T", steps: None, rows: &rows, focus: 0, tick: 0, show_cursor: true };
        let theme = Theme::new(ColorMode::TrueColor);
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();
        terminal.draw(|frame| { draw(frame, frame.area(), &view, &theme, &ZH); }).unwrap();
        let buf = terminal.backend().buffer();
        let width = buf.area.width;

        let y = (0..buf.area.height)
            .find(|&y| (0..width).any(|x| buf[(x, y)].symbol() == "["))
            .unwrap_or_else(|| panic!("应该能找到按钮行\n{}", terminal.backend()));
        let start = (0..width).find(|&x| buf[(x, y)].symbol() == "[").expect("已经确认这一行有 [");
        let end = (0..width).rev().find(|&x| buf[(x, y)].symbol() == "]").expect("应该能找到右方括号");

        for x in 0..width {
            let reversed = buf[(x, y)].style().add_modifier.contains(Modifier::REVERSED);
            if (start..=end).contains(&x) {
                assert!(reversed, "标签范围内 (x={x}) 应该反色\n{}", terminal.backend());
            } else {
                assert!(!reversed, "标签范围外 (x={x}) 不该反色\n{}", terminal.backend());
            }
        }
    }
}
