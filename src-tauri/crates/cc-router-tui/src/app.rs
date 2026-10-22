//! 应用状态 + 外壳 (标签栏 / 底栏 / 帮助 / toast)。不碰网络也不碰真实终端:
//! `handle_key` / `update` 是同步纯逻辑, `draw` 只写 `Frame` —— 所以整个文件可以用 `TestBackend` 测。
//! 把它接到键盘、定时器、HTTP 上的是 `runtime.rs`。

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Tabs};
use ratatui::Frame;
use throbber_widgets_tui::{Throbber, BRAILLE_SIX};

use crate::action::{Action, BusyKey, Cmd, Fetch, FetchData, FetchKind, Mutation, MutationOutcome, OnYes, Tab, WizardCmd};
use crate::client::dto::{RefreshBalanceResult, RefreshModelsResult};
use crate::format::Tz;
use crate::fx::{self, Dir, Fx};
use crate::i18n::Strings;
use crate::pages::{Component, DrawCtx, Pages, StreamEvent};
use crate::popup::{ConfirmState, Popup, PopupCtx};
use crate::store::Store;
use crate::theme::Theme;
use crate::widgets::detail::DetailState;
use crate::widgets::keybar::Hint;
use crate::widgets::picker::PickerState;
use crate::widgets::toast::{self, Toast, ToastKind};
use crate::widgets::{keybar, spinner_state};
use crate::wizard::Wizard;

/// 版本不一致横幅第一行的前缀与续行缩进 (两者等宽: `⚠` 占一列)。
const BANNER_MARK: &str = " ⚠ ";
const BANNER_INDENT: &str = "   ";
const BANNER_MAX_ROWS: usize = 3;

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;
/// 可见页面每 5 秒重拉一次用量类数字 (20 × 250ms)。
const POLL_EVERY_TICKS: u64 = 20;
/// toast 队列上限, 含正在屏幕上的那条; 超出丢最旧的排队项 (F4)。
const MAX_TOASTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Conn {
    Connecting,
    Connected,
    Reconnecting,
}

/// 等下一帧知道几何信息之后才能触发的弹窗动效。
#[derive(Debug, Clone, Copy)]
enum PopupFx {
    Open,
    Close(Rect),
}

pub struct AppOptions {
    pub strings: &'static Strings,
    pub theme: Theme,
    pub fx_enabled: bool,
    /// Unix 毫秒; 之后由 `Action::Tick` 推进。
    pub now_ms: i64,
    /// 生产传 `env!("CARGO_PKG_VERSION")`。做成参数是为了快照测试不随发版改版本号而失效。
    pub tui_version: &'static str,
    /// 显示时间用的时区。生产传 `Tz::Local`; 测试传 `Tz::Fixed`, 快照不随机器时区变化。
    pub tz: Tz,
}

pub struct App {
    s: &'static Strings,
    theme: Theme,
    fx: Fx,
    tui_version: &'static str,
    tab: Tab,
    store: Store,
    pages: Pages,
    /// 新建订阅向导。不是标签页也不是弹窗, 是夹在弹窗与全局键之间的一层——见
    /// `wizard::Wizard` 模块顶部的文档注释。
    wizard: Option<Wizard>,
    /// 最近一次打开的向导的代次, 每次 `OpenWizard` 加一; 只在 `wizard` 为 `Some` 时有意义。向导发出
    /// 的每个请求都盖上它, 结果带回来时不等于它就丢弃——见 `action::WizardResult` 的文档注释。
    wizard_epoch: u64,
    popup: Option<Popup>,
    popup_area: Option<Rect>,
    pending_popup_fx: Option<PopupFx>,
    pending_page_fx: Option<Dir>,
    startup_played: bool,
    toasts: VecDeque<Toast>,
    conn: Conn,
    app_version: Option<String>,
    now_ms: i64,
    tick: u64,
    tz: Tz,
    /// 正在进行的就地操作, 键是 [`BusyKey`] (目前只会出现 `Subscription` 变体)。**按
    /// `Mutation::busy_key()` 判重, 不按 `Mutation` 整体** —— 同一条订阅同时只能有一个操作在跑,
    /// 但不同操作 (比如先 `t` 再 `e`) 仍然互斥, 不是各自独立排队。
    busy: HashMap<BusyKey, Mutation>,
    /// 每条订阅最近一次就地操作的结果, 与对应 toast 用的是**同一份文本**: toast 只能显示
    /// 一行 (≤72 列, 3 秒就消失), 详情面板的「上次操作」行拿这份存档展示完整文案。发起新操作时
    /// (`start_mutation` 真的派发出去那一刻) 移除对应条目, 不是等结果回来才清。
    last_outcome: HashMap<String, (ToastKind, String)>,
}

impl App {
    pub fn new(opts: AppOptions) -> Self {
        Self {
            s: opts.strings,
            theme: opts.theme,
            fx: Fx::new(opts.fx_enabled),
            tui_version: opts.tui_version,
            tab: Tab::Overview,
            store: Store::default(),
            pages: Pages::default(),
            wizard: None,
            wizard_epoch: 0,
            popup: None,
            popup_area: None,
            pending_popup_fx: None,
            pending_page_fx: None,
            startup_played: false,
            toasts: VecDeque::new(),
            conn: Conn::Connecting,
            app_version: None,
            now_ms: opts.now_ms,
            tick: 0,
            tz: opts.tz,
            busy: HashMap::new(),
            last_outcome: HashMap::new(),
        }
    }

    /// 有动效在播: 主循环应该按约 60fps 继续画; 否则等下一个事件再画。
    pub fn wants_fast_frames(&self) -> bool {
        self.fx.is_running()
    }

