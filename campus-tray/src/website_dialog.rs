use crate::{
    websites::{MAX_NAME, MAX_URL, Website},
    wide,
};
use std::cell::Cell;
use windows_sys::Win32::{
    Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*,
};

const NAME: i32 = 1001;
const URL: i32 = 1002;
const EM_SETLIMITTEXT: u32 = 0x00c5;
// DWLP_USER follows the LRESULT and DLGPROC slots (both pointer sized).
const DIALOG_USER: i32 = (2 * std::mem::size_of::<isize>()) as i32;
thread_local! { static ACTIVE: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) }; }

struct State {
    initial: Option<Website>,
    other_urls: Vec<String>,
    result: Option<Website>,
}

pub fn active_window() -> HWND {
    ACTIVE.with(Cell::get)
}

pub fn prompt(
    owner: HWND,
    initial: Option<&Website>,
    existing: &[Website],
) -> Result<Option<Website>, String> {
    if !active_window().is_null() {
        return Ok(None);
    }
    let mut state = State {
        initial: initial.cloned(),
        other_urls: existing
            .iter()
            .filter(|website| Some(*website) != initial)
            .map(|website| website.url.clone())
            .collect(),
        result: None,
    };
    let code = unsafe {
        DialogBoxParamW(
            GetModuleHandleW(std::ptr::null()),
            101usize as *const u16,
            owner,
            Some(dialog_proc),
            &mut state as *mut State as isize,
        )
    };
    ACTIVE.with(|active| active.set(std::ptr::null_mut()));
    if code == -1 {
        return Err("无法打开网址编辑窗口。".into());
    }
    Ok(state.result)
}

unsafe extern "system" fn dialog_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    std::panic::catch_unwind(|| handle_message(window, message, wparam, lparam)).unwrap_or_else(
        |_| {
            unsafe {
                EndDialog(window, -1);
            }
            1
        },
    )
}

fn handle_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    if let Some(result) = crate::ui::message(window, message, wparam, lparam) {
        return result;
    }
    unsafe {
        if message == WM_INITDIALOG {
            SetWindowLongPtrW(window, DIALOG_USER, lparam);
            ACTIVE.with(|active| active.set(window));
            let state = &*(lparam as *const State);
            SetWindowTextW(
                window,
                wide(if state.initial.is_some() {
                    "SHUNet · 编辑常用网址"
                } else {
                    "SHUNet · 新增常用网址"
                })
                .as_ptr(),
            );
            for (id, text) in [
                (1003, "名称"),
                (1004, "网址"),
                (1005, "仅支持网页地址；省略协议时自动使用 https://。"),
                (IDOK, "保存"),
                (IDCANCEL, "取消"),
            ] {
                SetDlgItemTextW(window, id, wide(text).as_ptr());
            }
            SendDlgItemMessageW(window, NAME, EM_SETLIMITTEXT, MAX_NAME, 0);
            SendDlgItemMessageW(window, URL, EM_SETLIMITTEXT, MAX_URL, 0);
            SetDlgItemTextW(
                window,
                NAME,
                wide(state.initial.as_ref().map_or("", |w| &w.name)).as_ptr(),
            );
            SetDlgItemTextW(
                window,
                URL,
                wide(state.initial.as_ref().map_or("https://", |w| &w.url)).as_ptr(),
            );
            crate::ui::init(window, crate::ui::Kind::Website);
            SetDlgItemTextW(
                window,
                1600,
                wide(if state.initial.is_some() {
                    "编辑网址"
                } else {
                    "新增网址"
                })
                .as_ptr(),
            );
            SetForegroundWindow(window);
            return 1;
        }
        if message == WM_CLOSE || (message == WM_COMMAND && (wparam & 0xffff) == IDCANCEL as usize)
        {
            EndDialog(window, IDCANCEL as isize);
            return 1;
        }
        if message == WM_COMMAND && (wparam & 0xffff) == IDOK as usize {
            let name = text(window, NAME, MAX_NAME);
            let url = text(window, URL, MAX_URL);
            let state = GetWindowLongPtrW(window, DIALOG_USER) as *mut State;
            if state.is_null() {
                EndDialog(window, -1);
                return 1;
            }
            let result = Website::new(&name, &url).and_then(|website| {
                if (*state).other_urls.iter().any(|url| url == &website.url) {
                    Err("这个网址已经在收藏中，可从“编辑网址”修改名称。".into())
                } else {
                    Ok(website)
                }
            });
            match result {
                Ok(website) => {
                    (*state).result = Some(website);
                    EndDialog(window, IDOK as isize);
                }
                Err(error) => {
                    MessageBoxW(
                        window,
                        wide(&error).as_ptr(),
                        wide("SHUNet · 请检查网址").as_ptr(),
                        MB_OK | MB_ICONWARNING,
                    );
                }
            }
            return 1;
        }
    }
    0
}

fn text(window: HWND, id: i32, limit: usize) -> String {
    let mut buffer = vec![0u16; limit + 1];
    let count = unsafe { GetDlgItemTextW(window, id, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..count as usize])
}
