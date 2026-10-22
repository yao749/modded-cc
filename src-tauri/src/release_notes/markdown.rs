//! 发版说明 Markdown 子集的解析器, 规则见 spec §3.2。
//! 刻意严格: 不认识的语法一律报错并带行号, 由 release_notes 的内嵌守卫测试在 cargo test 阶段拦住,
//! 这样前端只需按结构渲染, 不需要 (也不允许) 注入 HTML。

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inline {
    Text { text: String },
    Bold { text: String },
    Code { text: String },
    Link { text: String, url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub text: Vec<Inline>,
    /// 二级列表项, 只允许一层嵌套
    pub children: Vec<Vec<Inline>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    pub heading: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NotesDoc {
    /// 第一个 `## ` 之前的段落, 一行一段
    pub summary: Vec<Vec<Inline>>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1 起算; 0 表示不针对某一行 (整份文件的问题)
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "第 {} 行: {}", self.line, self.message)
        }
    }
}

pub fn parse(src: &str) -> Result<NotesDoc, ParseError> {
    let mut doc = NotesDoc::default();
    // str::lines 同时去掉 "\n" 与 "\r\n", CRLF 文件与 LF 等价
    for (idx, raw) in src.lines().enumerate() {
        let line = idx + 1;
        let err = |m: &str| ParseError { line, message: m.to_string() };
        if raw.trim().is_empty() {
            continue;
        }
        if let Some(rest) = raw.strip_prefix("## ") {
            let heading = rest.trim();
            if heading.is_empty() {
                return Err(err("分节标题为空"));
            }
            doc.sections.push(Section { heading: heading.to_string(), items: Vec::new() });
            continue;
        }
        if let Some(rest) = raw.strip_prefix("  - ") {
            let item = doc
                .sections
                .last_mut()
                .and_then(|s| s.items.last_mut())
                .ok_or_else(|| err("二级列表项前面没有一级列表项"))?;
            item.children.push(inlines(rest.trim(), line)?);
            continue;
        }
        if let Some(rest) = raw.strip_prefix("- ") {
            let section = doc
                .sections
                .last_mut()
                .ok_or_else(|| err("列表项必须写在 `## 分节` 下面"))?;
            section.items.push(Item { text: inlines(rest.trim(), line)?, children: Vec::new() });
            continue;
        }
        if raw.trim_end() == "-" {
            return Err(err("列表项为空"));
        }
        if raw.starts_with(char::is_whitespace) {
            return Err(err("不支持的缩进: 二级列表项用两个空格加 `- `, 只允许一层嵌套"));
        }
        if raw.starts_with('#') {
            return Err(err("标题只支持 `## `"));
        }
        if raw.starts_with('>') {
            return Err(err("不支持引用"));
        }
        if raw.starts_with('|') {
            return Err(err("不支持表格"));
        }
        if raw.starts_with("* ") || raw.starts_with("+ ") {
            return Err(err("列表项请用 `- `"));
        }
        if let Some((n, _)) = raw.split_once(". ") {
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
                return Err(err("不支持有序列表"));
            }
        }
        if !doc.sections.is_empty() {
            return Err(err("分节里只能写列表项; 段落只能出现在第一个 `## ` 之前 (摘要)"));
        }
        doc.summary.push(inlines(raw.trim(), line)?);
    }
    if doc.summary.is_empty() && doc.sections.is_empty() {
        return Err(ParseError { line: 0, message: "内容为空".into() });
    }
    if let Some(s) = doc.sections.iter().find(|s| s.items.is_empty()) {
        return Err(ParseError { line: 0, message: format!("分节「{}」下没有列表项", s.heading) });
    }
    Ok(doc)
}

