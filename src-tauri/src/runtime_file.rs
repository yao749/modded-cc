//! `app_data_dir/runtime.json`: 本次进程的运行事实, 给同机的 cc-router-tui 读。
//!
//! 与 settings.json 分开: 那个是用户配置, 这个每次启动覆盖重写。
//! `local_secret` 只存在于进程内存和这个文件, 见 docs/superpowers/specs/2026-09-17-tui-design.md §3.2。

use std::path::Path;

use serde::Serialize;

use crate::error::AppResult;

pub const FILE_NAME: &str = "runtime.json";

#[derive(Clone, Serialize)]
pub struct RuntimeFile {
    pub pid: u32,
    pub app_version: String,
    pub http_port: Option<u16>,
    pub https_port: Option<u16>,
    pub ca_pem_path: Option<String>,
    pub local_secret: String,
    /// `tauri_plugin_os::locale()` 的原始标签 (如 `"zh-Hans-CN"`), 给 TUI 在「跟随系统」时用,
    /// 与托盘 `tray.rs::TrayLocale::resolve` 走同一条映射规则。省略而不是写 `null`——文件里
    /// 少一个字段比多一行 `"system_locale": null` 更容易看懂「这台机器探测不到系统语言」。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_locale: Option<String>,
    /// 写文件那一刻的 `settings.preferred_language` (`"system"` / `"zh"` / `"en"` / `"ja"`), 给 TUI
    /// 在连上之前 (「未运行 / 未启用」这类提示) 就能用上桌面端显式选的语言。只在写 runtime.json 时
    /// 取一次, 用户在 app 运行期间改语言不会重写这个文件——连上之后 TUI 以 `get_settings` 的实时值
    /// 为准, 这里只是连接前的近似。省略规则同 `system_locale`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_language: Option<String>,
}

impl std::fmt::Debug for RuntimeFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeFile")
            .field("pid", &self.pid)
            .field("app_version", &self.app_version)
            .field("http_port", &self.http_port)
            .field("https_port", &self.https_port)
            .field("ca_pem_path", &self.ca_pem_path)
            .field("local_secret", &"<redacted>")
            .field("system_locale", &self.system_locale)
            .field("preferred_language", &self.preferred_language)
            .finish()
    }
}

/// 32 字节随机数的 base64url (无 padding)。
///
/// 刻意不引 `rand`: 两个 UUIDv4 拼起来是 32 字节, 其中每个 UUID 有 6 个固定的版本 / 变体位,
/// 实际熵 244 bit, 对一个只在回环上校验的密钥绰绰有余。uuid 内部走 getrandom (OS CSPRNG)。
pub fn generate_secret() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

