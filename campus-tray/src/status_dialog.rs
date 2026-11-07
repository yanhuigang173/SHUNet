//! Local live dashboard; its UI timer never triggers network requests.
use crate::{
    runtime_history::{self, Entry, Event},
    service::View,
    status_presenter as present,
};
use std::cell::{Cell, RefCell};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{EM_SCROLLCARET, EM_SETLIMITTEXT, EM_SETSEL},
        Input::KeyboardAndMouse::EnableWindow,
        WindowsAndMessaging::*,
    },
};

const PAGE: usize = 200;
thread_local! { static WINDOW: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) }; }
#[derive(Default)]
struct State {
    date: usize,
    reconnect_only: bool,
    page: usize,
    log_text: String,
    log_history: Option<std::sync::Arc<runtime_history::Entries>>,
    log_filter: Option<(usize, bool, usize, i64)>,
    log_expires: i64,
    log_at: i64,
}

pub fn translate(message: &MSG) -> bool {
    let window = WINDOW.with(Cell::get);
    !window.is_null() && unsafe { IsDialogMessageW(window, message) != 0 }
}
pub fn show(owner: HWND) -> Result<(), String> {
    let existing = WINDOW.with(Cell::get);
    if !existing.is_null() {
        unsafe {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
        }
        return Ok(());
    }
    let window = unsafe {
        CreateDialogParamW(
            GetModuleHandleW(std::ptr::null()),
            107usize as _,
            owner,
            Some(proc),
            0,
        )
    };
    if window.is_null() {
        return Err("无法打开运行状态窗口。".into());
    }
    unsafe {
        ShowWindow(window, SW_SHOW);
        // The first ShowWindow can obey STARTUPINFO's hidden launch flag.
        ShowWindow(window, SW_RESTORE);
        SetForegroundWindow(window);
    }
    Ok(())
}
fn set(window: HWND, id: i32, text: &str) {
    // Avoid resetting selection or invalidating the entire window every second.
    unsafe {
        let control = GetDlgItem(window, id);
        let count = GetWindowTextLengthW(control).max(0) as usize;
        let mut current = vec![0u16; count + 1];
        let n = GetWindowTextW(control, current.as_mut_ptr(), current.len() as i32);
        if String::from_utf16_lossy(&current[..n.max(0) as usize]) != text {
            SetWindowTextW(control, crate::wide(text).as_ptr());
        }
    }
}
fn filtered<'a>(
    entries: &'a runtime_history::Entries,
    date: usize,
    reconnect_only: bool,
    now: i64,
) -> Vec<&'a Entry> {
    let day = runtime_history::day(now - (date.saturating_sub(1) as i64) * 86_400_000);
    entries
        .iter()
        .filter(|entry| entry.timestamp >= now - 72 * 3_600_000)
        .filter(|entry| date == 0 || runtime_history::day(entry.timestamp) == day)
        .filter(|entry| !reconnect_only || entry.event != Event::Message)
        .collect()
}
fn render(window: HWND, state: &mut State, view: &View) {
    let now = runtime_history::now_ms();
    let stats = view.history.stats(now);
    use crate::{connection_state::ConnectionState, ui::Tone};
    crate::ui::status_tone(
        window,
        match view.state {
            ConnectionState::Online => Tone::Success,
            ConnectionState::AuthenticationFailed
            | ConnectionState::GatewayUnavailable
            | ConnectionState::Offline => Tone::Warning,
            _ => Tone::Working,
        },
    );
    for (id, text) in [
        (
            1601,
            format!(
                "{}  ·  SHUNet v{}",
                if view.ssid.is_empty() {
                    "Wi-Fi 未连接"
                } else {
                    &view.ssid
                },
                env!("CARGO_PKG_VERSION")
            ),
        ),
        (1811, present::state_label(view).into()),
        (1812, present::duration(view)),
        (1821, format!("{} 次", stats.attempts)),
        (
            1822,
            format!(
                "已确认恢复 {} 次 · 未确认 {} 次",
                stats.recovered,
                stats.attempts.saturating_sub(stats.recovered)
            ),
        ),
        (1831, view.detection_level.label().into()),
        (1832, present::next_check(view)),
        (
            1840,
            format!(
                "{}    本机 IP：{}    下次认证：{}",
                view.network_type_text(),
                view.ip,
                view.operator.label()
            ),
        ),
        (
            1841,
            format!(
                "最近检测 {}    上次在线 {}    {}",
                view.checked_at,
                if view.confirmed_at.is_empty() {
                    "—"
                } else {
                    &view.confirmed_at
                },
                view.detection_reason
            ),
        ),
        (
            1842,
            view.history_warning
                .clone()
                .unwrap_or_else(|| view.state_detail.clone()),
        ),
    ] {
        set(window, id, &text);
    }
    unsafe {
        EnableWindow(
            GetDlgItem(window, 1870),
            i32::from(view.campus && !view.busy),
        );
    }
    let filter = (
        state.date,
        state.reconnect_only,
        state.page,
        (now + 8 * 3_600_000).div_euclid(86_400_000),
    );
    if state
        .log_history
        .as_ref()
        .is_some_and(|old| std::sync::Arc::ptr_eq(old, &view.history))
        && state.log_filter == Some(filter)
        && now >= state.log_at
        && now < state.log_expires
    {
        return;
    }
    let records = filtered(&view.history, state.date, state.reconnect_only, now);
    let pages = records.len().div_ceil(PAGE).max(1);
    state.page = state.page.min(pages - 1);
    state.log_history = Some(view.history.clone());
    state.log_at = now;
    state.log_filter = Some((state.date, state.reconnect_only, state.page, filter.3));
    state.log_expires = view
        .history
        .iter()
        .filter(|entry| entry.timestamp >= now - 72 * 3_600_000)
        .map(|entry| entry.timestamp.saturating_add(72 * 3_600_000 + 1))
        .min()
        .unwrap_or(i64::MAX);
    let end = records.len().saturating_sub(state.page * PAGE);
    let start = end.saturating_sub(PAGE);
    let text = records[start..end]
        .iter()
        .map(|entry| entry.line())
        .collect::<Vec<_>>()
        .join("\r\n");
    if state.log_text != text {
        set(
            window,
            1860,
            &if text.is_empty() {
                "当前筛选条件下没有记录。".into()
            } else {
                text.clone()
            },
        );
        state.log_text = text;
        let end = state.log_text.encode_utf16().count();
        unsafe {
            SendDlgItemMessageW(window, 1860, EM_SETSEL, end, end as isize);
            SendDlgItemMessageW(window, 1860, EM_SCROLLCARET, 0, 0);
        }
    }
    set(window, 1855, &format!("{}/{}", state.page + 1, pages));
    set(
        window,
        1861,
        &format!(
            "共 {} 条 · 每页 {} 条 · 按北京时间统计 · 本地保留 72 小时，启动及每小时自动清理",
            records.len(),
            PAGE
        ),
    );
    unsafe {
        EnableWindow(GetDlgItem(window, 1853), i32::from(state.page + 1 < pages));
        EnableWindow(GetDlgItem(window, 1854), i32::from(state.page > 0));
        EnableWindow(
            GetDlgItem(window, 1870),
            i32::from(view.campus && !view.busy),
        );
    }
}
unsafe extern "system" fn proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    std::panic::catch_unwind(|| handle(window, message, wparam, lparam)).unwrap_or(0)
}
fn handle(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    unsafe {
        if message == WM_INITDIALOG {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                Box::into_raw(Box::new(RefCell::new(State::default()))) as isize,
            );
            WINDOW.with(|cell| cell.set(window));
            SetWindowTextW(window, crate::wide("SHUNet · 运行状态与记录").as_ptr());
            crate::ui::init(window, crate::ui::Kind::Status);
            for (id, label) in [
                (1810, "连接状态"),
                (1820, "今日自动重连"),
                (1830, "检测策略"),
                (1850, "运行记录"),
                (1853, "较早记录"),
                (1854, "较新记录"),
                (1870, "立即检测"),
                (1871, "复制状态"),
                (1872, "复制记录"),
                (1873, "日志目录"),
                (1874, "状态说明"),
                (IDCANCEL, "关闭"),
            ] {
                set(window, id, label);
            }
            for (id, values) in [
                (1851, vec!["最近 72 小时", "今天", "昨天", "前天"]),
                (1852, vec!["全部记录", "仅自动重连"]),
            ] {
                for value in values {
                    SendDlgItemMessageW(
                        window,
                        id,
                        CB_ADDSTRING,
                        0,
                        crate::wide(value).as_ptr() as isize,
                    );
                }
                SendDlgItemMessageW(window, id, CB_SETCURSEL, 0, 0);
            }
            SendDlgItemMessageW(window, 1860, EM_SETLIMITTEXT, 300_000, 0);
            set(window, 1860, "当前筛选条件下没有记录。");
            SetTimer(window, 1, 1000, None);
            let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut RefCell<State>;
            render(window, &mut (&*pointer).borrow_mut(), &crate::snapshot());
            return 1;
        }
        if let Some(result) = crate::ui::message(window, message, wparam, lparam) {
            return result;
        }
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut RefCell<State>;
        if pointer.is_null() {
            return 0;
        }
        if message == WM_DESTROY {
            KillTimer(window, 1);
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            WINDOW.with(|cell| cell.set(std::ptr::null_mut()));
            drop(Box::from_raw(pointer));
            return 0;
        }
        if message == WM_CLOSE || (message == WM_COMMAND && wparam & 0xffff == IDCANCEL as usize) {
            if (&*pointer).try_borrow_mut().is_err() {
                PostMessageW(window, WM_CLOSE, 0, 0);
                return 1;
            }
            DestroyWindow(window);
            return 1;
        }
        if message == WM_COMMAND && (1871..=1874).contains(&((wparam & 0xffff) as i32)) {
            let (date, reconnect_only) = {
                let Ok(state) = (&*pointer).try_borrow() else {
                    return 0;
                };
                (state.date, state.reconnect_only)
            };
            let result = match (wparam & 0xffff) as i32 {
                1871 => crate::platform::copy_text(&present::details(&crate::snapshot())),
                1872 => {
                    let view = crate::snapshot();
                    let text = filtered(
                        &view.history,
                        date,
                        reconnect_only,
                        runtime_history::now_ms(),
                    )
                    .iter()
                    .map(|entry| entry.line())
                    .collect::<Vec<_>>()
                    .join("\r\n");
                    crate::platform::copy_text(&text)
                }
                1873 => crate::platform::try_open(&runtime_history::folder().to_string_lossy()),
                _ => {
                    crate::alert(window, crate::connection_state::LEGEND);
                    Ok(())
                }
            };
            if let Err(error) = result {
                crate::alert(window, &error);
            }
            return 1;
        }
        let Ok(mut state) = (&*pointer).try_borrow_mut() else {
            return 0;
        };
        if message == WM_COMMAND {
            let id = (wparam & 0xffff) as i32;
            match id {
                1851 | 1852 if wparam >> 16 == CBN_SELCHANGE as usize => {
                    state.date =
                        SendDlgItemMessageW(window, 1851, CB_GETCURSEL, 0, 0).max(0) as usize;
                    state.reconnect_only =
                        SendDlgItemMessageW(window, 1852, CB_GETCURSEL, 0, 0) == 1;
                    state.page = 0;
                }
                1853 => state.page = state.page.saturating_add(1),
                1854 => state.page = state.page.saturating_sub(1),
                1870 => crate::send(crate::service::Command::Refresh),
                _ => return 0,
            }
            render(window, &mut state, &crate::snapshot());
            return 1;
        }
        if message == WM_TIMER {
            render(window, &mut state, &crate::snapshot());
            return 1;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_do_not_mix_dates_or_manual_events() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-18T00:00:05+08:00")
            .unwrap()
            .timestamp_millis();
        let entries: runtime_history::Entries = vec![
            Entry {
                id: 1,
                timestamp: now - 10_000,
                event: Event::AutoReconnectRequest,
                attempt: None,
                text: "昨天".into(),
            },
            Entry {
                id: 2,
                timestamp: now,
                event: Event::Message,
                attempt: None,
                text: "手动".into(),
            },
            Entry {
                id: 3,
                timestamp: now,
                event: Event::AutoReconnectOnline,
                attempt: Some(1),
                text: "恢复".into(),
            },
        ]
        .into();
        assert_eq!(
            filtered(&entries, 1, true, now)
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![3]
        );
        assert_eq!(
            filtered(&entries, 2, true, now)
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(filtered(&entries, 0, false, now).len(), 3);
    }

    #[test]
    #[ignore = "Opens only the local status window with synthetic data; no network calls or credential access"]
    fn native_status_window_smoke() {
        let _desktop = crate::menu_summary::tests::TestDesktop::new();
        unsafe {
            windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            );
        }
        let now = runtime_history::now_ms();
        let mut view = View {
            ssid: "Shu(ForAll)".into(),
            ip: "10.98.135.180".into(),
            campus: true,
            state: crate::connection_state::ConnectionState::Online,
            checked_at: "16:51:20".into(),
            confirmed_at: "16:51:20".into(),
            state_detail: "已通过当前物理 Wi-Fi 查询并确认本机会话在线；无需重复登录".into(),
            detection_reason: "按所选固定档位查询".into(),
            ..View::default()
        };
        view.online_session.confirm(None);
        let mut entries = vec![
            Entry {
                id: 1,
                timestamp: now - 120_000,
                event: Event::AutoReconnectRequest,
                attempt: None,
                text: "自动重连：已发送认证请求，结果见后续记录。".into(),
            },
            Entry {
                id: 2,
                timestamp: now - 115_000,
                event: Event::AutoReconnectOnline,
                attempt: Some(1),
                text: "自动重连已通过门户确认恢复在线。".into(),
            },
        ];
        for index in 0..220 {
            entries.push(Entry {
                id: index + 3,
                timestamp: now - 100_000 + index as i64 * 100,
                event: Event::Message,
                attempt: None,
                text: format!(
                    "布局测试记录 {}：校园网已认证在线，无需重复登录。",
                    index + 1
                ),
            });
        }
        view.history = std::sync::Arc::new(entries.into());
        show(std::ptr::null_mut()).unwrap();
        let window = WINDOW.with(Cell::get);
        let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut RefCell<State> };
        assert_ne!(
            unsafe { IsWindowVisible(window) },
            0,
            "status window must be visible even when the process starts hidden"
        );
        unsafe {
            KillTimer(window, 1);
        }
        render(window, &mut unsafe { &*pointer }.borrow_mut(), &view);
        let pump = || unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        };
        for _ in 0..12 {
            pump();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let text = |id| unsafe {
            let control = GetDlgItem(window, id);
            let mut data = vec![0u16; GetWindowTextLengthW(control) as usize + 1];
            let length = GetWindowTextW(control, data.as_mut_ptr(), data.len() as i32);
            String::from_utf16_lossy(&data[..length as usize])
        };
        assert_eq!(text(1821), "1 次");
        assert_eq!(text(1855), "1/2");
        assert!(
            unsafe {
                SendDlgItemMessageW(
                    window,
                    1860,
                    windows_sys::Win32::UI::Controls::EM_GETFIRSTVISIBLELINE,
                    0,
                    0,
                )
            } > 0,
            "latest page should initially scroll to its newest records"
        );
        assert!(!text(1860).contains("布局测试记录 1："));
        {
            let mut state = unsafe { &*pointer }.borrow_mut();
            state.page = 1;
            render(window, &mut state, &view);
            assert_eq!(text(1855), "2/2");
            assert!(text(1860).contains("自动重连已通过门户确认"));
            state.page = 0;
            state.reconnect_only = true;
            render(window, &mut state, &view);
            assert!(text(1860).contains("自动重连"));
            assert!(!text(1860).contains("布局测试记录"));
            state.reconnect_only = false;
            render(window, &mut state, &view);
        }
        pump();
        crate::network_tools_dialog::tests::capture(window, "status-window");
        unsafe {
            SendMessageW(window, WM_CLOSE, 0, 0);
        }
        assert!(WINDOW.with(Cell::get).is_null());
        show(std::ptr::null_mut()).unwrap();
        unsafe {
            SendMessageW(WINDOW.with(Cell::get), WM_CLOSE, 0, 0);
        }
        assert!(WINDOW.with(Cell::get).is_null());
    }
}
