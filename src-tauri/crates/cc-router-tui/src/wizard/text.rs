//! 向导的文本字段。**文本只存一处**: 值就在 `tui_input::Input` 里, 草稿直接持有这两种类型。
//! 草稿与输入框各存一份的话, 每次程序化地改草稿 (选厂商时自动填备注名、换协议时重置 Base URL……)
//! 都得记得重建输入框, 漏一处, 下一次按键就会拿输入框里的旧值把草稿盖回去。
//!
//! `SecretField` 的明文只在本文件里读 (输入框私有, 外面只能拿到 `Secret` 或掩码后的显示文本)。
//! 显示直接读输入框, 不经 `Secret` 的明文出口, 所以本文件不在 `secret.rs::EXPOSE_ALLOWLIST` 里。

use ratatui::crossterm::event::{Event, KeyEvent};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::secret::Secret;

/// 能接收文本编辑按键的字段——向导「按焦点找到那一格、交给它处理」那条统一路径只认这个。
pub trait TextInput {
    /// 处理一个按键, 返回**值**是否改变 (只移动光标返回 `false`)。
    fn handle(&mut self, key: KeyEvent) -> bool;

    /// 切换明文 / 掩码显示。只有 [`SecretField`] 有这个开关, 普通文本字段什么都不做。
    fn toggle_reveal(&mut self) {}
}

/// 普通文本字段。`PartialEq` 只比较值 (光标位置不是草稿内容)。
#[derive(Clone, Default)]
pub struct TextField {
    input: Input,
}

impl TextField {
    pub fn new(value: impl Into<String>) -> Self {
        Self { input: Input::new(value.into()) }
    }

    pub fn value(&self) -> &str {
        self.input.value()
    }

    /// 整体替换值, 光标放到末尾 (与用户刚打完这段文字时一致)。
    pub fn set(&mut self, value: impl Into<String>) {
        *self = Self::new(value);
    }

    /// 光标在值里的**显示列**偏移 (宽字符占两列), 交给 `FormRow::Field::cursor`。
    pub fn visual_cursor(&self) -> usize {
        self.input.visual_cursor()
    }
}

impl TextInput for TextField {
    fn handle(&mut self, key: KeyEvent) -> bool {
        self.input.handle_event(&Event::Key(key)).is_some_and(|changed| changed.value)
    }
}

impl PartialEq for TextField {
    fn eq(&self, other: &Self) -> bool {
        self.value() == other.value()
    }
}

impl Eq for TextField {}

impl std::fmt::Debug for TextField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TextField").field(&self.value()).finish()
    }
}

/// API Key 一类的敏感文本字段: 编辑行为同 [`TextField`], 外加 `Ctrl+R` 切换的明文显示开关。
/// `Debug` 与 [`Secret`] 一样只印长度 (草稿派生了 `Debug`, 不能从这里漏出明文); `PartialEq` 只比较值。
#[derive(Clone, Default)]
pub struct SecretField {
    input: TextField,
    reveal: bool,
}

impl SecretField {
    /// 只有测试需要直接造一个有值的 (界面上它总是从空值开始, 靠按键填)。
    #[cfg(test)]
    pub fn new(value: impl Into<String>) -> Self {
        Self { input: TextField::new(value), reveal: false }
    }

    /// 发请求时用的值: 去掉首尾空白 (粘贴常带进来, 不该落库), 按需生成, 草稿里不另存一份。
    pub fn secret(&self) -> Secret {
        Secret::new(self.input.value().trim())
    }

    /// 与 [`SecretField::secret`] 同一个口径: 只有空白也算空——校验什么就发送什么。
    pub fn is_empty(&self) -> bool {
        self.input.value().trim().is_empty()
    }

    pub fn visual_cursor(&self) -> usize {
        self.input.visual_cursor()
    }

    /// 界面上显示 API Key 的**唯一**出口: 没按 `Ctrl+R` 时是掩码, 按了是明文。
    ///
    /// 显示的是**输入框里的原样文本** (含首尾空白), 不是 `secret()` 那份 trim 过的值: 表单的光标 /
    /// 横向滚动按输入框的 `visual_cursor()` 算 (`widgets::form`), 显示文本必须与它逐字对齐。
    ///
    /// **掩码逐字一个点、不封顶**: 一旦掩码文本比明文短, 光标就会飞到掩码串右边的空白里——超长
    /// key (Anthropic 的约 108 字符) 封顶后看起来像没填, 用户会以为粘贴失败再粘一次, 内容被拼成
    /// 两份。这里要的是「挡住肉眼」而不是「隐藏长度」(表单正在编辑一条还没保存的 key, 长度泄露不是
    /// 这个场景的威胁模型)。
    pub fn display(&self) -> String {
        let plain = self.input.value();
        if self.reveal { plain.to_string() } else { "•".repeat(plain.chars().count()) }
    }
}

