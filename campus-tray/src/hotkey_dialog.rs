use crate::{
    hotkeys::{self, ACTIONS, Binding, Manager, Shortcut},
    service::{Command, Controller},
    wide,
};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const LIST: i32 = 1301;
const CAPTURE: i32 = 1303;
const CLEAR: i32 = 1304;
const HINT: i32 = 1305;
const DEFAULTS: i32 = 1306;
const USER: i32 = (2 * std::mem::size_of::<isize>()) as i32;
const SAVE_TIMER: usize = 2;
const GUIDE: &str = "支持 Ctrl / Alt / Shift / Win；Backspace 清除。非功能键需搭配 Ctrl、Alt 或 Win。F12 由系统保留。";

struct State {
    working: Vec<Binding>,
    manager: *mut Manager,
    controller: Controller,
    pending: Option<Receiver<Result<(), String>>>,
    original_proc: WNDPROC,
}

pub fn prompt(
    owner: HWND,
    bindings: &[Binding],
    controller: Controller,
    manager: &mut Manager,
) -> Result<(), String> {
    manager.suspend();
    let mut state = State {
        working: bindings.to_vec(),
        manager,
        controller,
        pending: None,
        original_proc: None,
    };
    let result = unsafe {
        DialogBoxParamW(
            GetModuleHandleW(std::ptr::null()),
            103usize as *const u16,
            owner,
            Some(dialog_proc),
            &mut state as *mut State as isize,
        )
    };
    // Keys used while recording must not become actions just after the editor closes.
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, owner, WM_HOTKEY, WM_HOTKEY, PM_REMOVE) != 0 {}
    }
    if result == -1 {
        return Err("无法打开快捷键设置，或设置窗口发生异常。".into());
    }
    Ok(())
}

unsafe extern "system" fn dialog_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    std::panic::catch_unwind(|| handle_dialog(window, message, wparam, lparam)).unwrap_or_else(
        |_| {
            unsafe {
                EndDialog(window, -1);
            }
            1
        },
    )
}

fn state(window: HWND) -> *mut State {
    unsafe { GetWindowLongPtrW(window, USER) as *mut State }
}

fn selected(window: HWND) -> usize {
    unsafe { SendDlgItemMessageW(window, LIST, LB_GETCURSEL, 0, 0).max(0) as usize }
        .min(ACTIONS.len() - 1)
}

fn refresh(window: HWND, data: &State, row: usize, rebuild: bool) {
    unsafe {
        if rebuild {
            SendDlgItemMessageW(window, LIST, LB_RESETCONTENT, 0, 0);
            for action in ACTIONS {
                let key = data
                    .working
                    .iter()
                    .find(|entry| entry.action == *action)
                    .map_or("未设置", |entry| entry.shortcut.as_str());
                let text = wide(&format!("{}\t{key}", action.label()));
                SendDlgItemMessageW(window, LIST, LB_ADDSTRING, 0, text.as_ptr() as isize);
            }
            SendDlgItemMessageW(window, LIST, LB_SETCURSEL, row, 0);
        }
        let action = ACTIONS[row];
        SetDlgItemTextW(
            window,
            1302,
            wide(&format!("{}：", action.label())).as_ptr(),
        );
        let key = data
            .working
            .iter()
            .find(|entry| entry.action == action)
            .map_or("", |entry| entry.shortcut.as_str());
        SetDlgItemTextW(window, CAPTURE, wide(key).as_ptr());
    }
}

fn set_busy(window: HWND, busy: bool) {
    unsafe {
        for id in [LIST, CAPTURE, CLEAR, DEFAULTS, IDOK, IDCANCEL] {
            EnableWindow(GetDlgItem(window, id), (!busy) as i32);
        }
        SetDlgItemTextW(
            window,
            HINT,
            wide(if busy {
                "正在保存，请稍候…"
            } else {
                GUIDE
            })
            .as_ptr(),
        );
        EnableMenuItem(
            GetSystemMenu(window, 0),
            SC_CLOSE,
            MF_BYCOMMAND | if busy { MF_GRAYED } else { MF_ENABLED },
        );
    }
}

