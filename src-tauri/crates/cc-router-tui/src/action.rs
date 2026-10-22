//! 单向数据流的两种消息: [`Action`] 进 (按键 / 定时 / 网络结果), [`Cmd`] 出 (要主循环去做的副作用)。
//! `App::update` 是 `(状态, Action) → (新状态, Vec<Cmd>)` 的同步函数, 不碰网络也不碰终端, 所以能直接单测。

use crate::client::dto::{
    CreateInput, CreatedSubscription, ModelSlots, OverallStats, ProbeInput, ProbeModelsResult, Provider, ProxyStatus, RefreshBalanceResult,
    RefreshModelsResult, RequestPage, RequestQuery, RoutingMode, SeriesPoint, Settings, SlotEfforts, Subscription, TestConnectionResult,
    VirtualModel,
};
use crate::widgets::detail::DetailSpec;
use crate::widgets::picker::{PickerChoice, PickerSpec, PickerTag};
use crate::widgets::toast::ToastKind;

/// 五个标签页, 顺序即 `1`–`5` 与 `Strings::tabs` 的下标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Overview,
    Subscriptions,
    VirtualModels,
    Live,
    Logs,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Overview, Tab::Subscriptions, Tab::VirtualModels, Tab::Live, Tab::Logs];

    /// 0 起算的下标, 与 `Strings::tabs` 对齐。
    pub fn index(self) -> usize {
        match self {
            Tab::Overview => 0,
            Tab::Subscriptions => 1,
            Tab::VirtualModels => 2,
            Tab::Live => 3,
            Tab::Logs => 4,
        }
    }

    pub fn from_index(i: usize) -> Option<Tab> {
        Tab::ALL.get(i).copied()
    }

    /// 末尾绕回开头。
    pub fn next(self) -> Tab {
        Tab::ALL[(self.index() + 1) % Tab::ALL.len()]
    }

    /// 开头绕回末尾。
    pub fn prev(self) -> Tab {
        let n = Tab::ALL.len();
        Tab::ALL[(self.index() + n - 1) % n]
    }
}

/// 总览页一次整页加载的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct OverviewData {
    pub status: ProxyStatus,
    pub settings: Settings,
    pub stats: OverallStats,
    pub series: Vec<SeriesPoint>,
    pub subscriptions: Vec<Subscription>,
}

/// [`Fetch`] 去重用的键。`Fetch::Requests` 带查询参数, 去重按种类而不是按
/// 整个值——两个页码不同的 `Requests` 仍然是「同一种」加载, 只应该让最新那次真的发出去。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchKind {
    Overview,
    Subscriptions,
    VirtualModels,
    Requests,
}

/// 可去重、可补跑的加载。不是 `Copy`: `Requests` 带着查询参数。去重按 [`FetchKind`], 不按参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetch {
    Overview,
    Subscriptions,
    VirtualModels,
    Requests(RequestQuery),
}

impl Fetch {
    pub fn kind(&self) -> FetchKind {
        match self {
            Fetch::Overview => FetchKind::Overview,
            Fetch::Subscriptions => FetchKind::Subscriptions,
            Fetch::VirtualModels => FetchKind::VirtualModels,
            Fetch::Requests(_) => FetchKind::Requests,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FetchData {
    Overview(Box<OverviewData>),
    Subscriptions(Vec<Subscription>),
    VirtualModels(Vec<VirtualModel>),
    Requests(RequestPage),
}

/// 订阅页/虚拟模型页的就地操作。**永不去重、永不补跑** (与 [`Fetch`] 相反): `runtime.rs` 对每一个
/// `Cmd::Mutate` 都直接 `spawn`, 不经过 `Fetches`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Mutation {
    SetEnabled { id: String, enabled: bool },
    TestConnection { id: String },
    RefreshModels { id: String },
    RefreshBalance { id: String },
    /// 保存一条订阅的模型槽位 + 槽位 effort。两块都整块替换。
    UpdateSlots { id: String, model_slots: ModelSlots, slot_efforts: SlotEfforts },
    /// 保存一个虚拟模型的调度模式 + 订阅列表。
    UpdateVirtualModel { name: String, mode: RoutingMode, subscription_ids: Vec<String> },
    /// 删除一条订阅。`delete_subscription` 不拒绝被虚拟模型引用的订阅, 静默把它从
    /// 每个虚拟模型的 `subscription_ids` 里摘掉——「列出引用方」只能由 TUI 在删之前从
    /// `Subscription.referenced_by` 读, 见 `pages/subscriptions.rs` 的确认文案拼接。
    Delete { id: String },
}

/// 忙碌表 (`App::busy`) 判重用的键。订阅相关的就地操作 (含 `UpdateSlots`) 产生 `Subscription`
/// 变体; 虚拟模型页的编辑操作 (`UpdateVirtualModel`) 产生 `VirtualModel` 变体。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BusyKey {
    Subscription(String),
    VirtualModel(String),
}

impl Mutation {
    /// 取代旧的 `subscription_id()`: 旧函数假设「所有变更都作用于某条订阅」, 虚拟模型页的编辑
    /// 操作不满足这个假设, 需要一个能区分「这次变更判重键属于哪一类」的类型。
    pub fn busy_key(&self) -> BusyKey {
        match self {
            Mutation::SetEnabled { id, .. }
            | Mutation::TestConnection { id }
            | Mutation::RefreshModels { id }
            | Mutation::RefreshBalance { id }
            | Mutation::UpdateSlots { id, .. }
            | Mutation::Delete { id } => BusyKey::Subscription(id.clone()),
            Mutation::UpdateVirtualModel { name, .. } => BusyKey::VirtualModel(name.clone()),
        }
    }

