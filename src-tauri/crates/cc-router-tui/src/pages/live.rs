//! 实时路由页: `tail -f` 式的路由尝试流 + 最近 60 秒每秒次数 + 暂停 / 选中 / 过滤。
//!
//! 后端事件只有 (虚拟模型, 订阅 id) 两个维度, 不带请求 id ——同一对键上的并发尝试按 FIFO 配对
//! (最早的 `Pending` 先被下一条 `Finished` 结束)。断线会把所有 `Pending` 标成 `Interrupted`,
//! 并插入一条 `Gap` 分隔行 (幂等: 重连期间 `Lost` 会反复到来, 不能每次都插一条)。

use std::collections::VecDeque;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, HighlightSpacing, Padding, Row, Sparkline, Table, TableState};
use ratatui::Frame;
use throbber_widgets_tui::{Throbber, BRAILLE_SIX};
use unicode_width::UnicodeWidthStr;

use super::{Component, DrawCtx, StreamEvent};
use crate::action::{Action, Cmd, Fetch};
use crate::client::dto::RouteAttempt;
use crate::client::events::{ROUTE_ATTEMPT_FINISHED, ROUTE_ATTEMPT_STARTED};
use crate::format::{clock, duration, fit, Tz};
use crate::i18n::Strings;
use crate::store::Store;
use crate::theme::Theme;
use crate::widgets::keybar::Hint;
use crate::widgets::picker::{PickerChoice, PickerItem, PickerSpec, PickerTag};
use crate::widgets::spinner_state;

/// 缓冲上限 (spec §5.5), 超出丢最旧的。
pub const MAX_ENTRIES: usize = 2000;
/// 待播 `row_new` 闪烁只保留最新的这么多个序号; 页面在后台攒下的旧行不用补播。
const MAX_FLASH: usize = 32;
/// `PageUp`/`PageDown` 在第一帧画出来之前没有真实的可视行数可用, 先给个不至于原地不动的默认值
/// (仿 `pages::subscriptions::DEFAULT_PAGE_ROWS` 同款写法)。
const DEFAULT_PAGE_ROWS: usize = 10;

const TIME_COL: usize = 8;
const VM_COL: usize = 14;
/// 耗时列下限 (放得下 `59m59s`); 「中断」的译文更宽时按 [`elapsed_col`] 放宽。
const ELAPSED_MIN_COL: usize = 7;

fn elapsed_col(s: &Strings) -> usize {
    s.live_interrupted.width().max(ELAPSED_MIN_COL)
}
/// 表的选中前缀 (`highlight_symbol("▌ ")`, `HighlightSpacing::Always`) 固定宽度, 用于手算
/// `Constraint::Fill(1)` 那一列 (订阅名) 的实际宽度给 `format::fit`——与订阅页 `draw_list` 手算
/// `sonnet_col`、日志页 `draw_table` 手算 `model_col` 同一个公式。
const HIGHLIGHT_COL: u16 = 2;

/// 超过这么久还没等到 `finished` 的尝试, 认为它的 `finished` 永远不会来了 (客户端中途断开,
/// 比如用户在 Claude Code 里按 Esc 取消了请求)。FIFO 配对 (`on_finished`) 天然假设 `Pending`
/// 按时间顺序被逐个结束——一旦有一条 `started` 永远等不到自己的 `finished`, 配对会永远错位一位:
/// 每条更晚到达的 `finished` 都会先结束这条陈旧的 `Pending` (把两次尝试之间的间隔算成它的耗时),
/// 而真正该配对的那条反而一直显示成 Pending 转圈。10 分钟是刻意留出的余量——非流式的长生成后端
/// 只在整个响应体结束后才发一次 `finished`, 不能因为等太久就提前把它判成中断。
const PENDING_STALE_MS: i64 = 600_000;

/// 按虚拟模型或订阅过滤当前显示的尝试; `Gap` 行不受过滤影响, 总是可见。
#[derive(Debug, Clone, PartialEq, Eq)]
enum LiveFilter {
    VirtualModel(String),
    Subscription(String),
}

/// 一次路由尝试的结果; `Pending` 还没等到 `finished` 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Pending,
    Done { ok: bool, elapsed_ms: Option<i64> },
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryKind {
    Attempt { vm: String, sub_id: String, outcome: Outcome },
    /// 断线分隔行。
    Gap,
}

/// `at_ms`: 有 started 的尝试取 started 的时刻; 只有 finished 的尝试取 finished 的时刻;
/// `Gap` 取断线时刻。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    seq: u64,
    at_ms: i64,
    kind: EntryKind,
}

pub struct Live {
    entries: VecDeque<Entry>,
    /// 从 1 起。
    next_seq: u64,
    /// `Some(seq)` 表示暂停, 只显示序号 `<= seq` 的条目。
    paused_at: Option<u64>,
    /// `None` = 跟随最新。
    selected: Option<u64>,
    filter: Option<LiveFilter>,
    /// 待播 `row_new` 的序号; `draw` 取走。
    flash: Vec<u64>,
    table_state: TableState,
    /// 上一帧表体的可视行数, `PageUp`/`PageDown` 按这个翻页。
    last_page_rows: usize,
    /// 上一帧窗口的起始下标 (过滤后列表里的位置)。只用于让窗口在选中项还在视野里时保持不动。
    last_window_start: usize,
}

/// 手写而不是派生: `next_seq` 从 1 起。
impl Default for Live {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            next_seq: 1,
            paused_at: None,
            selected: None,
            filter: None,
            flash: Vec::new(),
            table_state: TableState::default(),
            last_page_rows: DEFAULT_PAGE_ROWS,
            last_window_start: 0,
        }
    }
}