impl RuntimeFile {
    /// `ca_pem_path` 只在 HTTPS listener 起来时给: TUI 只有走 https 才需要信任本地 CA。
    /// `system_locale` 是外部探测好的原始标签 (通常是 `tauri_plugin_os::locale()`), 纯函数不
    /// 自己去读 OS——测试里到处在调它, 读 OS 会让映射规则没法在任何平台的 CI 上单测。
    pub fn new(
        app_data_dir: &Path,
        http_port: Option<u16>,
        https_port: Option<u16>,
        local_secret: &str,
        system_locale: Option<String>,
    ) -> Self {
        Self {
            pid: std::process::id(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            http_port,
            https_port,
            ca_pem_path: https_port
                .map(|_| crate::tls::ca_pem_path(app_data_dir).to_string_lossy().into_owned()),
            local_secret: local_secret.to_string(),
            system_locale,
            preferred_language: None,
        }
    }

    /// 单独一个方法而不是 `new` 的第六个位置参数: 它与 `system_locale` 同为 `Option<String>`,
    /// 并排的两个同类型位置参数写反了编译器也发现不了。
    pub fn with_preferred_language(mut self, preferred_language: Option<String>) -> Self {
        self.preferred_language = preferred_language;
        self
    }
}

/// 先以 0600 建临时文件写完, 再 rename 覆盖。
/// 直接 `fs::write` 再 chmod 会有一个「文件已含密钥但权限还是默认值」的窗口;
/// 而且覆盖一个已存在的宽权限文件时 `fs::write` 不会收紧权限。
pub fn write(app_data_dir: &Path, file: &RuntimeFile) -> AppResult<()> {
    use std::io::Write;

    let final_path = app_data_dir.join(FILE_NAME);
    let tmp_path = app_data_dir.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    let body = serde_json::to_vec_pretty(file)?;

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| -> AppResult<()> {
        let mut f = opts.open(&tmp_path)?;
        f.write_all(&body)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_43_chars_of_base64url_and_never_repeats() {
        let a = generate_secret();
        let b = generate_secret();
        // 32 字节 → base64url 无 padding = ceil(32 * 4 / 3) = 43 字符
        assert_eq!(a.len(), 43);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{a}");
        assert_ne!(a, b);
    }

    #[test]
    fn ca_path_is_present_only_when_https_listener_is_up() {
        let dir = std::path::Path::new("/data");
        let http_only = RuntimeFile::new(dir, Some(23456), None, "s", None);
        assert_eq!(http_only.ca_pem_path, None);
        let https = RuntimeFile::new(dir, None, Some(23457), "s", None);
        assert_eq!(
            https.ca_pem_path.as_deref(),
            Some(crate::tls::ca_pem_path(dir).to_string_lossy().as_ref())
        );
        assert_eq!(https.http_port, None);
        assert_eq!(https.local_secret, "s");
        assert_eq!(https.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn write_produces_parseable_json_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &RuntimeFile::new(dir.path(), Some(1), None, "first", None)).unwrap();
        write(dir.path(), &RuntimeFile::new(dir.path(), Some(2), None, "second", None)).unwrap();
        let raw = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["http_port"], 2);
        assert_eq!(v["local_secret"], "second");
        assert!(v["https_port"].is_null());
        // 临时文件不应残留
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != FILE_NAME)
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// `system_locale` 为 `None` 时整个键不出现 (探测不到系统语言, 不是「探测到了空字符串」);
    /// 有值时原样写出, 供 TUI 直接读。
    #[test]
    fn system_locale_is_omitted_when_absent_and_written_when_present() {
        let dir = std::path::Path::new("/data");
        let without = RuntimeFile::new(dir, Some(1), None, "s", None);
        let raw = serde_json::to_string(&without).unwrap();
        assert!(!raw.contains("system_locale"), "{raw}");

        let with = RuntimeFile::new(dir, Some(1), None, "s", Some("zh-Hans-CN".into()));
        let v: serde_json::Value = serde_json::to_value(&with).unwrap();
        assert_eq!(v["system_locale"], "zh-Hans-CN");
    }

    #[test]
    fn preferred_language_is_omitted_when_absent_and_written_when_present() {
        let dir = std::path::Path::new("/data");
        let without = RuntimeFile::new(dir, Some(1), None, "s", None);
        let raw = serde_json::to_string(&without).unwrap();
        assert!(!raw.contains("preferred_language"), "{raw}");

        let with = RuntimeFile::new(dir, Some(1), None, "s", Some("zh-Hans-CN".into())).with_preferred_language(Some("ja".into()));
        let v: serde_json::Value = serde_json::to_value(&with).unwrap();
        assert_eq!(v["preferred_language"], "ja");
        assert_eq!(v["system_locale"], "zh-Hans-CN");
    }

    #[cfg(unix)]
    #[test]
    fn file_is_owner_only_even_when_replacing_a_world_readable_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        // 模拟一个权限过宽的旧文件 (例如被用户手动 chmod 过)
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write(dir.path(), &RuntimeFile::new(dir.path(), Some(1), None, "s", None)).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
    }

    /// 字面量 `local_secret` 只允许出现在这几个文件里。这是一道「字段名」层面的栅栏:
    /// 它拦得住有人在 commands/ 或 DTO 里直接读 `state.local_secret`, 但拦不住先在白名单文件里
    /// 加一个不同名的访问器再从别处调用 —— 加访问器本身就应该在 review 里被质疑。
    #[test]
    fn local_secret_is_only_touched_by_allowlisted_files() {
        const ALLOWED: &[&str] = &[
            "runtime_file.rs",
            "state.rs",
            "lib.rs",
            "proxy/server.rs",
            "proxy/web/gate.rs",
            "proxy/web/auth.rs",
            "tui_contract.rs", // 仅 cfg(test): 验证 TUI 读得懂 runtime.json
        ];
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    out.push(p);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        let offenders: Vec<String> = files
            .iter()
            .filter(|p| std::fs::read_to_string(p).unwrap().contains("local_secret"))
            .map(|p| p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/"))
            .filter(|rel| !ALLOWED.contains(&rel.as_str()))
            .collect();
        assert!(offenders.is_empty(), "local_secret 出现在白名单之外: {offenders:?}");
    }

    #[test]
    fn settings_serialization_never_contains_the_secret_field() {
        let raw = serde_json::to_string(&crate::settings::model::Settings::default()).unwrap();
        assert!(!raw.contains("local_secret"), "{raw}");
    }

    #[test]
    fn debug_output_redacts_the_secret() {
        let f = RuntimeFile::new(
            std::path::Path::new("/data"),
            Some(1),
            None,
            "super-secret-value",
            Some("ja-JP".into()),
        );
        let shown = format!("{f:?}");
        assert!(!shown.contains("super-secret-value"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(shown.contains("http_port"), "{shown}");
        // system_locale 不是秘密, Debug 应该照常打印它的值。
        assert!(shown.contains("ja-JP"), "{shown}");
    }

    #[test]
    fn write_into_a_missing_directory_errors_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let f = RuntimeFile::new(&missing, Some(1), None, "s", None);
        assert!(write(&missing, &f).is_err());
        assert!(!missing.exists());
    }
}