    /// `runtime.rs` 用它把 `ClientError` 格式化成当前语言的文字 (`i18n::client_error`)——
    /// `App` 是唯一持有 `&'static Strings` 的地方, 主循环没有自己的一份。
    pub fn strings(&self) -> &'static Strings {
        self.s
    }

    /// 当前向导的代次; 没有向导时 `None`。只读: 投递 `Action::WizardDone` 的一方 (测试) 要知道
    /// 该带哪个代次, 生产路径上它由 `Cmd::Wizard` 原样带回, 用不着这个。
    pub fn wizard_epoch(&self) -> Option<u64> {
        self.wizard.as_ref().map(|_| self.wizard_epoch)
    }

    fn version_mismatch(&self) -> Option<String> {
        let app = self.app_version.as_deref()?;
        let tui = self.tui_version;
        (app != tui).then(|| (self.s.version_mismatch)(tui, app))
    }

    /// 版本不一致横幅按屏幕宽度折行, 第一行带 `⚠`, 续行与正文对齐。可操作的那半句 (重新添加到
    /// PATH) 在句尾, 单行显示会在 80 列上被截掉。最多 [`BANNER_MAX_ROWS`] 行, 再长也不挤占页面。
    fn banner_lines(&self, width: u16) -> Option<Vec<String>> {
        let text = self.version_mismatch()?;
        let mut lines = crate::format::wrap(&text, usize::from(width).saturating_sub(BANNER_INDENT.len()));
        lines.truncate(BANNER_MAX_ROWS);
        Some(lines.into_iter().enumerate().map(|(i, l)| format!("{}{l}", if i == 0 { BANNER_MARK } else { BANNER_INDENT })).collect())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Windows 上按下和松开各报一次, 只认按下。
        if key.kind != KeyEventKind::Press {
            return None;
        }
        // Ctrl+C 永远立即退出, 不确认——弹窗打开、页面有未保存修改都不例外, 这是它与 `q`
        // (`Action::Quit`, 会先检查 dirty / 被弹窗吞掉) 唯一的区别。
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Action::ForceQuit);
        }
        // 弹窗打开时按变体各自决定按键含义 (`impl Popup::handle_key`); 键盘完全归
        // 弹窗, 到不了下面的全局键 / 页面。`&mut self.popup`: `Popup::Picker`/`Popup::Detail` 的
        // 按键 (打字 / 移动选中 / 滚动) 直接改自身状态, 不像 Help / Confirm 那样只读。
        if let Some(popup) = &mut self.popup {
            return popup.handle_key(key);
        }
        // 向导那一层排在弹窗之后、全局键之前: 存在时除 `Ctrl+C` 外的全部按键归它 (所以 `q` / `r` /
        // `1`-`5` 能被向导当普通字符输入), 全局键与页面都到不了。
        if let Some(wizard) = &mut self.wizard {
            return wizard.handle_key(key, &self.store, self.s);
        }
        match key.code {
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Char('r') => Some(Action::Refresh),
            KeyCode::Tab => Some(Action::NextTab),
            KeyCode::BackTab => Some(Action::PrevTab),
            KeyCode::Char(c @ '1'..='5') => Tab::from_index(c as usize - '1' as usize).map(Action::SwitchTab),
            _ => self.pages.get_mut(self.tab).handle_key(key, &self.store, self.s),
        }
    }

    fn switch_tab(&mut self, to: Tab, dir: Dir) -> Vec<Cmd> {
        if to == self.tab {
            return Vec::new();
        }
        self.tab = to;
        self.pending_page_fx = Some(dir);
        // 不可见的页面不轮询, 所以切回来的那一刻要补一次。
        if self.conn == Conn::Connected {
            self.update_page(self.tab, &Action::Refresh)
        } else {
            Vec::new()
        }
    }

    fn close_popup(&mut self) {
        if self.popup.take().is_some() {
            self.pending_popup_fx = self.popup_area.take().map(PopupFx::Close);
        }
    }

    /// 打开一个新弹窗, 替换掉已经打开的那个 (如果有)。`Action::OpenConfirm` / `Action::OpenPicker` /
    /// `Action::OpenDetail` / `Action::ToggleHelp` 四个来源共用。
    ///
    /// 先走正常的关闭路径——如果已经有另一个弹窗开着, 这样才会清掉它的 `popup_area`
    /// (不清的话, 如果新弹窗在第一次 draw() 之前就被关掉, 关闭动效会拿旧弹窗的几何去播), 而不是
    /// 直接覆盖 `self.popup` 留下不一致的状态。
    fn open_popup(&mut self, popup: Popup) {
        self.close_popup();
        self.popup = Some(popup);
        self.pending_popup_fx = Some(PopupFx::Open);
    }

    /// 当前的「编辑上下文」是不是有未保存的东西。向导只要存在就算——它一定在收集输入。
    fn current_dirty(&self) -> bool {
        self.wizard.is_some() || self.pages.get(self.tab).is_dirty()
    }

    /// 丢弃当前的编辑上下文: **有向导时关掉向导** (并补一次订阅列表重拉, 因为向导可能已经创建了
    /// 订阅), 否则丢弃当前页草稿。
    fn discard_current(&mut self) -> Vec<Cmd> {
        if self.wizard.take().is_some() {
            vec![Cmd::Fetch(Fetch::Subscriptions)]
        } else {
            self.pages.get_mut(self.tab).discard_changes();
            Vec::new()
        }
    }

    /// `Quit` / `SwitchTab` / `NextTab` / `PrevTab` 四个来源共用: 当前「编辑上下文」`current_dirty()`
    /// 时不直接执行 `action`, 而是打开确认弹窗把它包进 `on_yes`; 否则调用 `run` 真正执行。
    /// `OnYes::DiscardThen` 在 `discard_current()` 之后再 `update(*inner)`, 会重新走到这里, 但那时
    /// `current_dirty()` 已经是 `false`——天然放行, 不需要另一条「不检查 dirty」的旁路。
    fn guard_dirty(&mut self, action: Action, run: impl FnOnce(&mut Self) -> Vec<Cmd>) -> Vec<Cmd> {
        if self.current_dirty() {
            let prompt = self.s.confirm_discard.to_string();
            self.open_popup(Popup::Confirm(ConfirmState { prompt, on_yes: OnYes::discard_then(action) }));
            Vec::new()
        } else {
            run(self)
        }
    }

    /// 调用向导的 `update`, 再轮询它的两个待办 —— 与 `App::update_page` 对页面做的事一一对应。
    /// 顺序要紧: 先取 notice (关闭之后向导就没了), 再看 close 请求。
    ///
    /// `notice`/`should_close` 先落进局部变量、`w` 借用到此为止, 再调 `self.push_toast`/
    /// `self.update`——这两个都要重新独占借用整个 `self`, 不能跟仍然存活的 `w`
    /// (借用自 `self.wizard`) 同时成立 (借用检查器不按字段做跨方法调用的不相交分析)。
    fn update_wizard(&mut self, action: &Action) -> Vec<Cmd> {
        let epoch = self.wizard_epoch;
        let Some(w) = &mut self.wizard else { return Vec::new() };
        let mut cmds = stamp(epoch, w.update(action, &self.store, self.s));
        let notice = w.take_notice();
        let should_close = w.take_close_request();
        if let Some((kind, text)) = notice {
            self.push_toast(Toast::new(kind, text));
        }
        if should_close {
            cmds.extend(self.update(Action::CloseWizard));
        }
        cmds
    }

    /// `Store` 刚接受了一份订阅列表 (`FetchDone` 两个分支共用): 广播给所有页面。**唯一**列出
    /// 全部页面字段的地方 —— 以后加新页面只改这一处, 不会出现"某个 FetchDone 分支忘了通知新
    /// 页面"这种只有部分刷新路径才触发、没有测试能咬住的漏更。
    ///
    /// `on_subscriptions_changed` 可能顺带产出一条通知 (订阅详情页的
    /// 草稿对应的订阅这一刻从 `Store` 里消失了) —— 这条通知不是从 `Component::update()` 触发的,
    /// 不会被 `update_page` 那条轮询路径捡到, 所以这里也照 `update_page` 的规矩轮询一次取走,
    /// 不然要等下一次真正的 `update()` 调用才会显示, 违背"立刻生效"的本意。
    ///
    /// 「订阅消失时关掉指着它的选择器」**不**靠这里的通知内容判断 (那条通知只在已经有草稿时才会
    /// 产出), 而是由 [`App::close_picker_if_subscription_vanished`] 直接查弹窗的 `tag`。
    fn notify_subscriptions_changed(&mut self, changed: &[String]) {
        let store = &self.store;
        let s = self.s;
        let mut notices = Vec::new();
        self.pages.for_each_mut(|page| {
            page.on_subscriptions_changed(changed, store, s);
            if let Some(notice) = page.take_notice() {
                notices.push(notice);
            }
        });
        for (kind, text) in notices {
            self.push_toast(Toast::new(kind, text));
        }
    }

    /// `Store` 刚**接受**了一份订阅列表之后调用 (`FetchDone` 两个分支共用, 紧跟在
    /// `notify_subscriptions_changed` / `notify_store_changed` 之后) —— 如果当前弹窗是选择器
    /// (模型 / 思考档位), 且它是为某条订阅开的 (`PickerTag::subscription_id()`), 而这条订阅这一刻
    /// 已经不在 `Store` 里了 (被别处删除), 就关掉它并提示。直接查弹窗自己的 `tag()`, 不依赖任何
    /// 页面主动发出通知, 弹窗打开之后、用户还没选过任何值 (没有草稿) 的场景也能正确关闭。
    fn close_picker_if_subscription_vanished(&mut self) {
        let Some(Popup::Picker(state)) = &self.popup else { return };
        let Some(sub_id) = state.tag().subscription_id() else { return };
        if self.store.subscription(sub_id).is_some() {
            return;
        }
        self.close_popup();
        // 与订阅页自己发出的 `sub_gone` 通知同类型同文案 (草稿存在时那条仍会照常从
        // `notify_subscriptions_changed` 产出), `push_toast` 的去重规则会把两条合并成一条。
        self.push_toast(Toast::new(ToastKind::Info, self.s.sub_gone));
    }

    /// `Store` 刚**接受**了一份订阅列表或虚拟模型列表 (不管内容变没变)——
    /// 广播给全部页面, 让编辑类页面借这个时机核对一遍 `is_dirty()` 缓存, 不用等下一次真正的
    /// `Component::update()` 调用 (轮询最多要等 5 秒)。与 `notify_subscriptions_changed` 分开:
    /// 后者只在订阅列表**变化**时触发、带着"哪些 id 变了"的 diff (给闪烁用); 这个方法订阅列表和
    /// 虚拟模型列表都触发、不需要 diff。
    fn notify_store_changed(&mut self) {
        let store = &self.store;
        self.pages.for_each_mut(|page| page.on_store_changed(store));
    }

    /// 两处 toast 入口共用: 与最新排队的一条重复 (同 kind 同 text) 就丢弃, 否则挤掉最旧的排队项
    /// (下标 1, 下标 0 是正在屏幕上的那条, 不能被挤走)。
    fn push_toast(&mut self, toast: Toast) {
        let is_dup = self.toasts.back().is_some_and(|t| t.kind == toast.kind && t.text == toast.text);
        if is_dup {
            return;
        }
        self.toasts.push_back(toast);
        while self.toasts.len() > MAX_TOASTS {
            self.toasts.remove(1);
        }
    }

    /// 调用某个页面的 `update`, 并顺带处理它可能产出的通知——`Component::update` 签名只能返回
    /// `Vec<Cmd>`, 塞不进一个 `Action` (那是给 `handle_key` 用的 `Action::Notify`), 页面想在
    /// `update()` 内部弹一条 toast (比如「输入的模型名不能为空」「草稿对应的订阅已经不存在了」)
    /// 就存进 `pending_notice`, 这里在 `update` 返回后轮询一次取走。所有会调
    /// `Component::update` 的地方都该走这个 helper, 不要再各自裸调 `pages.get_mut(tab).update(..)`
    /// ——否则新页面用上 `take_notice` 时会有调用点漏接的风险。
    fn update_page(&mut self, tab: Tab, action: &Action) -> Vec<Cmd> {
        let cmds = self.pages.get_mut(tab).update(action, &self.store, self.s);
        if let Some((kind, text)) = self.pages.get_mut(tab).take_notice() {
            self.push_toast(Toast::new(kind, text));
        }
        cmds
    }

    /// 订阅备注名, 取不到 (结果回来时订阅已经不在 `Store` 里了) 就退回用 id。
    fn subscription_name(&self, id: &str) -> String {
        self.store.subscription(id).map(|s| s.display_name.clone()).unwrap_or_else(|| id.to_string())
    }

    /// 忙碌键对应的显示名: 订阅用备注名, 虚拟模型用它自己的名字。
    fn busy_key_name(&self, key: &BusyKey) -> String {
        match key {
            BusyKey::Subscription(id) => self.subscription_name(id),
            BusyKey::VirtualModel(vm_name) => vm_name.clone(),
        }
    }

    /// `Action::Mutate` 落地成真正要发的 `Cmd`: 同一订阅已经有操作在跑 → 弹 toast 拒绝; 断线 → 弹
    /// toast 拒绝; 余额刷新在不支持的 provider 上 → 就地回答, 不发请求。三条判定都不进忙碌表, 只有
    /// 真的要发的那条才 `insert`。
    fn start_mutation(&mut self, m: Mutation) -> Vec<Cmd> {
        let key = m.busy_key();
        // 断线判定必须排在忙碌表前面: 断线期间按在一条正忙的订阅上 (比如上一次操作还没跑完就掉线了)
        // 也该看到「未连接」提示, 而不是被忙碌表悄悄吞掉、什么反馈都没有。
        if self.conn != Conn::Connected {
            self.push_toast(Toast::new(ToastKind::Error, self.s.toast_offline));
            return Vec::new();
        }
        // 不能静默丢弃: 用户确认过删除、却什么都没发生, 只会以为按键没生效。
        if self.busy.contains_key(&key) {
            let name = self.busy_key_name(&key);
            self.push_toast(Toast::new(ToastKind::Info, (self.s.toast_busy)(&name)));
            return Vec::new();
        }
        if matches!(m, Mutation::RefreshBalance { .. }) {
            // 订阅不在 Store 里 (理论上不该发生, 因为发起方是页面自己选中的一条) 时保守放行,
            // 交给后端去报错, 不在这里就地拦。
            if let BusyKey::Subscription(id) = &key {
                let supported = self.store.subscription(id).is_none_or(|s| s.balance_supported);
                if !supported {
                    self.push_toast(Toast::new(ToastKind::Info, self.s.sub_balance_unsupported));
                    return Vec::new();
                }
            }
        }
        self.busy.insert(key.clone(), m.clone());
        // 这条订阅上一次操作的结果 (如果还挂在详情面板上) 已经过时了, 新操作一发起就该
        // 隐去它, 不能让用户以为「上次操作」显示的是这次刚发出去的操作的结果。
        if let BusyKey::Subscription(id) = &key {
            self.last_outcome.remove(id);
        }
        // 紧跟在忙碌表真的插入之后广播给全部页面——`on_mutation_started` 与
        // `on_mutation_done` 成对, 让编辑类页面在这段窗口里拒绝任何会继续修改同一份草稿的按键。
        self.pages.for_each_mut(|page| page.on_mutation_started(&m));
        vec![Cmd::Mutate(Box::new(m))]
    }

    /// `Action::MutationDone`: 从忙碌表移除, 先对 `mutation.refetch()` 里每个目标调对应的
    /// `set_*_barrier(barrier)` (挡住晚到的、内容还是变更前旧值的加载), 再按结果弹一条 toast,
    /// **无论成败都追加一次 `mutation.refetch()` 声明的重拉** —— 状态 / 缓存 / 错误信息可能都变了。
    /// toast 与 `last_outcome` **共用同一份文本**: toast 只能显示一行 (≤72 列, 3 秒就
    /// 消失), 详情面板的「上次操作」行拿 `last_outcome` 展示完整文案, 不受 toast 单行截断的限制。
    /// `last_outcome` 只对 `BusyKey::Subscription` 的变更写 (虚拟模型页没有这块面板);
    /// `BusyKey::VirtualModel` 的变更仍然照常弹 toast、照常重拉, 只是不进这份存档。
    fn finish_mutation(&mut self, mutation: Mutation, barrier: u64, result: Result<MutationOutcome, String>) -> Vec<Cmd> {
        let key = mutation.busy_key();
        self.busy.remove(&key);
        // 每个页面各恰好一次的机会去响应「这次变更是不是我关心的那条草稿」(订阅页在
        // `ok && UpdateSlots && id 匹配当前草稿` 时清草稿); 对全部页面调, 不只当前可见的那个——
        // 用户切走之后保存结果才回来时, 原页面的草稿也该被正确清掉/保留。
        self.pages.for_each_mut(|page| page.on_mutation_done(&mutation, result.is_ok()));
        for fetch in mutation.refetch() {
            match fetch {
                // 总览一次整页加载也带着订阅列表 (`Action::FetchDone` 的 `FetchData::Overview`
                // 分支同样喂给 `Store::apply_subscriptions`), 所以必须和 `Fetch::Subscriptions`
                // 一样推进订阅屏障——目前没有 `Mutation` 会把 `Fetch::Overview` 放进 `refetch()`,
                // 但空着这个分支是一个等真用上才会炸的洞, 先堵上。
                Fetch::Subscriptions | Fetch::Overview => self.store.set_subscriptions_barrier(barrier),
                Fetch::VirtualModels => self.store.set_virtual_models_barrier(barrier),
                // 日志页自己管理分页状态, 不进 `Store`, 没有屏障可设——目前也没有任何
                // `Mutation` 会把 `Fetch::Requests` 放进 `refetch()`, 空着这个分支同样是先堵上。
                Fetch::Requests(_) => {}
            }
        }
        // `Fetch` 不是 `Copy` (`Requests` 带查询参数), `refetch()` 返回的是 `&'static [Fetch]`
        // 的只读切片, 塞进 `Cmd::Fetch` 需要各自拥有的所有权, 所以 `.cloned()`。
        let refetch_cmds: Vec<Cmd> = mutation.refetch().iter().cloned().map(Cmd::Fetch).collect();

        let name = self.busy_key_name(&key);
        let (kind, text) = match &result {
            Ok(MutationOutcome::EnabledSet) => {
                let enabled = matches!(mutation, Mutation::SetEnabled { enabled: true, .. });
                // 乐观更新 Store, 不等下一次 Fetch::Subscriptions 落地: 否则「按 e、还没刷新完又按
                // 一次 e」会从 Store 读到没改过的旧 enabled, 算出同一个目标值发给后端 (no-op),
                // 且第二条 toast 文案与第一条相同, 被 push_toast 的去重规则吞掉, 用户毫无反馈。
                if let BusyKey::Subscription(id) = &key {
                    self.store.set_enabled(id, enabled);
                }
                let text = if enabled { (self.s.toast_enabled)(&name) } else { (self.s.toast_disabled)(&name) };
                (ToastKind::Success, text)
            }
            Ok(MutationOutcome::Tested(r)) if r.ok => (ToastKind::Success, (self.s.toast_test_ok)(&name, r.model_used.as_deref())),
            Ok(MutationOutcome::Tested(r)) => (ToastKind::Error, (self.s.toast_test_failed)(&name, &r.message)),
            Ok(MutationOutcome::Models(RefreshModelsResult::Auto { models, .. })) => {
                (ToastKind::Success, (self.s.toast_models_ok)(&name, models.len()))
            }
            Ok(MutationOutcome::Models(RefreshModelsResult::ManualFallback { reason })) => {
                (ToastKind::Info, (self.s.toast_models_manual)(&name, reason))
            }
            Ok(MutationOutcome::Balance(RefreshBalanceResult::Success { .. })) => (ToastKind::Success, (self.s.toast_balance_ok)(&name)),
            Ok(MutationOutcome::Balance(RefreshBalanceResult::Failed { reason })) => {
                (ToastKind::Error, (self.s.toast_balance_failed)(&name, reason))
            }
            Ok(MutationOutcome::Balance(RefreshBalanceResult::Unsupported)) => (ToastKind::Info, self.s.sub_balance_unsupported.to_string()),
            Ok(MutationOutcome::SlotsSaved) => (ToastKind::Success, (self.s.toast_slots_saved)(&name)),
            Ok(MutationOutcome::VirtualModelSaved) => (ToastKind::Success, (self.s.toast_vm_saved)(&name)),
            Ok(MutationOutcome::Deleted) => (ToastKind::Success, (self.s.toast_deleted)(&name)),
            Err(message) => (ToastKind::Error, (self.s.toast_mutation_failed)(&name, message)),
        };
        self.push_toast(Toast::new(kind, text.clone()));
        if let BusyKey::Subscription(id) = key {
            self.last_outcome.insert(id, (kind, text));
        }
        refetch_cmds
    }

    pub fn update(&mut self, action: Action) -> Vec<Cmd> {
        match action {
            Action::Quit => self.guard_dirty(Action::Quit, |_| vec![Cmd::Quit]),
            Action::ForceQuit => vec![Cmd::Quit],
            Action::SwitchTab(to) => self.guard_dirty(Action::SwitchTab(to), move |app| {
                let dir = if to.index() > app.tab.index() { Dir::Forward } else { Dir::Backward };
                app.switch_tab(to, dir)
            }),
            Action::NextTab => self.guard_dirty(Action::NextTab, |app| {
                let to = app.tab.next();
                app.switch_tab(to, Dir::Forward)
            }),
            Action::PrevTab => self.guard_dirty(Action::PrevTab, |app| {
                let to = app.tab.prev();
                app.switch_tab(to, Dir::Backward)
            }),
            Action::ToggleHelp => {
                if self.popup.is_some() {
                    self.close_popup();
                } else {
                    self.open_popup(Popup::Help);
                }
                Vec::new()
            }
            Action::ClosePopup => {
                self.close_popup();
                Vec::new()
            }
            Action::OpenConfirm { prompt, on_yes } => {
                self.open_popup(Popup::Confirm(ConfirmState { prompt, on_yes }));
                Vec::new()
            }
            Action::Confirmed(on_yes) => {
                self.close_popup();
                match on_yes {
                    OnYes::DiscardThen(inner) => {
                        let mut cmds = self.discard_current();
                        cmds.extend(self.update(*inner));
                        cmds
                    }
                    OnYes::Run(inner) => self.update(*inner),
                }
            }
            Action::DiscardDraft => self.discard_current(),
            Action::OpenWizard => {
                self.wizard_epoch += 1;
                let mut wizard = Wizard::new();
                let cmds = stamp(self.wizard_epoch, wizard.on_open());
                self.wizard = Some(wizard);
                cmds
            }
            // `self.wizard.take().is_some()`: 只有真的关掉了 (调用时向导确实存在) 才补一次重拉——
            // `Confirmed(DiscardThen(CloseWizard))` 这条路径上 `discard_current()` 已经关过一次向导了, 这里再
            // 无条件发一次会产出两条一模一样的 `Cmd::Fetch(Subscriptions)`。
            Action::CloseWizard => {
                if self.wizard.take().is_some() {
                    vec![Cmd::Fetch(Fetch::Subscriptions)]
                } else {
                    Vec::new()
                }
            }
            // 代次不符 = 别的 (已经关掉的) 向导实例发的请求, 这时哪怕阶段与身份字段碰巧都对得上,
            // 结果也不属于眼前这个向导。
            Action::WizardDone { epoch, .. } => {
                if self.wizard.is_some() && epoch == self.wizard_epoch {
                    self.update_wizard(&action)
                } else {
                    Vec::new()
                }
            }
            // 向导表单提交按钮触发的请求。`handle_key` 已经校验并打包好 `WizardCmd`,
            // 这里直接转成 `Cmd::Wizard`——跟上面 `OpenWizard` 直接调用 `wizard.on_open()` 是
            // 同一条思路 (触发点是按键本身, 不经过 `Wizard::update()` 那条给结果用的路径)。
            // **必须判 `self.wizard.is_some()` 才转发**——"没有向导也无害" 这个说法只
            // 对 `WizardDone` 成立 (它落地时向导已经不在, 上面那条"没有向导就丢弃"的分支正好
            // 接住), 对 `Create` 这类会写库的请求不成立: 如果这个 action 被塞进某个
            // 确认弹窗的 `on_yes` (`OnYes::DiscardThen` 会先 `discard_current()` 关掉向导再执行
            // `inner`), 订阅会在向导已经关闭、没人接收结果的情况下照常建到后端, 订阅列表里凭空
            // 多出一条 `(pending)`, 用户毫无察觉也没有 toast。
            Action::WizardRequest(cmd) => {
                if self.wizard.is_some() {
                    vec![Cmd::Wizard { epoch: self.wizard_epoch, cmd }]
                } else {
                    Vec::new()
                }
            }
            Action::Tick { now_ms } => {
                self.now_ms = now_ms;
                self.tick += 1;
                while self.toasts.front().is_some_and(|t| t.expired(now_ms)) {
                    self.toasts.pop_front();
                }
                if self.conn == Conn::Connected && self.tick.is_multiple_of(POLL_EVERY_TICKS) {
                    self.update_page(self.tab, &Action::Poll)
                } else {
                    Vec::new()
                }
            }
            Action::Connected { ref app_version } => {
                if self.conn == Conn::Reconnecting {
                    self.push_toast(Toast::new(ToastKind::Success, self.s.toast_reconnected));
                }
                self.conn = Conn::Connected;
                self.app_version = Some(app_version.clone());
                self.update_page(self.tab, &action)
            }
            Action::ConnectionLost => {
                self.conn = Conn::Reconnecting;
                // 每次都广播, 不判断是不是刚发生状态切换——重连期间这条 action 会反复到来, 页面
                // 处理它必须幂等。
                let at_ms = self.now_ms;
                self.pages.for_each_mut(|page| page.on_event(StreamEvent::Lost { at_ms }));
                Vec::new()
            }
            Action::FetchDone { fetch, issued, result } => match result {
                Err(message) => {
                    // 断线期间每次轮询都会失败, 状态点已经在说「重连中」了, 不再刷屏。
                    if self.conn == Conn::Connected {
                        self.push_toast(Toast::new(ToastKind::Error, (self.s.toast_load_failed)(&message)));
                    }
                    // 加载结果永远交给发起它的页面, 哪怕是失败——日志页据此停掉「加载中」
                    // 状态 (toast 已经在上面弹过, 页面自己不用再弹一次)。其它三种加载没有任何页面
                    // 需要在失败时收到通知 (Overview/Subscriptions/VirtualModels 都只在 Ok 时才有事
                    // 要做), 维持原样不转发。
                    if fetch.kind() == FetchKind::Requests {
                        let action = Action::FetchDone { fetch, issued, result: Err(message) };
                        self.update_page(Tab::Logs, &action)
                    } else {
                        Vec::new()
                    }
                }
                // 总览一次整页加载: 订阅列表 `mem::take` 挪给 Store (不克隆——克隆一次给 Store、
                // 整个 OverviewData 转给总览页时又整体克隆一次, 同一份列表会被复制两遍), 状态变了
                // 的 id 广播给所有页面; 不管 Store 是不是接受了这份订阅列表 (可能是晚到的旧结果),
                // 总览页自己的其它字段 (今日统计 / 每小时序列 / 代理状态) 都照常更新。
                Ok(FetchData::Overview(mut data)) => {
                    debug_assert_eq!(fetch, Fetch::Overview, "spawn_fetch must pair FetchData::Overview only with Fetch::Overview");
                    let subs = std::mem::take(&mut data.subscriptions);
                    let changed = self.store.apply_subscriptions(issued, subs);
                    if let Some(changed) = &changed {
                        self.notify_subscriptions_changed(changed);
                        // `Store` 接受了这份订阅列表, 广播给全部页面核对草稿。
                        self.notify_store_changed();
                        // 再核对一遍当前打开的选择器弹窗 (如果有) 是否还指着一条存在的订阅。
                        self.close_picker_if_subscription_vanished();
                    }
                    let action = Action::FetchDone { fetch, issued, result: Ok(FetchData::Overview(data)) };
                    // 加载结果永远交给发起它的页面, 哪怕用户已经切走了; 总览页目前不产出通知,
                    // 直接调 `update` 不经过 `update_page` (那个 helper 是按 `Tab` 索引页面的,
                    // 这里明确就是总览页自己)。
                    self.pages.overview.update(&action, &self.store, self.s)
                }
                // 单独的订阅列表刷新 (SSE 触发): 只进 Store, 不需要转给任何页面的 update ——
                // 各页面画的时候直接读 ctx.store。
                Ok(FetchData::Subscriptions(subs)) => {
                    debug_assert_eq!(fetch, Fetch::Subscriptions, "spawn_fetch must pair FetchData::Subscriptions only with Fetch::Subscriptions");
                    let changed = self.store.apply_subscriptions(issued, subs);
                    if let Some(changed) = &changed {
                        self.notify_subscriptions_changed(changed);
                        self.notify_store_changed();
                        // 同上——SSE 触发的单独订阅列表刷新同样要核对选择器弹窗。
                        self.close_picker_if_subscription_vanished();
                    }
                    Vec::new()
                }
                // 虚拟模型列表刷新: 只进 Store, 不需要转给任何页面的 update ——虚拟模型页
                // 画的时候直接读 ctx.store, 与订阅列表同一套约定。
                Ok(FetchData::VirtualModels(vms)) => {
                    debug_assert_eq!(fetch, Fetch::VirtualModels, "spawn_fetch must pair FetchData::VirtualModels only with Fetch::VirtualModels");
                    let accepted = self.store.apply_virtual_models(issued, vms);
                    if accepted {
                        // 虚拟模型页没有 `on_subscriptions_changed` 那条早通知路径 (那是给订阅
                        // 列表用的), 这份广播是它唯一能立刻核对草稿的机会——不然要等下一次真正的
                        // `Component::update()` 调用 (最多 5 秒轮询) 才会发现"新列表其实已经和
                        // 草稿相等了"。
                        self.notify_store_changed();
                    }
                    Vec::new()
                }
                // 日志页自己的分页 / 过滤状态——不进 `Store`, 加载结果原样转给发起它的
                // 页面 (哪怕它此刻不可见: 用户翻页/改过滤之后立刻切走是常见操作)。
                Ok(FetchData::Requests(page)) => {
                    debug_assert_eq!(fetch.kind(), FetchKind::Requests, "spawn_fetch must pair FetchData::Requests only with Fetch::Requests");
                    let action = Action::FetchDone { fetch, issued, result: Ok(FetchData::Requests(page)) };
                    self.update_page(Tab::Logs, &action)
                }
            },
            Action::Refresh | Action::Poll => self.update_page(self.tab, &action),
            Action::Sse { ref name, ref data, at_ms } => {
                // 先广播给所有页面 (含不可见的), 再照旧转给可见页的 update——广播只能改页面自己的
                // 状态, 能产出 Cmd 的反应仍然只走后面这一步。
                self.pages.for_each_mut(|page| page.on_event(StreamEvent::Message { name, data, at_ms }));
                self.update_page(self.tab, &action)
            }
            Action::Mutate(m) => self.start_mutation(m),
            Action::MutationDone { mutation, barrier, result } => self.finish_mutation(mutation, barrier, result),
            Action::OpenPicker(spec) => {
                self.open_popup(Popup::Picker(PickerState::new(spec)));
                Vec::new()
            }
            Action::PickerDone { tag, choice } => {
                self.close_popup();
                let action = Action::PickerDone { tag, choice };
                // 有向导时交给向导 (选厂商 / 选模型等弹窗是它开的); 没有向导时交给当前页面
                // (改槽位 / 改虚拟模型成员)。
                if self.wizard.is_some() {
                    self.update_wizard(&action)
                } else {
                    self.update_page(self.tab, &action)
                }
            }
            Action::OpenDetail(spec) => {
                self.open_popup(Popup::Detail(DetailState::new(spec)));
                Vec::new()
            }
            Action::Notify { kind, text } => {
                self.push_toast(Toast::new(kind, text));
                Vec::new()
            }
            Action::OpenLogsFor { subscription_id } => {
                self.pages.logs.show_subscription(&subscription_id);
                if self.tab == Tab::Logs {
                    self.update_page(Tab::Logs, &Action::Refresh)
                } else {
                    // 切页本身会 (可见页变了 + 已连接) 补一次 `Refresh`, 这次会发出
                    // `show_subscription` 刚设好的新查询——不需要在这里再手动拉一次。
                    self.update(Action::SwitchTab(Tab::Logs))
                }
            }
        }
    }

    fn draw_tabs(&self, frame: &mut Frame, area: Rect) {
        let status = match self.conn {
            Conn::Connected => Line::from(vec![
                Span::styled(" ● ", Style::new().fg(self.theme.ok)),
                Span::raw(self.s.conn_connected),
                Span::styled(
                    format!(" · v{} ", self.app_version.as_deref().unwrap_or("?")),
                    self.theme.muted_style(),
                ),
            ]),
            Conn::Connecting | Conn::Reconnecting => {
                let state = spinner_state(self.tick);
                let spinner = Throbber::default().throbber_set(BRAILLE_SIX).to_symbol_span(&state);
                let text = if self.conn == Conn::Connecting { self.s.conn_connecting } else { self.s.conn_reconnecting };
                Line::from(vec![Span::raw(" "), spinner, Span::raw(format!("{text} "))]).style(Style::new().fg(self.theme.warn))
            }
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.theme.border_style())
            .title_top(Line::styled(" cc-router ", self.theme.accent_bold()))
            .title_top(status.right_aligned());
        let titles = self.s.tabs.iter().enumerate().map(|(i, name)| format!(" {} {name} ", i + 1));
        let tabs = Tabs::new(titles)
            .select(self.tab.index())
            .style(self.theme.muted_style())
            .highlight_style(self.theme.accent_bold().add_modifier(Modifier::REVERSED))
            .divider(" ")
            .padding("", "")
            .block(block);
        frame.render_widget(tabs, area);
    }

    fn draw_toast(&mut self, frame: &mut Frame, screen: Rect, top: u16) {
        let now = self.now_ms;
        let Some(toast) = self.toasts.front_mut() else { return };
        let area = toast::area(screen, top, &toast.text);
        match toast.shown_at {
            None => {
                toast.shown_at = Some(now);
                self.fx.toast_in(area, self.theme.border);
            }
            Some(at) if !toast.fading && now - at >= toast::LIFETIME_MS - i64::from(fx::ms::TOAST_OUT) => {
                toast.fading = true;
                self.fx.toast_out(area);
            }
            Some(_) => {}
        }
        // 消散效果播完之后不再画它: 它要等下一次 `Action::Tick` (可能晚 200ms 才到) 才会被真正
        // 弹出队列, 这段空隙里任何重画 (按键 / SSE 事件) 都不能让它以全亮度闪回 (H2)。
        if toast.fading && self.fx.toast_out_finished() {
            return;
        }
        toast::draw(frame, area, toast, &self.theme);
    }

    /// `elapsed`: 距上一帧的时间, 用来推进动效。
    pub fn draw(&mut self, frame: &mut Frame, elapsed: Duration) {
        let screen = frame.area();
        if screen.width < MIN_WIDTH || screen.height < MIN_HEIGHT {
            let line = Line::raw(self.s.too_small).centered();
            frame.render_widget(line, screen.centered_vertically(Constraint::Length(1)));
            // 太小画不出可以叠动效的内容; 不清掉的话 is_running() 会一直为真, 把主循环锁在 60fps。
            self.fx.clear();
            return;
        }
        // 空闲时两帧之间可能隔了几百毫秒; 动效是这一帧才加进来的话, 不能把这段空闲算成它已经播过的时间。
        let was_running = self.fx.is_running();

        let banner = self.banner_lines(screen.width).unwrap_or_default();
        let [tabs, banner_area, content, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(banner.len() as u16),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(screen);

        self.draw_tabs(frame, tabs);
        if !banner.is_empty() {
            let lines: Vec<Line> = banner.into_iter().map(Line::raw).collect();
            frame.render_widget(ratatui::widgets::Paragraph::new(lines).style(Style::new().fg(self.theme.warn)), banner_area);
        }

        let s = self.s;
        let mut ctx = DrawCtx {
            theme: &self.theme,
            s,
            now_ms: self.now_ms,
            tick: self.tick,
            fx: &mut self.fx,
            store: &self.store,
            busy: &self.busy,
            last_outcome: &self.last_outcome,
            tz: self.tz,
        };
        // 有向导时内容区整个归它 (先 `Clear` 再画, 不叠在页面上面); 底栏左侧换成向导自己的键位,
        // 右侧固定只剩 `Esc` (`?` 帮助 / `q` 退出在向导里按不出来, 继续提示会误导)——
        // **除非落库请求正在飞** (`can_cancel()` 为假): 那种情况下 `Esc` 也按不出反应,
        // 继续显示这条提示就是纯误导, 右侧整个留空。`self.popup.is_some()` 在调 `wizard.draw`
        // 之前先取出来: 向导据此决定要不要设终端光标——有弹窗叠在上面时它自己的输入框
        // 不该再显示光标, 交给弹窗决定 (picker 会设、confirm/help 不设即隐藏)。
        let popup_open = self.popup.is_some();
        let (left, right): (Vec<Hint>, Vec<Hint>) = if let Some(wizard) = &mut self.wizard {
            frame.render_widget(Clear, content);
            wizard.draw(frame, content, &mut ctx, popup_open);
            let right = if wizard.can_cancel() { vec![("Esc", s.key_cancel)] } else { Vec::new() };
            (wizard.hints(s), right)
        } else {
            let page = self.pages.get_mut(self.tab);
            page.draw(frame, content, &mut ctx);
            let mut left = page.hints(s);
            left.insert(0, ("1-5", s.key_switch_tab));
            (left, vec![("?", s.key_help), ("q", s.key_quit)])
        };
        keybar::draw(frame, footer, &left, &right, &self.theme);

        self.draw_toast(frame, screen, content.y);

        // 弹窗打开时才用得到, 但取值本身不依赖弹窗状态 (纯查表), 先取出来免得在下面的可变借用里
        // 再跟 `self.pages` 打交道 (按键 / 画 / 尺寸计算都在 `impl Popup` 里, `App` 这里只负责备好
        // `PopupCtx` 需要的环境)。
        let page_help = self.pages.get(self.tab).help(s);
        if let Some(popup) = &mut self.popup {
            // 压暗背景用静态的 DIM 修饰符而不是动效: 16 色 / 无色终端下同样成立。
            frame.buffer_mut().set_style(screen, Style::new().add_modifier(Modifier::DIM));
            let ctx = PopupCtx { theme: &self.theme, s, page_help, tz: self.tz };
            let area = popup.area(screen, &ctx);
            popup.draw(frame, area, &ctx);
            self.popup_area = Some(area);
        }

        if !self.startup_played {
            self.startup_played = true;
            self.fx.startup(self.pages.overview.logo_area().unwrap_or_default(), screen, self.theme.border);
        }
        if let Some(dir) = self.pending_page_fx.take() {
            self.fx.page_enter(dir, content, self.theme.border);
        }
        match self.pending_popup_fx.take() {
            Some(PopupFx::Open) => self.fx.popup_open(self.popup_area.unwrap_or_default(), self.theme.border),
            Some(PopupFx::Close(area)) => self.fx.popup_close(area),
            None => {}
        }
        let dt = if was_running { elapsed } else { Duration::ZERO };
        self.fx.process(dt, frame.buffer_mut(), screen);
    }
}

