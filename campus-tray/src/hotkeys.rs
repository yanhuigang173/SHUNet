use serde::{Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND},
    UI::Input::KeyboardAndMouse::*,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Login,
    Logout,
    ToggleProxy,
    ToggleReconnect,
    ToggleAutoAuth,
    ScanWifi,
    RefreshNetwork,
    DisconnectWifi,
    OpenPortal,
    ShowMenu,
    Credentials,
    HotkeySettings,
    Help,
    WifiSettings,
    ProxySettings,
    ConfigFolder,
    CopyConfigPath,
    ImportConfig,
    ExportConfig,
    AddWebsite,
    CopyIp,
    CopyLogs,
    SpeedDirect,
    SpeedProxy,
    CancelSpeed,
    SpeedResults,
    CopySpeed,
}

pub const ACTIONS: &[Action] = &[
    Action::Login,
    Action::Logout,
    Action::ToggleProxy,
    Action::ToggleReconnect,
    Action::ToggleAutoAuth,
    Action::ScanWifi,
    Action::RefreshNetwork,
    Action::DisconnectWifi,
    Action::OpenPortal,
    Action::ShowMenu,
    Action::Credentials,
    Action::HotkeySettings,
    Action::Help,
    Action::WifiSettings,
    Action::ProxySettings,
    Action::ConfigFolder,
    Action::CopyConfigPath,
    Action::ImportConfig,
    Action::ExportConfig,
    Action::AddWebsite,
    Action::CopyIp,
    Action::CopyLogs,
    Action::SpeedDirect,
    Action::SpeedProxy,
    Action::CancelSpeed,
    Action::SpeedResults,
    Action::CopySpeed,
];

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "一键认证",
            Self::Logout => "校园网下线",
            Self::ToggleProxy => "系统代理开关",
            Self::ToggleReconnect => "自动重连开关",
            Self::ToggleAutoAuth => "自动认证开关",
            Self::ScanWifi => "刷新附近 Wi-Fi",
            Self::RefreshNetwork => "立即查询一次",
            Self::DisconnectWifi => "断开当前 Wi-Fi",
            Self::OpenPortal => "打开认证网页",
            Self::ShowMenu => "打开托盘菜单",
            Self::Credentials => "账号与密码设置",
            Self::HotkeySettings => "快捷键设置",
            Self::Help => "帮助与使用说明",
            Self::WifiSettings => "Windows Wi-Fi 设置",
            Self::ProxySettings => "Windows 代理设置",
            Self::ConfigFolder => "打开配置文件夹",
            Self::CopyConfigPath => "复制配置路径",
            Self::ImportConfig => "导入配置",
            Self::ExportConfig => "导出配置",
            Self::AddWebsite => "新增常用网址",
            Self::CopyIp => "复制本机 IP",
            Self::CopyLogs => "复制运行记录",
            Self::SpeedDirect => "一键测速（直连）",
            Self::SpeedProxy => "一键测速（通过代理）",
            Self::CancelSpeed => "取消测速",
            Self::SpeedResults => "查看测速结果",
            Self::CopySpeed => "复制测速结果",
        }
    }

    pub fn command(self) -> u32 {
        match self {
            Self::Login => crate::LOGIN,
            Self::Logout => crate::LOGOUT,
            Self::ToggleProxy => crate::PROXY,
            Self::ToggleReconnect => crate::AUTO_RECONNECT,
            Self::ToggleAutoAuth => crate::AUTO_AUTH,
            Self::ScanWifi => crate::SCAN,
            Self::RefreshNetwork => crate::REFRESH,
            Self::DisconnectWifi => crate::DISCONNECT,
            Self::OpenPortal => crate::PORTAL,
            Self::ShowMenu => crate::SHOW_MENU,
            Self::Credentials => crate::CONFIG,
            Self::HotkeySettings => crate::HOTKEY_SETTINGS,
            Self::Help => crate::HELP,
            Self::WifiSettings => crate::WIFI_SETTINGS,
            Self::ProxySettings => crate::PROXY_SETTINGS,
            Self::ConfigFolder => crate::CONFIG_FOLDER,
            Self::CopyConfigPath => crate::CONFIG_PATH_COPY,
            Self::ImportConfig => crate::IMPORT_FILE,
            Self::ExportConfig => crate::EXPORT_FILE,
            Self::AddWebsite => crate::WEBSITE_ADD,
            Self::CopyIp => crate::COPY_IP,
            Self::CopyLogs => crate::COPY_LOGS,
            Self::SpeedDirect => crate::SPEED_DIRECT,
            Self::SpeedProxy => crate::SPEED_PROXY,
            Self::CancelSpeed => crate::SPEED_CANCEL,
            Self::SpeedResults => crate::SPEED_RESULTS,
            Self::CopySpeed => crate::SPEED_COPY,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub action: Action,
    pub shortcut: String,
}

pub const DEFAULTS_VERSION: u8 = 1;

pub fn defaults() -> Vec<Binding> {
    vec![
        Binding {
            action: Action::Login,
            shortcut: "Ctrl+Shift+U".into(),
        },
        default_query_binding(),
    ]
}

fn default_query_binding() -> Binding {
    Binding {
        action: Action::RefreshNetwork,
        shortcut: "Ctrl+Shift+S".into(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub modifiers: u32,
    pub key: u32,
}

impl Shortcut {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 64 {
            return Err("快捷键文本过长，请重新设置。".into());
        }
        let mut modifiers = 0;
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            let upper = part.to_ascii_uppercase();
            let modifier = match upper.as_str() {
                "CTRL" | "CONTROL" => MOD_CONTROL,
                "ALT" => MOD_ALT,
                "SHIFT" => MOD_SHIFT,
                "WIN" | "WINDOWS" => MOD_WIN,
                _ => 0,
            };
            if modifier != 0 {
                if modifiers & modifier != 0 {
                    return Err("修饰键不能重复。".into());
                }
                modifiers |= modifier;
            } else {
                if key.is_some() {
                    return Err("请设置修饰键和一个主按键。".into());
                }
                key = if upper.len() == 1 && upper.as_bytes()[0].is_ascii_alphanumeric() {
                    Some(upper.as_bytes()[0] as u32)
                } else if let Some(number) = upper
                    .strip_prefix('F')
                    .and_then(|n| n.parse::<u32>().ok())
                    .filter(|n| (1..=24).contains(n))
                {
                    Some(VK_F1 as u32 + number - 1)
                } else {
                    special_keys()
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(part))
                        .map(|(_, key)| *key)
                };
                if key.is_none() {
                    return Err("不支持此按键，请使用字母、数字、功能键或导航键。".into());
                }
            }
        }
        Self::new(modifiers, key.ok_or("请选择一个主按键。")?)
    }

    pub fn new(modifiers: u32, key: u32) -> Result<Self, String> {
        if modifiers & !(MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_WIN) != 0 {
            return Err("快捷键修饰键无效。".into());
        }
        if key == VK_F12 as u32 {
            return Err("F12 由 Windows 调试器保留，请换一个按键。".into());
        }
        let function_key = (VK_F1 as u32..=VK_F24 as u32).contains(&key);
        if !function_key
            && !(0x30..=0x39).contains(&key)
            && !(0x41..=0x5a).contains(&key)
            && !special_keys().iter().any(|(_, value)| *value == key)
        {
            return Err("不支持此按键，请使用字母、数字、功能键或导航键。".into());
        }
        if !function_key && modifiers & (MOD_CONTROL | MOD_ALT | MOD_WIN) == 0 {
            return Err("请至少搭配 Ctrl、Alt 或 Win，避免影响日常输入。".into());
        }
        Ok(Self { modifiers, key })
    }

    pub fn text(self) -> String {
        let mut parts = Vec::new();
        for (flag, name) in [
            (MOD_CONTROL, "Ctrl"),
            (MOD_ALT, "Alt"),
            (MOD_SHIFT, "Shift"),
            (MOD_WIN, "Win"),
        ] {
            if self.modifiers & flag != 0 {
                parts.push(name.to_owned());
            }
        }
        parts.push(if (VK_F1 as u32..=VK_F24 as u32).contains(&self.key) {
            format!("F{}", self.key - VK_F1 as u32 + 1)
        } else if let Some((name, _)) = special_keys().iter().find(|(_, key)| *key == self.key) {
            (*name).into()
        } else {
            char::from_u32(self.key).unwrap_or('?').to_string()
        });
        parts.join("+")
    }
}