/// 这一帧要画哪一段: 返回 `(起始下标, 窗口内的选中下标)`。
///
/// - `total`: 过滤后的条目数；`sel`: 选中项在过滤后列表里的下标, `None` = 跟随最新。
/// - 跟随最新时窗口贴底 (`total - capacity`), 与原来 `*table_state.offset_mut() = …` 的行为一致。
/// - 有选中时, 只在它跑出 `[prev_start, prev_start + capacity)` 时才移动窗口, 移动后它正好贴边 ——
///   这是 ratatui `Table` 自己的滚动行为, 我们把它搬出来, 是因为 `Table` 只能对"已经建好的全部行"
///   做这件事, 而我们想先知道该建哪些行 (P4 终审 Minor)。
/// - `capacity == 0` 时提前返回 `(0, None)`——这不是「保证全函数」式的收尾, 是必需的守卫:
///   `draw_table` 只在 `idx.is_empty()` 时提前返回, 对 `capacity == 0`(比如终端矮到内容区被
///   边框/内距吃没) 没有任何拦截。删掉这个分支, 带选中项时会算出 `start = sel + 1`(`sel >=
///   prev_start.saturating_add(0)` 恒成立)、夹进 `[0, total]` 后仍是 `sel + 1`, `sel_in_window`
///   则 `saturating_sub` 成 `Some(0)`——而 `draw_table` 按 `[start, start + 0)` 切出来的 `rows`
///   是空的: ratatui `Table` 内部按 `selected` 找可见行时会对着这个空 `Vec` 取下标, 渲染时直接
///   panic。
fn window(total: usize, sel: Option<usize>, capacity: usize, prev_start: usize) -> (usize, Option<usize>) {
    if capacity == 0 {
        return (0, None);
    }
    let max_start = total.saturating_sub(capacity);
    let Some(sel) = sel else { return (max_start, None) };
    let start = if sel < prev_start {
        sel
    } else if sel >= prev_start.saturating_add(capacity) {
        (sel + 1).saturating_sub(capacity)
    } else {
        prev_start
    };
    let start = start.min(max_start);
    (start, Some(sel.saturating_sub(start)))
}

/// `kind` 是否符合当前过滤: `Gap` 总是符合; 尝试要满足 `VirtualModel(x)` 时 `vm == x`,
/// `Subscription(x)` 时 `sub_id == x`。纯函数, 供 `draw`/测试共用。
fn matches_filter(kind: &EntryKind, filter: Option<&LiveFilter>) -> bool {
    match (kind, filter) {
        (EntryKind::Gap, _) | (EntryKind::Attempt { .. }, None) => true,
        (EntryKind::Attempt { vm, .. }, Some(LiveFilter::VirtualModel(x))) => vm == x,
        (EntryKind::Attempt { sub_id, .. }, Some(LiveFilter::Subscription(x))) => sub_id == x,
    }
}

/// `"vm:<name>"` / `"sub:<id>"` / `"*"` → 过滤条件; 其它 (理论上不会发生) 静默当作清除过滤。
fn parse_filter(id: &str) -> Option<LiveFilter> {
    if let Some(name) = id.strip_prefix("vm:") {
        return Some(LiveFilter::VirtualModel(name.to_string()));
    }
    id.strip_prefix("sub:").map(|sub_id| LiveFilter::Subscription(sub_id.to_string()))
}

fn filter_label(filter: &LiveFilter, store: &Store) -> String {
    match filter {
        LiveFilter::VirtualModel(name) => name.clone(),
        LiveFilter::Subscription(id) => store.subscription(id).map(|s| s.display_name.clone()).unwrap_or_else(|| id.clone()),
    }
}

impl Live {
    /// 追加一条新记录: 分配 `seq`、推进 `flash`、超出 `MAX_ENTRIES` 时从最旧的开始丢 (若丢掉的
    /// 恰好是当前选中项, `selected` 改回 `None`)。返回分配到的 `seq`。
    fn push(&mut self, at_ms: i64, kind: EntryKind) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.push_back(Entry { seq, at_ms, kind });

        self.flash.push(seq);
        let excess = self.flash.len().saturating_sub(MAX_FLASH);
        if excess > 0 {
            self.flash.drain(0..excess);
        }

