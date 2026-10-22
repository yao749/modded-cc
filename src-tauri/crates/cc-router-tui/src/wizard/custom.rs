//! 自定义厂商路径的单页表单: 协议 / 厂商名 / Base URL / 请求路径 / 鉴权 / API Key / 备注名 /
//! [获取模型列表] / 五个槽位 / [创建]。槽位在创建前就是真值, 所以没有第二步。

use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use super::common::{self, Cell, FieldKind, FormFields, KeyOutcome, RowHints, Rows};
use super::fields::{validate_custom, validate_probe, CustomDraft, CustomField};
use super::form_state::FormState;
use super::text::TextInput;
use super::{follow_display_name, CustomPhase, Paint};
use crate::action::{Action, WizardCmd};
use crate::client::dto::{
    AuthHeaderFormat, CreateInput, CreateSource, CustomProtocol, CustomSource, ProbeInput, Slot, ANTHROPIC_AUTH_PRESETS,
    CUSTOM_BASE_URL_PLACEHOLDER,
};
use crate::i18n::Strings;
use crate::store::Store;
use crate::widgets::form::{self, FormView};
use crate::widgets::keybar::Hint;
use crate::widgets::picker::{PickerChoice, PickerItem, PickerSpec, PickerTag};

pub(super) struct CustomForm {
    pub(super) draft: CustomDraft,
    pub(super) state: FormState<CustomField>,
    /// 上一次自动算出来的备注名, 见 `follow_display_name`。
    pub(super) last_auto_name: Option<String>,
    /// 探测失败 / 创建失败的原因, 挂在表单顶部; 谁最后发生显示谁。
    pub(super) note: Option<String>,
    /// 同 `BasicsForm::pending_field_err`——`Probe`/`Submit` 两个按钮都可能触发校验失败。
    pub(super) pending_field_err: bool,
}

impl FormFields for CustomForm {
    type Field = CustomField;

    fn state(&self) -> &FormState<CustomField> {
        &self.state
    }

    fn state_mut(&mut self) -> &mut FormState<CustomField> {
        &mut self.state
    }

    fn order(&self) -> Vec<CustomField> {
        CustomField::all(self.draft.protocol)
    }

    fn kind(&self, field: CustomField, s: &'static Strings) -> FieldKind {
        match field {
            CustomField::Protocol | CustomField::Slot(_) => FieldKind::Pick,
            CustomField::Auth if self.draft.protocol.auth_locked() => FieldKind::Locked,
            CustomField::Auth => FieldKind::Pick,
            CustomField::ProviderName | CustomField::BaseUrl | CustomField::MessagesPath | CustomField::DisplayName => FieldKind::Text,
            CustomField::ApiKey => FieldKind::Secret,
            CustomField::Probe => FieldKind::Button(s.wiz_btn_probe),
            CustomField::Submit => FieldKind::Button(s.wiz_btn_create),
        }
    }

    fn text_field(&mut self, field: CustomField) -> Option<&mut dyn TextInput> {
        self.draft.text_field(field)
    }

    fn cursor_for(&self, field: CustomField) -> Option<usize> {
        match field {
            CustomField::ProviderName => Some(self.draft.provider_display_name.visual_cursor()),
            CustomField::BaseUrl => Some(self.draft.base_url.visual_cursor()),
            CustomField::MessagesPath => Some(self.draft.messages_path.visual_cursor()),
            CustomField::ApiKey => Some(self.draft.api_key.visual_cursor()),
            CustomField::DisplayName => Some(self.draft.display_name.visual_cursor()),
            CustomField::Protocol | CustomField::Auth | CustomField::Probe | CustomField::Slot(_) | CustomField::Submit => None,
        }
    }
}

impl CustomForm {
    pub(super) fn new(protocol: CustomProtocol) -> Self {
        Self {
            draft: CustomDraft::new(protocol),
            state: FormState::new(CustomField::ProviderName),
            last_auto_name: None,
            note: None,
            pending_field_err: false,
        }
    }