fn special_keys() -> &'static [(&'static str, u32)] {
    &[
        ("Space", 0x20),
        ("Enter", 0x0d),
        ("Tab", 0x09),
        ("Esc", 0x1b),
        ("Backspace", 0x08),
        ("Insert", 0x2d),
        ("Delete", 0x2e),
        ("Home", 0x24),
        ("End", 0x23),
        ("PageUp", 0x21),
        ("PageDown", 0x22),
        ("Left", 0x25),
        ("Up", 0x26),
        ("Right", 0x27),
        ("Down", 0x28),
    ]
}

pub fn validate(bindings: &[Binding]) -> Result<(), String> {
    if bindings.len() > ACTIONS.len() {
        return Err("快捷键数量超出功能数量。".into());
    }
    let mut shortcuts = Vec::new();
    for (index, binding) in bindings.iter().enumerate() {
        let shortcut = Shortcut::parse(&binding.shortcut)?;
        if shortcut.text() != binding.shortcut {
            return Err("快捷键格式未规范化，请重新设置。".into());
        }
        if bindings[..index]
            .iter()
            .any(|old| old.action == binding.action)
        {
            return Err("一个功能只能配置一个快捷键。".into());
        }
        if shortcuts.contains(&shortcut) {
            return Err(format!(
                "{} 已用于其他功能，请更换组合键。",
                binding.shortcut
            ));
        }
        shortcuts.push(shortcut);
    }
    Ok(())
}

