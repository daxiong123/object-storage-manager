//! 应用设置（settings.json，`~/Library/Application Support/CloudStorage/`）。
//!
//! 红线：永不存 Secret（agents.md §6）——本文件只有 UI 偏好与数值配置。
//! 读写是整体快照（serde_json pretty），设置项极少（个位数），无并发写
//! 竞争面：UI 单点（SettingsModal 保存按钮）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{PersistenceError, default_data_dir};

/// 签名链接默认有效期（秒）。规范 §21：Signed URL。
pub const SIGNED_URL_TTL_DEFAULT: u64 = 3600;
/// 签名链接写入剪贴板后自动清除（秒）。0 = 不自动清除。
pub const CLIPBOARD_CLEAR_DEFAULT: u64 = 60;
pub const TRANSFER_CONCURRENCY_DEFAULT: u32 = 2;
pub const UI_FONT_SCALE_DEFAULT: f32 = 1.0;
pub const CODE_FONT_SIZE_DEFAULT: u32 = 13;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

fn default_transfer_concurrency() -> u32 {
    TRANSFER_CONCURRENCY_DEFAULT
}

fn default_ui_font_scale() -> f32 {
    UI_FONT_SCALE_DEFAULT
}

fn default_code_font_size() -> u32 {
    CODE_FONT_SIZE_DEFAULT
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// 签名 GET 链接有效期（秒）
    pub signed_url_ttl_secs: u64,
    /// 复制签名链接后自动清剪贴板（秒；0 = 关闭）
    pub clipboard_clear_secs: u64,
    /// 外观模式：默认跟随系统，可手动固定浅色/深色。
    #[serde(default)]
    pub appearance_mode: AppearanceMode,
    /// 界面字体族。None = 系统 UI 字体。
    #[serde(default)]
    pub ui_font_family: Option<String>,
    /// 界面字号缩放系数。
    #[serde(default = "default_ui_font_scale")]
    pub ui_font_scale: f32,
    /// 代码/技术字段字体族。None = macOS 默认等宽字体。
    #[serde(default)]
    pub code_font_family: Option<String>,
    /// 代码/技术字段字号。
    #[serde(default = "default_code_font_size")]
    pub code_font_size: u32,
    /// 传输并发上限。
    #[serde(default = "default_transfer_concurrency")]
    pub transfer_concurrency: u32,
    /// 默认下载目录。None = 使用 HOME 作为保存面板初始目录。
    #[serde(default)]
    pub default_download_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            signed_url_ttl_secs: SIGNED_URL_TTL_DEFAULT,
            clipboard_clear_secs: CLIPBOARD_CLEAR_DEFAULT,
            appearance_mode: AppearanceMode::System,
            ui_font_family: None,
            ui_font_scale: UI_FONT_SCALE_DEFAULT,
            code_font_family: None,
            code_font_size: CODE_FONT_SIZE_DEFAULT,
            transfer_concurrency: TRANSFER_CONCURRENCY_DEFAULT,
            default_download_dir: None,
        }
    }
}

impl Settings {
    /// 从 `~/Library/Application Support/CloudStorage/settings.json` 读取；
    /// 文件不存在 = 默认值（首次启动），损坏 = 显式报错（Fail Fast，不静默重置）。
    pub fn load() -> Result<Self, PersistenceError> {
        Self::load_at(settings_path()?)
    }

    /// 指定路径读取（测试用）。
    pub fn load_at(path: PathBuf) -> Result<Self, PersistenceError> {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(source) => {
                return Err(PersistenceError::SettingsIo { path, source });
            }
        };
        serde_json::from_str(&text).map_err(|e| PersistenceError::SettingsParse {
            path,
            message: e.to_string(),
        })
    }

    /// 写入默认位置。
    pub fn save(&self) -> Result<(), PersistenceError> {
        self.save_at(settings_path()?)
    }

    /// 指定路径写入（测试用）。
    pub fn save_at(&self, path: PathBuf) -> Result<(), PersistenceError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| PersistenceError::SettingsIo {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| PersistenceError::SettingsParse {
                path: path.clone(),
                message: e.to_string(),
            })?;
        std::fs::write(&path, text).map_err(|source| PersistenceError::SettingsIo { path, source })
    }

    /// 校验：TTL 必须 > 0；清除秒数 >= 0（0 = 关闭）。
    pub fn validate(&self) -> Result<(), String> {
        if self.signed_url_ttl_secs == 0 {
            return Err("签名链接有效期必须大于 0 秒".into());
        }
        if self.signed_url_ttl_secs > 7 * 24 * 3600 {
            return Err("签名链接有效期不能超过 7 天".into());
        }
        if self.clipboard_clear_secs > 24 * 3600 {
            return Err("剪贴板自动清除不能超过 24 小时".into());
        }
        if !(0.85..=1.40).contains(&self.ui_font_scale) {
            return Err("界面字号缩放必须在 0.85 到 1.40 之间".into());
        }
        if !(10..=24).contains(&self.code_font_size) {
            return Err("代码字体字号必须在 10 到 24 之间".into());
        }
        if !(1..=8).contains(&self.transfer_concurrency) {
            return Err("传输并发数必须在 1 到 8 之间".into());
        }
        if let Some(path) = &self.default_download_dir
            && !path.is_dir()
        {
            return Err(format!("默认下载目录不存在或不是目录：{}", path.display()));
        }
        Ok(())
    }
}

