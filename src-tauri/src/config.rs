use std::path::{Path, PathBuf};

use crate::models::LlmSettings;

/// 应用配置以 JSON 持久化在 <app_data_dir>/config.json。
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 读取配置；文件不存在或损坏时回退到默认值。
    pub fn load(&self) -> LlmSettings {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str::<LlmSettings>(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, settings: &LlmSettings) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("无法创建配置目录：{e}"))?;
        }
        let json =
            serde_json::to_string_pretty(settings).map_err(|e| format!("序列化配置失败：{e}"))?;
        std::fs::write(&self.path, json).map_err(|e| format!("无法写入配置文件：{e}"))?;
        Ok(())
    }
}

pub fn config_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ai-teacher-config-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_when_missing_or_corrupt() {
        let dir = temp_dir("defaults");
        let store = ConfigStore::new(dir.join("config.json"));
        let s = store.load();
        assert_eq!(s.base_url, "https://api.deepseek.com");
        assert_eq!(s.model, "deepseek-chat");
        assert_eq!(s.api_key, "");
        std::fs::write(dir.join("config.json"), "这不是 JSON").unwrap();
        let s = store.load();
        assert_eq!(s.base_url, "https://api.deepseek.com");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrip_and_partial_files() {
        let dir = temp_dir("roundtrip");
        let store = ConfigStore::new(dir.join("config.json"));
        store
            .save(&LlmSettings {
                base_url: "https://api.example.com".to_string(),
                api_key: "sk-test".to_string(),
                model: "my-model".to_string(),
            })
            .unwrap();
        let s = store.load();
        assert_eq!(s.base_url, "https://api.example.com");
        assert_eq!(s.api_key, "sk-test");
        assert_eq!(s.model, "my-model");

        // 缺字段的配置文件按默认值补全
        std::fs::write(dir.join("config.json"), r#"{"model":"m2"}"#).unwrap();
        let s = store.load();
        assert_eq!(s.model, "m2");
        assert_eq!(s.base_url, "https://api.deepseek.com");
        assert_eq!(s.api_key, "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