        while self.entries.len() > MAX_ENTRIES {
            if let Some(evicted) = self.entries.pop_front() {
                if self.selected == Some(evicted.seq) {
                    self.selected = None;
                }
            }
        }
        seq
    }

    fn on_started(&mut self, data: &str, at_ms: i64) {
        let Ok(attempt) = serde_json::from_str::<RouteAttempt>(data) else { return };
        self.push(at_ms, EntryKind::Attempt { vm: attempt.virtual_model, sub_id: attempt.subscription_id, outcome: Outcome::Pending });
    }

    fn on_finished(&mut self, data: &str, at_ms: i64) {
        let Ok(attempt) = serde_json::from_str::<RouteAttempt>(data) else { return };
        let ok = attempt.success.unwrap_or(false);
        // 配对前先让陈旧的 Pending 让位 (`PENDING_STALE_MS` 常量注释): 不这样做的话, 一条永远等
        // 不到 finished 的 started 会让后面所有 (vm, sub_id) 相同的配对都错位一位。
        let stale_before = at_ms - PENDING_STALE_MS;
        for entry in self.entries.iter_mut() {
            if let EntryKind::Attempt { outcome, .. } = &mut entry.kind {
                if *outcome == Outcome::Pending && entry.at_ms < stale_before {
                    *outcome = Outcome::Interrupted;
                }
            }
        }
        // 找最早一条相同 (vm, sub_id) 且仍是 Pending 的尝试 (VecDeque 从队头到队尾正好是
        // 从最早到最新), 先进先出配对。找不到就自成一行。
        let idx = self.entries.iter().position(|e| {
            matches!(&e.kind, EntryKind::Attempt { vm, sub_id, outcome: Outcome::Pending }
                if *vm == attempt.virtual_model && *sub_id == attempt.subscription_id)
        });
        match idx {
            Some(i) => {
                let started_at = self.entries[i].at_ms;
                if let EntryKind::Attempt { outcome, .. } = &mut self.entries[i].kind {
                    *outcome = Outcome::Done { ok, elapsed_ms: Some((at_ms - started_at).max(0)) };
                }
            }
            None => {
                self.push(
                    at_ms,
                    EntryKind::Attempt { vm: attempt.virtual_model, sub_id: attempt.subscription_id, outcome: Outcome::Done { ok, elapsed_ms: None } },
                );
            }
        }
    }

    fn on_lost(&mut self, at_ms: i64) {
        for entry in self.entries.iter_mut() {
            if let EntryKind::Attempt { outcome, .. } = &mut entry.kind {
                if *outcome == Outcome::Pending {
                    *outcome = Outcome::Interrupted;
                }
            }
        }
        // 幂等: 目前没有任何条目, 或最新一条已经是 Gap 时不再追加 (重连期间 Lost 会反复到来)。
        let already_gap = matches!(self.entries.back().map(|e| &e.kind), Some(EntryKind::Gap));
        if !self.entries.is_empty() && !already_gap {
            self.push(at_ms, EntryKind::Gap);
        }
    }

    fn is_visible(&self, entry: &Entry) -> bool {
        let within_pause = self.paused_at.is_none_or(|p| entry.seq <= p);
        within_pause && matches_filter(&entry.kind, self.filter.as_ref())
    }

    fn visible_seqs(&self) -> Vec<u64> {
        self.entries.iter().filter(|e| self.is_visible(e)).map(|e| e.seq).collect()
    }

    /// 同 [`Live::visible_seqs`], 但返回在 `self.entries` (`VecDeque`) 里的下标而不是序号——
    /// `draw_table` 需要下标去算 `TableState` 的选中位置 / offset 换算, 用**下标**而不是持有
    /// `&Entry` 借用是刻意的: `draw_table` 之后还要动 `self.table_state`/`self.last_page_rows`
    /// 这些字段, 借用检查器不允许一份跨语句存活的 `Vec<&Entry>` 与后续的 `&mut self` 同时存在,
    /// 拿下标 (`usize`, 不借用) 每次现用现取 `self.entries[i]` 才能两全。
    fn visible_indices(&self) -> Vec<usize> {
        self.entries.iter().enumerate().filter(|(_, e)| self.is_visible(e)).map(|(i, _)| i).collect()
    }

    /// 暂停后新增、且符合过滤的尝试数 (`Gap` 不计入, 未暂停恒为 0)。
    fn paused_new_count(&self) -> usize {
        let Some(p) = self.paused_at else { return 0 };
        self.entries
            .iter()
            .filter(|e| e.seq > p && matches_filter(&e.kind, self.filter.as_ref()) && matches!(&e.kind, EntryKind::Attempt { .. }))
            .count()
    }

    /// 过滤变化 (`PickerDone`) 之后调用: 当前选中项如果不再可见, 改回 `None`。
    fn clear_selection_if_hidden(&mut self) {
        if let Some(seq) = self.selected {
            let still_visible = self.entries.iter().any(|e| e.seq == seq && self.is_visible(e));
            if !still_visible {
                self.selected = None;
            }
        }
    }

    fn toggle_pause(&mut self) {
        self.paused_at = match self.paused_at {
            Some(_) => None,
            None => Some(self.next_seq.saturating_sub(1)),
        };
    }

    /// `↑`/`↓`/`PageUp`/`PageDown` 共用: `delta` 为负是向「更早」移动, 为正是向「更新」移动。
    /// - 没有选中 (跟随最新) 时: 向早移动直接选中最后一个可见条目 (`↑` 的语义); 向新移动是
    ///   no-op (已经在跟随最新, 没有更新的可去)。
    /// - 已有选中时按 `delta` 步移动, 越过起点钳在第一条, 越过终点回到 `None` (跟随最新)。
    fn move_selection(&mut self, delta: isize) {
        let visible = self.visible_seqs();
        if visible.is_empty() {
            return;
        }
        let idx = match self.selected.and_then(|seq| visible.iter().position(|&s| s == seq)) {
            Some(idx) => idx,
            None => {
                if delta < 0 {
                    self.selected = Some(visible[visible.len() - 1]);
                }
                return;
            }
        };
        let next = idx as isize + delta;
        if next < 0 {
            self.selected = Some(visible[0]);
        } else if next as usize >= visible.len() {
            self.selected = None;
        } else {
            self.selected = Some(visible[next as usize]);
        }
    }

    fn select_first(&mut self) {
        if let Some(&first) = self.visible_seqs().first() {
            self.selected = Some(first);
        }
    }

    /// `⏎`: 选中的条目是尝试时取它的订阅; 跟随最新时取最后一个**可见**的尝试的订阅 (尊重当前的
    /// 暂停 / 过滤状态——用户看到的最后一行是什么, 跳转就该跟着那一行走)。选中的是 `Gap`, 或者
    /// 压根没有任何尝试, 返回 `None`。
    fn target_subscription_for_logs(&self) -> Option<String> {
        match self.selected {
            Some(seq) => match &self.entries.iter().find(|e| e.seq == seq)?.kind {
                EntryKind::Attempt { sub_id, .. } => Some(sub_id.clone()),
                EntryKind::Gap => None,
            },
            None => self.visible_indices().iter().rev().find_map(|&i| match &self.entries[i].kind {
                EntryKind::Attempt { sub_id, .. } => Some(sub_id.clone()),
                EntryKind::Gap => None,
            }),
        }
    }

    /// `/`: 打开过滤选择弹窗。虚拟模型名取 `store.virtual_models()` 的后端顺序; 未加载时改用
    /// 条目里出现过的虚拟模型名, 按首次出现排序。
    fn open_filter_picker(&self, store: &Store, s: &'static Strings) -> Action {
        let mut items = vec![PickerItem { id: "*".to_string(), label: s.live_filter_all.to_string(), hint: None }];

        let vm_names: Vec<String> = if store.virtual_models_loaded() {
            store.virtual_models().iter().map(|vm| vm.name.clone()).collect()
        } else {
            let mut seen = Vec::new();
            for entry in &self.entries {
                if let EntryKind::Attempt { vm, .. } = &entry.kind {
                    if !seen.contains(vm) {
                        seen.push(vm.clone());
                    }
                }
            }
            seen
        };
        for name in vm_names {
            items.push(PickerItem { id: format!("vm:{name}"), label: name, hint: Some(s.filter_dim_vm.to_string()) });
        }
        for sub in store.subscriptions() {
            items.push(PickerItem {
                id: format!("sub:{}", sub.id),
                label: sub.display_name.clone(),
                hint: Some(format!("{} · {}", s.filter_dim_sub, sub.provider_name(s.lang))),
            });
        }

        let initial = match &self.filter {
            None => "*".to_string(),
            Some(LiveFilter::VirtualModel(name)) => format!("vm:{name}"),
            Some(LiveFilter::Subscription(id)) => format!("sub:{id}"),
        };
        Action::OpenPicker(PickerSpec { tag: PickerTag::LiveFilter, title: s.live_filter_title.to_string(), items, allow_custom: false, initial })
    }

    /// 测试专用: 当前缓冲的条目数 (含 `Gap` 分隔行, 不受暂停 / 过滤影响)。Task 8 起用它取代已删除
    /// 的 `Placeholder::seen`, 验证事件广播确实到达了不可见的实时路由页。
    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// 测试专用: 上一帧窗口的起始下标。P4 终审 Minor——验证「跟随最新时只建视口那一屏, 窗口贴底」,
    /// 与 `entry_count` 同一套写法。
    #[cfg(test)]
    pub(crate) fn last_window_start(&self) -> usize {
        self.last_window_start
    }

    fn draw_spark(&self, frame: &mut Frame, area: Rect, ctx: &DrawCtx) {
        let s = ctx.s;
        let theme = ctx.theme;
        let idx = self.visible_indices();
        let timestamps = idx.iter().filter_map(|&i| {
            let entry = &self.entries[i];
            matches!(&entry.kind, EntryKind::Attempt { .. }).then_some(entry.at_ms)
        });
        let counts = per_second(timestamps, ctx.now_ms);
        let total: u64 = counts.iter().sum();

        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme.border_style())
            .title_top(format!(" {} ", s.live_spark_title))
            .title_top(Line::from(format!(" {} ", (s.live_spark_total)(total))).right_aligned());
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }
        // 固定 60 列, 靠右对齐; 窄于 60 时用能放下的最右侧那些桶 (最右一格恒是当前这一秒)。
        let width = inner.width.min(60);
        let data = &counts[60 - width as usize..];
        let max = counts.iter().copied().max().unwrap_or(0).max(1);
        let spark_area = Rect::new(inner.x + (inner.width - width), inner.y, width, inner.height);
        frame.render_widget(Sparkline::default().data(data).max(max).style(Style::new().fg(theme.accent)), spark_area);
    }

    fn draw_table(&mut self, frame: &mut Frame, area: Rect, ctx: &mut DrawCtx, flash: &[u64]) {
        let s = ctx.s;
        let theme = ctx.theme;
        let store = ctx.store;
        let tz = ctx.tz;
        let tick = ctx.tick;

        let idx = self.visible_indices();
        let attempt_count = idx.iter().filter(|&&i| matches!(&self.entries[i].kind, EntryKind::Attempt { .. })).count();
        let mut footer_parts = Vec::new();
        if let Some(filter) = &self.filter {
            footer_parts.push((s.filter_summary)(&filter_label(filter, store)));
        }
        if self.paused_at.is_some() {
            footer_parts.push((s.live_paused)(self.paused_new_count()));
        } else if self.selected.is_none() {
            footer_parts.push(s.live_following.to_string());
        }
        footer_parts.push((s.live_count)(attempt_count));
        let footer = footer_parts.join(" · ");

        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme.border_style())
            .title_top(format!(" {} ", s.live_title))
            .title_bottom(Line::from(format!(" {footer} ")).right_aligned().style(theme.muted_style()))
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        let capacity = inner.height as usize;
        self.last_page_rows = capacity.max(1);

        if idx.is_empty() {
            frame.render_widget(block, area);
            let msg_text = if self.filter.is_some() { s.live_empty_filtered } else { s.live_empty };
            let msg = Line::styled(msg_text, theme.muted_style()).centered();
            frame.render_widget(msg, inner.centered_vertically(Constraint::Length(1)));
            return;
        }

        // 只建这一帧真的要画的那一屏 (P4 终审 Minor: release 下 2000 条约 1.2ms/帧, 每帧上万次
        // 分配) —— `window` 算出该建哪一段, 自己维护 `TableState.offset`(恒 0, 因为 `Table` 拿到的
        // 已经是裁过的那一屏) 与 `select`(换算成窗口内下标)。
        let sel_pos = self.selected.and_then(|seq| idx.iter().position(|&i| self.entries[i].seq == seq));
        let (start, sel_in_window) = window(idx.len(), sel_pos, capacity, self.last_window_start);
        self.last_window_start = start;
        self.table_state.select(sel_in_window);
        *self.table_state.offset_mut() = 0;

        // `Constraint::Fill(1)` (订阅名) 的实际宽度: `inner.width` 已经减掉了边框 + 内距, 再减选中
        // 前缀 (`HIGHLIGHT_COL`)、其余四个定宽列、以及列间距 (`column_spacing(2)`, 5 列 4 个间隔)——
        // 不这样算的话 `format::fit` 不知道该截到多宽, 订阅名超长时 ratatui 会不带省略号地硬切
        // (Finding 5)。
        let elapsed_col = elapsed_col(s);
        let fixed_cols = TIME_COL as u16 + VM_COL as u16 + 1 /* 结果符号列 */ + elapsed_col as u16;
        let sub_col = inner.width.saturating_sub(HIGHLIGHT_COL).saturating_sub(fixed_cols).saturating_sub(4 * 2) as usize;

        let end = (start + capacity).min(idx.len());
        let rows: Vec<Row> = idx[start..end].iter().map(|&i| build_row(&self.entries[i], store, theme, s, tz, tick, sub_col)).collect();
        let widths = [
            Constraint::Length(TIME_COL as u16),
            Constraint::Length(VM_COL as u16),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(elapsed_col as u16),
        ];
        let table = Table::new(rows, widths)
            .column_spacing(2)
            .highlight_symbol("▌ ")
            .highlight_spacing(HighlightSpacing::Always)
            .row_highlight_style(Style::new().add_modifier(ratatui::style::Modifier::REVERSED))
            .block(block);
        frame.render_stateful_widget(&table, area, &mut self.table_state);

        // 行入场动效: 只对「此刻画在屏幕上、且 at_ms >= now_ms - 1000」的闪烁, 页面在后台攒下的
        // 旧行不补播 (与订阅页 draw_list / 虚拟模型页 draw_members 同一套「渲染完读 offset 换算
        // rect」写法)。`pos` 是全局下标 (过滤后列表里的位置), 要减去窗口起点 `start` 才是这一屏
        // 里的行号——`rows` 现在只建了 `[start, start+capacity)` 这一段, 不再是全部可见条目。
        for &seq in flash {
            let Some(pos) = idx.iter().position(|&i| self.entries[i].seq == seq) else { continue };
            if self.entries[idx[pos]].at_ms < ctx.now_ms - 1000 {
                continue;
            }
            if pos < start || pos >= start + capacity {
                continue;
            }
            let rect = Rect::new(inner.x, inner.y + (pos - start) as u16, inner.width, 1);
            ctx.fx.row_new(seq, rect, ctx.theme.accent);
        }
    }
}