fn show_error(window: HWND, error: &str) {
    unsafe {
        MessageBoxW(
            window,
            wide(error).as_ptr(),
            wide("SHUNet · 快捷键设置").as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn handle_dialog(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    if let Some(result) = crate::ui::message(window, message, wparam, lparam) {
        return result;
    }
    unsafe {
        if message == WM_INITDIALOG {
            SetWindowLongPtrW(window, USER, lparam);
            let pointer = lparam as *mut State;
            SetWindowTextW(window, wide("SHUNet · 全局快捷键").as_ptr());
            for (id, text) in [
                (
                    1300,
                    "选择功能后录入组合键。默认认证 Ctrl+Shift+U，查询 Ctrl+Shift+S；修改后保存。",
                ),
                (CLEAR, "清除当前"),
                (DEFAULTS, "恢复默认"),
                (IDOK, "保存"),
                (IDCANCEL, "取消"),
                (HINT, GUIDE),
            ] {
                SetDlgItemTextW(window, id, wide(text).as_ptr());
            }
            let tab = 220i32;
            SendDlgItemMessageW(window, LIST, LB_SETTABSTOPS, 1, &tab as *const i32 as isize);
            let edit = GetDlgItem(window, CAPTURE);
            let previous =
                SetWindowLongPtrW(edit, GWLP_WNDPROC, capture_proc as *const () as isize);
            if previous == 0 {
                EndDialog(window, -1);
                return 1;
            }
            (*pointer).original_proc = std::mem::transmute::<isize, WNDPROC>(previous);
            refresh(window, &*pointer, 0, true);
            crate::ui::init(window, crate::ui::Kind::Hotkeys);
            return 1;
        }
        let pointer = state(window);
        if pointer.is_null() {
            return 0;
        }
        if message == WM_CLOSE || (message == WM_COMMAND && (wparam & 0xffff) == IDCANCEL as usize)
        {
            if (*pointer).pending.is_none() {
                EndDialog(window, IDCANCEL as isize);
            }
            return 1;
        }
        if message == WM_TIMER && wparam == SAVE_TIMER {
            let outcome = (*pointer)
                .pending
                .as_ref()
                .map(|receiver| receiver.try_recv());
            match outcome {
                Some(Ok(Ok(()))) => {
                    KillTimer(window, SAVE_TIMER);
                    (*pointer).pending = None;
                    EndDialog(window, IDOK as isize);
                }
                Some(Ok(Err(_))) | Some(Err(TryRecvError::Disconnected)) => {
                    KillTimer(window, SAVE_TIMER);
                    (*pointer).pending = None;
                    (&mut *(*pointer).manager).suspend();
                    set_busy(window, false);
                    let error = match outcome {
                        Some(Ok(Err(error))) => format!("保存失败，原配置已保留：\r\n{error}"),
                        _ => "后台任务不可用，原配置已保留，请重试。".into(),
                    };
                    show_error(window, &error);
                }
                _ => {}
            }
            return 1;
        }
        if message != WM_COMMAND || (*pointer).pending.is_some() {
            return 0;
        }
        let id = (wparam & 0xffff) as i32;
        let row = selected(window);
        if id == LIST && (wparam >> 16) as u32 == LBN_SELCHANGE {
            refresh(window, &*pointer, row, false);
            SetDlgItemTextW(window, HINT, wide(GUIDE).as_ptr());
            return 1;
        }
        if id == CLEAR || id == DEFAULTS {
            if id == DEFAULTS {
                (*pointer).working = hotkeys::defaults();
            } else {
                (*pointer)
                    .working
                    .retain(|entry| entry.action != ACTIONS[row]);
            }
            refresh(
                window,
                &*pointer,
                if id == DEFAULTS { 0 } else { row },
                true,
            );
            SetDlgItemTextW(window, HINT, wide(GUIDE).as_ptr());
            return 1;
        }
        if id == IDOK {
            let bindings = (*pointer).working.clone();
            if let Err(error) = hotkeys::validate(&bindings).and_then(|_| {
                (&mut *(*pointer).manager)
                    .install(&bindings, true)
                    .map(|_| ())
            }) {
                show_error(window, &error);
                return 1;
            }
            let (reply, receiver) = mpsc::sync_channel(1);
            if !(*pointer)
                .controller
                .send(Command::SaveHotkeys { bindings, reply })
            {
                (&mut *(*pointer).manager).suspend();
                show_error(window, "后台队列繁忙，请稍后再次保存。");
                return 1;
            }
            (*pointer).pending = Some(receiver);
            set_busy(window, true);
            SetTimer(window, SAVE_TIMER, 100, None);
            return 1;
        }
    }
    0
}

unsafe extern "system" fn capture_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    std::panic::catch_unwind(|| handle_capture(window, message, wparam, lparam))
        .unwrap_or_else(|_| unsafe { DefWindowProcW(window, message, wparam, lparam) })
}

fn handle_capture(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let owner = GetParent(window);
        let pointer = state(owner);
        if pointer.is_null() {
            return DefWindowProcW(window, message, wparam, lparam);
        }
        if message == WM_GETDLGCODE {
            return DLGC_WANTALLKEYS as isize;
        }
        if message == WM_KEYUP || message == WM_SYSKEYUP {
            return 0;
        }
        if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
            if (*pointer).pending.is_some() || lparam as usize & (1 << 30) != 0 {
                return 0;
            }
            let key = wparam as u32;
            if [
                VK_SHIFT,
                VK_CONTROL,
                VK_MENU,
                VK_LSHIFT,
                VK_RSHIFT,
                VK_LCONTROL,
                VK_RCONTROL,
                VK_LMENU,
                VK_RMENU,
                VK_LWIN,
                VK_RWIN,
            ]
            .iter()
            .any(|vk| *vk as u32 == key)
            {
                return 0;
            }
            let mut modifiers = 0;
            for (vk, flag) in [
                (VK_CONTROL, MOD_CONTROL),
                (VK_MENU, MOD_ALT),
                (VK_SHIFT, MOD_SHIFT),
                (VK_LWIN, MOD_WIN),
                (VK_RWIN, MOD_WIN),
            ] {
                if GetKeyState(vk as i32) < 0 {
                    modifiers |= flag;
                }
            }
            if key == VK_TAB as u32 && modifiers & !MOD_SHIFT == 0 {
                SendMessageW(
                    owner,
                    WM_NEXTDLGCTL,
                    (modifiers & MOD_SHIFT != 0) as usize,
                    0,
                );
                return 0;
            }
            if key == VK_ESCAPE as u32 && modifiers == 0 {
                SendMessageW(owner, WM_COMMAND, IDCANCEL as usize, 0);
                return 0;
            }
            let row = selected(owner);
            if modifiers == 0 && [VK_BACK, VK_DELETE].iter().any(|vk| *vk as u32 == key) {
                (*pointer)
                    .working
                    .retain(|entry| entry.action != ACTIONS[row]);
            } else {
                match Shortcut::new(modifiers, key) {
                    Ok(shortcut) => {
                        (*pointer)
                            .working
                            .retain(|entry| entry.action != ACTIONS[row]);
                        (*pointer).working.push(Binding {
                            action: ACTIONS[row],
                            shortcut: shortcut.text(),
                        });
                    }
                    Err(error) => {
                        SetDlgItemTextW(owner, HINT, wide(&error).as_ptr());
                        return 0;
                    }
                }
            }
            refresh(owner, &*pointer, row, true);
            SetDlgItemTextW(owner, HINT, wide(GUIDE).as_ptr());
            return 0;
        }
        // Do not accept pasted text or characters: combinations are captured as key events.
        if [WM_CHAR, WM_SYSCHAR, WM_PASTE, WM_CUT, WM_CLEAR].contains(&message) {
            return 0;
        }
        CallWindowProcW((*pointer).original_proc, window, message, wparam, lparam)
    }
}