    /// 只在 `CustomPhase::Editing` 下被调用。探测 / 创建发出时把 `phase` 推进到对应的在飞阶段。
    /// 编辑 Base URL 不清 `probe`: `CustomDraft::models_url()` 自己按值比对, 提前清空会丢掉
    /// 「改回去又生效」。
    pub(super) fn handle_key(&mut self, key: KeyEvent, phase: &mut CustomPhase, store: &Store, s: &'static Strings) -> Option<Action> {
        match common::handle_key(self, key, s) {
            KeyOutcome::Handled => None,
            KeyOutcome::Edited(CustomField::ProviderName) => {
                self.follow_provider_name(store);
                None
            }
            KeyOutcome::Edited(_) => None,
            KeyOutcome::Activate(CustomField::Protocol) => Some(self.protocol_picker(s)),
            KeyOutcome::Activate(CustomField::Auth) => Some(self.auth_picker(s)),
            KeyOutcome::Activate(CustomField::Slot(slot)) => {
                Some(common::slot_picker(slot, self.draft.slots.slots.get(slot), &self.draft.slots.models, &[], s))
            }
            KeyOutcome::Activate(CustomField::Probe) => self.submit_probe(phase, s),
            KeyOutcome::Activate(CustomField::Submit) => self.submit(phase, s),
            KeyOutcome::Activate(
                CustomField::ProviderName | CustomField::BaseUrl | CustomField::MessagesPath | CustomField::ApiKey | CustomField::DisplayName,
            ) => None,
        }
    }

    /// 备注名跟着厂商名自动生成; 厂商名 trim 后为空时不跟 (还没打字就不该凭空冒出默认值)。
    fn follow_provider_name(&mut self, store: &Store) {
        let name = self.draft.provider_display_name.value().trim().to_string();
        if !name.is_empty() && follow_display_name(&mut self.draft.display_name, &mut self.last_auto_name, &name, store) {
            self.state.clear(CustomField::DisplayName);
        }
    }

    /// `Protocol` 行 `⏎`: label 与厂商选择器里的 `wiz_custom_labels` 一致。
    fn protocol_picker(&self, s: &'static Strings) -> Action {
        let items = CustomProtocol::ALL
            .iter()
            .zip(s.wiz_custom_labels.iter())
            .map(|(p, label)| PickerItem { id: p.as_wire().to_string(), label: (*label).to_string(), hint: None })
            .collect();
        Action::OpenPicker(PickerSpec {
            tag: PickerTag::WizardProtocol,
            title: s.wiz_pick_protocol.to_string(),
            items,
            allow_custom: false,
            initial: self.draft.protocol.as_wire().to_string(),
        })
    }

    /// `Auth` 行 `⏎` (锁定时它是 `FieldKind::Locked`, 到不了这里): 两个预设的头名不同, 直接拿
    /// 头名当 id。
    fn auth_picker(&self, s: &'static Strings) -> Action {
        let items = ANTHROPIC_AUTH_PRESETS
            .iter()
            .zip(s.wiz_auth_labels.iter())
            .map(|((header, _format), label)| PickerItem { id: (*header).to_string(), label: (*label).to_string(), hint: None })
            .collect();
        Action::OpenPicker(PickerSpec {
            tag: PickerTag::WizardAuth,
            title: s.wiz_pick_auth.to_string(),
            items,
            allow_custom: false,
            initial: self.draft.auth_header_name.clone(),
        })
    }

    /// 换协议: 连接字段重置成新协议的预设, 清掉这几个字段的错误和过期的说明行, 焦点若停在刚被
    /// 锁定的 `Auth` 行上就挪到相邻的 `ApiKey`。**重选同一个协议什么都不做**——否则用户手填的
    /// Base URL / 请求路径 / 鉴权会在无意中确认同一项时被弹回预设 (Anthropic 的预设 Base URL 是
    /// 空串, 用户只会看到灰字占位符, 以为那就是真值)。
    pub(super) fn apply_protocol_choice(&mut self, choice: &PickerChoice) {
        let PickerChoice::Item(id) = choice else { return };
        let Some(protocol) = CustomProtocol::ALL.iter().find(|p| p.as_wire() == id).copied() else { return };
        if protocol == self.draft.protocol {
            return;
        }
        self.draft.apply_protocol(protocol);
        self.note = None;
        self.state.clear(CustomField::BaseUrl);
        self.state.clear(CustomField::MessagesPath);
        self.state.clear(CustomField::Auth);
        if self.state.focus() == CustomField::Auth && protocol.auth_locked() {
            self.state.focus_on(CustomField::ApiKey);
        }
    }

    pub(super) fn apply_auth_choice(&mut self, choice: &PickerChoice) {
        let PickerChoice::Item(id) = choice else { return };
        let Some((header, format)) = ANTHROPIC_AUTH_PRESETS.iter().find(|entry| entry.0 == id.as_str()) else { return };
        self.draft.auth_header_name = (*header).to_string();
        self.draft.auth_header_format = *format;
        self.state.clear(CustomField::Auth);
    }