    /// 这次变更完成后该重新拉取哪些加载。`UpdateVirtualModel` 额外影响虚拟模型列表本身 (顺序:
    /// 先虚拟模型后订阅, 与 `runtime.rs`/`tests/ui.rs` 断言的顺序一致); 其余都只影响订阅列表。
    ///
    /// `Fetch` 不是 `Copy` (`Requests` 带 `String` 参数), 但数组字面量 `&[...]` 里只出现不带参数的
    /// 变体, 编译器仍然把它按常量提升成 `'static` 临时值; 将来若不再接受这个提升, 换成命名
    /// `const`/`static` 项再借用即可, 语义不变。
    pub fn refetch(&self) -> &'static [Fetch] {
        match self {
            Mutation::UpdateVirtualModel { .. } => &[Fetch::VirtualModels, Fetch::Subscriptions],
            _ => &[Fetch::Subscriptions],
        }
    }
}

/// 一次就地操作的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum MutationOutcome {
    EnabledSet,
    Tested(TestConnectionResult),
    Models(RefreshModelsResult),
    Balance(RefreshBalanceResult),
    SlotsSaved,
    VirtualModelSaved,
    Deleted,
}

/// 向导要发的请求。**与 [`Mutation`] 刻意分开**: 它们没有订阅 id (创建的那一刻还没有), 不进忙碌表,
/// 不进「上次操作」存档, 完成后也不自动重拉——结果只回给向导自己。向导同一时刻最多一个请求在飞,
/// 由它自己的阶段保证。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardCmd {
    /// `list_providers`。向导打开时发一次。
    LoadProviders,
    /// `create_subscription`。
    Create(CreateInput),
    /// `refresh_model_list`。内置厂商创建成功之后拉一次候选模型给槽位选择用。
    LoadModels { id: String },
    /// `probe_custom_models`。自定义厂商在保存前先探测一次模型列表。
    Probe(ProbeInput),
    /// `update_subscription`, 只带 `model_slots` 这一块 patch: 向导不设置 effort, 少发一个字段就不会
    /// 把已有值清掉 (用户建完在订阅详情页按 `o` 就能改)。
    SaveSlots { id: String, model_slots: ModelSlots },
}

