//! 找到正在运行的 cc-router: 数据目录规则 + 读 runtime.json。
//!
//! 数据目录必须与 Tauri 对 identifier 的解析结果一致, 由主 crate 的契约测试锁住
//! (`tui_contract::identifier_matches_tauri_conf`)。

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// 必须等于 src-tauri/tauri.conf.json 的 `identifier`。
pub const IDENTIFIER: &str = "com.cc-router.desktop";
pub const RUNTIME_FILE: &str = "runtime.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Linux,
    Windows,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// **用户可见文字一律走 `i18n::client_error`, 不看这里的 `Display`**——这个 derive 现在是英文
/// 开发者文字, 只给日志 / `Debug` 场景用。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiscoveryError {
    #[error("cannot determine data directory: environment variable {0} is not set")]
    MissingEnv(&'static str),
    #[error("not found: {0}")]
    NoRuntimeFile(PathBuf),
    #[error("{0} is corrupt: {1}")]
    Corrupt(PathBuf, String),
    #[error("{0} has no usable port")]
    NoPort(PathBuf),
}

/// `env` 注入是为了测试; 生产传 `|k| std::env::var(k).ok()`。空字符串视同未设置。
pub fn default_data_dir(
    platform: Platform,
    env: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf, DiscoveryError> {
    let get = |k: &'static str| env(k).filter(|v| !v.is_empty());
    let need = |k: &'static str| get(k).ok_or(DiscoveryError::MissingEnv(k));
    let base = match platform {
        Platform::MacOs => PathBuf::from(need("HOME")?).join("Library/Application Support"),
        Platform::Windows => PathBuf::from(need("APPDATA")?),
        Platform::Linux => match get("XDG_DATA_HOME") {
            Some(x) => PathBuf::from(x),
            None => PathBuf::from(need("HOME")?).join(".local/share"),
        },
    };
    Ok(base.join(IDENTIFIER))
}

/// runtime.json 的视图。未知字段忽略, 这样 app 先升级加字段时旧 TUI 不会读不了。
#[derive(Clone, Deserialize, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub pid: u32,
    pub app_version: String,
    pub http_port: Option<u16>,
    pub https_port: Option<u16>,
    pub ca_pem_path: Option<String>,
    pub local_secret: String,
    /// 桌面端 `tauri_plugin_os::locale()` 的原始标签 (如 `"zh-Hans-CN"`), 用来在「跟随系统」时
    /// 与桌面端/托盘选出同一种语言。`#[serde(default)]`: 旧版桌面端写的 runtime.json 没有这个
    /// 字段, 缺省即 `None` (退回环境变量探测)。
    #[serde(default)]
    pub system_locale: Option<String>,
    /// 桌面端写 runtime.json 那一刻的 `settings.preferred_language`, 只给连接**之前**的提示用
    /// (连上之后以 `get_settings` 的实时值为准)。缺省规则同 `system_locale`。
    #[serde(default)]
    pub preferred_language: Option<String>,
}

/// 手写 Debug: 密钥不能进日志 / panic 信息, 但 `system_locale` 不是秘密, 照常打印。
impl std::fmt::Debug for RuntimeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeInfo")
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

impl RuntimeInfo {
    /// 优先 HTTP (不需要处理证书); 只开了 HTTPS 时才走 https。
    pub fn base_url(&self) -> Option<String> {
        match (self.http_port, self.https_port) {
            (Some(p), _) => Some(format!("http://127.0.0.1:{p}")),
            (None, Some(p)) => Some(format!("https://127.0.0.1:{p}")),
            (None, None) => None,
        }
    }
}

