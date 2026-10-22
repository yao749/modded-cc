//! 数字 / 时间 / 定宽文本的格式化。全部是纯函数。

use chrono::{DateTime, FixedOffset, Local, TimeZone};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::client::dto::Slot;
use crate::i18n::Strings;

/// 显示时间用的时区。生产 `Local` (系统时区, 按每个时间戳自己的偏移算, 夏令时正确); 测试用 `Fixed`,
/// 快照不随机器时区变化。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tz {
    Local,
    /// 相对 UTC 的偏移, 秒, 东正。
    Fixed(i32),
}

/// `ms` 按 `tz` 格式化成 `fmt`; 时间戳超出 chrono 可表示范围或 `Fixed` 偏移非法时返回 `None`。
/// `Tz::Local` 每次都重新查一次偏移 (不缓存), 保证跨 DST 边界的时间戳偏移正确。
fn try_format(ms: i64, tz: Tz, fmt: &str) -> Option<String> {
    match tz {
        Tz::Local => Local.timestamp_millis_opt(ms).single().map(|dt| dt.format(fmt).to_string()),
        Tz::Fixed(offset_secs) => {
            let offset = FixedOffset::east_opt(offset_secs)?;
            let utc = DateTime::from_timestamp_millis(ms)?;
            Some(utc.with_timezone(&offset).format(fmt).to_string())
        }
    }
}

/// 同上, 但只要本地日历日 (给 `short_stamp` 比较用)。
fn try_local_date(ms: i64, tz: Tz) -> Option<chrono::NaiveDate> {
    match tz {
        Tz::Local => Local.timestamp_millis_opt(ms).single().map(|dt| dt.date_naive()),
        Tz::Fixed(offset_secs) => {
            let offset = FixedOffset::east_opt(offset_secs)?;
            let utc = DateTime::from_timestamp_millis(ms)?;
            Some(utc.with_timezone(&offset).date_naive())
        }
    }
}

/// `"14:02:31"`。超出可表示范围 (或 `Fixed` 偏移非法) 时返回 `"—"`, 不 panic。
pub fn clock(ms: i64, tz: Tz) -> String {
    try_format(ms, tz, "%H:%M:%S").unwrap_or_else(|| "—".into())
}

/// 与 `now_ms` 同一本地日 (按 `tz` 的日历日比较) → `"14:02:31"`; 否则 `"09-18 14:02"`。两种都 ≤ 11 列。
pub fn short_stamp(ms: i64, now_ms: i64, tz: Tz) -> String {
    match (try_local_date(ms, tz), try_local_date(now_ms, tz)) {
        (Some(d), Some(now_d)) if d == now_d => clock(ms, tz),
        (Some(_), Some(_)) => try_format(ms, tz, "%m-%d %H:%M").unwrap_or_else(|| "—".into()),
        _ => "—".into(),
    }
}

/// `"2026-09-19 14:02:31"`。超出可表示范围 (或 `Fixed` 偏移非法) 时返回 `"—"`, 不 panic。
pub fn full_stamp(ms: i64, tz: Tz) -> String {
    try_format(ms, tz, "%Y-%m-%d %H:%M:%S").unwrap_or_else(|| "—".into())
}

/// 耗时: `< 1000` → `"850ms"`; `< 60_000` → 一位小数 `"1.8s"`; 其余 `"2m05s"`; 负数按 0。
pub fn duration(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        // 截断到 0.1 秒, 不用 `{:.1}`: 它会四舍五入, 把 59_950ms 印成 "60.0s", 而 60_000ms 起
        // 走的分钟分支是截断的, 两个分支的舍入方向必须一致, 否则这一档会跳到一个下一个分支
        // 永远不会出现的值 (P4 终审 Minor)。
        let tenths = ms / 100;
        format!("{}.{}s", tenths / 10, tenths % 10)
    } else {
        let total_secs = ms / 1000;
        format!("{}m{:02}s", total_secs / 60, total_secs % 60)
    }
}

