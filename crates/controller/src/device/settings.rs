use super::DeviceMode;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceSettings {
    pub mode: DeviceMode,
    pub v3_url: String,
    pub v4_url: String,
}
impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            mode: DeviceMode::App3,
            v3_url: "wss://ws.dungeon-lab.cn/".into(),
            v4_url: "wss://trex.dungeon-lab.cn/v4".into(),
        }
    }
}
impl DeviceSettings {
    pub fn validate(&self) -> Result<(), String> {
        validate_url(&self.v3_url, DeviceMode::App3)?;
        validate_url(&self.v4_url, DeviceMode::App4)?;
        Ok(())
    }
    pub fn endpoint(&self) -> &str {
        match self.mode {
            DeviceMode::App3 => &self.v3_url,
            DeviceMode::App4 => &self.v4_url,
        }
    }
    pub fn load() -> Result<Self, String> {
        Self::load_at(&gmod_dglab_common::runtime::path().map_err(|e| e.to_string())?)
    }
    fn load_at(dir: &Path) -> Result<Self, String> {
        let path = dir.join("device.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        if std::fs::metadata(&path)
            .map_err(|_| "无法读取设备连接设置")?
            .len()
            > 16384
        {
            return Err("设备连接设置过大".into());
        }
        let settings: Self =
            serde_json::from_slice(&std::fs::read(path).map_err(|_| "无法读取设备连接设置")?)
                .map_err(|_| "设备连接设置损坏，请重新设置")?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn save(&self) -> Result<(), String> {
        self.validate()?;
        crate::hub::atomic_write(
            &gmod_dglab_common::runtime::path().map_err(|e| e.to_string())?,
            "device.json",
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
    }
}
pub(crate) fn validate_url(text: &str, mode: DeviceMode) -> Result<url::Url, String> {
    if text.len() > 2048 {
        return Err("中继地址过长".into());
    }
    let url = url::Url::parse(text).map_err(|_| "请输入有效的 WebSocket 中继地址")?;
    if !matches!(url.scheme(), "ws" | "wss")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("中继地址须使用 ws/wss，且不能包含账号、查询参数或片段".into());
    }
    if mode == DeviceMode::App3 && url.path() != "/" {
        return Err("APP 3.x 的中继地址不能包含额外路径".into());
    }
    Ok(url)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_settings_roundtrip_without_pairing() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!(
                "device-settings-{}",
                gmod_dglab_common::runtime::random_token().unwrap()
            ));
        let settings = DeviceSettings::default();
        crate::hub::atomic_write(&dir, "device.json", &serde_json::to_vec(&settings).unwrap())
            .unwrap();
        assert_eq!(
            DeviceSettings::load_at(&dir).unwrap().mode,
            DeviceMode::App3
        );
        let text = std::fs::read_to_string(dir.join("device.json")).unwrap();
        assert!(!text.contains("clientId"));
        assert!(!text.contains("enabled"));
        std::fs::write(dir.join("device.json"), "{broken").unwrap();
        assert!(DeviceSettings::load_at(&dir).is_err());
        std::fs::remove_file(dir.join("device.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