/// 向导请求的结果。**绝不带 `Secret`**——去程带 key, 回程一律不带, 这样 key 只在单向的一段消息里
/// 存在过。
///
/// **跨向导实例的晚到结果靠代次挡住。** 关闭向导不会取消在飞的请求 (只读请求最长等到 30 秒超时),
/// 用户可以退出向导 A、马上开向导 B、B 也走到同一个阶段, 这时 A 的结果晚到。`App` 每次打开向导
/// 都换一个新代次, 经 `Cmd::Wizard` 带出、`Action::WizardDone` 带回, 与当前向导的代次不符就在
/// `App::update` 里整个丢弃 (没有向导也丢弃)——结果里的任何字段都不足以证明「是这个实例发的」:
/// 同一个中转、另一种协议 / 另一个 key 的探测结果, `base_url` 完全一样。
///
/// 代次之外还有两层纵深防御, 都不能拿掉: 每个结果只在发起它的阶段被接受 (挡同一实例内的晚到),
/// `Probed` 带 `base_url`、`Models` 带 `id`, 落地时与向导当前记着的同一份值比对 (见
/// `wizard::Wizard::apply_wizard_result`)。以后加新的请求, 代次自动覆盖, 阶段守卫照旧要写。
#[derive(Debug, Clone, PartialEq)]
pub enum WizardResult {
    Providers(Result<Vec<Provider>, String>),
    Created(Result<CreatedSubscription, String>),
    /// `id`: 发起这次 `LoadModels` 请求时的订阅 id (即 `WizardCmd::LoadModels { id }` 里的那个)。
    Models { id: String, result: Result<RefreshModelsResult, String> },
    /// `base_url`: 发起这次 `Probe` 请求时 (已经 trim 过) 的 `base_url`(即
    /// `ProbeInput::base_url`)。
    Probed { base_url: String, result: Result<ProbeModelsResult, String> },
    SlotsSaved(Result<(), String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Quit,
    Fetch(Fetch),
    /// `Box`: `Mutation::UpdateSlots`/`UpdateVirtualModel` 带 `ModelSlots` + `SlotEfforts`
    /// 这类整块负载, 让 `Cmd` 最大变体比其它变体 (`Quit` 零字节、`Fetch` 几字节) 大出一大截,
    /// clippy 的 `large_enum_variant` 会警告——`Vec<Cmd>` 到处传递 (`App::update` 的返回值), 每个
    /// 元素都按最大变体的尺寸分配。只在这一层加间接: `Action` 本来就因为别的变体 (比如
    /// `OpenConfirm { prompt: String, .. }`) 不算「小」, 那边的 `Mutate` / `MutationDone` 不触发这条
    /// lint。
    Mutate(Box<Mutation>),
    /// 向导的一次请求。`epoch`: 发出它的向导实例的代次, 结果原样带回 (见 [`WizardResult`])。
    /// `Box` 同上一条注释的理由: `WizardCmd::Create` 带整块 `CreateInput` (含 `Secret` +
    /// `ModelSlots`), 提前用 `Box` 避免每个 `Vec<Cmd>` 元素都按最大变体分配。
    Wizard { epoch: u64, cmd: Box<WizardCmd> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 来自 `q`: 当前页面有未保存修改时会先弹确认弹窗, 不直接退出——与 [`Action::ForceQuit`]
    /// 的区别只在这一点, `Cmd::Quit` 本身不变。
    Quit,
    /// 来自 `Ctrl+C`: 无论是否有弹窗打开、当前页面是否有未保存修改, 都立即退出, 不确认。
    ForceQuit,
    SwitchTab(Tab),
    NextTab,
    PrevTab,
    ToggleHelp,
    ClosePopup,
    /// 打开一个「是 / 否」确认弹窗; `on_yes` 说明选「是」之后做什么, 其中带着真正要执行的
    /// `Action`。页面自己想请求确认时也可以从 `handle_key` 直接返回这个。**如果已经有另一个弹窗
    /// 打开着 (哪怕是另一个 `Confirm` 或 `Picker`), 会直接替换它**——不播放旧弹窗的关闭动效,
    /// 也不留旧弹窗的几何。
    OpenConfirm { prompt: String, on_yes: OnYes },
    /// 用户在确认弹窗里选了「是」, 带着弹窗打开时给的 [`OnYes`] 原样回来, 按它的变体决定要不要
    /// 先丢弃当前编辑上下文。
    Confirmed(OnYes),
    /// 页面主动清空自己的草稿 (比如按 Esc 放弃编辑) 时用; `App` 收到后调用当前页面的
    /// `discard_changes()`, 不产出任何 `Cmd`。
    DiscardDraft,
    /// 订阅页按 `n`: 打开新建向导。
    OpenWizard,
    /// 关掉向导 (完成 / 取消 / 确认放弃都走这一个)。`App` **真的关掉了向导** (调用时向导确实存在)
    /// 才补一次 `Fetch::Subscriptions`——向导可能已经创建了订阅, 而它不走 `Mutation` 那条自动重拉
    /// 的路; 向导已经不存在时 (比如 `OnYes::DiscardThen` 先经 `discard_current()` 关过一次, 又把
    /// 这个 action 当 `inner` 执行了一遍) 不重复发, 与 `discard_current()` 幂等。
    CloseWizard,
    /// 一次向导请求的结果。`epoch` 是发出请求的那个 `Cmd::Wizard` 带的代次; 与当前向导的代次
    /// 不符、或者没有向导时 (用户在结果回来之前就退出了) 直接丢弃。
    WizardDone { epoch: u64, result: Box<WizardResult> },
    /// 向导表单的提交类按钮 (「下一步」「保存」「获取模型列表」「创建」) 校验通过时触发。
    /// **`handle_key` 阶段就已经把 `WizardCmd` 打包好了**,
    /// `App::update` 原样转成 `Cmd::Wizard`——跟 `Action::OpenWizard` 直接调用
    /// `wizard.on_open()` 拿 `Cmd` 是同一条思路: 触发点是一次按键而不是收到的某个异步结果, 不需要
    /// 也不该走 `Wizard::update()` 那条专给"结果"设计的路径 (那条路径靠 `WizardDone` 触发, 且
    /// `update()` 的返回值语义是"这次 action 引出的新请求", 不适合用来表达"这次按键本身就是一个
    /// 请求")。`Box`: `WizardCmd::Create`/`SaveSlots` 都带整块 `CreateInput`/`ModelSlots`, 同
    /// `Cmd::Wizard` 一样提前装箱, 避免 `Action` 的每个变体都按最大的那个分配。
    WizardRequest(Box<WizardCmd>),
    Refresh,
    /// 250ms 一次。`now_ms` 是 Unix 毫秒 —— 冷却倒计时要和后端给的 `cooldown_until` 比。
    Tick { now_ms: i64 },
    /// 事件流连上了 (首次或重连)。
    Connected { app_version: String },
    /// 事件流断了, 主循环正在退避重连。
    ConnectionLost,
    /// 后端推来的事件; `data` 是原始 JSON 文本。`at_ms`: TUI 收到它的时刻 (Unix 毫秒), 由 runtime.rs 盖——
    /// 实时路由页据此计时 (App 自己的 now_ms 只按 250ms tick 前进, 精度不够)。
    Sse { name: String, data: String, at_ms: i64 },
    /// 可见页面每 5 秒一次的自动刷新。与 `Refresh` 分开, 是因为日志页翻到第 2 页以后
    /// 只认用户按的 `r`。
    Poll,
    /// `issued`: 主循环发起这次加载时盖的单调递增序号。
    FetchDone { fetch: Fetch, issued: u64, result: Result<FetchData, String> },
    /// 订阅页的一次就地操作。
    Mutate(Mutation),
    /// 一次就地操作跑完了 (成功或失败)。`barrier`: 变更完成那一刻已发起的所有加载的最大序号——
    /// 由 `runtime.rs::process_action` 在**收到**这条消息时补盖 (`spawn_mutation` 发送时只是占位符
    /// `0`); `App` 据此对 `mutation.refetch()` 里每个目标调用对应的 `set_*_barrier`, 挡住那些在
    /// 变更完成前就已经发起、内容还是变更前旧值的加载晚到时把乐观更新冲回去。
    MutationDone { mutation: Mutation, barrier: u64, result: Result<MutationOutcome, String> },
    /// 打开一个过滤选择弹窗 (选模型 / 选 effort / 给虚拟模型加订阅 / 向导里的各种选择)。
    /// **如果已经有另一个弹窗打开着, 会直接替换它**——语义与 `OpenConfirm` 相同。
    OpenPicker(PickerSpec),
    /// 用户在选择弹窗里选定了一行 (或输入了自定义值)。`App` 收到后先关弹窗 (带关闭动效), 再原样
    /// 转给向导 (存在时) 或当前页面的 `update`——据 `tag` 知道该把 `choice` 填到哪; 不认识的
    /// `tag` 直接忽略。
    PickerDone { tag: PickerTag, choice: PickerChoice },
    /// 打开一个只读可滚动的详情弹窗 (日志页发起)。**如果已经有另一个弹窗打开着, 会直接替换它**——
    /// 语义与 `OpenConfirm` / `OpenPicker` 相同。
    OpenDetail(DetailSpec),
    /// 实时路由页 ⏎: 切到日志页并只看这条订阅。
    OpenLogsFor { subscription_id: String },
    /// 页面想弹一条 toast, 但自己不能直接碰 `App` 的 toast 队列。只从
    /// `Component::handle_key` 的返回值这条路走——`update()` 内部想弹通知 (比如处理
    /// `PickerDone` 时发现输入为空) 用的是另一条路 (`Component::take_notice`, 见 `pages/mod.rs`),
    /// 不产出这个 `Action` (那个签名返回 `Vec<Cmd>`, 塞不进一个 `Action`)。
    Notify { kind: ToastKind, text: String },
}

/// 确认弹窗选「是」之后做什么。**两种语义刻意做成两个变体, 没有默认值**: 每个确认的发起方都
/// 必须明说「是」会不会丢掉当前的编辑上下文——删除这类执行类确认与草稿无关, 丢了草稿就是把用户
/// 没保存的修改悄悄扔掉。
#[derive(Debug, Clone, PartialEq)]
pub enum OnYes {
    /// 「放弃修改」类: 先丢弃当前编辑上下文 (有向导时关掉向导, 否则丢弃当前页草稿), 再执行这个
    /// `Action`——它走一次普通 `App::update`, 此时 dirty 已经清空, 不会被再次拦截确认。
    /// `App::guard_dirty` 自动包出来的就是这种。
    DiscardThen(Box<Action>),
    /// 执行类 (比如删除订阅): 只执行这个 `Action`, 不碰任何草稿。
    Run(Box<Action>),
}

impl OnYes {
    pub fn discard_then(action: Action) -> Self {
        OnYes::DiscardThen(Box::new(action))
    }

    pub fn run(action: Action) -> Self {
        OnYes::Run(Box::new(action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::dto::RequestFilters;

    fn slots() -> ModelSlots {
        ModelSlots { fable: String::new(), opus: String::new(), sonnet: String::new(), haiku: String::new(), fallback: String::new(), jev: String::new() }
    }

    #[test]
    fn every_mutation_declares_its_busy_key_and_refetch() {
        let subscription_scoped = [
            Mutation::SetEnabled { id: "1".into(), enabled: true },
            Mutation::TestConnection { id: "1".into() },
            Mutation::RefreshModels { id: "1".into() },
            Mutation::RefreshBalance { id: "1".into() },
            Mutation::UpdateSlots { id: "1".into(), model_slots: slots(), slot_efforts: SlotEfforts::default() },
            Mutation::Delete { id: "1".into() },
        ];
        for m in subscription_scoped {
            assert_eq!(m.busy_key(), BusyKey::Subscription("1".into()), "{m:?} 应该产出 Subscription 忙碌键");
            assert_eq!(m.refetch(), &[Fetch::Subscriptions], "{m:?} 完成后应该重拉订阅列表");
        }

        let vm = Mutation::UpdateVirtualModel { name: "model-sonnet".into(), mode: RoutingMode::Sequential, subscription_ids: vec![] };
        assert_eq!(vm.busy_key(), BusyKey::VirtualModel("model-sonnet".into()), "虚拟模型编辑应该产出 VirtualModel 忙碌键");
        assert_eq!(vm.refetch(), &[Fetch::VirtualModels, Fetch::Subscriptions], "完成后应该先重拉虚拟模型再重拉订阅");
    }

    /// 两个页码不同的 `Requests` 仍然是「同一种」加载——`Fetches` 按 `kind()` 去重, 不按
    /// 整个 `Fetch` 值, 否则「第 1 页还没回来时又翻到第 2 页」会被当成两种不同的加载各自去重,
    /// 而不是「同一种, 最新为准」。
    #[test]
    fn fetch_kind_ignores_parameters() {
        let page1 = Fetch::Requests(RequestQuery { page: 1, filters: RequestFilters::default() });
        let page2 = Fetch::Requests(RequestQuery { page: 2, filters: RequestFilters::default() });
        assert_ne!(page1, page2, "两页本身应该不相等");
        assert_eq!(page1.kind(), FetchKind::Requests);
        assert_eq!(page1.kind(), page2.kind(), "kind() 应该忽略参数");
    }

    #[test]
    fn tab_index_round_trips_and_wraps() {
        for (i, tab) in Tab::ALL.into_iter().enumerate() {
            assert_eq!(tab.index(), i);
            assert_eq!(Tab::from_index(i), Some(tab));
        }
        assert_eq!(Tab::from_index(5), None);
        assert_eq!(Tab::from_index(usize::MAX), None);

        assert_eq!(Tab::Overview.next(), Tab::Subscriptions);
        assert_eq!(Tab::Logs.next(), Tab::Overview, "末尾绕回开头");
        assert_eq!(Tab::Overview.prev(), Tab::Logs, "开头绕回末尾");
        assert_eq!(Tab::Logs.prev(), Tab::Live);
    }
}