/// `1284` → `1,284`
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `3_200_000` → `3.2M`; 小于 1000 原样。
pub fn compact(n: i64) -> String {
    let abs = n.unsigned_abs() as f64;
    let (div, unit) = match abs {
        a if a >= 1e9 => (1e9, "B"),
        a if a >= 1e6 => (1e6, "M"),
        a if a >= 1e3 => (1e3, "K"),
        _ => return n.to_string(),
    };
    format!("{}{:.1}{unit}", if n < 0 { "-" } else { "" }, abs / div)
}

/// `98.64` → `98.6%`
pub fn percent(p: f64) -> String {
    format!("{p:.1}%")
}

/// 剩余毫秒 → `mm:ss`; 超过 99 分钟显示 `99:59`, 负数显示 `00:00`。
pub fn mmss(remaining_ms: i64) -> String {
    let secs = (remaining_ms.max(0) + 999) / 1000; // 向上取整: 还剩 0.2s 时显示 00:01 而不是 00:00
    let secs = secs.min(99 * 60 + 59);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// 一组文字里最宽的那个的显示宽度 (CJK 占两列)。定宽列按当前语言的实际文案推导宽度时用——
/// 英文普遍比中文宽, 写死成按中文量出来的列宽会把英文截成无意义的片段。
pub fn widest<'a>(texts: impl IntoIterator<Item = &'a str>) -> usize {
    texts.into_iter().map(UnicodeWidthStr::width).max().unwrap_or(0)
}

/// 按**显示宽度**截断或右补空格到恰好 `width` 列。CJK 字符占两列, 不能用 `{:<16}`。
/// 截断时以 `…` 结尾; 宽字符放不下时用空格补齐那一列。
pub fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return format!("{text}{}", " ".repeat(width - text.width()));
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
        used += 1;
    }
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

/// 按**显示宽度**折行 (CJK 占两列)。先按 `'\n'` 拆成若干段, 每段贪心装满 `width` 列, 断行点优先
/// 落在词边界: ASCII 空格处可断 (断点处的空格全部丢掉, 见 [`LineFill`]), 每个宽字符 (CJK) 前后都可断; 一个词
/// (连续的非空格窄字符) 放不进当前行时整个挪到下一行, 比一整行还宽的词退回按字符切 (接着当前行
/// 往下填)。空段折成一个空串; 单个字符比 `width` 还宽时单独成一行 (保证每轮至少前进一个字符,
/// 不会死循环)。`width == 0` 时只按 `'\n'` 拆、不折——`Popup::Detail` (`widgets/detail.rs`) 的字段值
/// 列宽是 `内宽 - LABEL_COL`, 内宽小到夹不住时会 `saturating_sub` 到 0, 这里必须是全函数 (不 panic、
/// 不死循环), 不依赖调用方保证 `width > 0`。
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for segment in text.split('\n') {
        if width == 0 || segment.is_empty() {
            out.push(segment.to_string());
            continue;
        }
        let before = out.len();
        let mut lines = LineFill { width, line: String::new(), used: 0, continuation: false, out: &mut out };
        for token in tokens(segment) {
            lines.push_token(token);
        }
        lines.finish();
        // 全是空格的段落: 每个空格都落在断点上会被逐个丢弃, 段落本身产不出任何一行。调用方
        // (`widgets/detail.rs` 的 `FieldFirst`) 靠"第一段的第一行"取标签行, 段落消失会连带把
        // 那一整行标签吞掉, 所以这里补一行空串, 保证非空段落至少产出一行。
        if out.len() == before {
            out.push(String::new());
        }
    }
    out
}

/// 折行的最小单位。
enum Token<'a> {
    /// 一个 ASCII 空格: 可断点。
    Space,
    /// 一个宽字符 (CJK 等): 前后都可断。
    Wide(char),
    /// 一段连续的非空格窄字符: 整体挪行, 比一行还宽才按字符切。
    Word(&'a str),
}

