//! 弹窗状态与行为。`Popup::Help` 沿用 P1 的无状态弹窗; `Popup::Confirm` 是 P3b 新增的「y 是 / n 否」
//! 确认弹窗——打开时除 `Ctrl+C` 外的所有按键都归它。`Popup::Picker` (Task 3) 是过滤选择弹窗: 输入框
//! / 选中下标是它自己的状态, 打字 / 移动选中直接改 `PickerState` 自身, 不经过 `Action` 往返 (与
//! `Component::handle_key` 里方向键直接改页面自己的选中下标是同一套约定); 只有「选定」(⏎ →
//! `PickerDone`) 与「取消」(Esc → `ClosePopup`) 两件事才产出 `Action`。`Popup::Detail` (Task 1) 是
//! 只读可滚动的详情弹窗, 折行/滚动/画法都在 `widgets::detail`, 这里只负责按变体分派。
//!
//! 按键路由 / 尺寸计算 / 画法 (Task 1 起) 收在下面的 `impl Popup` 里——`App` 只负责打开/替换/关闭
//! 弹窗与压暗背景, 具体交给这三个方法的穷尽 `match`。以后再加新弹窗变体, 只需在这里的三个 match
//! 各补一个分支, 编译器的穷尽检查会逼着改全, `App` 那边不用再跟着改。

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::action::{Action, OnYes};
use crate::format::Tz;
use crate::i18n::Strings;
use crate::theme::Theme;
use crate::widgets::detail::DetailState;
use crate::widgets::picker::PickerState;
use crate::widgets::{confirm, detail, help, picker};

#[derive(Debug, Clone, PartialEq)]
pub enum Popup {
    Help,
    Confirm(ConfirmState),
    Picker(PickerState),
    Detail(DetailState),
}

/// 一次「是 / 否」确认: `prompt` 是正文, `on_yes` 是用户选「是」时做什么——经
/// `Action::Confirmed` 原样交回 `App`, 要不要先丢弃草稿由它的变体决定 (见 [`OnYes`])。
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmState {
    pub prompt: String,
    pub on_yes: OnYes,
}

/// 画弹窗需要的只读环境。
pub struct PopupCtx<'a> {
    pub theme: &'a Theme,
    pub s: &'static Strings,
    /// 当前页面的键位 (只有 `Help` 用)。
    pub page_help: &'static [(&'static str, &'static str)],
    /// 显示时间用的时区, `Popup::Detail` 的 `Stamp` 行拿它调用 `full_stamp`。
    pub tz: Tz,
}

impl Popup {
    /// 弹窗打开时除 `Ctrl+C` 外的全部按键。`None` = 吞掉, 或者只改了弹窗自身的状态。
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match self {
            Popup::Help => match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => Some(Action::ClosePopup),
                _ => None,
            },
            Popup::Confirm(state) => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => Some(Action::Confirmed(state.on_yes.clone())),
                // 默认 N: Esc / ⏎ 与显式的 n/N 一样只关弹窗, 不执行 on_yes。
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => Some(Action::ClosePopup),
                _ => None,
            },
            Popup::Picker(state) => state.handle_key(key),
            Popup::Detail(state) => state.handle_key(key),
        }
    }

    /// 这一帧弹窗占的矩形 (居中)。
    pub fn area(&self, screen: Rect, ctx: &PopupCtx) -> Rect {
        match self {
            Popup::Help => help::area(screen, ctx.s, ctx.page_help),
            Popup::Confirm(state) => confirm::area(screen, &state.prompt),
            Popup::Picker(_) => picker::area(screen),
            Popup::Detail(state) => detail::area(screen, state.spec(), ctx.tz),
        }
    }

    /// 画在 `area` 里; 压暗背景由 `App` 先做。`&mut self`: Picker / Detail 要记录上一帧的可视行数。
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &PopupCtx) {
        match self {
            Popup::Help => help::draw(frame, area, ctx.theme, ctx.s, ctx.page_help),
            Popup::Confirm(state) => confirm::draw(frame, area, state, ctx.theme, ctx.s),
            Popup::Picker(state) => picker::draw(frame, area, state, ctx.theme, ctx.s),
            Popup::Detail(state) => detail::draw(frame, area, state, ctx.theme, ctx.s, ctx.tz),
        }
    }
}
