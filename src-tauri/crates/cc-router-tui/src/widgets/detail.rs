//! 只读可滚动的详情弹窗 (`Popup::Detail`, Task 1 新增; Task 8 起被请求日志详情页使用)。与
//! `Popup::Picker` 同一套约定: 打开时除 `Ctrl+C` 外所有按键归它, 滚动位置是它自己的可变状态,
//! 只有「关闭」(Esc / q / ⏎) 一件事才产出 `Action`。
//!
//! 折行 (`format::wrap`) 与几何 (`area`) 共用同一个私有的 `layout` helper——颜色不影响折行结果,
//! 所以 `area` 不需要 `&Theme` 参数就能算出总行数, 只有真正要画的 `lines` 才把折行结果套上样式。

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Margin, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::action::Action;
use crate::format::{fit, full_stamp, widest, wrap, Tz};
use crate::i18n::Strings;
use crate::theme::Theme;

/// 边框(左右各 1) + `Padding::new(2, 2, 1, 1)` 的左右内距(各 2) = 6; 边框(上下各 1) + 上下内距
/// (各 1) = 4 —— 必须与 `draw` 里的 `Padding::new(2, 2, 1, 1)` 保持一致 (仿 `help::area` 同款「常量
/// 与 `draw` 的 `Padding` 手动对齐」写法, 而不是现搭一个 `Block` 去反推)。
const BORDER_PAD_H: u16 = 6;
const BORDER_PAD_V: u16 = 4;