/// 行内语法: `**粗体**`、`` `代码` ``、`[文字](http(s)://…)`; 其余字符原样。
/// 单个 `*` 是普通字符 (如 `x-stainless-*`); `<字母` 或 `</` 视为 HTML 报错。
fn inlines(s: &str, line: usize) -> Result<Vec<Inline>, ParseError> {
    let err = |message: String| ParseError { line, message };
    let mut out = Vec::new();
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut Vec<Inline>| {
        if !text.is_empty() {
            out.push(Inline::Text { text: std::mem::take(text) });
        }
    };
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if let Some(body) = rest.strip_prefix("**") {
            let end = body.find("**").ok_or_else(|| err("`**` 没有闭合".into()))?;
            if end == 0 {
                return Err(err("粗体内容为空".into()));
            }
            flush(&mut text, &mut out);
            out.push(Inline::Bold { text: body[..end].to_string() });
            rest = &body[end + 2..];
        } else if let Some(body) = rest.strip_prefix('`') {
            let end = body.find('`').ok_or_else(|| err("反引号没有闭合".into()))?;
            if end == 0 {
                return Err(err("代码内容为空".into()));
            }
            flush(&mut text, &mut out);
            out.push(Inline::Code { text: body[..end].to_string() });
            rest = &body[end + 1..];
        } else if rest.starts_with("![") {
            return Err(err("不支持图片".into()));
        } else if c == '[' {
            let close = rest.find("](").ok_or_else(|| err("`[` 后面缺少 `](链接)`".into()))?;
            let label = &rest[1..close];
            let after = &rest[close + 2..];
            // Find closing ')' with balanced parenthesis matching
            let mut depth = 0;
            let mut end = None;
            for (i, ch) in after.char_indices() {
                if ch == '(' {
                    depth += 1;
                } else if ch == ')' {
                    if depth == 0 {
                        end = Some(i);
                        break;
                    } else {
                        depth -= 1;
                    }
                }
            }
            let end = end.ok_or_else(|| err("链接缺少 `)`".into()))?;
            let url = &after[..end];
            if label.is_empty() {
                return Err(err("链接文字为空".into()));
            }
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(err(format!("链接只允许 http(s): {url}")));
            }
            flush(&mut text, &mut out);
            out.push(Inline::Link { text: label.to_string(), url: url.to_string() });
            rest = &after[end + 1..];
        } else if c == '<' && rest[1..].starts_with(|n: char| n.is_ascii_alphabetic() || n == '/') {
            return Err(err("不支持 HTML".into()));
        } else {
            text.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    flush(&mut text, &mut out);
    if out.is_empty() {
        return Err(err("列表项为空".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Inline {
        Inline::Text { text: s.into() }
    }

    #[test]
    fn parses_summary_sections_items_and_children() {
        let src = "摘要第一段。\n\n第二段。\n\n## 新功能\n- **外观**：换新\n  - 子项 A\n  - 子项 B\n- 第二条\n\n## 修复\n- 修了\n";
        let doc = parse(src).unwrap();
        assert_eq!(doc.summary, vec![vec![t("摘要第一段。")], vec![t("第二段。")]]);
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].heading, "新功能");
        let first = &doc.sections[0].items[0];
        assert_eq!(first.text, vec![Inline::Bold { text: "外观".into() }, t("：换新")]);
        assert_eq!(first.children, vec![vec![t("子项 A")], vec![t("子项 B")]]);
        assert_eq!(doc.sections[0].items[1].text, vec![t("第二条")]);
        assert_eq!(doc.sections[1].items.len(), 1);
    }

    #[test]
    fn parses_code_and_links_and_keeps_lone_asterisks() {
        let doc = parse("## A\n- 字段 `stop_reason` 见 [文档](https://ccrouter.app) 与 x-stainless-*\n").unwrap();
        assert_eq!(
            doc.sections[0].items[0].text,
            vec![
                t("字段 "),
                Inline::Code { text: "stop_reason".into() },
                t(" 见 "),
                Inline::Link { text: "文档".into(), url: "https://ccrouter.app".into() },
                t(" 与 x-stainless-*"),
            ]
        );
    }

    #[test]
    fn crlf_is_same_as_lf() {
        let lf = "摘要\n\n## A\n- 一\n  - 二\n";
        assert_eq!(parse(lf).unwrap(), parse(&lf.replace('\n', "\r\n")).unwrap());
    }

    #[test]
    fn parses_links_with_parentheses_in_url() {
        let doc = parse("## A\n- 见 [x](https://a.b/(c)) 完\n").unwrap();
        assert_eq!(
            doc.sections[0].items[0].text,
            vec![
                t("见 "),
                Inline::Link { text: "x".into(), url: "https://a.b/(c)".into() },
                t(" 完"),
            ]
        );
    }

    #[test]
    fn rejects_unclosed_link_with_parentheses() {
        assert_eq!(err_line("## A\n- [x](https://a.b/(c)\n"), 2);
    }

    #[test]
    fn parses_links_with_multibyte_chars_in_url() {
        let doc = parse("## 更新\n- [文档](https://例子.wiki/(例))\n").unwrap();
        assert_eq!(
            doc.sections[0].items[0].text,
            vec![Inline::Link { text: "文档".into(), url: "https://例子.wiki/(例)".into() }]
        );
    }

    #[test]
    fn parses_multibyte_chars_around_link() {
        let doc = parse("## A\n- 见 [x](https://a.b/中) 完\n").unwrap();
        assert_eq!(
            doc.sections[0].items[0].text,
            vec![
                t("见 "),
                Inline::Link { text: "x".into(), url: "https://a.b/中".into() },
                t(" 完"),
            ]
        );
    }

    fn err_line(src: &str) -> usize {
        parse(src).expect_err("应当报错").line
    }

    #[test]
    fn rejects_unsupported_syntax_with_line_numbers() {
        assert_eq!(err_line("## A\n- ![图](https://x/y.png)\n"), 2); // 图片 (行内 `![` 分支)
        assert_eq!(err_line("<img src=\"x\">\n"), 1); // HTML
        assert_eq!(err_line("### 三级标题\n"), 1);
        assert_eq!(err_line("# 一级标题\n"), 1);
        assert_eq!(err_line("## A\n1. 有序\n"), 2);
        assert_eq!(err_line("## A\n- 一\n  - 二\n    - 三\n"), 4); // 三层嵌套
        assert_eq!(err_line("## A\n- 一\n段落不许出现在分节里\n"), 3);
        assert_eq!(err_line("- 分节之前的列表项\n"), 1);
        assert_eq!(err_line("  - 没有父项的二级项\n"), 1);
        assert_eq!(err_line("## A\n- **没闭合\n"), 2);
        assert_eq!(err_line("## A\n- `没闭合\n"), 2);
        assert_eq!(err_line("## A\n- [x](javascript:alert(1))\n"), 2);
        assert_eq!(err_line("## A\n- \n"), 2); // 空列表项
        assert_eq!(err_line("> 引用\n"), 1);
        assert_eq!(err_line("| 表 | 格 |\n"), 1);
        assert_eq!(err_line("* 星号列表\n"), 1);
    }

    #[test]
    fn rejects_empty_doc_and_empty_section() {
        assert_eq!(parse("\n\n").unwrap_err().line, 0);
        // version:set 生成的骨架 (只有标题) 必须报错, 提醒发版前填内容
        assert!(parse("## 新功能\n\n## 修复\n").is_err());
    }
}
