use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::{
    detection::DetectionLevel,
    hotkeys::{self, Binding},
    network_policy::NetworkPreference,
    operator::Operator,
    platform,
    websites::{self, Website},
};

#[derive(Clone)]
pub struct Settings {
    pub operator: Operator,
    pub menu_theme: crate::menu_theme::MenuTheme,
    pub username: String,
    pub password: String,
    pub remember_password: bool,
    pub auto_auth: bool,
    pub auto_reconnect: bool,
    pub status_detection: bool,
    pub detection_level: DetectionLevel,
    pub proxy_server: String,
    pub websites: Vec<Website>,
    pub hotkeys: Vec<Binding>,
    pub network_preferences: BTreeMap<String, NetworkPreference>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            operator: Operator::default(),
            menu_theme: Default::default(),
            username: String::new(),
            password: String::new(),
            remember_password: false,
            auto_auth: true,
            auto_reconnect: false,
            status_detection: true,
            detection_level: DetectionLevel::default(),
            proxy_server: "127.0.0.1:7890".to_owned(),
            websites: websites::defaults(),
            hotkeys: hotkeys::defaults(),
            network_preferences: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct StoredSettings {
    #[serde(default)]
    operator: Option<serde_json::Value>,
    #[serde(default)]
    menu_theme: Option<serde_json::Value>,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password_dpapi: String,
    #[serde(default)]
    remember_password: bool,
    #[serde(default = "auto_auth_default")]
    auto_auth: bool,
    #[serde(default)]
    auto_reconnect: bool,
    #[serde(default = "auto_auth_default")]
    status_detection: bool,
    #[serde(default)]
    detection_level: Option<serde_json::Value>,
    #[serde(default = "proxy_default")]
    proxy_server: String,
    // Keep malformed bookmark entries isolated from the account settings.
    #[serde(default)]
    websites: Option<serde_json::Value>,
    #[serde(default)]
    hotkeys: Option<serde_json::Value>,
    #[serde(default)]
    hotkey_defaults_version: u8,
    #[serde(default)]
    network_preferences: BTreeMap<String, NetworkPreference>,
}

fn auto_auth_default() -> bool {
    true
}
fn proxy_default() -> String {
    "127.0.0.1:7890".to_owned()
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    #[cfg(test)]
    pub fn test_at(path:PathBuf)->Self {Self {path}}
    pub fn new() -> Self {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .unwrap_or_else(std::env::temp_dir);
        Self {
            // Keep the existing directory so renaming the app preserves all preferences.
            path: base.join("CampusLinkTray").join("settings.json"),
        }
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn desktop() -> Self {
        let mut store = Self::new();
        store.path = store
            .path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("CampusLink")
            .join("settings.json");
        store
    }

    pub fn load(&self) -> (Settings, Option<String>) {
        if !self.exists() {
            return (Settings::default(), None);
        }
        match read_stored(&self.path, false).and_then(|stored| decode(stored, false)) {
            Ok((mut settings,mut warning)) => {
                if websites::migrate_welcome(&mut settings.websites) && warning.is_none() {
                    if let Err(error)=self.save(&settings) { warning=Some(format!("迎新网址已在本次运行更新，但保存失败：{error}")); }
                }
                (settings,warning)
            },
            Err(error) => (
                Settings::default(),
                Some(format!("设置文件无法读取，已使用默认设置：{error}")),
            ),
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), String> {
        atomic_write(&self.path, &encode(settings)?)
    }

    pub fn import_file(path: &Path) -> Result<Settings, String> {
        read_stored(path, true)
            .and_then(|stored| decode(stored, true))
            .map(|(mut settings, _)| { websites::migrate_welcome(&mut settings.websites); settings })
    }

    pub fn export_file(&self, path: &Path, settings: &Settings) -> Result<(), String> {
        if same_path(path, &self.path) {
            return Err("目标文件是正在使用的默认配置，请另选备份文件。".into());
        }
        atomic_write(path, &encode(settings)?)
    }

    pub fn backup_before_import(&self) -> Result<Option<PathBuf>, String> {
        if !self.exists() {
            return Ok(None);
        }
        let backup = self.path.with_file_name("settings.before-import.json");
        let bytes = read_bytes(&self.path)?;
        atomic_write(&backup, &bytes)?;
        Ok(Some(backup))
    }
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    const LIMIT: u64 = 1024 * 1024;
    let file = fs::File::open(path).map_err(|error| format!("无法读取配置：{error}"))?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > LIMIT as usize {
        return Err("配置文件超过 1 MB，请选择正确的 JSON 配置。".into());
    }
    Ok(bytes)
}

fn read_stored(path: &Path, require_settings: bool) -> Result<StoredSettings, String> {
    let bytes = read_bytes(path)?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "JSON 格式不正确，请选择本工具的配置文件。".to_owned())?;
    let object = value.as_object().ok_or("配置必须是一个 JSON 对象。")?;
    if require_settings
        && ![
            "operator",
            "menu_theme",
            "username",
            "password_dpapi",
            "remember_password",
            "auto_auth",
            "auto_reconnect",
            "status_detection",
            "detection_level",
            "proxy_server",
            "websites",
            "hotkeys",
            "network_preferences",
        ]
        .iter()
        .any(|key| object.contains_key(*key))
    {
        return Err("文件中没有可识别的 SHUNet 配置，现有设置已保留。".into());
    }
    serde_json::from_value(value).map_err(|_| "配置字段的格式不正确，现有设置已保留。".into())
}

fn decode(stored: StoredSettings, strict: bool) -> Result<(Settings, Option<String>), String> {
    let mut warnings = Vec::new();
    let menu_theme = match stored.menu_theme {
        None => crate::menu_theme::MenuTheme::default(),
        Some(value) => match serde_json::from_value(value) {
            Ok(theme) => theme,
            Err(_) if strict => return Err("菜单主题无效，请使用 default / classic / modern。".into()),
            Err(_) => { warnings.push("菜单主题无效，已恢复为默认；其他配置继续使用。".into()); crate::menu_theme::MenuTheme::default() }
        }
    };
    let operator = match stored.operator {
        None => Operator::default(),
        Some(value) => match serde_json::from_value(value) {
            Ok(operator) => operator,
            Err(_) if strict => return Err("运营商配置无效，请使用 campus / telecom / unicom / mobile。".into()),
            Err(_) => {
                warnings.push("运营商配置无效，已恢复为校园网；其他配置继续使用。".into());
                Operator::default()
            }
        },
    };
    let detection_level = match stored.detection_level {
        None => DetectionLevel::default(),
        Some(value) => match serde_json::from_value::<DetectionLevel>(value) {
            Ok(level) => level,
            Err(_) => {
                if strict {
                    return Err(
                        "检测档位无效，请使用 adaptive / extreme / high / medium / low。".into(),
                    );
                }
                warnings.push("检测档位无效，已恢复为中档；其他配置继续使用。".into());
                DetectionLevel::default()
            }
        },
    };
    let password = if stored.remember_password && !stored.password_dpapi.is_empty() {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&stored.password_dpapi)
            .ok()
            .and_then(|encrypted| platform::protect_user_data(&encrypted, false).ok())
            .and_then(|plain| String::from_utf8(plain).ok());
        match decoded {
            Some(password) => password,
            None => {
                let error = "保存的密码无法在当前 Windows 用户下解密，请在原账户重新导出，或重新配置账号密码。";
                if strict {
                    return Err(error.into());
                }
                warnings.push(error.to_owned());
                String::new()
            }
        }
    } else {
        String::new()
    };
    let (websites, warning) = websites::from_stored(stored.websites);
    if let Some(warning) = warning {
        if strict {
            return Err("网址列表格式有误、包含重复项或超过 64 项，请修正后重新导入。".into());
        }
        warnings.push(warning);
    }
    let (hotkeys, warning) = hotkeys::from_stored(stored.hotkeys, stored.hotkey_defaults_version);
    if let Some(warning) = warning {
        if strict {
            return Err("快捷键配置包含无效或重复条目，请修正后重新导入。".into());
        }
        warnings.push(warning);
    }
    let settings = Settings {
        operator,
        menu_theme,
        username: stored.username,
        password,
        remember_password: stored.remember_password,
        auto_auth: stored.auto_auth,
        auto_reconnect: stored.auto_reconnect,
        status_detection: stored.status_detection,
        detection_level,
        proxy_server: stored.proxy_server,
        websites,
        hotkeys,
        network_preferences: stored.network_preferences,
    };
    validate(&settings)?;
    Ok((
        settings,
        (!warnings.is_empty()).then(|| warnings.join("\r\n")),
    ))
}

fn validate(settings: &Settings) -> Result<(), String> {
    if settings.network_preferences.len() > 128
        || settings
            .network_preferences
            .keys()
            .any(|ssid| ssid.is_empty() || ssid.len() > 512 || ssid.chars().any(char::is_control))
    {
        return Err("网络偏好配置无效或超过 128 项。".into());
    }
    websites::validate_all(&settings.websites)?;
    hotkeys::validate(&settings.hotkeys)?;
    if settings.username.chars().count() > 512
        || settings.username.chars().any(char::is_control)
        || settings.proxy_server.len() > 4096
        || settings.proxy_server.chars().any(char::is_control)
    {
        return Err("账号或代理地址格式不正确。".into());
    }
    Ok(())
}

fn encode(settings: &Settings) -> Result<Vec<u8>, String> {
    validate(settings)?;
    let password_dpapi = if settings.remember_password && !settings.password.is_empty() {
        let protected = platform::protect_user_data(settings.password.as_bytes(), true)?;
        base64::engine::general_purpose::STANDARD.encode(protected)
    } else {
        String::new()
    };
    let stored = StoredSettings {
        menu_theme: Some(serde_json::to_value(settings.menu_theme).map_err(|error| error.to_string())?),
        operator: Some(serde_json::to_value(settings.operator).map_err(|error| error.to_string())?),
        username: settings.username.clone(),
        password_dpapi,
        remember_password: settings.remember_password,
        auto_auth: settings.auto_auth,
        auto_reconnect: settings.auto_reconnect,
        status_detection: settings.status_detection,
        detection_level: Some(
            serde_json::to_value(settings.detection_level).map_err(|error| error.to_string())?,
        ),
        proxy_server: settings.proxy_server.clone(),
        websites: Some(
            serde_json::to_value(&settings.websites).map_err(|error| error.to_string())?,
        ),
        hotkeys: Some(serde_json::to_value(&settings.hotkeys).map_err(|error| error.to_string())?),
        hotkey_defaults_version: hotkeys::DEFAULTS_VERSION,
        network_preferences: settings.network_preferences.clone(),
    };
    serde_json::to_vec_pretty(&stored).map_err(|error| error.to_string())
}

fn same_path(first: &Path, second: &Path) -> bool {
    let first = fs::canonicalize(first).unwrap_or_else(|_| first.to_owned());
    let second = fs::canonicalize(second).unwrap_or_else(|_| second.to_owned());
    first.to_string_lossy().to_lowercase() == second.to_string_lossy().to_lowercase()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or("无法确定配置目录。")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(
        ".campus-link-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    // create_new avoids truncating another file; rename preserves the old config on failure.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod operator_settings_tests {
    use super::*;

    #[test]
    fn old_configuration_defaults_to_campus_and_preserves_account() {
        let stored = serde_json::from_str(r#"{"username":"demo","auto_auth":false}"#).unwrap();
        let (settings, _) = decode(stored, true).unwrap();
        assert_eq!(settings.operator, Operator::Campus);
        assert_eq!(settings.username, "demo");
        assert!(!settings.auto_auth);
    }

    #[test]
    fn all_four_operator_choices_round_trip_in_exports() {
        for operator in Operator::ALL {
            let settings = Settings { operator, ..Default::default() };
            let stored = serde_json::from_slice(&encode(&settings).unwrap()).unwrap();
            assert_eq!(decode(stored, true).unwrap().0.operator, operator);
        }
    }

    #[test]
    fn invalid_operator_is_rejected_on_import_and_isolated_on_local_load() {
        let json = r#"{"operator":"unexpected","username":"demo"}"#;
        assert!(decode(serde_json::from_str(json).unwrap(), true).is_err());
        let (settings, warning) = decode(serde_json::from_str(json).unwrap(), false).unwrap();
        assert_eq!(settings.operator, Operator::Campus);
        assert_eq!(settings.username, "demo");
        assert!(warning.unwrap().contains("运营商"));
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;
    use crate::menu_theme::MenuTheme;
    #[test]
    fn themes_round_trip_and_legacy_account_is_preserved() {
        let (settings,_) = decode(serde_json::from_str(r#"{"username":"demo","auto_auth":false}"#).unwrap(),true).unwrap();
        assert_eq!(settings.menu_theme,MenuTheme::Default);
        assert_eq!(settings.username,"demo"); assert!(!settings.auto_auth);
        for theme in MenuTheme::ALL {
            let settings=Settings {menu_theme:theme,..Settings::default()};
            let decoded=decode(serde_json::from_slice(&encode(&settings).unwrap()).unwrap(),true).unwrap().0;
            assert_eq!(decoded.menu_theme,theme);
        }
    }
    #[test]
    fn bad_theme_does_not_discard_local_account_but_import_is_rejected() {
        for value in [r#""unknown""#,"123","{}"] {
            let json=format!(r#"{{"menu_theme":{},"username":"demo"}}"#,value);
            assert!(decode(serde_json::from_str(&json).unwrap(),true).is_err());
            let (settings,warning)=decode(serde_json::from_str(&json).unwrap(),false).unwrap();
            assert_eq!(settings.menu_theme,MenuTheme::Default);assert_eq!(settings.username,"demo");
            assert!(warning.unwrap().contains("菜单主题"));
        }
    }
}