pub fn read_runtime(data_dir: &Path) -> Result<RuntimeInfo, DiscoveryError> {
    let path = data_dir.join(RUNTIME_FILE);
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return Err(DiscoveryError::NoRuntimeFile(path)),
    };
    let info: RuntimeInfo =
        serde_json::from_str(&raw).map_err(|e| DiscoveryError::Corrupt(path.clone(), e.to_string()))?;
    if info.base_url().is_none() {
        return Err(DiscoveryError::NoPort(path));
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn data_dir_follows_tauri_rules_per_platform() {
        assert_eq!(
            default_data_dir(Platform::MacOs, env_of(&[("HOME", "/Users/u")])).unwrap(),
            PathBuf::from("/Users/u/Library/Application Support/com.cc-router.desktop")
        );
        assert_eq!(
            default_data_dir(Platform::Linux, env_of(&[("HOME", "/home/u")])).unwrap(),
            PathBuf::from("/home/u/.local/share/com.cc-router.desktop")
        );
        assert_eq!(
            default_data_dir(Platform::Linux, env_of(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/x")])).unwrap(),
            PathBuf::from("/x/com.cc-router.desktop")
        );
        assert_eq!(
            default_data_dir(Platform::Windows, env_of(&[("APPDATA", "C:\\Users\\u\\AppData\\Roaming")])).unwrap(),
            PathBuf::from("C:\\Users\\u\\AppData\\Roaming").join("com.cc-router.desktop")
        );
    }

    #[test]
    fn empty_env_value_counts_as_unset() {
        assert_eq!(
            default_data_dir(Platform::Linux, env_of(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "")])).unwrap(),
            PathBuf::from("/home/u/.local/share/com.cc-router.desktop")
        );
        assert_eq!(
            default_data_dir(Platform::MacOs, env_of(&[("HOME", "")])),
            Err(DiscoveryError::MissingEnv("HOME"))
        );
    }

    const SAMPLE: &str = r#"{"pid":7,"app_version":"5.1.0","http_port":23456,"https_port":null,
        "ca_pem_path":null,"local_secret":"abc","some_future_field":true}"#;

    #[test]
    fn reads_runtime_file_and_ignores_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(RUNTIME_FILE), SAMPLE).unwrap();
        let info = read_runtime(dir.path()).unwrap();
        assert_eq!(info.pid, 7);
        assert_eq!(info.base_url().as_deref(), Some("http://127.0.0.1:23456"));
    }

    #[test]
    fn https_only_runtime_uses_https() {
        let info: RuntimeInfo = serde_json::from_str(
            r#"{"pid":1,"app_version":"x","http_port":null,"https_port":23457,"ca_pem_path":"/d/tls/ca.pem","local_secret":"s"}"#,
        )
        .unwrap();
        assert_eq!(info.base_url().as_deref(), Some("https://127.0.0.1:23457"));
    }

    #[test]
    fn missing_corrupt_and_portless_files_are_distinct_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RUNTIME_FILE);
        assert_eq!(read_runtime(dir.path()), Err(DiscoveryError::NoRuntimeFile(path.clone())));
        std::fs::write(&path, "{not json").unwrap();
        assert!(matches!(read_runtime(dir.path()), Err(DiscoveryError::Corrupt(..))));
        std::fs::write(&path, r#"{"pid":1,"app_version":"x","http_port":null,"https_port":null,"ca_pem_path":null,"local_secret":"s"}"#).unwrap();
        assert_eq!(read_runtime(dir.path()), Err(DiscoveryError::NoPort(path)));
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let info: RuntimeInfo = serde_json::from_str(SAMPLE).unwrap();
        let shown = format!("{info:?}");
        assert!(!shown.contains("abc"), "{shown}");
        assert!(shown.contains("<redacted>"));
    }

    /// 旧版桌面端写的 runtime.json 没有 `system_locale` 字段, 必须仍然解析成功
    /// (缺省为 `None`), 否则升级后 TUI 会突然连不上运行中的旧 app。
    #[test]
    fn runtime_file_without_system_locale_still_parses() {
        let info: RuntimeInfo = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(info.system_locale, None);
    }

    #[test]
    fn runtime_file_with_system_locale_parses() {
        let raw = r#"{"pid":7,"app_version":"5.1.0","http_port":23456,"https_port":null,
            "ca_pem_path":null,"local_secret":"abc","system_locale":"zh-Hans-CN"}"#;
        let info: RuntimeInfo = serde_json::from_str(raw).unwrap();
        assert_eq!(info.system_locale.as_deref(), Some("zh-Hans-CN"));
        assert_eq!(info.preferred_language, None);
    }

    #[test]
    fn runtime_file_with_preferred_language_parses() {
        let raw = r#"{"pid":7,"app_version":"5.1.0","http_port":23456,"https_port":null,
            "ca_pem_path":null,"local_secret":"abc","preferred_language":"ja"}"#;
        let info: RuntimeInfo = serde_json::from_str(raw).unwrap();
        assert_eq!(info.preferred_language.as_deref(), Some("ja"));
    }
}
