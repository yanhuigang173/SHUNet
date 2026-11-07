//! Only the explicit tray menu action writes the current user's startup entry.
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE},
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "SHUNet";
const LEGACY_VALUE: &str = "CampusLinkTray";

#[derive(Default)]
pub struct Status {
    pub registered: bool,
    pub current_copy: bool,
}

fn command_line() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|_| "无法确定当前程序路径。".to_owned())?;
    let path = path.to_str().ok_or("程序路径包含不支持的字符。")?;
    // Quote the executable directly. No shell, helper script or administrator token.
    let command = format!("\"{path}\" --autostart");
    if command.encode_utf16().count() > 260 {
        return Err("程序路径过长，请先把 exe 移到较短路径再开启自启动。".to_owned());
    }
    Ok(command)
}

pub fn status() -> Result<Status, String> {
    let expected = command_line()?;
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Status::default()),
        Err(_) => return Err("无法读取当前用户的启动设置。".to_owned()),
    };
    let value = read_value(&key, VALUE)?;
    let legacy = read_value(&key, LEGACY_VALUE)?;
    Ok(Status {
        registered: !value.trim().is_empty() || !legacy.trim().is_empty(),
        current_copy: value.trim().eq_ignore_ascii_case(&expected) && legacy.trim().is_empty(),
    })
}

fn read_value(key: &RegKey, name: &str) -> Result<String, String> {
    match key.get_value(name) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(_) => Err("当前启动项格式异常，请在 Windows 启动应用设置中检查。".into()),
    }
}

fn delete_value(key: &RegKey, name: &str) -> Result<(), String> {
    match key.delete_value(name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("无法更新开机启动项，请检查系统策略。".into()),
    }
}

pub fn toggle() -> Result<(), String> {
    if status()?.current_copy {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
            .map_err(|_| "无法修改启动设置。".to_owned())?;
        delete_value(&key, VALUE)?;
        delete_value(&key, LEGACY_VALUE)
    } else {
        let command = command_line()?;
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(RUN_KEY)
            .map_err(|_| "无法修改启动设置。".to_owned())?;
        key.set_value(VALUE, &command)
            .map_err(|_| "开启开机启动失败，请检查系统策略。".to_owned())?;
        // Migrate only on the explicit menu action, avoiding two startup entries.
        delete_value(&key, LEGACY_VALUE)
    }
}
