use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::{
    HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
    WindowsAndMessaging::*,
};

#[repr(usize)]
#[derive(Clone, Copy)]
pub enum State {
    Disconnected,
    Connected,
    Authenticated,
    Authenticating,
    Error,
}

const ICO: [&[u8]; 5] = [
    include_bytes!("../assets/icons/disconnected.ico"),
    include_bytes!("../assets/icons/connected.ico"),
    include_bytes!("../assets/icons/authenticated.ico"),
    include_bytes!("../assets/icons/authenticating.ico"),
    include_bytes!("../assets/icons/error.ico"),
];

const QUERY_ICO: [&[u8]; 5] = [
    include_bytes!("../assets/icons/disconnected-querying.ico"),
    include_bytes!("../assets/icons/connected-querying.ico"),
    include_bytes!("../assets/icons/authenticated-querying.ico"),
    include_bytes!("../assets/icons/authenticating-querying.ico"),
    include_bytes!("../assets/icons/error-querying.ico"),
];

pub struct IconSet {
    handles: [HICON; 5],
    query_handles: [HICON; 5],
    pub size: i32,
}
impl IconSet {
    pub fn for_taskbar(window: HWND) -> Result<Self, String> {
        let size = unsafe {
            let taskbar: Vec<u16> = "Shell_TrayWnd".encode_utf16().chain(Some(0)).collect();
            let taskbar = FindWindowW(taskbar.as_ptr(), std::ptr::null());
            let dpi = GetDpiForWindow(if taskbar.is_null() { window } else { taskbar }).max(96);
            GetSystemMetricsForDpi(SM_CXSMICON, dpi).clamp(16, 64)
        };
        let mut set = Self {
            handles: [std::ptr::null_mut(); 5],
            query_handles: [std::ptr::null_mut(); 5],
            size,
        };
        for (index, data) in ICO.iter().enumerate() {
            set.handles[index] = load(data, size)?;
        }
        for (index, data) in QUERY_ICO.iter().enumerate() {
            set.query_handles[index] = load(data, size)?;
        }
        Ok(set)
    }
    pub fn get(&self, state: State, querying: bool) -> HICON {
        if querying {
            self.query_handles[state as usize]
        } else {
            self.handles[state as usize]
        }
    }
}
impl Drop for IconSet {
    fn drop(&mut self) {
        for icon in self.handles.into_iter().chain(self.query_handles) {
            if !icon.is_null() {
                unsafe {
                    DestroyIcon(icon);
                }
            }
        }
    }
}

fn load(data: &[u8], size: i32) -> Result<HICON, String> {
    let word = |offset| {
        data.get(offset..offset + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let dword = |offset| {
        data.get(offset..offset + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    if word(0) != Some(0) || word(2) != Some(1) {
        return Err("内嵌托盘图标格式无效。".to_owned());
    }
    let count = word(4).ok_or("图标目录缺失。")? as usize;
    let mut entries = Vec::new();
    for index in 0..count.min(32) {
        let base = 6 + 16 * index;
        let width = *data.get(base).ok_or("图标尺寸缺失。")? as i32;
        let width = if width == 0 { 256 } else { width };
        let length = dword(base + 8).ok_or("图标长度缺失。")?;
        let offset = dword(base + 12).ok_or("图标位置缺失。")?;
        // Prefer an exact size, then a larger image, to avoid enlarging small artwork.
        let score = if width >= size {
            width - size
        } else {
            256 + size - width
        };
        entries.push((score, offset, length));
    }
    entries.sort_unstable();
    let (_, offset, length) = entries.first().copied().ok_or("图标目录为空。")?;
    let end = offset.checked_add(length).ok_or("图标范围异常。")?;
    let image = data.get(offset..end).ok_or("图标数据不完整。")?;
    let icon = unsafe {
        CreateIconFromResourceEx(
            image.as_ptr(),
            image.len() as u32,
            1,
            0x0003_0000,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
    };
    if icon.is_null() {
        Err("Windows 无法加载托盘图标。".to_owned())
    } else {
        Ok(icon)
    }
}