impl TextInput for SecretField {
    fn handle(&mut self, key: KeyEvent) -> bool {
        self.input.handle(key)
    }

    fn toggle_reveal(&mut self) {
        self.reveal = !self.reveal;
    }
}

impl PartialEq for SecretField {
    fn eq(&self, other: &Self) -> bool {
        self.input == other.input
    }
}

impl Eq for SecretField {}

impl std::fmt::Debug for SecretField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretField({:?}, reveal: {})", self.secret(), self.reveal)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn set_replaces_the_value_and_puts_the_cursor_at_the_end() {
        let mut f = TextField::new("abc");
        assert_eq!(f.value(), "abc");
        f.set("中转站");
        assert_eq!(f.value(), "中转站");
        assert_eq!(f.visual_cursor(), 6, "光标应该在末尾 (三个宽字符 = 6 列)");
        // 接着打字应该追加在末尾, 不是插在旧光标位置。
        f.handle(key(KeyCode::Char('X')));
        assert_eq!(f.value(), "中转站X");
    }

    #[test]
    fn handle_reports_whether_the_value_changed() {
        let mut f = TextField::default();
        assert!(f.handle(key(KeyCode::Char('a'))), "打字改变了值");
        assert!(!f.handle(key(KeyCode::Left)), "只移动光标不算改变");
        assert!(!f.handle(key(KeyCode::Right)), "只移动光标不算改变");
        assert!(!f.handle(key(KeyCode::Delete)), "光标在末尾, 删除什么也没改");
        assert!(f.handle(key(KeyCode::Backspace)), "退格删掉了 a");
        assert_eq!(f.value(), "");
        assert!(!f.handle(key(KeyCode::Backspace)), "空值再退格不算改变");
    }

    #[test]
    fn text_fields_compare_by_value_only() {
        let mut moved = TextField::new("abc");
        moved.handle(key(KeyCode::Left));
        assert_eq!(moved, TextField::new("abc"), "光标位置不同不影响相等");
        assert_ne!(TextField::new("abc"), TextField::new("abd"));
    }

    #[test]
    fn secret_field_display_masks_unless_revealed() {
        let mut key_field = SecretField::new("sk-test");
        assert_eq!(key_field.display(), "•".repeat(7));
        key_field.toggle_reveal();
        assert_eq!(key_field.display(), "sk-test");
        key_field.toggle_reveal();
        assert_eq!(key_field.display(), "•".repeat(7));
    }

    /// 掩码不能封顶 —— 否则超长 key (Anthropic 实测约 108 字符) 掩码后比明文短, 靠明文
    /// `visual_cursor()` 算的光标会飞到掩码串右边的空白里。
    #[test]
    fn secret_field_display_masks_without_a_length_cap() {
        let long_key = "x".repeat(108);
        let mut field = SecretField::new(long_key.clone());
        let masked = field.display();
        assert_eq!(masked.chars().count(), 108, "掩码应该逐字对应明文长度, 不能封顶");
        field.toggle_reveal();
        assert_eq!(field.display(), long_key);
    }

    /// 草稿派生了 `Debug`, `SecretField` 的 `Debug` 不能漏出明文 (与 `Secret` 同一条纪律)。
    #[test]
    fn secret_field_debug_never_prints_the_plaintext() {
        let mut field = SecretField::new("sk-abcdef");
        field.toggle_reveal(); // 明文显示开关不影响 Debug
        let out = format!("{field:?}");
        assert!(!out.contains("sk-"), "{out}");
        assert!(out.contains("9 chars"), "{out}");
    }

    /// 发出去的值与判空都按 trim 后的口径 (粘贴带进来的首尾空白不该落库); 显示仍是输入框原样,
    /// 否则掩码与光标对不齐。
    #[test]
    fn secret_field_sends_and_validates_the_trimmed_value_but_displays_the_raw_one() {
        let mut field = SecretField::new("  sk-test\t");
        assert_eq!(field.secret(), Secret::new("sk-test"));
        assert!(!field.is_empty());
        assert_eq!(field.display().chars().count(), 10, "掩码应该逐字对应输入框里的原样文本");
        field.toggle_reveal();
        assert_eq!(field.display(), "  sk-test\t");

        assert!(SecretField::new("   ").is_empty(), "只有空白也算空");
    }

    #[test]
    fn secret_field_edits_like_a_text_field() {
        let mut field = SecretField::default();
        assert!(field.is_empty());
        assert!(field.handle(key(KeyCode::Char('s'))));
        assert!(field.handle(key(KeyCode::Char('k'))));
        assert!(!field.handle(key(KeyCode::Left)));
        assert_eq!(field.secret(), Secret::new("sk"));
        assert!(!field.is_empty());
    }
}