pub fn from_stored(
    value: Option<serde_json::Value>,
    defaults_version: u8,
) -> (Vec<Binding>, Option<String>) {
    let Some(value) = value else {
        return (defaults(), None);
    };
    let Some(entries) = value.as_array() else {
        return (
            defaults(),
            Some("快捷键配置格式无效，已使用默认快捷键。".into()),
        );
    };
    let mut bindings = Vec::new();
    for entry in entries.iter().take(ACTIONS.len()) {
        if let Ok(mut binding) = serde_json::from_value::<Binding>(entry.clone()) {
            if let Ok(shortcut) = Shortcut::parse(&binding.shortcut) {
                binding.shortcut = shortcut.text();
                let mut candidate = bindings.clone();
                candidate.push(binding);
                if validate(&candidate).is_ok() {
                    bindings = candidate;
                }
            }
        }
    }
    let warning = (bindings.len() != entries.len())
        .then(|| "已跳过无效或重复的快捷键配置，其他设置已保留。".into());
    // Upgrade older nonempty configurations once. Never replace a custom query
    // binding, reuse an occupied shortcut, or re-enable explicitly disabled keys.
    // Saving the version also makes clearing the new binding persist on restart.
    if defaults_version < DEFAULTS_VERSION && !bindings.is_empty() && warning.is_none() {
        let query = default_query_binding();
        if !bindings
            .iter()
            .any(|binding| binding.action == query.action || binding.shortcut == query.shortcut)
        {
            bindings.push(query);
        }
    }
    (bindings, warning)
}

struct Registered {
    id: i32,
    action: Action,
    text: String,
}

pub struct Manager {
    window: HWND,
    desired: Vec<Binding>,
    active: Vec<Registered>,
    next_id: i32,
    suspended: bool,
}

impl Manager {
    pub fn new(window: HWND) -> Self {
        Self {
            window,
            desired: Vec::new(),
            active: Vec::new(),
            next_id: 1,
            suspended: true,
        }
    }

    pub fn suspend(&mut self) {
        for registered in self.active.drain(..) {
            unsafe {
                UnregisterHotKey(self.window, registered.id);
            }
        }
        self.suspended = true;
    }

    // Strict registration is used before saving in the editor. A failure keeps
    // the editor open, with every temporary registration removed.
    pub fn install(&mut self, bindings: &[Binding], strict: bool) -> Result<Vec<String>, String> {
        validate(bindings)?;
        self.suspend();
        self.desired = bindings.to_vec();
        let mut errors = Vec::new();
        for binding in bindings {
            let shortcut = Shortcut::parse(&binding.shortcut)?;
            let id = self.next_id;
            self.next_id = if id == 0xbfff { 1 } else { id + 1 };
            if unsafe {
                RegisterHotKey(
                    self.window,
                    id,
                    shortcut.modifiers | MOD_NOREPEAT,
                    shortcut.key,
                )
            } != 0
            {
                self.active.push(Registered {
                    id,
                    action: binding.action,
                    text: binding.shortcut.clone(),
                });
            } else {
                let error = unsafe { GetLastError() };
                errors.push(format!(
                    "{}：{} 被占用、由系统保留或无法注册（Windows {error}）。",
                    binding.action.label(),
                    binding.shortcut
                ));
                if strict {
                    self.suspend();
                    return Err(errors.join("\r\n"));
                }
            }
        }
        self.suspended = false;
        Ok(errors)
    }

    pub fn sync(&mut self, bindings: &[Binding]) -> Option<String> {
        if !self.suspended && self.desired == bindings {
            return None;
        }
        match self.install(bindings, false) {
            Ok(errors) if errors.is_empty() => None,
            Ok(errors) => Some(format!(
                "部分快捷键未生效：\r\n{}\r\n\r\n请在“设置 → 快捷键”中修改。",
                errors.join("\r\n")
            )),
            Err(error) => Some(format!("快捷键未能应用：{error}")),
        }
    }

    pub fn action(&self, id: usize) -> Option<Action> {
        self.active
            .iter()
            .find(|entry| entry.id as usize == id)
            .map(|entry| entry.action)
    }
    pub fn label_for_command(&self, command: u32) -> Option<&str> {
        self.active
            .iter()
            .find(|entry| entry.action.command() == command)
            .map(|entry| entry.text.as_str())
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.suspend();
    }
}
