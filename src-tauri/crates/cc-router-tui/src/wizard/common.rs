//! 三张表单共用的部分: 按字段种类派生的按键与底栏提示、行组装、槽位选择器。
//!
//! 每张表单只回答三个问题——焦点顺序是什么、每个字段是哪一种 ([`FieldKind`])、文本字段在哪
//! ([`FormFields`])——方向键 / `Tab` / 文本行 `⏎` / `Ctrl+R` / 打字、底栏提示、行内提示与
//! 锁定态都由种类决定, 三张表单的行为因此不会各自走样。

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::form_state::FormState;
use super::text::TextInput;
use crate::action::Action;
use crate::client::dto::{ModelInfo, ModelSlots, Slot};
use crate::format::slot_label;
use crate::i18n::Strings;
use crate::widgets::form::{FormBuilder, FormRow, FormRows};
use crate::widgets::keybar::Hint;
use crate::widgets::picker::{PickerChoice, PickerItem, PickerSpec, PickerTag};

/// 五个槽位的画法与选择顺序。
pub(super) const SLOTS: [Slot; 5] = [Slot::Fable, Slot::Opus, Slot::Sonnet, Slot::Haiku, Slot::Fallback];

/// 一个字段在按键、提示与画法上属于哪一种。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FieldKind {
    /// 选择行: `⏎` 打开选择弹窗, 不能直接打字。
    Pick,
    /// 文本行: 直接打字, `⏎` 移到下一行。
    Text,
    /// 敏感文本行: 同 `Text`, 外加 `Ctrl+R` 切换明文 / 掩码。
    Secret,
    /// 按钮行: `⏎` 执行; 底栏提示显示按钮自己的标签。
    Button(&'static str),
    /// 画出来但不可操作 (锁定的鉴权头)。它不在焦点顺序里, 万一被聚焦, `⏎` 什么都不做。
    Locked,
}

pub(super) trait FormFields {
    type Field: Copy + Eq;
    fn state(&self) -> &FormState<Self::Field>;
    fn state_mut(&mut self) -> &mut FormState<Self::Field>;
    /// 可聚焦字段, 顺序即上下键的顺序。
    fn order(&self) -> Vec<Self::Field>;
    fn kind(&self, field: Self::Field, s: &'static Strings) -> FieldKind;
    /// 字段对应的文本输入; 非文本行是 `None`。
    fn text_field(&mut self, field: Self::Field) -> Option<&mut dyn TextInput>;
    /// 文本行的光标显示列, 由字段自己的输入框现算 (`visual_cursor()`); 其它行恒 `None`。
    /// 与 [`Self::text_field`] 分开是因为这里只需要只读访问, 调用方 (`Rows::field`) 不该为了
    /// 读一个数字去拿 `&mut` —— 这样 `Cell` 就不用再带一个容易被忘记填的 `cursor` 字段
    /// (穷尽 `match` 保证漏填这里编译就过不去, 不用等到某个字段光标飞到别处才发现)。
    fn cursor_for(&self, field: Self::Field) -> Option<usize>;
}

/// [`handle_key`] 处理完之后, 表单自己还要做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyOutcome<F> {
    /// 已经处理完 (移动了焦点 / 编辑了文字 / 切换了明文) 或者被吞掉。
    Handled,
    /// 这个文本字段的**值**变了 (只移动光标不算)。
    Edited(F),
    /// 在选择行或按钮行上按了 `⏎`。
    Activate(F),
}