/// settings.json 路径（`~/Library/Application Support/CloudStorage/settings.json`）。
pub fn settings_path() -> Result<PathBuf, PersistenceError> {
    Ok(default_data_dir()?.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_default() {
        let path = std::env::temp_dir().join(format!(
            "cloudstorage-settings-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let settings = Settings::load_at(path).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-settings-rt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("settings.json");
        let settings = Settings {
            signed_url_ttl_secs: 600,
            clipboard_clear_secs: 0,
            appearance_mode: AppearanceMode::Dark,
            ui_font_family: Some("PingFang SC".into()),
            ui_font_scale: 1.15,
            code_font_family: Some("SF Mono".into()),
            code_font_size: 15,
            transfer_concurrency: 4,
            default_download_dir: Some(dir.clone()),
        };
        settings.save_at(path.clone()).unwrap();
        let loaded = Settings::load_at(path).unwrap();
        assert_eq!(loaded.signed_url_ttl_secs, 600);
        assert_eq!(loaded.clipboard_clear_secs, 0);
        assert_eq!(loaded.appearance_mode, AppearanceMode::Dark);
        assert_eq!(loaded.ui_font_family.as_deref(), Some("PingFang SC"));
        assert_eq!(loaded.ui_font_scale, 1.15);
        assert_eq!(loaded.code_font_family.as_deref(), Some("SF Mono"));
        assert_eq!(loaded.code_font_size, 15);
        assert_eq!(loaded.transfer_concurrency, 4);
        assert_eq!(loaded.default_download_dir.as_deref(), Some(dir.as_path()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_old_settings_file_fills_new_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-settings-old-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{
  "signed_url_ttl_secs": 900,
  "clipboard_clear_secs": 0
}"#,
        )
        .unwrap();

        let loaded = Settings::load_at(path).unwrap();
        assert_eq!(loaded.signed_url_ttl_secs, 900);
        assert_eq!(loaded.clipboard_clear_secs, 0);
        assert_eq!(loaded.appearance_mode, AppearanceMode::System);
        assert_eq!(loaded.ui_font_family, None);
        assert_eq!(loaded.ui_font_scale, UI_FONT_SCALE_DEFAULT);
        assert_eq!(loaded.code_font_family, None);
        assert_eq!(loaded.code_font_size, CODE_FONT_SIZE_DEFAULT);
        assert_eq!(loaded.transfer_concurrency, TRANSFER_CONCURRENCY_DEFAULT);
        assert_eq!(loaded.default_download_dir, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_corrupt_file_is_error_not_silent_reset() {
        let dir = std::env::temp_dir().join(format!(
            "cloudstorage-settings-bad-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{not json").unwrap();
        let err = Settings::load_at(path).unwrap_err();
        assert!(
            matches!(err, PersistenceError::SettingsParse { .. }),
            "损坏文件必须显式报错，实际 {err:?}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn validate_bounds() {
        let mut s = Settings::default();
        assert!(s.validate().is_ok());
        s.signed_url_ttl_secs = 0;
        assert!(s.validate().is_err());
        s.signed_url_ttl_secs = 8 * 24 * 3600;
        assert!(s.validate().is_err());
        s.signed_url_ttl_secs = 3600;
        s.clipboard_clear_secs = 25 * 3600;
        assert!(s.validate().is_err());
        // 0 = 关闭自动清除，合法
        s.clipboard_clear_secs = 0;
        assert!(s.validate().is_ok());
        s.ui_font_scale = 0.84;
        assert!(s.validate().is_err());
        s.ui_font_scale = 1.41;
        assert!(s.validate().is_err());
        s.ui_font_scale = 1.0;
        s.code_font_size = 9;
        assert!(s.validate().is_err());
        s.code_font_size = 25;
        assert!(s.validate().is_err());
        s.code_font_size = CODE_FONT_SIZE_DEFAULT;
        s.transfer_concurrency = 0;
        assert!(s.validate().is_err());
        s.transfer_concurrency = 9;
        assert!(s.validate().is_err());
        s.transfer_concurrency = TRANSFER_CONCURRENCY_DEFAULT;
        s.default_download_dir = Some(std::env::temp_dir().join("missing-cloudstorage-dir"));
        assert!(s.validate().is_err());
    }
}
