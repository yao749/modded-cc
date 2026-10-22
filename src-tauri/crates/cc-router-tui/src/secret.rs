//! API Key 一类的敏感文本。`Debug` 只印长度, 所以它可以安全地待在派生了 `Debug` 的 `Action` /
//! `Cmd` 里 (主循环、测试失败信息、将来任何 `{:?}` 都不会漏出明文)。读明文只有 `expose()` 一个
//! 入口, 名字刻意刺眼, 且有源码扫描测试限制它的调用点 (见 `expose_is_only_called_in_allowlisted_files`)。

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// **唯一的明文出口。** 只允许出现在白名单文件里 (见本文件的
    /// `expose_is_only_called_in_allowlisted_files`)。
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            write!(f, "Secret(empty)")
        } else {
            write!(f, "Secret({} chars)", self.0.chars().count())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_plaintext() {
        // "sk-abcdef" 是 9 个字符——与本文件顶部文档注释里许诺的例子对齐。
        let out = format!("{:?}", Secret::new("sk-abcdef"));
        assert!(!out.contains("sk-"), "{out}");
        assert!(out.contains("9 chars"), "{out}");

        let empty = format!("{:?}", Secret::default());
        assert_eq!(empty, "Secret(empty)");
    }

    /// 照抄主 crate `runtime_file.rs::tests::local_secret_is_only_touched_by_allowlisted_files`
    /// 的写法: 遍历 `src/` 下的 `.rs`, 凡是出现 `expose(` 的文件名必须在白名单里——防止有人在
    /// 表单渲染 / 日志 / 别的随手一个地方直接读明文。白名单成员见各自的注释。
    ///
    /// - 白名单写**相对 `src/` 的路径** (`"client/dto.rs"`, 不是只写文件名), 与后端
    ///   `runtime_file.rs` 白名单写 `"proxy/server.rs"` 同一套规则; 只写文件名的条目永远不命中。
    /// - 匹配串是 `expose(` 而不是 `.expose(`: 后者认不出 UFCS 写法 `Secret::expose(&x)`。这样会
    ///   多抓到本文件自己的定义 / 文档, 但白名单本来就含 `secret.rs`。
    /// - `runtime.rs` **刻意不在**白名单里: `call_wizard` / `call_mutation` 调的是 `to_args()`, 自己
    ///   不读明文, 而它恰好是最容易不小心把明文拼进错误文本 / 日志的地方。
    /// - 白名单成员必须真的存在于 `src/` 下, 防止死条目。
    #[test]
    fn expose_is_only_called_in_allowlisted_files() {
        const EXPOSE_ALLOWLIST: [&str; 2] = [
            "secret.rs",     // 定义处与它自己的测试
            "client/dto.rs", // `CreateInput::to_args()` / `ProbeInput::to_args()`, 唯一把它变成线上 JSON 的地方
        ];

        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.filter_map(Result::ok) {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    out.push(p);
                }
            }
        }

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

        for member in EXPOSE_ALLOWLIST {
            assert!(src.join(member).is_file(), "白名单成员 {member:?} 在 src/ 下不存在, 是个死条目");
        }

        let mut files = Vec::new();
        walk(&src, &mut files);
        let offenders: Vec<String> = files
            .iter()
            .filter(|p| {
                // 读不了源码文件说明这条扫描规则没有真正覆盖全部文件——宁可 panic 也不要把它当
                // 「没有 expose(」悄悄放过 (fail-closed)。
                let text = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("读取 {p:?} 失败: {e}"));
                text.contains("expose(")
            })
            .filter_map(|p| p.strip_prefix(&src).ok().map(|rel| rel.to_string_lossy().replace('\\', "/")))
            .filter(|rel| !EXPOSE_ALLOWLIST.contains(&rel.as_str()))
            .collect();
        assert!(offenders.is_empty(), "expose( 出现在白名单之外: {offenders:?}");
    }
}