fn tokens(segment: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut word_start: Option<usize> = None;
    for (i, c) in segment.char_indices() {
        let breaks = c == ' ' || c.width().unwrap_or(0) > 1;
        if breaks {
            if let Some(start) = word_start.take() {
                out.push(Token::Word(&segment[start..i]));
            }
            out.push(if c == ' ' { Token::Space } else { Token::Wide(c) });
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(start) = word_start {
        out.push(Token::Word(&segment[start..]));
    }
    out
}

/// 往 `out` 里一行一行地填 (`wrap` 的内部状态)。
///
/// 空白规则: 折行处的空格 (不论连续几个) 全部丢掉——行尾的不留, 续行开头的也不带过去, 所以折行
/// 永远不会产出空行或只有空格的行。只有段首 (文本开头或显式 `'\n'` 之后) 的空格算缩进保留, 除非
/// 它们本身就落在了折行处——已知取舍: 缩进后的第一个词放不进这一行时, 缩进本身也跟着这次折行
/// 一起被丢弃 (`wrap_drops_every_space_at_a_break` 的最后一条断言), 不会把缩进带到下一行重新对齐。
struct LineFill<'o> {
    width: usize,
    line: String,
    used: usize,
    /// 当前行是折行产生的续行、且还没放进任何非空格内容: 此时来的空格直接丢。
    continuation: bool,
    out: &'o mut Vec<String>,
}

impl LineFill<'_> {
    fn push_token(&mut self, token: Token) {
        match token {
            Token::Space => {
                if self.continuation && self.line.is_empty() {
                    // 续行开头的空格属于上一个断点。
                } else if self.used + 1 > self.width {
                    // 行尾恰好落在空格上: 在这里断, 这个空格不带到下一行。
                    self.break_line();
                } else {
                    self.push_char(' ', 1);
                }
            }
            Token::Wide(c) => {
                let w = c.width().unwrap_or(0);
                if self.used > 0 && self.used + w > self.width {
                    self.break_line();
                }
                self.push_char(c, w);
            }
            Token::Word(word) => {
                let w = word.width();
                if self.used + w <= self.width {
                    self.push_str(word, w);
                } else if w <= self.width {
                    self.break_line();
                    self.push_str(word, w);
                } else {
                    for c in word.chars() {
                        let cw = c.width().unwrap_or(0);
                        if self.used > 0 && self.used + cw > self.width {
                            self.break_line();
                        }
                        self.push_char(c, cw);
                    }
                }
            }
        }
    }

    fn push_char(&mut self, c: char, w: usize) {
        self.line.push(c);
        self.used += w;
        if c != ' ' {
            self.continuation = false;
        }
    }

    fn push_str(&mut self, word: &str, w: usize) {
        self.line.push_str(word);
        self.used += w;
        self.continuation = false;
    }

    /// 折行: 丢掉行尾全部空格 (它们就是断点); 丢完什么都不剩就不出这一行。
    fn break_line(&mut self) {
        let kept = self.line.trim_end_matches(' ').len();
        self.line.truncate(kept);
        if !self.line.is_empty() {
            self.out.push(std::mem::take(&mut self.line));
        }
        self.used = 0;
        self.continuation = true;
    }

    /// 段尾: 折行之后什么都没剩 (比如段尾的空格恰好落在断点) 就不出空行; 没折过行的空段由 `wrap`
    /// 自己处理, 走不到这里。
    fn finish(self) {
        if !(self.continuation && self.line.is_empty()) {
            self.out.push(self.line);
        }
    }
}