pub(super) fn handle_key<M: FormFields>(form: &mut M, key: KeyEvent, s: &'static Strings) -> KeyOutcome<M::Field> {
    let focus = form.state().focus();
    let kind = form.kind(focus, s);
    match key.code {
        KeyCode::Up | KeyCode::BackTab => step(form, -1),
        KeyCode::Down | KeyCode::Tab => step(form, 1),
        KeyCode::Enter => match kind {
            FieldKind::Text | FieldKind::Secret => step(form, 1),
            FieldKind::Pick | FieldKind::Button(_) => KeyOutcome::Activate(focus),
            FieldKind::Locked => KeyOutcome::Handled,
        },
        // 必须排在打字之前, 否则 Ctrl+R 会被当成字符 'r' 输进去。
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) && kind == FieldKind::Secret => {
            if let Some(field) = form.text_field(focus) {
                field.toggle_reveal();
            }
            KeyOutcome::Handled
        }
        // 非文本行没有输入框, 按键 (含字符键) 一律吞掉。
        _ => {
            let Some(field) = form.text_field(focus) else { return KeyOutcome::Handled };
            if !field.handle(key) {
                return KeyOutcome::Handled;
            }
            // 值真的变了才清错误, 而且只清这个字段自己的。
            form.state_mut().clear(focus);
            KeyOutcome::Edited(focus)
        }
    }
}

fn step<M: FormFields>(form: &mut M, delta: isize) -> KeyOutcome<M::Field> {
    let order = form.order();
    form.state_mut().step(delta, &order);
    KeyOutcome::Handled
}

/// 底栏左侧: `↑↓ 字段` 常驻, 再按聚焦行的种类追加一两条。
pub(super) fn hints<M: FormFields>(form: &M, s: &'static Strings) -> Vec<Hint<'static>> {
    let mut hints = vec![("↑↓", s.key_field)];
    match form.kind(form.state().focus(), s) {
        FieldKind::Pick => hints.push(("⏎", s.key_pick)),
        FieldKind::Text => hints.push(("⏎", s.key_next_field)),
        FieldKind::Secret => {
            hints.push(("⏎", s.key_next_field));
            hints.push(("Ctrl+R", s.key_reveal));
        }
        FieldKind::Button(label) => hints.push(("⏎", label)),
        FieldKind::Locked => {}
    }
    hints
}

/// 字段行里由表单给出的内容。错误、锁定态、光标与行右端的提示都由 [`Rows::field`] 按字段补齐——
/// 光标**不**在这里手传 (见 [`FormFields::cursor_for`] 的文档注释): 这个结构体派生了 `Default`,
/// 手传的话一个文本行写 `..Cell::default()` 时忘了给 `cursor` 照样编译通过, 快照又不含终端光标,
/// 这种遗漏没有测试能咬住。
#[derive(Default)]
pub(super) struct Cell<'a> {
    pub label: &'a str,
    /// 已经处理好的显示文本 (掩码由调用方决定)。
    pub value: &'a str,
    pub placeholder: &'a str,
}

/// 行右端的固定提示, 每帧拼一次。
pub(super) struct RowHints {
    pick: String,
    reveal: String,
}

impl RowHints {
    pub(super) fn new(s: &'static Strings) -> Self {
        Self { pick: format!("⏎ {}", s.key_pick), reveal: format!("Ctrl+R {}", s.key_reveal) }
    }
}

/// 把一张表单的状态组装成 `FormRow`: 焦点标记、错误、锁定态、行内提示都从字段种类与
/// `FormState` 派生, 调用方只按显示顺序列出行。
pub(super) struct Rows<'a, M: FormFields> {
    b: FormBuilder<'a>,
    form: &'a M,
    hints: &'a RowHints,
    s: &'static Strings,
    /// 请求在飞: 全部字段画成只读。
    busy: bool,
}

impl<'a, M: FormFields> Rows<'a, M> {
    pub(super) fn new(form: &'a M, hints: &'a RowHints, s: &'static Strings, busy: bool) -> Self {
        Self { b: FormBuilder::new(), form, hints, s, busy }
    }

    fn focused(&self, field: M::Field) -> bool {
        self.form.state().focus() == field
    }

    /// 上一次请求的结果说明 (失败原因等), 挂在表单顶部, 后面空一行。
    pub(super) fn note(&mut self, note: Option<&'a str>) {
        if let Some(text) = note {
            self.b.push(FormRow::Note { text }, false);
            self.b.push(FormRow::Spacer, false);
        }
    }

    pub(super) fn spacer(&mut self) {
        self.b.push(FormRow::Spacer, false);
    }