    pub(super) fn apply_slot_choice(&mut self, slot: Slot, choice: &PickerChoice) {
        self.draft.slots.slots.set(slot, common::slot_choice_value(choice));
        self.state.clear(CustomField::Slot(slot));
    }

    /// `Probe` 行 `⏎`。发起时清掉上一次的说明行, 否则在飞期间「上次失败的原因」和「正在获取…」
    /// 会同时出现。
    fn submit_probe(&mut self, phase: &mut CustomPhase, s: &'static Strings) -> Option<Action> {
        if !self.state.validate(validate_probe(&self.draft, s)) {
            self.pending_field_err = true;
            return None;
        }
        self.note = None;
        let cmd = WizardCmd::Probe(ProbeInput {
            base_url: self.draft.base_url.value().trim().to_string(),
            auth_header_name: self.draft.auth_header_name.clone(),
            auth_header_format: self.draft.auth_header_format,
            api_key: self.draft.api_key.secret(),
            protocol: self.draft.protocol,
        });
        *phase = CustomPhase::Probing;
        Some(Action::WizardRequest(Box::new(cmd)))
    }

    /// `Submit`(「创建」) 行 `⏎`: 一次创建, `model_slots` 是真实选值, `models_url` 按「探测后
    /// Base URL 没再改过」的规则回传。发起时同样清掉说明行。
    fn submit(&mut self, phase: &mut CustomPhase, s: &'static Strings) -> Option<Action> {
        if !self.state.validate(validate_custom(&self.draft, s)) {
            self.pending_field_err = true;
            return None;
        }
        self.note = None;
        let draft = &self.draft;
        let cmd = WizardCmd::Create(CreateInput {
            // 校验按 trim 后判空, 发出去的也是 trim 后的值——校验什么就发送什么。
            display_name: draft.display_name.value().trim().to_string(),
            api_key: draft.api_key.secret(),
            model_slots: draft.slots.slots.clone(),
            source: CreateSource::Custom(Box::new(CustomSource {
                provider_display_name: draft.provider_display_name.value().trim().to_string(),
                base_url: draft.base_url.value().trim().to_string(),
                messages_path: draft.messages_path.value().trim().to_string(),
                auth_header_name: draft.auth_header_name.clone(),
                auth_header_format: draft.auth_header_format,
                protocol: draft.protocol,
                models_url: draft.models_url().map(str::to_string),
            })),
        });
        *phase = CustomPhase::Creating;
        Some(Action::WizardRequest(Box::new(cmd)))
    }