/// `Slot` 的显示名: 四个主槽用英文原名 (与后端 `ModelSlots` 的字段名一致), `Fallback` 用现有的
/// `s.sub_slot_fallback` (中文「兜底」)。订阅详情页与向导 (槽位行、槽位选择器标题) 共用——两处对
/// 「槽位怎么叫」必须是同一个答案; 放在这里而不是任何一方的模块里, 免得一方依赖另一方。
pub(crate) fn slot_label(slot: Slot, s: &'static Strings) -> &'static str {
    match slot {
        Slot::Fable => "fable",
        Slot::Opus => "opus",
        Slot::Sonnet => "sonnet",
        Slot::Haiku => "haiku",
        Slot::Fallback => s.sub_slot_fallback,
        Slot::Jev => "jev",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_groups_by_three() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1284), "1,284");
        assert_eq!(thousands(1_000_000), "1,000,000");
        assert_eq!(thousands(-12345), "-12,345");
    }

    #[test]
    fn compact_picks_the_unit() {
        assert_eq!(compact(950), "950");
        assert_eq!(compact(3_200_000), "3.2M");
        assert_eq!(compact(15_300), "15.3K");
        assert_eq!(compact(2_000_000_000), "2.0B");
    }

    #[test]
    fn mmss_rounds_up_and_clamps() {
        assert_eq!(mmss(42_000), "00:42");
        assert_eq!(mmss(200), "00:01");
        assert_eq!(mmss(-5), "00:00");
        assert_eq!(mmss(61_000), "01:01");
        assert_eq!(mmss(10 * 3600 * 1000), "99:59");
    }

    #[test]
    fn fit_counts_display_width_not_chars() {
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("智谱主号", 10), "智谱主号  "); // 8 列 + 2 空格
        assert_eq!(fit("智谱主号备用", 8).width(), 8);
        assert_eq!(fit("智谱主号备用", 8), "智谱主… "); // 第 4 个字放不下, 省略号后补一格
        assert_eq!(fit("abcdefgh", 5), "abcd…");
        assert_eq!(fit("abc", 0), "");
    }

    #[test]
    fn wrap_breaks_by_display_width_and_keeps_newlines() {
        assert_eq!(wrap("abcdef", 4), vec!["abcd", "ef"]);
        assert_eq!(wrap("智谱主号", 5), vec!["智谱", "主号"]);
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("智", 1), vec!["智"]);
        assert_eq!(wrap("ab\ncd", 0), vec!["ab", "cd"]);
    }

    /// 英文按词折行: 断在空格处 (那个空格丢掉), 不从单词中间切开。
    #[test]
    fn wrap_breaks_english_at_spaces() {
        assert_eq!(wrap("the quick brown fox jumps", 10), vec!["the quick", "brown fox", "jumps"]);
        // 行尾正好落在空格上: 空格不带到下一行开头。
        assert_eq!(wrap("abcd efgh", 4), vec!["abcd", "efgh"]);
        // 没落在断点上的连续空白原样保留 (落在断点上的空格不论几个全部丢掉, 见
        // `wrap_drops_every_space_at_a_break`)。
        assert_eq!(wrap("ab  cd", 10), vec!["ab  cd"]);
    }

    /// 比一整行还宽的词 (URL、长串数字) 退回按字符切, 接着当前行往下填。
    #[test]
    fn wrap_char_splits_a_token_wider_than_the_line() {
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("see https://example.com/x", 10), vec!["see https:", "//example.", "com/x"]);
    }

    /// 纯中文仍按字符折行 (每个宽字符前后都是断点), 结果与只按字符切相同。
    #[test]
    fn wrap_keeps_cjk_char_breaking() {
        assert_eq!(wrap("智谱主号备用", 5), vec!["智谱", "主号", "备用"]);
        assert_eq!(wrap("已达到本分钟请求数上限", 8), vec!["已达到本", "分钟请求", "数上限"]);
    }

    /// 断点处的空格不论几个都丢掉, 不产出空行或只有空格的行。
    #[test]
    fn wrap_drops_every_space_at_a_break() {
        // 断点前后各有空格。
        assert_eq!(wrap("abcd  efgh", 4), vec!["abcd", "efgh"]);
        assert_eq!(wrap("abc   def", 3), vec!["abc", "def"]);
        // 段尾的空格恰好落在断点。
        assert_eq!(wrap("abc ", 3), vec!["abc"]);
        // 段首的空格落在断点上 (下一个词放不进这一行) 也一并丢掉。
        assert_eq!(wrap(" abcd", 4), vec!["abcd"]);
        assert_eq!(wrap("  abcdefghi", 10), vec!["abcdefghi"]);
    }

    /// 全是空格且比 `width` 还宽的段落: 每个空格都落在断点上被逐个丢掉, 修复前会产不出任何一行
    /// (`widgets/detail.rs::FieldFirst` 靠第一行取标签, 段落消失=标签行消失)。修复后至少一行空串。
    #[test]
    fn wrap_keeps_a_line_for_an_all_whitespace_paragraph() {
        assert_eq!(wrap("   ", 2), vec![""]);
        assert_eq!(wrap("a\n     \nb", 3), vec!["a", "", "b"]);
    }

    /// 段首 (文本开头或 `'\n'` 之后) 没落在断点上的空格是缩进, 保留; 续行开头的空格不保留。
    #[test]
    fn wrap_keeps_leading_indent_but_not_on_continuation_lines() {
        assert_eq!(wrap("  ab cd", 10), vec!["  ab cd"]);
        assert_eq!(wrap("x\n  ab", 10), vec!["x", "  ab"]);
        assert_eq!(wrap("  ab cd", 5), vec!["  ab", "cd"]);
    }

    /// 中英混排: 英文词整体挪行, 中文字之间照样可断。
    #[test]
    fn wrap_mixes_words_and_cjk() {
        assert_eq!(wrap("API Key 无效", 8), vec!["API Key", "无效"]);
        assert_eq!(wrap("API Key 无效", 6), vec!["API", "Key 无", "效"]);
        assert_eq!(wrap("上游返回 Too Many", 12), vec!["上游返回 Too", "Many"]);
    }

    /// `2023-11-15 06:13:20 +08:00` == `2023-11-14 22:13:20 UTC`。
    const NOW: i64 = 1_700_000_000_000;

    #[test]
    fn clock_and_stamps_use_the_given_offset() {
        assert_eq!(clock(NOW, Tz::Fixed(8 * 3600)), "06:13:20");
        assert_eq!(full_stamp(NOW, Tz::Fixed(8 * 3600)), "2023-11-15 06:13:20");

        assert_eq!(clock(NOW, Tz::Fixed(0)), "22:13:20");
        assert_eq!(full_stamp(NOW, Tz::Fixed(0)), "2023-11-14 22:13:20");
    }

    #[test]
    fn short_stamp_shows_the_date_only_for_other_days() {
        let tz = Tz::Fixed(8 * 3600);
        assert_eq!(short_stamp(NOW - 3_600_000, NOW, tz), "05:13:20", "同一天只显示时刻");
        assert_eq!(short_stamp(NOW - 7 * 3_600_000, NOW, tz), "11-14 23:13", "跨天要带上日期");
    }

    #[test]
    fn local_tz_formats_without_panicking() {
        assert_eq!(clock(NOW, Tz::Local).len(), 8, "无论机器时区, \"HH:MM:SS\" 都是 8 个字符");
    }

    #[test]
    fn out_of_range_timestamps_render_a_dash() {
        assert_eq!(clock(i64::MAX, Tz::Fixed(0)), "—");
    }

    #[test]
    fn duration_formats() {
        assert_eq!(duration(0), "0ms");
        assert_eq!(duration(850), "850ms");
        assert_eq!(duration(1_849), "1.8s");
        assert_eq!(duration(59_949), "59.9s");
        assert_eq!(duration(125_000), "2m05s");
        assert_eq!(duration(-5), "0ms");
    }

    #[test]
    fn duration_truncates_instead_of_rounding_at_the_minute_boundary() {
        assert_eq!(duration(59_949), "59.9s");
        assert_eq!(duration(59_950), "59.9s", "不能四舍五入成 60.0s —— 分钟分支永远不会产出这个值");
        assert_eq!(duration(59_999), "59.9s");
        assert_eq!(duration(60_000), "1m00s");
        // 既有行为不变
        assert_eq!(duration(999), "999ms");
        assert_eq!(duration(1_000), "1.0s");
        assert_eq!(duration(1_949), "1.9s");
        assert_eq!(duration(1_950), "1.9s");
    }
}