    pub(super) fn field(&mut self, field: M::Field, cell: Cell<'a>) {
        let kind = self.form.kind(field, self.s);
        let hint = match kind {
            FieldKind::Pick => Some(self.hints.pick.as_str()),
            FieldKind::Secret => Some(self.hints.reveal.as_str()),
            FieldKind::Text | FieldKind::Button(_) | FieldKind::Locked => None,
        };
        let row = FormRow::Field {
            label: cell.label,
            value: cell.value,
            placeholder: cell.placeholder,
            hint,
            cursor: self.form.cursor_for(field),
            error: self.form.state().error_for(field),
            locked: self.busy || kind == FieldKind::Locked,
        };
        let focused = self.focused(field);
        self.b.push(row, focused);
    }

    /// 五个槽位行。兜底槽留空画成灰字「(未配置)」; 核心槽留空没有占位——保存时校验会拦住。
    pub(super) fn slots(&mut self, slots: &'a ModelSlots, field_of: impl Fn(Slot) -> M::Field) {
        for slot in SLOTS {
            let placeholder = if slot == Slot::Fallback { self.s.sub_slot_unset } else { "" };
            self.field(field_of(slot), Cell { label: slot_label(slot, self.s), value: slots.get(slot), placeholder });
        }
    }

    /// 按钮行。`busy_label` 是这个按钮自己的请求在飞时的文案 (前面画 throbber); `None` 时画
    /// 按钮本来的标签。
    pub(super) fn button(&mut self, field: M::Field, busy_label: Option<&'static str>) {
        // 按钮的标签只有 `FieldKind::Button` 一个来源 (底栏提示用的也是它); 把非按钮字段当按钮画
        // 是调用方写错了, 画成空标签, 快照测试会立刻看出来。
        let label = match self.form.kind(field, self.s) {
            FieldKind::Button(label) => label,
            FieldKind::Pick | FieldKind::Text | FieldKind::Secret | FieldKind::Locked => "",
        };
        let focused = self.focused(field);
        self.b.push(FormRow::Button { label: busy_label.unwrap_or(label), busy: busy_label.is_some() }, focused);
    }

    pub(super) fn finish(self) -> FormRows<'a> {
        self.b.finish()
    }
}

/// 槽位行 `⏎` 的选择弹窗。候选是拉到 / 探测到的真实模型; 没有时退回 `examples` (厂商 yaml 里的
/// `example_models`, 只有 id; 自定义厂商没有这个概念, 传空), 引导手输。兜底槽与 Jev 槽额外在最前面
/// 放一项「清空」(兜底空 = 未配置, Jev 空 = 透传)。
pub(super) fn slot_picker(slot: Slot, current: &str, models: &[ModelInfo], examples: &[String], s: &'static Strings) -> Action {
    let mut items = Vec::new();
    if slot == Slot::Fallback {
        items.push(PickerItem { id: String::new(), label: s.pick_clear_fallback.to_string(), hint: None });
    }
    if slot == Slot::Jev {
        items.push(PickerItem { id: String::new(), label: s.pick_clear_jev.to_string(), hint: None });
    }
    if models.is_empty() {
        items.extend(examples.iter().map(|id| PickerItem { id: id.clone(), label: id.clone(), hint: None }));
    } else {
        items.extend(models.iter().map(|m| PickerItem { id: m.id.clone(), label: m.id.clone(), hint: m.display_name.clone() }));
    }
    Action::OpenPicker(PickerSpec {
        tag: PickerTag::WizardSlot { slot },
        title: (s.wiz_pick_model)(slot_label(slot, s)),
        items,
        allow_custom: true,
        initial: current.to_string(),
    })
}

/// 槽位选择弹窗的结果要写进草稿的值。兜底槽的「清空」项 (`id: ""`) 与正常选值走同一条路径——
/// 空串本来就是「未配置」; 自定义输入 `trim` 一下, 与订阅详情页同规则。
pub(super) fn slot_choice_value(choice: &PickerChoice) -> String {
    match choice {
        PickerChoice::Item(id) => id.clone(),
        PickerChoice::Custom(text) => text.trim().to_string(),
    }
}
