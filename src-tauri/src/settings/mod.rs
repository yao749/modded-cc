pub mod model;

use std::path::{Path, PathBuf};

use tokio::fs;
use tracing::warn;

use crate::error::AppResult;
use crate::settings::model::Settings;

fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

pub async fn load_or_default(app_data_dir: &Path) -> AppResult<Settings> {
    let path = settings_path(app_data_dir);
    if !path.exists() {
        // 全新安装 (或 Factory Reset 之后): 视为已看过当前版本的更新内容, 不弹窗。
        let default = Settings {
            last_seen_release_notes: Some(env!("CARGO_PKG_VERSION").to_string()),
            ..Settings::default()
        };
        save(app_data_dir, &default).await?;
        return Ok(default);
    }
    match fs::read_to_string(&path).await {
        Ok(raw) => match serde_json::from_str::<Settings>(&raw) {
            Ok(s) => Ok(s),
            Err(e) => {
                warn!(?e, "settings.json 解析失败, 使用默认值");
                Ok(Settings::default())
            }
        },
        Err(e) => {
            warn!(?e, "settings.json 读取失败, 使用默认值");
            Ok(Settings::default())
        }
    }
}

pub async fn save(app_data_dir: &Path, settings: &Settings) -> AppResult<()> {
    if !app_data_dir.exists() {
        fs::create_dir_all(app_data_dir).await?;
    }
    let path = settings_path(app_data_dir);
    let raw = serde_json::to_string_pretty(settings)?;
    fs::write(path, raw).await?;
    Ok(())
}

pub fn generate_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 首次启动或老用户升级到加 token 字段的版本时,auth_token 为空。
/// 在这里生成一个 32 字符 hex token 并立即持久化,这样下次启动会读到同一个 token。
pub async fn ensure_auth_token(app_data_dir: &Path, settings: &mut Settings) -> AppResult<()> {
    if settings.auth_token.is_empty() {
        settings.auth_token = generate_token();
        save(app_data_dir, settings).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_install_marks_current_release_notes_seen() {
        let dir = tempfile::tempdir().unwrap();
        let s = load_or_default(dir.path()).await.unwrap();
        assert_eq!(s.last_seen_release_notes.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        // 落盘的也是这个值, 第二次启动不会变回 None 而误弹
        let again = load_or_default(dir.path()).await.unwrap();
        assert_eq!(again.last_seen_release_notes.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[tokio::test]
    async fn upgrade_from_version_without_the_field_reads_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), r#"{"proxy_port": 23456}"#).unwrap();
        let s = load_or_default(dir.path()).await.unwrap();
        assert_eq!(s.last_seen_release_notes, None);
    }
}
