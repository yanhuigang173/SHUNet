use crate::{platform, settings::SettingsStore, wide};
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::Dialogs::*, WindowsAndMessaging::*},
};

pub fn choose_config(owner: HWND, export: bool) -> Result<Option<PathBuf>, String> {
    let mut filename = vec![0u16; 32768];
    if export {
        let default = wide(&format!(
            "SHUNet配置-{}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        filename[..default.len()].copy_from_slice(&default);
    }
    let title = wide(if export {
        "SHUNet · 导出配置（密码仅限当前 Windows 用户恢复）"
    } else {
        "SHUNet · 选择配置文件"
    });
    let filter = wide("JSON 配置文件 (*.json)\0*.json\0所有文件 (*.*)\0*.*\0");
    let extension = wide("json");
    let mut options = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: filename.as_mut_ptr(),
        nMaxFile: filename.len() as u32,
        lpstrTitle: title.as_ptr(),
        lpstrDefExt: extension.as_ptr(),
        Flags: OFN_EXPLORER
            | OFN_ENABLESIZING
            | OFN_NOCHANGEDIR
            | OFN_DONTADDTORECENT
            | OFN_PATHMUSTEXIST
            | if export {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            },
        ..Default::default()
    };
    let selected = unsafe {
        if export {
            GetSaveFileNameW(&mut options)
        } else {
            GetOpenFileNameW(&mut options)
        }
    };
    if selected == 0 {
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(None)
        } else {
            Err(format!("文件选择窗口打开失败（{error:#x}）。"))
        };
    }
    let length = filename
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(filename.len());
    Ok(Some(PathBuf::from(OsString::from_wide(
        &filename[..length],
    ))))
}

pub fn open_config_folder() -> Result<(), String> {
    let store = SettingsStore::new();
    let folder = store.path().parent().ok_or("无法确定默认配置目录。")?;
    std::fs::create_dir_all(folder).map_err(|error| format!("无法打开配置目录：{error}"))?;
    platform::try_open(&folder.to_string_lossy())
}

pub fn help_text() -> String {
    include_str!("../assets/使用说明.txt")
        .replace("{VERSION}", env!("CARGO_PKG_VERSION"))
        .replace(
            "{CONFIG_PATH}",
            &SettingsStore::new().path().display().to_string(),
        )
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
}

pub fn show_help(owner: HWND) -> Result<(), String> {
    show_text(owner, "使用说明", &help_text())
}

struct TextContent<'a> {
    title: &'a str,
    text: &'a str,
}

pub fn show_text(owner: HWND, title: &str, text: &str) -> Result<(), String> {
    let content = TextContent { title, text };
    let result = unsafe {
        DialogBoxParamW(
            GetModuleHandleW(std::ptr::null()),
            102usize as *const u16,
            owner,
            Some(help_proc),
            &content as *const TextContent<'_> as isize,
        )
    };
    if result == -1 {
        return Err("无法打开内容窗口。".into());
    }
    Ok(())
}

const DIALOG_USER: i32 = (2 * std::mem::size_of::<isize>()) as i32;
unsafe extern "system" fn help_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    std::panic::catch_unwind(|| handle_help(window, message, wparam, lparam)).unwrap_or_else(|_| {
        unsafe {
            EndDialog(window, -1);
        }
        1
    })
}

fn handle_help(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    if let Some(result) = crate::ui::message(window, message, wparam, lparam) {
        return result;
    }
    unsafe {
        if message == WM_INITDIALOG {
            SetWindowLongPtrW(window, DIALOG_USER, lparam);
            let content = &*(lparam as *const TextContent<'_>);
            SetWindowTextW(
                window,
                wide(&format!("SHUNet · {}", content.title)).as_ptr(),
            );
            SetDlgItemTextW(window, 1101, wide(content.text).as_ptr());
            SetDlgItemTextW(window, 1102, wide("复制全文").as_ptr());
            SetDlgItemTextW(window, IDOK, wide("关闭").as_ptr());
            crate::ui::init(window, crate::ui::Kind::Help);
            SetDlgItemTextW(window, 1600, wide(content.title).as_ptr());
            return 1;
        }
        if message == WM_CLOSE
            || (message == WM_COMMAND
                && [IDOK as usize, IDCANCEL as usize].contains(&(wparam & 0xffff)))
        {
            EndDialog(window, IDOK as isize);
            return 1;
        }
        if message == WM_COMMAND && (wparam & 0xffff) == 1102 {
            let content = GetWindowLongPtrW(window, DIALOG_USER) as *const TextContent<'_>;
            if !content.is_null() {
                if let Err(error) = platform::copy_text((*content).text) {
                    crate::alert(window, &error);
                }
            }
            return 1;
        }
    }
    0
}

pub fn center_on_cursor(window: HWND) {
    unsafe {
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mut rect = RECT::default();
        if GetMonitorInfoW(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST), &mut info) != 0
            && GetWindowRect(window, &mut rect) != 0
        {
            let area = info.rcWork;
            let x = area.left + ((area.right - area.left - (rect.right - rect.left)) / 2).max(0);
            let y = area.top + ((area.bottom - area.top - (rect.bottom - rect.top)) / 2).max(0);
            SetWindowPos(
                window,
                std::ptr::null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER,
            );
        }
    }
}