// 「未保存修改 → 先确认」流程靠 `pages::logs::Logs` 的 `#[cfg(test)]` 专用钩子 (`set_force_dirty`)
// 验证——那个钩子只在编译本 crate 的单测时存在 (原样从已删除的 `pages::placeholder::Placeholder`
// 搬过来, 见 `logs.rs` 顶部的注释), `tests/ui.rs` 这样的集成测试够不到它, 所以这批用例必须留在
// 这个 `mod tests` 里, 不能挪到 `tests/ui.rs`。不需要这个钩子的弹窗测试 (帮助弹窗按键、确认弹窗
// 渲染、帮助高度守卫、快照) 仍然放在 `tests/ui.rs`, 跟其它界面测试一起维护。
/// 给向导产出的请求盖上发出它的向导实例的代次。
fn stamp(epoch: u64, cmds: Vec<WizardCmd>) -> Vec<Cmd> {
    cmds.into_iter().map(|cmd| Cmd::Wizard { epoch, cmd: Box::new(cmd) }).collect()
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::action::{Tab, WizardCmd, WizardResult};
    use crate::client::events::{ROUTE_ATTEMPT_STARTED, SUBSCRIPTION_STATE_CHANGED};
    use crate::i18n::ZH;
    use crate::widgets::detail::{DetailRow, DetailSpec, Tone};
    use crate::widgets::picker;

    const NOW: i64 = 1_700_000_000_000;
    const VERSION: &str = "9.9.9";

    fn app() -> App {
        App::new(AppOptions {
            strings: &ZH,
            theme: Theme::new(crate::theme::ColorMode::TrueColor),
            fx_enabled: false,
            now_ms: NOW,
            tui_version: VERSION,
            tz: Tz::Fixed(8 * 3600),
        })
    }

    /// 切到日志页 (`Tab::Logs`) 并把它标记成 dirty——日志页本身是纯只读页面, 永远不会真的变脏,
    /// `set_force_dirty` 是它从已删除的占位页原样带过来的测试专用钩子, 专门给这批「未保存修改 →
    /// 先确认」流程的用例用。
    fn dirty_app() -> App {
        let mut a = app();
        a.update(Action::SwitchTab(Tab::Logs));
        a.pages.logs.set_force_dirty(true);
        a
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// 带着当前向导代次的结果——「这个向导自己发的请求回来了」。
    fn current_done(a: &App, result: WizardResult) -> Action {
        Action::WizardDone { epoch: a.wizard_epoch().expect("准备: 向导应该开着"), result: Box::new(result) }
    }

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn render(a: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| a.draw(f, Duration::ZERO)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn ctrl_c_quits_even_when_dirty_or_with_a_popup_open() {
        let mut a = dirty_app();
        assert_eq!(a.handle_key(ctrl_c()), Some(Action::ForceQuit), "dirty 页面上 Ctrl+C 也不该问");
        assert_eq!(a.update(Action::ForceQuit), vec![Cmd::Quit]);

        // 弹窗打开时 (比如已经在问「要不要放弃」) Ctrl+C 依然立即退出, 不再多问一次。
        let mut b = dirty_app();
        b.update(Action::Quit);
        assert!(b.popup.is_some(), "上一步应该已经弹出确认弹窗");
        assert_eq!(b.handle_key(ctrl_c()), Some(Action::ForceQuit));
        assert_eq!(b.update(Action::ForceQuit), vec![Cmd::Quit]);
    }

    #[test]
    fn q_on_a_dirty_page_asks_first() {
        let mut a = dirty_app();
        assert_eq!(a.handle_key(key(KeyCode::Char('q'))), Some(Action::Quit));
        assert!(a.update(Action::Quit).is_empty(), "dirty 页面上 q 不该立即退出");
        let out = render(&mut a, 80, 24);
        assert!(out.contains(ZH.confirm_discard), "应该弹出确认提示\n{out}");

        assert_eq!(a.handle_key(key(KeyCode::Char('y'))), Some(Action::Confirmed(OnYes::discard_then(Action::Quit))));
        assert_eq!(a.update(Action::Confirmed(OnYes::discard_then(Action::Quit))), vec![Cmd::Quit], "y 之后应该真的退出");

        // 另起一局: n 应该只关掉弹窗, 页面仍然 dirty (草稿没被丢弃, 也没有退出)。
        let mut b = dirty_app();
        b.update(Action::Quit);
        assert_eq!(b.handle_key(key(KeyCode::Char('n'))), Some(Action::ClosePopup));
        assert!(b.update(Action::ClosePopup).is_empty());
        assert!(b.popup.is_none(), "n 应该关掉弹窗");
        assert!(b.pages.get(b.tab).is_dirty(), "n 不该丢弃草稿");
    }

    #[test]
    fn switching_tabs_on_a_dirty_page_asks_first_and_yes_discards() {
        let mut a = dirty_app();
        assert!(a.update(Action::SwitchTab(Tab::Overview)).is_empty(), "dirty 时切页应该先确认");
        assert_eq!(a.tab, Tab::Logs, "确认之前不该真的切走");
        assert!(a.pages.logs.is_dirty());

        a.update(Action::Confirmed(OnYes::discard_then(Action::SwitchTab(Tab::Overview))));
        assert_eq!(a.tab, Tab::Overview, "y 之后应该真的切过去");
        assert!(!a.pages.logs.is_dirty(), "y 之后原页面的草稿应该被丢弃");
    }

    /// `NextTab` / `PrevTab` 与 `Quit` / `SwitchTab` 走同一个 `guard_dirty`, 各自单独断言一遍。
    #[test]
    fn next_tab_and_prev_tab_on_a_dirty_page_ask_first_and_yes_discards() {
        let mut a = dirty_app(); // tab = Logs (index 4)
        assert!(a.update(Action::NextTab).is_empty(), "dirty 时 NextTab 应该先确认");
        assert_eq!(a.tab, Tab::Logs, "确认之前不该真的切走");
        a.update(Action::Confirmed(OnYes::discard_then(Action::NextTab)));
        assert_eq!(a.tab, Tab::Overview, "y 之后应该真的切到下一页 (末尾绕回开头)");

        let mut b = dirty_app();
        assert!(b.update(Action::PrevTab).is_empty(), "dirty 时 PrevTab 应该先确认");
        assert_eq!(b.tab, Tab::Logs);
        b.update(Action::Confirmed(OnYes::discard_then(Action::PrevTab)));
        assert_eq!(b.tab, Tab::Live, "y 之后应该真的切到上一页");
    }

    /// `Action::DiscardDraft` (页面自己按 Esc 放弃编辑时用) 应该直接调用当前页面的
    /// `discard_changes()`, 不产出任何 `Cmd`, 也不需要经过确认弹窗。
    #[test]
    fn discard_draft_clears_the_current_pages_dirty_flag_without_a_cmd() {
        let mut a = dirty_app();
        assert!(a.pages.logs.is_dirty());
        assert!(a.update(Action::DiscardDraft).is_empty(), "不应该产出任何 Cmd");
        assert!(!a.pages.logs.is_dirty(), "草稿应该被丢弃");
    }

    /// 打开一个新弹窗 (确认 / picker) 时, 如果已经有另一个弹窗开着, 应该直接替换它,
    /// 并且清掉旧弹窗的 `popup_area`——不清的话, 如果新弹窗在第一次 `draw()` 之前就被关掉, 关闭
    /// 动效会拿旧弹窗的几何去播。
    #[test]
    fn opening_a_popup_while_another_is_open_replaces_it_and_resets_popup_area() {
        let mut a = app();
        a.update(Action::ToggleHelp);
        assert!(matches!(a.popup, Some(Popup::Help)));
        render(&mut a, 80, 24); // 让 App 记住 Help 弹窗的 popup_area。
        assert!(a.popup_area.is_some());

        a.update(Action::OpenConfirm { prompt: "测试".into(), on_yes: OnYes::discard_then(Action::Refresh) });
        assert!(matches!(a.popup, Some(Popup::Confirm(_))), "应该直接替换成确认弹窗");
        assert!(a.popup_area.is_none(), "替换时应该清掉旧弹窗的 popup_area, 不留到下一次关闭时误用");

        render(&mut a, 80, 24); // 再记一次, 这次是 Confirm 弹窗的 popup_area。
        assert!(a.popup_area.is_some());
        a.update(Action::OpenPicker(picker::PickerSpec {
            tag: picker::PickerTag::VmAddSubscription { vm: "model-fable".into() },
            title: "选择".into(),
            items: vec![],
            allow_custom: false,
            initial: String::new(),
        }));
        assert!(matches!(a.popup, Some(Popup::Picker(_))), "应该直接替换成 picker 弹窗");
        assert!(a.popup_area.is_none(), "同样应该清掉 Confirm 弹窗的 popup_area");

        // `Action::OpenDetail` 应该遵守同一条「替换掉已经打开的弹窗」规则。
        render(&mut a, 80, 24); // 记一次 picker 弹窗的 popup_area。
        assert!(a.popup_area.is_some());
        a.update(Action::OpenDetail(DetailSpec {
            title: "详情".into(),
            rows: vec![DetailRow::Field { label: "a".into(), value: "b".into(), tone: Tone::Normal }],
        }));
        assert!(matches!(a.popup, Some(Popup::Detail(_))), "应该直接替换成详情弹窗");
        assert!(a.popup_area.is_none(), "同样应该清掉 picker 弹窗的 popup_area");
    }

    /// 事件流上的消息与断线要广播给**所有**页面 (含不可见的), 但只有可见页才产出 `Cmd`。
    ///
    /// 「广播到不可见页」靠实时路由页的 `Live::entry_count` (`#[cfg(test)]` 钩子) 验证: 总览页可见时
    /// 发一条 `route_attempt_started`, 总览页不消费它 (不
    /// 产出 `Cmd`), 但不可见的实时路由页应该已经把这次尝试记进了自己的缓冲。
    #[test]
    fn stream_events_reach_hidden_pages_but_only_the_visible_page_returns_cmds() {
        let mut a = app();
        a.update(Action::Connected { app_version: VERSION.into() });
        assert_eq!(a.tab, Tab::Overview, "准备: 当前标签是总览");

        // 1) 总览可见: 订阅相关的事件应该让它产出一次 Fetch::Subscriptions。
        let cmds = a.update(Action::Sse { name: SUBSCRIPTION_STATE_CHANGED.into(), data: "\"1\"".into(), at_ms: NOW });
        assert_eq!(cmds, vec![Cmd::Fetch(Fetch::Subscriptions)]);

        // 2) 一条路由尝试事件在总览页可见时不产出任何 Cmd (总览页不消费它), 但广播仍然发生——
        //    不可见的实时路由页应该已经记下这次尝试。
        let route_data = r#"{"subscription_id":"1","virtual_model":"model-sonnet"}"#;
        let cmds = a.update(Action::Sse { name: ROUTE_ATTEMPT_STARTED.into(), data: route_data.into(), at_ms: NOW });
        assert!(cmds.is_empty(), "总览页可见时这条 SSE 不该产出 Cmd");
        assert_eq!(a.pages.live.entry_count(), 1, "实时路由页不可见也该记下这次尝试");

        // 3) 切到实时路由页 (真页面): 切页本身触发一次 Refresh, 返回它自己的两个 Fetch。
        let switch_cmds = a.update(Action::SwitchTab(Tab::Live));
        assert_eq!(
            switch_cmds,
            vec![Cmd::Fetch(Fetch::Subscriptions), Cmd::Fetch(Fetch::VirtualModels)],
            "实时路由页切入时应该补拉订阅与虚拟模型"
        );

        // 4) 同一个订阅事件在实时路由页可见时不该再产出任何 Cmd (它只在 Refresh/Poll/Connected
        //    时拉数据, 不消费这类 SSE)。
        let cmds = a.update(Action::Sse { name: SUBSCRIPTION_STATE_CHANGED.into(), data: "\"1\"".into(), at_ms: NOW });
        assert!(cmds.is_empty(), "实时路由页可见时这条 SSE 不该产出 Cmd");

        // 5) 断线: 广播 Lost——实时路由页应该把那条 Pending 尝试标成中断并插入一条分隔行
        //    (entry_count 从 1 变成 2)。
        a.update(Action::ConnectionLost);
        assert_eq!(a.pages.live.entry_count(), 2, "断线广播应该到达实时路由页, 插入一条分隔行");
    }

    /// `data` 为 `None` 时日志页一律显示加载中 (不看 `loading`, 见
    /// `logs.rs::draw_table` 的注释——断线/重连期间 `loading` 本来就一直是 false, 不能因此误报
    /// 「没有记录」), 所以「一次失败的加载真的停掉了 loading 标记」这件事在渲染文本层面已经看不出
    /// 来了, 只能靠 `Logs::is_loading()` 这个测试专用钩子直接读 (与 `set_force_dirty` 同一套「只在
    /// 编译本 crate 单测时存在」的做法)。toast 与无 `Cmd` 两件事仍然留在 `tests/ui.rs` 里靠渲染 /
    /// 返回值断言 (`a_failed_requests_fetch_toasts_and_returns_no_cmd`)。
    #[test]
    fn a_failed_requests_fetch_clears_the_logs_pages_loading_flag() {
        use crate::client::dto::RequestQuery;

        let mut a = app();
        a.update(Action::Connected { app_version: VERSION.into() });
        a.update(Action::SwitchTab(Tab::Logs));
        assert!(a.pages.logs.is_loading(), "切进日志页应该立刻开始加载第 1 页");

        a.update(Action::FetchDone { fetch: Fetch::Requests(RequestQuery::default()), issued: 1, result: Err("boom".into()) });
        assert!(!a.pages.logs.is_loading(), "失败的加载应该停掉「加载中」标记");
    }

    // ---------- 向导这一层的路由 ----------

    /// 向导打开后, `q` / `r` / `1` / `Tab` 这些平时会被当成全局键的按键应该被向导吞掉, 到不了
    /// `match key.code` 那一段——所以它们绝不会产出 `Quit`/`Refresh`/`SwitchTab`/`NextTab`。
    #[test]
    fn the_wizard_swallows_global_keys() {
        let mut a = app();
        a.update(Action::OpenWizard);
        for code in [KeyCode::Char('q'), KeyCode::Char('r'), KeyCode::Char('1'), KeyCode::Tab] {
            let action = a.handle_key(key(code));
            assert!(
                !matches!(action, Some(Action::Quit) | Some(Action::Refresh) | Some(Action::SwitchTab(_)) | Some(Action::NextTab)),
                "{code:?} 不该被当成全局键: {action:?}"
            );
        }
    }

    #[test]
    fn ctrl_c_still_force_quits_with_the_wizard_open() {
        let mut a = app();
        a.update(Action::OpenWizard);
        assert_eq!(a.handle_key(ctrl_c()), Some(Action::ForceQuit), "向导打开时 Ctrl+C 也不该问");
        assert_eq!(a.update(Action::ForceQuit), vec![Cmd::Quit]);
    }

    /// `current_dirty()` 把「向导存在」也算作有未保存的编辑上下文——切页 / 退出都要先问, 与页面
    /// 草稿走的是同一条 `guard_dirty`。`Confirmed` 之后 `discard_current()` 关掉向导并补一次订阅
    /// 列表重拉, 再执行被包住的原始 action。
    #[test]
    fn switching_tabs_or_quitting_with_the_wizard_open_asks_first() {
        let mut a = app();
        a.update(Action::OpenWizard);
        assert!(a.update(Action::SwitchTab(Tab::Subscriptions)).is_empty(), "向导打开时切页应该先确认");
        assert_eq!(a.tab, Tab::Overview, "确认之前不该真的切走");
        assert!(a.wizard.is_some(), "确认之前向导不该被关掉");

        let cmds = a.update(Action::Confirmed(OnYes::discard_then(Action::SwitchTab(Tab::Subscriptions))));
        assert_eq!(a.tab, Tab::Subscriptions, "y 之后应该真的切过去");
        assert!(a.wizard.is_none(), "y 之后向导应该被关掉");
        assert!(cmds.contains(&Cmd::Fetch(Fetch::Subscriptions)), "关向导应该补一次订阅列表重拉: {cmds:?}");

        // 退出: 键盘层面 `q` 已经在 `the_wizard_swallows_global_keys` 里确认到不了这里 (向导整个
        // 吞掉), 这里直接喂 `Action::Quit` 单独测 `update()` 自己的守卫, 与
        // `q_on_a_dirty_page_asks_first` 同一测法。
        let mut b = app();
        b.update(Action::OpenWizard);
        assert!(b.update(Action::Quit).is_empty(), "向导打开时 Quit 应该先确认");
        assert!(b.wizard.is_some(), "确认之前向导不该被关掉");

        assert_eq!(
            b.update(Action::Confirmed(OnYes::discard_then(Action::Quit))),
            vec![Cmd::Fetch(Fetch::Subscriptions), Cmd::Quit],
            "y 之后应该先补一次订阅列表重拉、再真的退出"
        );
        assert!(b.wizard.is_none());
    }

    #[test]
    fn closing_the_wizard_refetches_subscriptions() {
        let mut a = app();
        a.update(Action::OpenWizard);
        assert!(a.wizard.is_some());
        assert_eq!(a.update(Action::CloseWizard), vec![Cmd::Fetch(Fetch::Subscriptions)]);
        assert!(a.wizard.is_none());
    }

    #[test]
    fn a_wizard_result_arriving_after_the_wizard_closed_is_dropped() {
        let mut a = app();
        assert!(a.wizard.is_none(), "准备: 没有打开过向导");
        let result = WizardResult::Providers(Ok(vec![]));
        let done = Action::WizardDone { epoch: a.wizard_epoch, result: Box::new(result) };
        assert!(a.update(done).is_empty(), "没有向导时应该直接丢弃, 不 panic");
    }

    /// 没有向导时 `Action::WizardRequest` 不该被转成 `Cmd::Wizard`——不像
    /// `WizardDone`(落地时向导已经不在也无害, "没有向导就丢弃" 这条路径正好接住), `WizardRequest`
    /// 背后是 `create_subscription` 这类会写库的请求, 真发出去就会在没人接收结果的情况下把订阅
    /// 建到后端。`WizardCmd::LoadProviders` 不带负载, 用它当占位验证"没有向导就不转发"这条纯粹的
    /// 判断逻辑, 具体是哪个 `WizardCmd` 变体不重要。
    #[test]
    fn wizard_request_without_a_wizard_produces_nothing() {
        let mut a = app();
        assert!(a.wizard.is_none(), "准备: 没有打开过向导");
        let cmds = a.update(Action::WizardRequest(Box::new(WizardCmd::LoadProviders)));
        assert!(cmds.is_empty(), "没有向导时不该产出任何 Cmd: {cmds:?}");
    }

    #[test]
    fn opening_the_wizard_loads_providers() {
        let mut a = app();
        assert_eq!(a.update(Action::OpenWizard), vec![Cmd::Wizard { epoch: 1, cmd: Box::new(WizardCmd::LoadProviders) }]);
        assert!(a.wizard.is_some());
        a.update(Action::CloseWizard);
        assert_eq!(
            a.update(Action::OpenWizard),
            vec![Cmd::Wizard { epoch: 2, cmd: Box::new(WizardCmd::LoadProviders) }],
            "再开一个向导应该换一个新代次"
        );
    }

    /// `App::update_wizard` 转发 `take_close_request()` 的那条分支: `close_request` 只在向导处理
    /// 结果时被置位, 看着像没人写的死字段, 容易被当成死分支删掉 (`wizard/mod.rs` 自己的单测只测了
    /// `Wizard` 本身)。用 `request_close_for_test` 从外面戳一下, 确认 `WizardDone` 落地时
    /// `App` 真的会关掉向导、补一次订阅列表重拉。
    #[test]
    fn a_wizards_close_request_is_forwarded_and_refetches_subscriptions() {
        let mut a = app();
        a.update(Action::OpenWizard);
        a.wizard.as_mut().expect("刚打开").request_close_for_test();

        let cmds = a.update(current_done(&a, WizardResult::Providers(Ok(vec![]))));
        assert_eq!(cmds, vec![Cmd::Fetch(Fetch::Subscriptions)]);
        assert!(a.wizard.is_none(), "close_request 应该真的把向导关掉");
    }

    /// 同上, 补 `take_notice()` 那条分支: 向导存了一条待发通知, `App` 应该真的把它变成一条 toast。
    #[test]
    fn a_wizards_notice_becomes_a_toast() {
        let mut a = app();
        a.update(Action::OpenWizard);
        a.wizard.as_mut().expect("刚打开").request_notice_for_test(ToastKind::Error, "测试通知");

        a.update(current_done(&a, WizardResult::Providers(Ok(vec![]))));
        let out = render(&mut a, 80, 24);
        assert!(out.contains("测试通知"), "{out}");
    }

    /// 「弹窗排在向导前面」这条按键优先级: 直接喂 `Action::Confirmed` 测不到 `App::handle_key` 里
    /// 弹窗判断与向导判断的先后, 所以这里真的走一遍 `handle_key`: 向导打开时触发一次确认
    /// 弹窗 (切页), 确认「y」这个按键仍然落在弹窗上而不是被向导吞掉。
    #[test]
    fn a_confirm_popup_still_wins_over_the_wizard() {
        let mut a = app();
        a.update(Action::OpenWizard);
        assert!(a.update(Action::SwitchTab(Tab::Subscriptions)).is_empty(), "准备: 应该弹出确认框");
        assert!(a.popup.is_some(), "准备: 确认框应该已经打开");
        assert!(a.wizard.is_some(), "准备: 向导仍然存在 (还没确认)");

        let action = a.handle_key(key(KeyCode::Char('y')));
        assert!(matches!(action, Some(Action::Confirmed(_))), "弹窗应该排在向导前面, 按键不该被向导吞掉: {action:?}");
    }

    /// 带当前代次的结果照常交给向导; 同一个结果换成旧代次就被整个丢掉——`Providers` 在 `Loading`
    /// 阶段本来一定会被采纳, 所以「没被采纳」只能是代次比较挡住的。
    #[test]
    fn only_results_carrying_the_current_wizard_epoch_reach_the_wizard() {
        let mut a = app();
        let opened = a.update(Action::OpenWizard);
        let Some(Cmd::Wizard { epoch: old, .. }) = opened.first().cloned() else { panic!("应该发出拉厂商列表的请求: {opened:?}") };
        a.update(Action::CloseWizard);
        a.update(Action::OpenWizard);
        assert_ne!(a.wizard_epoch(), Some(old), "准备: 重开的向导应该换了代次");

        let stale = Action::WizardDone { epoch: old, result: Box::new(WizardResult::Providers(Err("旧向导的结果".into()))) };
        assert!(a.update(stale).is_empty());
        let out = render(&mut a, 80, 24);
        assert!(out.contains(ZH.wiz_loading_providers), "旧代次的结果不该被采纳, 新向导应该还在加载\n{out}");

        a.update(current_done(&a, WizardResult::Providers(Err("新向导的结果".into()))));
        let out = render(&mut a, 80, 24);
        assert!(out.contains("新向导的结果"), "当前代次的结果应该照常被采纳\n{out}");
    }
}