    pub(super) fn hints(&self, s: &'static Strings) -> Vec<Hint<'static>> {
        common::hints(self, s)
    }

    /// 14 行固定内容, 不加空行分组 (80×24 下内容区还有富余)。`Auth` 行总是画出来, 锁定时只是
    /// 只读——锁定与否不影响行数, 只影响焦点顺序。**返回聚焦行的下标 + 矩形**, 同 `BasicsForm::draw`。
    pub(super) fn draw(&self, frame: &mut Frame, area: Rect, phase: CustomPhase, p: &Paint) -> Option<(usize, Rect)> {
        let s = p.s;
        let d = &self.draft;
        let probing = phase == CustomPhase::Probing;
        let creating = phase == CustomPhase::Creating;
        let auth_label = format!("{} · {}", d.auth_header_name, auth_format_label(d.auth_header_format));
        let api_key_text = d.api_key.display();
        let hints = RowHints::new(s);

        let mut rows = Rows::new(self, &hints, s, probing || creating);
        rows.note(self.note.as_deref());
        rows.field(CustomField::Protocol, Cell { label: s.wiz_f_protocol, value: protocol_label(d.protocol, s), ..Cell::default() });
        rows.field(
            CustomField::ProviderName,
            Cell { label: s.wiz_f_provider_name, value: d.provider_display_name.value(), ..Cell::default() },
        );
        rows.field(
            CustomField::BaseUrl,
            Cell { label: s.wiz_f_base_url, value: d.base_url.value(), placeholder: CUSTOM_BASE_URL_PLACEHOLDER },
        );
        rows.field(
            CustomField::MessagesPath,
            Cell { label: s.wiz_f_messages_path, value: d.messages_path.value(), ..Cell::default() },
        );
        rows.field(CustomField::Auth, Cell { label: s.wiz_f_auth, value: &auth_label, ..Cell::default() });
        rows.field(CustomField::ApiKey, Cell { label: s.wiz_f_api_key, value: &api_key_text, ..Cell::default() });
        rows.field(
            CustomField::DisplayName,
            Cell { label: s.wiz_f_display_name, value: d.display_name.value(), ..Cell::default() },
        );
        if d.protocol.is_systemone() {
            rows.field(
                CustomField::Slot(Slot::Jev),
                Cell { label: crate::format::slot_label(Slot::Jev, s), value: d.slots.slots.get(Slot::Jev), ..Cell::default() },
            );
        } else {
            rows.button(CustomField::Probe, probing.then_some(s.wiz_probing));
            rows.slots(&d.slots.slots, CustomField::Slot);
        }
        rows.button(CustomField::Submit, creating.then_some(s.wiz_creating));
        let built = rows.finish();
        let focus_index = built.focus;

        // 单页没有步骤条, 标题换成专属的 `wiz_custom_title`。
        let view =
            FormView { title: s.wiz_custom_title, steps: None, rows: &built.rows, focus: built.focus, tick: p.tick, show_cursor: p.show_cursor };
        form::draw(frame, area, &view, p.theme, s).map(|rect| (focus_index, rect))
    }
}

/// 协议行的展示名 `wiz_protocol_names`, 与厂商选择器里带「自定义 · 」前缀的 `wiz_custom_labels`
/// 是两份独立文案 (标题已经写明「自定义」, 字段行不重复前缀)。不靠剥前缀派生: 那需要在代码里写
/// 中文字面量, 而且文案一改或一翻译就静默失效。两个数组顺序都与 `CustomProtocol::ALL` 一致。
fn protocol_label(protocol: CustomProtocol, s: &'static Strings) -> &'static str {
    let idx = CustomProtocol::ALL.iter().position(|p| *p == protocol).unwrap_or(0);
    s.wiz_protocol_names[idx]
}

/// 鉴权格式的展示名——英文技术词汇 (与请求头名同类), 不进 `Strings`。
fn auth_format_label(format: AuthHeaderFormat) -> &'static str {
    match format {
        AuthHeaderFormat::Bearer => "Bearer",
        AuthHeaderFormat::Raw => "Raw",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::dto::ModelInfo;

    /// 换协议清空探测到的候选模型与表单顶部的说明行——旧协议探测到的模型 (比如 OpenAI Responses
    /// 下的 `gpt-5.5`) 对新协议 (比如 Gemini) 没有意义, 留着会让用户在 Fable 的选择器里选出模型名
    /// 对不上协议的值。**已经填进槽位的值不该被清** (与 API Key / 备注名同规则)。
    #[test]
    fn apply_protocol_clears_stale_candidates_and_note_but_keeps_chosen_slots() {
        let mut form = CustomForm::new(CustomProtocol::OpenaiResponses);
        form.draft.slots.models = vec![ModelInfo { id: "gpt-5.5".into(), display_name: None }];
        form.note = Some("上一次自动获取失败的原因".into());
        form.draft.slots.slots.fable = "gpt-5.5".into();

        form.apply_protocol_choice(&PickerChoice::Item(CustomProtocol::Gemini.as_wire().to_string()));

        assert!(form.draft.slots.models.is_empty(), "换协议应该清空旧协议探测到的候选模型");
        assert!(form.note.is_none(), "换协议应该清空旧的说明行");
        assert_eq!(form.draft.slots.slots.fable, "gpt-5.5", "已经填进槽位的值不该被换协议清掉");
    }

    /// 锁定的鉴权行不在焦点顺序里; 就算焦点被程序化地放到它上面, `⏎` 也不开选择器、底栏不给
    /// 「⏎ 选择」, 行画成只读且没有行内提示。
    #[test]
    fn a_locked_auth_row_is_inert_even_if_focused() {
        let s = &crate::i18n::ZH;
        let mut form = CustomForm::new(CustomProtocol::Gemini);
        assert!(form.draft.protocol.auth_locked(), "准备: Gemini 的鉴权头是锁定的");
        assert!(!form.order().contains(&CustomField::Auth), "锁定的鉴权行不该在焦点顺序里");
        assert_eq!(form.kind(CustomField::Auth, s), FieldKind::Locked);

        form.state.focus_on(CustomField::Auth);
        let mut phase = CustomPhase::Editing;
        let enter = KeyEvent::new(ratatui::crossterm::event::KeyCode::Enter, ratatui::crossterm::event::KeyModifiers::NONE);
        assert_eq!(form.handle_key(enter, &mut phase, &Store::default(), s), None, "⏎ 不该打开鉴权选择器");
        assert_eq!(phase, CustomPhase::Editing);
        assert_eq!(form.hints(s), vec![("↑↓", s.key_field)], "锁定行不该提示 ⏎ 选择");
    }
}