/// 标签列宽度下限——字段值从标签列之后开始, 折行的续行也从那一列开始对齐。实际宽度见 [`label_col`]。
const LABEL_COL: usize = 12;
/// 画过一帧之前 `PageUp`/`PageDown` 用的默认步长 (仿 `picker::DEFAULT_LIST_ROWS`)。
const DEFAULT_ROWS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Muted,
    Ok,
    Warn,
    Err,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailRow {
    Section(String),
    Field { label: String, value: String, tone: Tone },
    Text { text: String, tone: Tone },
    /// 与 `Field` 同样排版, 值是按 `PopupCtx.tz` 显示的 `full_stamp(ms)`——这样组装详情的页面
    /// (目前只有请求日志页) 不需要知道时区, 只管把毫秒时间戳塞进来。
    Stamp { label: String, ms: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailSpec {
    pub title: String,
    pub rows: Vec<DetailRow>,
}

#[derive(Debug, Clone)]
pub struct DetailState {
    spec: DetailSpec,
    scroll: usize,
    last_rows: usize,
    last_total: usize,
}

impl PartialEq for DetailState {
    fn eq(&self, other: &Self) -> bool {
        self.spec == other.spec && self.scroll == other.scroll
    }
}

impl DetailState {
    pub fn new(spec: DetailSpec) -> Self {
        Self { spec, scroll: 0, last_rows: DEFAULT_ROWS, last_total: 0 }
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// `popup.rs::area` 用: `detail::area` 不需要整个 `DetailState`, 只需要它包的 `DetailSpec`。
    pub(crate) fn spec(&self) -> &DetailSpec {
        &self.spec
    }

    /// `↓`/`j` +1, `↑`/`k` −1, `PageDown`/空格 +`last_rows`, `PageUp` −`last_rows`, `g`/`Home` 回到
    /// 0, `G`/`End` 到最大值——全部夹在 `[0, last_total.saturating_sub(last_rows)]` 内。`Esc`/`q`/⏎
    /// 关闭弹窗, 其余按键被吞掉。
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.move_scroll(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_scroll(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.move_scroll(self.last_rows as isize),
            KeyCode::PageUp => self.move_scroll(-(self.last_rows as isize)),
            KeyCode::Char('g') | KeyCode::Home => self.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.scroll = self.max_scroll(),
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => return Some(Action::ClosePopup),
            _ => {}
        }
        None
    }

    fn max_scroll(&self) -> usize {
        self.last_total.saturating_sub(self.last_rows)
    }

    fn move_scroll(&mut self, delta: isize) {
        let max = self.max_scroll() as isize;
        self.scroll = (self.scroll as isize + delta).clamp(0, max) as usize;
    }
}

/// 一行折好但还没套样式的内容——`area` (只要行数) 与 `lines` (还要上色) 共用, 折行逻辑只写一遍。
enum Content {
    Blank,
    Section(String),
    FieldFirst { label: String, text: String, tone: Tone },
    FieldCont { text: String, tone: Tone },
    Text { text: String, tone: Tone },
}

/// 标签列宽度: 最宽的字段标签 + 1 格间隔, 不低于 [`LABEL_COL`]——标签由调用方按语言给出, 英文标签
/// 比中文宽, 写死的宽度会把它们截成片段。
fn label_col(spec: &DetailSpec) -> usize {
    let labels = spec.rows.iter().filter_map(|row| match row {
        DetailRow::Field { label, .. } | DetailRow::Stamp { label, .. } => Some(label.as_str()),
        DetailRow::Section(_) | DetailRow::Text { .. } => None,
    });
    (widest(labels) + 1).max(LABEL_COL)
}

/// `width`: 正文可用宽度 (不含边框/内距)。
fn layout(spec: &DetailSpec, width: usize, tz: Tz) -> Vec<Content> {
    let label_col = label_col(spec);
    let mut out = Vec::new();
    for (i, row) in spec.rows.iter().enumerate() {
        match row {
            DetailRow::Section(title) => {
                // 除第一行 (spec.rows 的下标 0) 外, 每个小节前面自动空一行。
                if i > 0 {
                    out.push(Content::Blank);
                }
                out.push(Content::Section(title.clone()));
            }
            DetailRow::Field { label, value, tone } => {
                let value_width = width.saturating_sub(label_col);
                for (j, part) in wrap(value, value_width).into_iter().enumerate() {
                    if j == 0 {
                        out.push(Content::FieldFirst { label: label.clone(), text: part, tone: *tone });
                    } else {
                        out.push(Content::FieldCont { text: part, tone: *tone });
                    }
                }
            }
            DetailRow::Stamp { label, ms } => {
                let value = full_stamp(*ms, tz);
                let value_width = width.saturating_sub(label_col);
                for (j, part) in wrap(&value, value_width).into_iter().enumerate() {
                    if j == 0 {
                        out.push(Content::FieldFirst { label: label.clone(), text: part, tone: Tone::Normal });
                    } else {
                        out.push(Content::FieldCont { text: part, tone: Tone::Normal });
                    }
                }
            }
            DetailRow::Text { text, tone } => {
                for part in wrap(text, width) {
                    out.push(Content::Text { text: part, tone: *tone });
                }
            }
        }
    }
    out
}

fn tone_style(tone: Tone, theme: &Theme) -> ratatui::style::Style {
    match tone {
        Tone::Normal => ratatui::style::Style::default(),
        Tone::Muted => theme.muted_style(),
        Tone::Ok => ratatui::style::Style::new().fg(theme.ok),
        Tone::Warn => ratatui::style::Style::new().fg(theme.warn),
        Tone::Err => ratatui::style::Style::new().fg(theme.err),
    }
}

/// 折好行的全部内容 (纯函数, `area` / `draw` / 测试共用)。`width` = 正文可用宽度。`tz`: `Stamp`
/// 行按它格式化, 其余行忽略。
pub fn lines(spec: &DetailSpec, width: u16, theme: &Theme, tz: Tz) -> Vec<Line<'static>> {
    let label_col = label_col(spec);
    layout(spec, width as usize, tz)
        .into_iter()
        .map(|c| match c {
            Content::Blank => Line::raw(""),
            Content::Section(title) => Line::styled(title, theme.accent_bold()),
            Content::FieldFirst { label, text, tone } => {
                Line::from(vec![Span::styled(fit(&label, label_col), theme.muted_style()), Span::styled(text, tone_style(tone, theme))])
            }
            Content::FieldCont { text, tone } => Line::from(vec![Span::raw(" ".repeat(label_col)), Span::styled(text, tone_style(tone, theme))]),
            Content::Text { text, tone } => Line::styled(text, tone_style(tone, theme)),
        })
        .collect()
}

/// 居中。宽 = `min(screen.width − 4, 100)`; 高 = `min(screen.height − 4, 折行后总行数 + 4)`。`tz`:
/// `Stamp` 行的格式化文本长度 (进而折行行数) 依赖它, 与 `lines` 必须传同一个值才能得到一致的几何。
pub fn area(screen: Rect, spec: &DetailSpec, tz: Tz) -> Rect {
    const WIDTH_MAX: u16 = 100;
    const SCREEN_MARGIN: u16 = 4;

    let width = WIDTH_MAX.min(screen.width.saturating_sub(SCREEN_MARGIN));
    let content_width = width.saturating_sub(BORDER_PAD_H);
    // Task 8: 上游响应体可能有几万行, 折行总数能轻松超过 u16::MAX (65 536)——`as u16` 会静默环绕
    // (比如 65 537 行变成 1 行), 用饱和转换钳在 u16::MAX, 不让巨大的行数绕回一个很小的值。
    let total_rows = u16::try_from(layout(spec, content_width as usize, tz).len()).unwrap_or(u16::MAX);
    let height = screen.height.saturating_sub(SCREEN_MARGIN).min(total_rows.saturating_add(BORDER_PAD_V));
    screen.centered(Constraint::Length(width), Constraint::Length(height))
}

pub fn draw(frame: &mut Frame, area: Rect, state: &mut DetailState, theme: &Theme, s: &Strings, tz: Tz) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .title_top(format!(" {} ", state.spec.title))
        .title_bottom(Line::from(format!(" {} ", s.detail_keys)).right_aligned())
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(area);
    let rows = inner.height.max(1) as usize;
    let rendered = lines(&state.spec, inner.width, theme, tz);
    let total = rendered.len();

    state.scroll = state.scroll.min(total.saturating_sub(rows));
    state.last_rows = rows;
    state.last_total = total;

    super::clear_popup_area(frame, area);
    // 同上: `scroll` 理论上可以超过 u16::MAX (巨大的上游响应体), 饱和转换而不是静默环绕。
    let scroll_y = u16::try_from(state.scroll).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(rendered).block(block).scroll((scroll_y, 0)), area);

    if total > rows {
        let mut sb_state = ScrollbarState::new(total.saturating_sub(rows)).position(state.scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut sb_state,
        );
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyModifiers;
    use ratatui::Terminal;

    use super::*;
    use crate::i18n::ZH;
    use crate::theme::ColorMode;

    fn theme() -> Theme {
        Theme::new(ColorMode::TrueColor)
    }

    fn tz() -> Tz {
        Tz::Fixed(8 * 3600)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn field_values_wrap_under_the_value_column() {
        let spec = DetailSpec {
            title: "详情".into(),
            rows: vec![DetailRow::Field { label: "值".into(), value: "x".repeat(100), tone: Tone::Normal }],
        };
        let rendered = lines(&spec, 40, &theme(), tz());
        assert!(rendered.len() > 1, "100 字符在 40 列宽下应该折成多行\n{rendered:?}");
        for line in &rendered[1..] {
            let text = line_text(line);
            assert!(text.starts_with(&" ".repeat(LABEL_COL)), "续行应该以 {LABEL_COL} 个空格开头, 实际 {text:?}");
        }
    }

    #[test]
    fn sections_after_the_first_get_a_blank_line_before_them() {
        let spec = DetailSpec {
            title: "详情".into(),
            rows: vec![
                DetailRow::Section("第一节".into()),
                DetailRow::Field { label: "a".into(), value: "b".into(), tone: Tone::Normal },
                DetailRow::Section("第二节".into()),
            ],
        };
        let rendered = lines(&spec, 40, &theme(), tz());
        assert_eq!(line_text(&rendered[0]), "第一节", "第一行是第一个小节, 前面不该有空行\n{rendered:?}");
        assert_eq!(line_text(&rendered[2]), "", "第二个小节前面应该有一行空行\n{rendered:?}");
        assert_eq!(line_text(&rendered[3]), "第二节", "{rendered:?}");
    }

    #[test]
    fn text_rows_keep_their_own_newlines() {
        let spec = DetailSpec { title: "详情".into(), rows: vec![DetailRow::Text { text: "第一行\n第二行".into(), tone: Tone::Err }] };
        let rendered = lines(&spec, 40, &theme(), tz());
        assert_eq!(rendered.len(), 2, "{rendered:?}");
        assert_eq!(line_text(&rendered[0]), "第一行");
        assert_eq!(line_text(&rendered[1]), "第二行");
    }

    #[test]
    fn area_fits_short_content_and_caps_long_content() {
        let screen = Rect::new(0, 0, 80, 24);
        let short = DetailSpec {
            title: "详情".into(),
            rows: vec![
                DetailRow::Field { label: "a".into(), value: "1".into(), tone: Tone::Normal },
                DetailRow::Field { label: "b".into(), value: "2".into(), tone: Tone::Normal },
                DetailRow::Field { label: "c".into(), value: "3".into(), tone: Tone::Normal },
            ],
        };
        let a = area(screen, &short, tz());
        assert_eq!((a.width, a.height), (76, 7), "3 行短内容: 76 x 7");

        let long =
            DetailSpec { title: "详情".into(), rows: (0..100).map(|i| DetailRow::Text { text: format!("row {i}"), tone: Tone::Normal }).collect() };
        let b = area(screen, &long, tz());
        assert_eq!(b.height, 20, "100 行内容应该被高度上限夹住");
    }

    #[test]
    fn scroll_keys_clamp_to_the_content() {
        // 80×24 下这份内容画出来是 `last_rows=16` (高度封顶在 20, 减掉边框+内距 4); 34 行文本让
        // `max_scroll = 34 - 16 = 18` 落在 `[last_rows, 20]` 区间——大到 `PageDown` 从 0 起不会被
        // 提前夹住 (能验证「正好前进 last_rows」), 又小到 `j` 连按 20 次一定能越过它 (能验证「到顶后
        // 不再往前」)。
        let spec =
            DetailSpec { title: "详情".into(), rows: (0..34).map(|i| DetailRow::Text { text: format!("row {i}"), tone: Tone::Normal }).collect() };
        let mut state = DetailState::new(spec.clone());
        let t = theme();
        let s = &ZH;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let popup = area(Rect::new(0, 0, 80, 24), &spec, tz());
        terminal.draw(|frame| draw(frame, popup, &mut state, &t, s, tz())).unwrap();
        assert!(state.last_total > state.last_rows, "测试前提: 内容要超出一屏, last_total={} last_rows={}", state.last_total, state.last_rows);

        for _ in 0..20 {
            state.handle_key(key(KeyCode::Char('j')));
        }
        assert_eq!(state.scroll(), state.last_total - state.last_rows, "j x20 应该停在最大值");

        state.handle_key(key(KeyCode::Char('g')));
        assert_eq!(state.scroll(), 0, "g 应该回到 0");
        state.handle_key(key(KeyCode::Char('G')));
        assert_eq!(state.scroll(), state.last_total - state.last_rows, "G 应该到最大值");

        state.handle_key(key(KeyCode::Char('g')));
        state.handle_key(key(KeyCode::PageDown));
        assert_eq!(state.scroll(), state.last_rows, "PageDown 从 0 起应该正好前进 last_rows");

        assert_eq!(state.handle_key(key(KeyCode::Esc)), Some(Action::ClosePopup));
        assert_eq!(state.handle_key(key(KeyCode::Char('q'))), Some(Action::ClosePopup));
        assert_eq!(state.handle_key(key(KeyCode::Enter)), Some(Action::ClosePopup));
        assert_eq!(state.handle_key(key(KeyCode::Char('x'))), None);
    }

    /// Task 6: `Stamp` 行不自带时区, 同一个 spec 在不同 `PopupCtx.tz` 下要渲染出不同的文本——
    /// 这样组装详情的页面不需要知道时区。
    #[test]
    fn stamp_rows_use_the_popup_tz() {
        const NOW: i64 = 1_700_000_000_000;
        let spec = DetailSpec { title: "详情".into(), rows: vec![DetailRow::Stamp { label: "时间".into(), ms: NOW }] };
        let east = lines(&spec, 40, &theme(), Tz::Fixed(8 * 3600));
        let utc = lines(&spec, 40, &theme(), Tz::Fixed(0));
        assert_ne!(line_text(&east[0]), line_text(&utc[0]), "同一个 Stamp 在不同时区下应该渲染出不同的文本");
        assert!(line_text(&east[0]).contains("2023-11-15 06:13:20"), "{:?}", line_text(&east[0]));
        assert!(line_text(&utc[0]).contains("2023-11-14 22:13:20"), "{:?}", line_text(&utc[0]));
    }

    /// Task 8 review item #2: `area()` 的 `total_rows as u16` 与 `draw()` 的 `state.scroll as u16`
    /// 曾经在折行数 / 滚动位置超过 `u16::MAX` (65 536) 时静默环绕 (比如 65 537 行变成 1 行)。
    /// 日志页的上游响应体第一次把这么大的内容塞进详情弹窗——用一段 7 万行的 `Text` 验证两处都
    /// 换成了饱和转换, 不 panic 也不会算出离谱的小值。
    #[test]
    fn a_huge_text_row_does_not_overflow_the_u16_casts() {
        let huge_text: String = (0..70_000).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
        let spec = DetailSpec { title: "详情".into(), rows: vec![DetailRow::Text { text: huge_text, tone: Tone::Normal }] };
        let screen = Rect::new(0, 0, 80, 24);

        let popup = area(screen, &spec, tz());
        assert_eq!(popup.height, 20, "高度应该被上限夹住, 不因环绕而算出异常小的值");

        let mut state = DetailState::new(spec.clone());
        let t = theme();
        let s = &ZH;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, popup, &mut state, &t, s, tz())).unwrap();
        assert!(state.last_total > 65_536, "折行总数应该真的超过 u16::MAX, last_total={}", state.last_total);

        state.handle_key(key(KeyCode::Char('G')));
        assert_eq!(state.scroll(), state.last_total - state.last_rows, "滚到底不该因为环绕而算错");
        terminal.draw(|frame| draw(frame, popup, &mut state, &t, s, tz())).unwrap(); // 不 panic
    }
}