fn outcome_result_span(outcome: Outcome, theme: &Theme, tick: u64) -> Span<'static> {
    match outcome {
        Outcome::Pending => {
            let glyph = Throbber::default().throbber_set(BRAILLE_SIX).to_symbol_span(&spinner_state(tick));
            Span::styled(glyph.content.to_string(), Style::new().fg(theme.warn))
        }
        Outcome::Done { ok: true, .. } => Span::styled("✓", Style::new().fg(theme.ok)),
        Outcome::Done { ok: false, .. } => Span::styled("✕", Style::new().fg(theme.err)),
        Outcome::Interrupted => Span::styled("?", theme.muted_style()),
    }
}

fn outcome_elapsed_span(outcome: Outcome, s: &'static Strings, theme: &Theme) -> Span<'static> {
    match outcome {
        Outcome::Pending => Span::styled("…", theme.muted_style()),
        Outcome::Done { elapsed_ms: Some(ms), .. } => Span::raw(duration(ms)),
        Outcome::Done { elapsed_ms: None, .. } => Span::raw("—"),
        Outcome::Interrupted => Span::styled(s.live_interrupted, theme.muted_style()),
    }
}

/// 订阅名取 `store` 的 `display_name`, 找不到就用 id 前 8 位 (`muted`)。`sub_col`: `Fill(1)` 列的
/// 实际宽度 (调用方按 `draw_table` 里的公式手算), 超长名字用 `fit` 截断补省略号, 不让 ratatui
/// 不带提示地硬切 (Finding 5)。
fn build_row(entry: &Entry, store: &Store, theme: &Theme, s: &'static Strings, tz: Tz, tick: u64, sub_col: usize) -> Row<'static> {
    let time_cell = Cell::from(fit(&clock(entry.at_ms, tz), TIME_COL));
    match &entry.kind {
        EntryKind::Gap => Row::new(vec![
            time_cell,
            Cell::from(""),
            Cell::from(Span::styled(fit(s.live_gap, sub_col), theme.muted_style())),
            Cell::from(""),
            Cell::from(""),
        ]),
        EntryKind::Attempt { vm, sub_id, outcome } => {
            let vm_cell = Cell::from(fit(vm, VM_COL));
            let (name, muted) = match store.subscription(sub_id) {
                Some(sub) => (sub.display_name.clone(), false),
                None => (sub_id.chars().take(8).collect::<String>(), true),
            };
            let sub_style = if muted { theme.muted_style() } else { Style::default() };
            let sub_cell = Cell::from(Span::styled(fit(&format!("→ {name}"), sub_col), sub_style));
            let result_cell = Cell::from(Line::from(outcome_result_span(*outcome, theme, tick)));
            let elapsed_cell = Cell::from(Line::from(outcome_elapsed_span(*outcome, s, theme)).right_aligned());
            Row::new(vec![time_cell, vm_cell, sub_cell, result_cell, elapsed_cell])
        }
    }
}

/// 最近 60 秒每秒的次数, 下标 59 = `now_ms` 所在的这一秒; 按整秒对齐 (`div_euclid(1000)`),
/// 更早或更晚 (未来) 的忽略。纯函数。
pub fn per_second(at_ms: impl IntoIterator<Item = i64>, now_ms: i64) -> [u64; 60] {
    let mut buckets = [0u64; 60];
    let now_sec = now_ms.div_euclid(1000);
    for ms in at_ms {
        let sec = ms.div_euclid(1000);
        let diff = now_sec - sec;
        if (0..60).contains(&diff) {
            buckets[59 - diff as usize] += 1;
        }
    }
    buckets
}

impl Component for Live {
    fn handle_key(&mut self, key: KeyEvent, store: &Store, s: &'static Strings) -> Option<Action> {
        match key.code {
            KeyCode::Char(' ') => {
                self.toggle_pause();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            KeyCode::PageUp => {
                self.move_selection(-(self.last_page_rows.max(1) as isize));
                None
            }
            KeyCode::PageDown => {
                self.move_selection(self.last_page_rows.max(1) as isize);
                None
            }
            KeyCode::Char('g') => {
                self.select_first();
                None
            }
            KeyCode::Char('G') => {
                self.selected = None;
                self.paused_at = None;
                None
            }
            KeyCode::Char('/') => Some(self.open_filter_picker(store, s)),
            KeyCode::Enter => self.target_subscription_for_logs().map(|subscription_id| Action::OpenLogsFor { subscription_id }),
            KeyCode::Esc => {
                if self.filter.is_some() {
                    self.filter = None;
                } else if self.selected.is_some() {
                    self.selected = None;
                }
                None
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _store: &Store, _s: &'static Strings) -> Vec<Cmd> {
        match action {
            Action::Refresh | Action::Poll | Action::Connected { .. } => {
                vec![Cmd::Fetch(Fetch::Subscriptions), Cmd::Fetch(Fetch::VirtualModels)]
            }
            Action::PickerDone { tag: PickerTag::LiveFilter, choice: PickerChoice::Item(id) } => {
                self.filter = if id == "*" { None } else { parse_filter(id) };
                self.clear_selection_if_hidden();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &mut DrawCtx) {
        let flash = std::mem::take(&mut self.flash);
        let [spark_area, table_area] = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).areas(area);

        self.draw_spark(frame, spark_area, ctx);
        self.draw_table(frame, table_area, ctx, &flash);
    }

    fn hints(&self, s: &'static Strings) -> Vec<Hint<'static>> {
        let space_hint = if self.paused_at.is_some() { s.key_resume } else { s.key_pause };
        let mut hints = vec![(s.key_space, space_hint), ("↑↓", s.key_select)];
        if self.selected.is_some() {
            hints.push(("G", s.key_latest));
        }
        hints.push(("⏎", s.key_logs));
        hints.push(("/", s.key_filter));
        if self.filter.is_some() {
            hints.push(("Esc", s.key_clear_filter));
        }
        hints
    }

    fn help(&self, s: &'static Strings) -> &'static [(&'static str, &'static str)] {
        s.live_help_rows
    }

    fn on_event(&mut self, ev: StreamEvent<'_>) {
        match ev {
            StreamEvent::Message { name, data, at_ms } if name == ROUTE_ATTEMPT_STARTED => self.on_started(data, at_ms),
            StreamEvent::Message { name, data, at_ms } if name == ROUTE_ATTEMPT_FINISHED => self.on_finished(data, at_ms),
            StreamEvent::Message { .. } => {}
            StreamEvent::Lost { at_ms } => self.on_lost(at_ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyModifiers;
    use ratatui::Terminal;

    use super::*;
    use crate::action::{Action, Cmd, Fetch};
    use crate::fx::Fx;
    use crate::i18n::ZH;
    use crate::theme::ColorMode;
    use crate::widgets::picker::{PickerChoice, PickerTag};

    const NOW: i64 = 1_700_000_000_000;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn send_started(live: &mut Live, vm: &str, sub: &str, at_ms: i64) {
        let data = format!(r#"{{"subscription_id":"{sub}","virtual_model":"{vm}"}}"#);
        live.on_event(StreamEvent::Message { name: ROUTE_ATTEMPT_STARTED, data: &data, at_ms });
    }

    fn send_finished(live: &mut Live, vm: &str, sub: &str, ok: bool, at_ms: i64) {
        let data = format!(r#"{{"subscription_id":"{sub}","virtual_model":"{vm}","success":{ok}}}"#);
        live.on_event(StreamEvent::Message { name: ROUTE_ATTEMPT_FINISHED, data: &data, at_ms });
    }

    fn send_lost(live: &mut Live, at_ms: i64) {
        live.on_event(StreamEvent::Lost { at_ms });
    }

    #[test]
    fn started_then_finished_pairs_first_in_first_out() {
        let mut live = Live::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        send_started(&mut live, "model-sonnet", "sub-1", NOW + 100);
        send_finished(&mut live, "model-sonnet", "sub-1", true, NOW + 500);

        assert_eq!(live.entries.len(), 2);
        assert_eq!(
            live.entries[0].kind,
            EntryKind::Attempt { vm: "model-sonnet".into(), sub_id: "sub-1".into(), outcome: Outcome::Done { ok: true, elapsed_ms: Some(500) } },
            "最早一条应该先被结束, 耗时相对它自己的 started 时刻算"
        );
        assert_eq!(
            live.entries[1].kind,
            EntryKind::Attempt { vm: "model-sonnet".into(), sub_id: "sub-1".into(), outcome: Outcome::Pending },
            "第二条仍应该是 Pending"
        );
    }

    /// Finding 1: 一条 `started` 永远等不到自己的 `finished` (客户端中途断开) 不该永久错位后面的
    /// 配对——过了 `PENDING_STALE_MS` 还没结束的 `Pending` 应该先被判成 `Interrupted`, 让位给更晚
    /// 到达的 `finished` 正确配对到它自己的 `started`。
    #[test]
    fn a_stale_pending_stops_absorbing_later_finishes() {
        let mut live = Live::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        let b_start = NOW + PENDING_STALE_MS + 1;
        send_started(&mut live, "model-sonnet", "sub-1", b_start);
        send_finished(&mut live, "model-sonnet", "sub-1", true, b_start + 50);

        assert_eq!(live.entries.len(), 2, "陈旧的 A 应该原地变成 Interrupted, 不该被 push 成新行");
        assert_eq!(
            live.entries[0].kind,
            EntryKind::Attempt { vm: "model-sonnet".into(), sub_id: "sub-1".into(), outcome: Outcome::Interrupted },
            "A 早已过期, 应该被判定为中断, 不再参与配对"
        );
        assert_eq!(
            live.entries[1].kind,
            EntryKind::Attempt { vm: "model-sonnet".into(), sub_id: "sub-1".into(), outcome: Outcome::Done { ok: true, elapsed_ms: Some(50) } },
            "B 应该配对到它自己的 finished, 耗时是 B 自己的间隔而不是 A 被顶替后算出来的间隔"
        );
    }

    /// Finding 1: 仍在阈值内的陈旧 `Pending` 不该被误伤——FIFO 规则照常生效。
    #[test]
    fn a_pending_within_the_threshold_still_pairs_fifo() {
        let mut live = Live::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        send_started(&mut live, "model-sonnet", "sub-1", NOW + 100);
        send_finished(&mut live, "model-sonnet", "sub-1", true, NOW + PENDING_STALE_MS - 1);

        assert_eq!(live.entries.len(), 2);
        assert_eq!(
            live.entries[0].kind,
            EntryKind::Attempt {
                vm: "model-sonnet".into(),
                sub_id: "sub-1".into(),
                outcome: Outcome::Done { ok: true, elapsed_ms: Some(PENDING_STALE_MS - 1) },
            },
            "仍在阈值内的最早一条应该正常配对, 不该被误判成过期"
        );
        assert_eq!(
            live.entries[1].kind,
            EntryKind::Attempt { vm: "model-sonnet".into(), sub_id: "sub-1".into(), outcome: Outcome::Pending },
            "第二条仍应该是 Pending"
        );
    }

    #[test]
    fn finished_without_a_started_becomes_its_own_row() {
        let mut live = Live::default();
        send_finished(&mut live, "model-opus", "sub-2", false, NOW);
        assert_eq!(live.entries.len(), 1);
        assert_eq!(
            live.entries[0].kind,
            EntryKind::Attempt { vm: "model-opus".into(), sub_id: "sub-2".into(), outcome: Outcome::Done { ok: false, elapsed_ms: None } }
        );
    }

    #[test]
    fn a_finished_only_matches_the_same_virtual_model_and_subscription() {
        let mut live = Live::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        send_finished(&mut live, "model-opus", "sub-1", true, NOW + 10); // 虚拟模型不同
        send_finished(&mut live, "model-sonnet", "sub-2", true, NOW + 20); // 订阅不同

        assert_eq!(live.entries.len(), 3, "两条不匹配的 finished 都应该自成新行");
        assert_eq!(
            live.entries[0].kind,
            EntryKind::Attempt { vm: "model-sonnet".into(), sub_id: "sub-1".into(), outcome: Outcome::Pending },
            "原来那条 Pending 不该被误配对"
        );
    }

    #[test]
    fn connection_lost_interrupts_pending_attempts_and_adds_one_gap() {
        let mut live = Live::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        send_lost(&mut live, NOW + 100);

        assert_eq!(live.entries.len(), 2);
        assert_eq!(
            live.entries[0].kind,
            EntryKind::Attempt { vm: "model-sonnet".into(), sub_id: "sub-1".into(), outcome: Outcome::Interrupted }
        );
        assert_eq!(live.entries[1].kind, EntryKind::Gap);
        assert_eq!(live.entries[1].at_ms, NOW + 100);

        send_lost(&mut live, NOW + 200); // 幂等: 第二次不该再加 Gap
        assert_eq!(live.entries.len(), 2, "重连期间 Lost 反复到来不该重复插入分隔行");

        // Lost 之后到来的 finished 不会匹配已经中断的那条, 而是自成新行。
        send_finished(&mut live, "model-sonnet", "sub-1", true, NOW + 300);
        assert_eq!(live.entries.len(), 3);
        assert_eq!(
            live.entries[2].kind,
            EntryKind::Attempt { vm: "model-sonnet".into(), sub_id: "sub-1".into(), outcome: Outcome::Done { ok: true, elapsed_ms: None } }
        );
    }

    #[test]
    fn malformed_payloads_and_unknown_events_are_ignored() {
        let mut live = Live::default();
        live.on_event(StreamEvent::Message { name: ROUTE_ATTEMPT_STARTED, data: "not json", at_ms: NOW });
        live.on_event(StreamEvent::Message { name: ROUTE_ATTEMPT_FINISHED, data: "{}garbage", at_ms: NOW });
        live.on_event(StreamEvent::Message { name: "some_other_event", data: r#"{"subscription_id":"1","virtual_model":"model-sonnet"}"#, at_ms: NOW });
        assert!(live.entries.is_empty());
    }

    #[test]
    fn keeps_only_the_newest_2000_entries() {
        let mut live = Live::default();
        for i in 0..2005 {
            send_started(&mut live, "model-sonnet", "sub-1", NOW + i);
        }
        assert_eq!(live.entries.len(), MAX_ENTRIES);
        assert_eq!(live.entries.front().unwrap().seq, 6, "序号 1..=5 应该被丢弃");

        let mut live2 = Live::default();
        for i in 0..10 {
            send_started(&mut live2, "model-sonnet", "sub-1", NOW + i);
        }
        live2.selected = Some(1);
        for i in 10..2005 {
            send_started(&mut live2, "model-sonnet", "sub-1", NOW + i);
        }
        assert_eq!(live2.selected, None, "选中过的那条被丢掉后, selected 应该变回 None");
    }

    #[test]
    fn pause_hides_new_rows_until_resumed_and_counts_them() {
        let mut live = Live::default();
        let store = Store::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);

        live.handle_key(key(KeyCode::Char(' ')), &store, &ZH);
        assert!(live.paused_at.is_some());

        send_started(&mut live, "model-opus", "sub-2", NOW + 10);
        assert_eq!(live.visible_seqs(), vec![1], "暂停后新增的不该出现在可见列表里");
        assert_eq!(live.entries.len(), 2, "但仍然被缓冲, 不丢事件");
        assert_eq!(live.paused_new_count(), 1);

        live.handle_key(key(KeyCode::Char(' ')), &store, &ZH);
        assert!(live.paused_at.is_none());
        assert_eq!(live.visible_seqs(), vec![1, 2], "继续后应该都看得到");
    }

    #[test]
    fn selection_leaves_follow_mode_and_capital_g_returns_and_resumes() {
        let mut live = Live::default();
        let store = Store::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        send_started(&mut live, "model-opus", "sub-2", NOW + 10);
        assert_eq!(live.selected, None, "默认跟随最新");

        live.handle_key(key(KeyCode::Up), &store, &ZH);
        assert_eq!(live.selected, Some(2), "第一次按上应该选中最后一个可见条目");

        live.handle_key(key(KeyCode::Down), &store, &ZH);
        assert_eq!(live.selected, None, "已经在最后一条, 再按下应该回到跟随最新");

        live.handle_key(key(KeyCode::Up), &store, &ZH);
        live.handle_key(key(KeyCode::Char(' ')), &store, &ZH); // 暂停
        live.handle_key(key(KeyCode::Char('G')), &store, &ZH);
        assert_eq!(live.selected, None, "G 应该回到跟随最新");
        assert!(live.paused_at.is_none(), "G 应该同时继续");
    }

    #[test]
    fn filter_by_virtual_model_or_subscription_and_esc_clears() {
        let mut live = Live::default();
        let store = Store::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        send_started(&mut live, "model-opus", "sub-2", NOW + 10);

        live.update(&Action::PickerDone { tag: PickerTag::LiveFilter, choice: PickerChoice::Item("vm:model-opus".into()) }, &store, &ZH);
        assert_eq!(live.visible_seqs(), vec![2]);

        live.update(&Action::PickerDone { tag: PickerTag::LiveFilter, choice: PickerChoice::Item("sub:sub-1".into()) }, &store, &ZH);
        assert_eq!(live.visible_seqs(), vec![1]);

        live.update(&Action::PickerDone { tag: PickerTag::LiveFilter, choice: PickerChoice::Item("*".into()) }, &store, &ZH);
        assert_eq!(live.visible_seqs(), vec![1, 2]);

        live.update(&Action::PickerDone { tag: PickerTag::LiveFilter, choice: PickerChoice::Item("vm:model-sonnet".into()) }, &store, &ZH);
        assert!(live.filter.is_some());
        live.handle_key(key(KeyCode::Esc), &store, &ZH);
        assert!(live.filter.is_none(), "Esc 应该清除过滤");
        assert_eq!(live.visible_seqs(), vec![1, 2]);
    }

    /// PickerDone 落地后, 若当前选中项不再符合新过滤, 应该改回 None。
    #[test]
    fn picker_done_drops_the_selection_when_it_falls_outside_the_new_filter() {
        let mut live = Live::default();
        let store = Store::default();
        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        send_started(&mut live, "model-opus", "sub-2", NOW + 10);
        live.selected = Some(1); // "model-sonnet"

        live.update(&Action::PickerDone { tag: PickerTag::LiveFilter, choice: PickerChoice::Item("vm:model-opus".into()) }, &store, &ZH);
        assert_eq!(live.selected, None, "选中的 sub-1 不再符合 model-opus 过滤, 应该改回 None");
    }

    #[test]
    fn per_second_counts_the_last_60_seconds() {
        let now = NOW + 500; // 刻意不落在整秒边界上, 与「按整秒对齐」的行为分得开
        let timestamps = vec![now - 200, now - 1500, now - 1500, now - 61_000];
        let buckets = per_second(timestamps, now);
        assert_eq!(buckets[59], 1, "now-200ms 与 now 同一秒");
        assert_eq!(buckets[58], 2, "now-1500ms 落在前一秒, 两条");
        assert_eq!(buckets.iter().sum::<u64>(), 3, "61 秒前的那条应该被忽略");
    }

    /// Task 8: `⏎` 的跳转目标——跟随最新时取最后一个可见尝试的订阅, 选中某一行时取它自己的订阅,
    /// 选中 `Gap` 或者压根没有任何尝试时返回 `None`。
    #[test]
    fn enter_targets_the_selected_subscription_or_the_last_visible_one_in_follow_mode() {
        let mut live = Live::default();
        let store = Store::default();

        assert_eq!(live.handle_key(key(KeyCode::Enter), &store, &ZH), None, "没有任何尝试时应该返回 None");

        send_started(&mut live, "model-sonnet", "sub-1", NOW);
        send_started(&mut live, "model-opus", "sub-2", NOW + 10);

        assert_eq!(
            live.handle_key(key(KeyCode::Enter), &store, &ZH),
            Some(Action::OpenLogsFor { subscription_id: "sub-2".into() }),
            "跟随最新时应该取最后一个可见尝试的订阅"
        );

        live.handle_key(key(KeyCode::Up), &store, &ZH); // 选中最后一条 (sub-2)
        assert_eq!(live.handle_key(key(KeyCode::Enter), &store, &ZH), Some(Action::OpenLogsFor { subscription_id: "sub-2".into() }));

        live.handle_key(key(KeyCode::Up), &store, &ZH); // 选中 sub-1
        assert_eq!(live.handle_key(key(KeyCode::Enter), &store, &ZH), Some(Action::OpenLogsFor { subscription_id: "sub-1".into() }));

        send_lost(&mut live, NOW + 100); // 插入一条 Gap, 两条 Pending 都变成 Interrupted
        assert_eq!(
            live.handle_key(key(KeyCode::Enter), &store, &ZH),
            Some(Action::OpenLogsFor { subscription_id: "sub-1".into() }),
            "断线不该改变已经选中的这一行"
        );

        live.handle_key(key(KeyCode::Down), &store, &ZH); // 移到 sub-2
        assert_eq!(live.handle_key(key(KeyCode::Enter), &store, &ZH), Some(Action::OpenLogsFor { subscription_id: "sub-2".into() }));

        live.handle_key(key(KeyCode::Down), &store, &ZH); // 移到 Gap 行
        assert_eq!(live.handle_key(key(KeyCode::Enter), &store, &ZH), None, "选中 Gap 应该返回 None");
    }

    #[test]
    fn refresh_fetches_subscriptions_and_virtual_models() {
        let mut live = Live::default();
        let store = Store::default();
        for action in [Action::Refresh, Action::Poll, Action::Connected { app_version: "1.0".into() }] {
            assert_eq!(
                live.update(&action, &store, &ZH),
                vec![Cmd::Fetch(Fetch::Subscriptions), Cmd::Fetch(Fetch::VirtualModels)],
                "{action:?} 应该同时补拉订阅与虚拟模型"
            );
        }
    }

    /// P4 终审 Minor: `window` 是 `draw_table` 只建视口那一屏的核心纯函数, 覆盖跟随最新 / 选中项
    /// 在窗口内外移动 / 上一帧起点越界 (条目被过滤裁掉) / `capacity == 0` 这几类边界。
    #[test]
    fn window_follows_the_tail_and_keeps_the_selection_visible() {
        // 跟随最新: 贴底
        assert_eq!(window(100, None, 10, 0), (90, None));
        // 条目比窗口少: 从头开始
        assert_eq!(window(3, None, 10, 0), (0, None));
        // 选中项已经在窗口里: 窗口不动
        assert_eq!(window(100, Some(45), 10, 40), (40, Some(5)));
        // 选中项在窗口上方: 窗口上移到它贴顶
        assert_eq!(window(100, Some(12), 10, 40), (12, Some(0)));
        // 选中项在窗口下方: 窗口下移到它贴底
        assert_eq!(window(100, Some(55), 10, 40), (46, Some(9)));
        // 上一帧的起点已经越界 (条目被裁掉了): 夹回合法范围
        assert_eq!(window(20, Some(19), 10, 500), (10, Some(9)));
        // 全函数: capacity 为 0 不 panic
        assert_eq!(window(20, Some(3), 0, 0), (0, None));
    }

    /// P4 终审 Minor: 2000 条、24 行高的终端 (内容区最多 20 行, 去掉边框与内距更少)——跟随最新时
    /// 窗口应该贴底, 也就是起点非常接近 2000, 而不是从 0 开始把 2000 行全建一遍。
    ///
    /// 光看 `last_window_start()` 不够: 它只是 `window()` 自己的记账, 哪怕以后有人把
    /// `draw_table` 里 `idx[start..end].iter()` 误改回 `idx.iter()`(比如合并冲突取错边),
    /// 记账照旧正确、这条断言照样通过, 但 `Table` 实际拿到的是全部 2000 行 + `offset = 0`——
    /// 真正渲染出来的会是**最旧的一屏**, 页面从此再也追不上最新的尝试, 而 6 条 fixture 的快照
    /// 测试 (`ui.rs::live_80x24` 等) 根本不会触发滚动, 也测不出这个回归。所以这里把新旧两批
    /// 分别塞进不同的虚拟模型名 (更醒目、比时间戳格式更不容易踩坑), 直接断言渲染文本里到底有
    /// 没有出现这两个名字。
    #[test]
    fn the_table_only_builds_the_rows_it_can_show() {
        let mut live = Live::default();
        // 前 1900 条是「旧」批次, 窗口贴底之后应该完全滚出视野; 后 100 条是「新」批次, 数量
        // 远超一屏的可视行数 (~18 行), 保证整个窗口都落在这一批里, 断言才不会因为窗口边界
        // 正好切在两批之间而变得脆弱。虚拟模型名故意各占 6 列 (远小于 `VM_COL = 14`), 保证
        // `fit()` 不会截断它们——用完整字符串做子串断言, 不用去操心「截断后的省略号会不会
        // 刚好吞掉我关心的那几个字符」。
        for i in 0..1900 {
            send_started(&mut live, "vm-old", "old-sub", NOW + i);
        }
        for i in 0..100 {
            send_started(&mut live, "vm-new", "new-sub", NOW + 65_000 + i); // 顺带跨过一次分钟边界
        }

        let store = Store::default();
        let theme = Theme::new(ColorMode::TrueColor);
        let mut fx = Fx::new(false);
        let busy = HashMap::new();
        let last_outcome = HashMap::new();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                let mut ctx = DrawCtx {
                    theme: &theme,
                    s: &ZH,
                    now_ms: NOW + 66_000,
                    tick: 0,
                    fx: &mut fx,
                    store: &store,
                    busy: &busy,
                    last_outcome: &last_outcome,
                    tz: Tz::Fixed(8 * 3600),
                };
                live.draw(f, area, &mut ctx);
            })
            .unwrap();
        let out = terminal.backend().to_string();

        assert!(live.last_window_start() >= 1900, "跟随最新时窗口应该贴底, 实际起点 {}", live.last_window_start());
        assert!(out.contains("vm-new"), "窗口应该真的建出了最新一批的行\n{out}");
        assert!(!out.contains("vm-old"), "窗口不该把已经滚出视野的最旧一批也建出来\n{out}");
    }
}
