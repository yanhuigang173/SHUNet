#![windows_subsystem = "windows"]
#![allow(dead_code)] // The independent backend also exposes desktop/diagnostic entry points.

mod connection_state;
mod auth_trace;
mod detection;
mod dialogs;
mod hotkey_dialog;
mod hotkeys;
mod icons;
mod network;
mod network_tools;
mod network_tools_dialog;
mod network_policy;
mod operator;
mod online_session;
mod platform;
mod power;
mod power_dialog;
mod reconnect;
mod runtime_history;
mod menu_summary;
mod menu_theme;
mod status_presenter;
mod status_dialog;
mod service;
mod settings;
mod speed_dialog;
mod speedtest;
mod startup;
mod ui;
mod website_dialog;
mod websites;
mod wifi;

use service::{Command, Controller, View};
use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{Shell::*, WindowsAndMessaging::*},
};

const TRAY_EVENT: u32 = WM_APP + 1;
// shellapi.h defines NIN_KEYSELECT as this expression; windows-sys omits the macro.
const TRAY_KEY_SELECT: u32 = NIN_SELECT | NINF_KEY;
const QUERY_FEEDBACK_TIMER: usize = 2;
const QUERY_FEEDBACK_DURATION: std::time::Duration = std::time::Duration::from_millis(700);
const MANUAL_QUERY_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);
const SERVICE_EVENT: u32 = WM_APP + 2;
const SPEED_EVENT: u32 = WM_APP + 3;
const LOGIN: u32 = 10;
const LOGOUT: u32 = 11;
const AUTO_RECONNECT: u32 = 12;
const AUTO_AUTH: u32 = 13;
const PROXY: u32 = 14;
const SCAN: u32 = 15;
const DISCONNECT: u32 = 16;
const REFRESH: u32 = 17;
const CONFIG: u32 = 18;
const IMPORT: u32 = 19;
const PORTAL: u32 = 20;
const WIFI_SETTINGS: u32 = 21;
const PROXY_SETTINGS: u32 = 22;
const COPY_IP: u32 = 23;
const COPY_LOGS: u32 = 24;
const EXIT: u32 = 25;
const STARTUP: u32 = 26;
const STARTUP_SETTINGS: u32 = 27;
const WEBSITE_ADD: u32 = 28;
const CONFIG_LOCATION: u32 = 29;
const CONFIG_FOLDER: u32 = 30;
const CONFIG_PATH_COPY: u32 = 31;
const IMPORT_FILE: u32 = 32;
const EXPORT_FILE: u32 = 33;
const HELP: u32 = 34;
const HOTKEY_SETTINGS: u32 = 35;
const SHOW_MENU: u32 = 36;
const ICON_LEGEND: u32 = 37;
const STATUS_DETECTION: u32 = 38;
const SPEED_DIRECT: u32 = 39;
const SPEED_PROXY: u32 = 40;
const SPEED_CANCEL: u32 = 41;
const SPEED_RESULTS: u32 = 42;
const SPEED_COPY: u32 = 43;
const NETWORK_AUTO: u32 = 44;
const NETWORK_CAMPUS: u32 = 45;
const NETWORK_ORDINARY: u32 = 46;
const RESUME_CAMPUS: u32 = 47;
const WIFI_PRIVACY: u32 = 48;
const DETECTION_ADAPTIVE: u32 = 49;
const DETECTION_HIGH: u32 = 50;
const DETECTION_MEDIUM: u32 = 51;
const DETECTION_LOW: u32 = 52;
const DETECTION_EXTREME: u32 = 53;
const OPERATOR_CAMPUS: u32 = 54;
const OPERATOR_TELECOM: u32 = 55;
const OPERATOR_UNICOM: u32 = 56;
const OPERATOR_MOBILE: u32 = 57;
const THEME_DEFAULT: u32 = 61;
const THEME_CLASSIC: u32 = 62;
const THEME_MODERN: u32 = 63;
const POWER_TOOLS: u32 = 64;
const IP_INFORMATION: u32 = 58;
const LAN_DEVICES: u32 = 59;
const STATUS_WINDOW: u32 = 60;
const NETWORK_BASE: u32 = 100;
const WEBSITE_BASE: u32 = 1000;
const WEBSITE_EDIT_BASE: u32 = 2000;
const WEBSITE_DELETE_BASE: u32 = 3000;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn copy_wide<const N: usize>(destination: &mut [u16; N], text: &str) {
    destination.fill(0);
    for (slot, code) in destination.iter_mut().take(N - 1).zip(text.encode_utf16()) {
        *slot = code;
    }
    if N > 1 && (0xd800..=0xdbff).contains(&destination[N - 2]) {
        destination[N - 2] = 0;
    }
}

struct App {
    window: HWND,
    controller: Controller,
    shared: Arc<Mutex<View>>,
    observer: Option<wifi::ConnectionObserver>,
    icons: icons::IconSet,
    taskbar_created: u32,
    last_tooltip: String,
    last_color: usize,
    last_query: u64,
    query_feedback_until: Option<std::time::Instant>,
    query_feedback_timer: bool,
    icon_added: bool,
    last_notice: u64,
    hotkeys: Option<hotkeys::Manager>,
    hotkey_warning: Option<String>,
    speedtest: speedtest::Manager,
    last_speed_notice: u64,
    pending_credentials: Option<std::sync::mpsc::Receiver<service::CredentialDraft>>,
}
thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static MENU_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static LAST_MANUAL_QUERY: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    static WIFI_MENU_HANDLE: Cell<HMENU> = const { Cell::new(std::ptr::null_mut()) };
    static WIFI_MENU_NETWORKS: RefCell<Vec<network::WifiNetwork>> = const { RefCell::new(Vec::new()) };
}

impl App {
    fn announce_speed(&self) {
        let speed = self.speedtest.snapshot();
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.window,
            uID: 1,
            uFlags: NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        copy_wide(
            &mut data.szInfoTitle,
            if speed.current.error.is_some() {
                "SHUNet · 测速未全部完成"
            } else {
                "SHUNet · 测速完成"
            },
        );
        copy_wide(
            &mut data.szInfo,
            &format!("{}\n点击查看进度与结果。", speed.current.compact()),
        );
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
    fn notify(&mut self, operation: u32) -> bool {
        let operation = if !self.icon_added { NIM_ADD } else { operation };
        let (mut tooltip, color, query, same_network, submitting, scanning, session_text) =
            if let Ok(view) = self.shared.lock() {
                let ssid = if view.ssid.is_empty() {
                    if view.state == connection_state::ConnectionState::Disconnected {
                        "未连接 Wi-Fi"
                    } else if view.state == connection_state::ConnectionState::OtherConnection {
                        "未连接 Wi-Fi（其他接口已连接）"
                    } else {
                        "Wi-Fi 状态未获取"
                    }
                } else {
                    &view.ssid
                };
                let color = view.state.icon();
                let session_text = format!("{}\n{}", view.network_type_text(), view.online_duration_text());
                (
                    format!(
                        "SHUNet\n{ssid}\n{session_text}\n{}",
                        view.state.label(),
                    ),
                    color,
                    view.query.clone(),
                    view.query.ssid == view.ssid && view.query.ip == view.ip,
                    matches!(
                        view.state,
                        connection_state::ConnectionState::Authenticating
                            | connection_state::ConnectionState::SigningOut
                    ),
                    view.scanning,
                    session_text,
                )
            } else {
                (
                    "SHUNet\n网络状态暂不可用".to_owned(),
                    icons::State::Connected,
                    service::QueryActivity::default(),
                    false,
                    false,
                    false,
                    String::new(),
                )
            };
        let now = std::time::Instant::now();
        if query.serial != 0 && query.serial != self.last_query {
            self.last_query = query.serial;
            // A fast manual read may finish before its first UI message is
            // handled. Its serial still guarantees a short visible acknowledgement.
            if query.manual {
                self.query_feedback_until = Some(now + QUERY_FEEDBACK_DURATION);
            }
        }
        let feedback = self.query_feedback_until.is_some_and(|until| until > now);
        if !feedback {
            self.query_feedback_until = None;
        }
        unsafe {
            // Repeated service notifications must not reset the cleanup timer.
            // Poll the short feedback deadline, then retire this timer when idle.
            if feedback && !self.query_feedback_timer {
                self.query_feedback_timer = SetTimer(self.window, QUERY_FEEDBACK_TIMER, 100, None) != 0;
            } else if !feedback && self.query_feedback_timer {
                KillTimer(self.window, QUERY_FEEDBACK_TIMER);
                self.query_feedback_timer = false;
            }
        }
        let queued = self.controller.refresh_queued() || (query.manual && query.active && scanning);
        let querying = query_badge_visible(&query, same_network, submitting, queued, feedback);
        let color = if querying && same_network && query.active && !submitting {
            query.previous_state.icon()
        } else {
            color
        };
        if queued {
            tooltip.push_str("\n手动查询已接收，等待后台处理");
        } else if querying && query.active && same_network && !submitting {
            tooltip = format!(
                "SHUNet\n{}\n{session_text}\n正在查询状态（只读）\n查询前：{}",
                if query.ssid.is_empty() {
                    "未连接 Wi-Fi"
                } else {
                    &query.ssid
                },
                query.previous_state.label(),
            );
        } else if query.manual && query.active && !same_network {
            tooltip.push_str("\n查询处理中，等待当前连接稳定");
        } else if feedback {
            tooltip.push_str("\n手动查询请求已接收；处理结果见运行状态与记录");
        }
        let speed = self.speedtest.snapshot();
        if speed.running {
            tooltip.push_str(&format!("\n{}", speed.progress_label()));
        }
        let icon_key = color as usize + if querying { 5 } else { 0 };
        if operation == NIM_MODIFY && tooltip == self.last_tooltip && icon_key == self.last_color {
            return true;
        }
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.window,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: TRAY_EVENT,
            hIcon: self.icons.get(color, querying),
            ..Default::default()
        };
        copy_wide(&mut data.szTip, &tooltip);
        let was_added = self.icon_added;
        let delivered = notify_icon_with_recovery(operation, |operation| unsafe { Shell_NotifyIconW(operation, &data) != 0 });
        if let Some(delivered) = delivered {
            self.icon_added = true;
            self.last_tooltip = tooltip;
            self.last_color = icon_key;
            if delivered == NIM_ADD || !was_added {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                unsafe {
                    Shell_NotifyIconW(NIM_SETVERSION, &data);
                }
            }
        }
        if delivered.is_none() {
            self.icon_added = false;
        }
        delivered.is_some()
    }
}

// Explorer can reject MODIFY transiently while the old icon still exists. ADD
// then also fails for that same identity. Always try the other operation so one
// failure cannot latch the app into ADD-only retries with a permanently stale icon.
fn notify_icon_with_recovery(operation: u32, mut deliver: impl FnMut(u32) -> bool) -> Option<u32> {
    if deliver(operation) { return Some(operation); }
    let fallback = if operation == NIM_ADD { NIM_MODIFY } else { NIM_ADD };
    if deliver(fallback) { return Some(fallback); }
    if operation == NIM_MODIFY && deliver(NIM_MODIFY) { return Some(NIM_MODIFY); }
    None
}

fn accept_manual_query(last: &mut Option<std::time::Instant>, now: std::time::Instant) -> bool {
    if last.is_some_and(|last| now.saturating_duration_since(last) < MANUAL_QUERY_DEBOUNCE) { return false; }
    *last = Some(now);
    true
}
fn query_badge_visible(
    query: &service::QueryActivity,
    same_network: bool,
    submitting: bool,
    queued: bool,
    feedback: bool,
) -> bool {
    queued || feedback || (query.active && !submitting && (same_network || query.manual))
}

impl Drop for App {
    fn drop(&mut self) {
        self.controller.send(Command::Stop);
        unsafe {
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.window,
                uID: 1,
                ..Default::default()
            };
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|arg|arg == "--power-run") {
        if arguments.len()!=2 || !power::valid_id(&arguments[1]) {std::process::exit(2);}
        if power::runner(&arguments[1]).is_err() {std::process::exit(1);}
        return;
    }

    if let Err(error) = run() {
        alert(std::ptr::null_mut(), &error);
    }
}

fn run() -> Result<(), String> {
    unsafe {
        // Retain the legacy identity so SHUNet and an old tray exe cannot run together.
        let mutex = CreateMutexW(
            std::ptr::null(),
            0,
            wide("Local\\CampusLinkTray.SingleInstance").as_ptr(),
        );
        if mutex.is_null() {
            return Err("无法创建程序实例锁。".to_owned());
        }
        struct Instance(HANDLE);
        impl Drop for Instance {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let _instance = Instance(mutex);
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        let module = GetModuleHandleW(std::ptr::null());
        let class = wide("CampusLinkTray.HiddenWindow");
        let definition = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&definition) == 0 {
            return Err("无法注册托盘消息窗口。".to_owned());
        }
        // A hidden top-level window receives TaskbarCreated and power broadcasts.
        // It is never shown and creates no taskbar button or main application UI.
        let window = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("SHUNet").as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            module,
            std::ptr::null(),
        );
        if window.is_null() {
            return Err("无法创建托盘消息窗口。".to_owned());
        }
        let shared = Arc::new(Mutex::new(View::default()));
        let controller = service::start(shared.clone(), window as usize, SERVICE_EVENT)?;
        let icons = icons::IconSet::for_taskbar(window)?;
        let mut app = App {
            window,
            controller,
            shared,
            observer: wifi::ConnectionObserver::new().ok(),
            icons,
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            last_tooltip: String::new(),
            last_color: usize::MAX,
            last_query: 0,
            query_feedback_until: None,
            query_feedback_timer: false,
            icon_added: false,
            last_notice: 0,
            hotkeys: Some(hotkeys::Manager::new(window)),
            hotkey_warning: None,
            speedtest: speedtest::Manager::new(window as usize, SPEED_EVENT),
            last_speed_notice: 0,
            pending_credentials: None,
        };
        // At logon Explorer may not have created its notification area yet.
        // Keep the hidden process alive; the timer / TaskbarCreated retries add.
        app.notify(NIM_ADD);
        APP.with(|slot| *slot.borrow_mut() = Some(app));
        SetTimer(window, 1, 1000, None);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, std::ptr::null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if power_dialog::translate(&message) || status_dialog::translate(&message) || network_tools_dialog::translate(&message) || speed_dialog::translate(&message) {
                continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        APP.with(|slot| {
            slot.borrow_mut().take();
        });
        Ok(())
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Rust must never unwind through a Windows callback boundary.
    std::panic::catch_unwind(|| handle_message(window, message, wparam, lparam))
        .unwrap_or_else(|_| unsafe { DefWindowProcW(window, message, wparam, lparam) })
}

fn handle_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(result)=menu_summary::message(window,message,wparam,lparam){return result;}
    if message == WM_INITMENUPOPUP
        && WIFI_MENU_HANDLE.with(|menu| !menu.get().is_null() && menu.get() == wparam as HMENU)
    {
        fill_wifi_menu(wparam as HMENU, &snapshot());
        return 0;
    }
    if message == TRAY_EVENT && (lparam as u32 & 0xffff) == NIN_BALLOONUSERCLICK {
        if let Err(error) = speed_dialog::show(window) {
            alert(window, &error);
        }
        return 0;
    }
    if message == WM_HOTKEY {
        if !MENU_ACTIVE.with(Cell::get) {
            let action = APP.with(|slot| {
                slot.try_borrow()
                    .ok()
                    .and_then(|slot| slot.as_ref()?.hotkeys.as_ref()?.action(wparam))
            });
            if let Some(action) = action {
                let id = action.command();
                let view = snapshot();
                if action_enabled(id, &view) {
                    if id == SHOW_MENU {
                        show_menu(window);
                    } else if let Some(_session) = MenuSession::begin() {
                        execute_action(window, id, &view);
                    }
                }
            }
        }
        return 0;
    }
    if message == TRAY_EVENT {
        match (lparam as u32) & 0xffff {
            NIN_SELECT | WM_LBUTTONUP => {
                if !MENU_ACTIVE.with(Cell::get) {
                    let view = snapshot();
                    if action_enabled(REFRESH, &view) {
                        if let Some(_session) = MenuSession::begin() {
                            execute_action(window, REFRESH, &view);
                        }
                    }
                }
                return 0;
            }
            WM_CONTEXTMENU | TRAY_KEY_SELECT => {
                show_menu(window);
                return 0;
            }
            _ => {}
        }
    }
    if message == WM_TIMER && wparam == QUERY_FEEDBACK_TIMER {
        // Keep it armed if a reentrant shell callback temporarily holds APP.
        // notify retires it after the minimum feedback time has elapsed.
        APP.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                if let Some(app) = slot.as_mut() {
                    app.notify(NIM_MODIFY);
                }
            }
        });
        return 0;
    }
    if message == SERVICE_EVENT || message == SPEED_EVENT || message == WM_TIMER {
        if message != WM_TIMER {
            speed_dialog::changed();
        }
        let notice = APP.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                if let Some(app) = slot.as_mut() {
                    if app
                        .observer
                        .as_ref()
                        .is_some_and(|observer| observer.take_changed())
                    {
                        app.speedtest
                            .cancel_for("网络连接发生变化，本次测速已取消；连接稳定后可重新测速。");
                        app.controller.send(Command::NetworkChanged);
                    }
                    if app.shared.lock().is_ok_and(|view| {
                        view.busy
                            && matches!(
                                view.state,
                                connection_state::ConnectionState::Authenticating
                                    | connection_state::ConnectionState::SigningOut
                            )
                    }) {
                        app.speedtest
                            .cancel_for("正在执行校园网认证或下线，本次测速已取消。");
                    }
                    app.notify(NIM_MODIFY);
                    let bindings = app.shared.lock().ok().map(|view| view.hotkeys.clone());
                    if let (Some(manager), Some(bindings)) = (app.hotkeys.as_mut(), bindings) {
                        if let Some(warning) = manager.sync(&bindings) {
                            app.hotkey_warning = Some(warning);
                        }
                    }
                    // Show completion messages only after menus / modal dialogs close.
                    if !MENU_ACTIVE.with(Cell::get) {
                        if let Some(warning) = app.hotkey_warning.take() {
                            return Some(warning);
                        }
                        if let Ok(view) = app.shared.lock() {
                            if let Some((serial, text)) = &view.notice {
                                if *serial != app.last_notice {
                                    app.last_notice = *serial;
                                    return Some(text.clone());
                                }
                            }
                        }
                        if let Some((serial, _)) = app.speedtest.snapshot().notice {
                            if serial != app.last_speed_notice {
                                app.last_speed_notice = serial;
                                if !speed_dialog::is_open() {
                                    app.announce_speed();
                                }
                            }
                        }
                    }
                }
            }
            None
        });
        if menu_summary::is_open() {
            let mut current = snapshot();
            if let Ok((enabled, _)) = platform::read_proxy() { current.proxy = enabled; }
            menu_summary::refresh(&current);
        }
        if let Some(notice) = notice {
            let _session = MenuSession::begin();
            alert(window, &notice);
        }
        complete_account_settings(window);
        return 0;
    }
    if message == WM_POWERBROADCAST && (wparam == 7 || wparam == 18) {
        send(Command::Wake);
        return 1;
    }
    if message == WM_POWERBROADCAST && wparam == 4 {
        APP.with(|slot| {
            if let Ok(slot) = slot.try_borrow() {
                if let Some(app) = slot.as_ref() {
                    app.speedtest.cancel();
                }
            }
        });
        return 1;
    }
    let restart = APP
        .with(|slot| {
            slot.try_borrow()
                .ok()
                .and_then(|slot| slot.as_ref().map(|app| app.taskbar_created))
        })
        .unwrap_or(0);
    if restart != 0 && message == restart {
        APP.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                if let Some(app) = slot.as_mut() {
                    app.icon_added = false;
                    if let Ok(icons) = icons::IconSet::for_taskbar(window) {
                        app.icons = icons;
                    }
                    app.notify(NIM_ADD);
                }
            }
        });
        return 0;
    }
    if [WM_DPICHANGED, WM_DISPLAYCHANGE, WM_SETTINGCHANGE].contains(&message) {
        APP.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                if let Some(app) = slot.as_mut() {
                    if let Ok(icons) = icons::IconSet::for_taskbar(window) {
                        if icons.size != app.icons.size {
                            app.icons = icons;
                            app.last_color = usize::MAX;
                            app.notify(NIM_MODIFY);
                        }
                    }
                }
            }
        });
    }
    if message == WM_CLOSE {
        unsafe {
            DestroyWindow(window);
        }
        return 0;
    }
    if message == WM_DESTROY {
        unsafe {
            KillTimer(window, 1);
            KillTimer(window, QUERY_FEEDBACK_TIMER);
            PostQuitMessage(0);
        }
        return 0;
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn snapshot() -> View {
    APP.with(|slot| {
        slot.try_borrow().ok().and_then(|slot| {
            slot.as_ref()
                .and_then(|app| app.shared.lock().ok().map(|view| view.clone()))
        })
    })
    .unwrap_or_default()
}
fn send(command: Command) {
    let refresh = matches!(&command, Command::Refresh);
    let ok = APP
        .with(|slot| {
            slot.try_borrow().ok().and_then(|slot| {
                slot.as_ref().map(|app| {
                    if matches!(
                        &command,
                        Command::Wake
                            | Command::Login
                            | Command::Logout
                            | Command::Connect(_)
                            | Command::Disconnect
                            | Command::Stop
                    ) {
                        app.speedtest.cancel();
                    }
                    app.controller.send(command)
                })
            })
        })
        .unwrap_or(false);
    if ok && refresh {
        // Acknowledge on the UI thread immediately, even if the worker is still
        // scanning/performing another task and hasn't published QueryActivity yet.
        APP.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                if let Some(app) = slot.as_mut() {
                    app.query_feedback_until = Some(
                        std::time::Instant::now() + QUERY_FEEDBACK_DURATION,
                    );
                    app.notify(NIM_MODIFY);
                }
            }
        });
    }
    if !ok {
        alert(
            std::ptr::null_mut(),
            "当前请求未加入后台队列，请等待当前操作结束后重试。",
        );
    }
}

struct Menu(HMENU, Vec<Box<menu_summary::LiveRow>>, std::rc::Rc<menu_summary::Resources>, bool);
struct WifiMenuSession;
impl WifiMenuSession {
    fn new(menu: &Menu) -> Self {
        let handle = menu.0;
        menu_summary::set_wifi_resources(Some(menu.2.clone()));
        WIFI_MENU_HANDLE.with(|menu| menu.set(handle));
        Self
    }
}
impl Drop for WifiMenuSession {
    fn drop(&mut self) {
        menu_summary::set_wifi_resources(None);
        WIFI_MENU_HANDLE.with(|menu| menu.set(std::ptr::null_mut()));
        WIFI_MENU_NETWORKS.with(|networks| {
            let _ = std::mem::take(&mut *networks.borrow_mut());
        });
    }
}
struct MenuSession;
impl MenuSession {
    fn begin() -> Option<Self> {
        MENU_ACTIVE.with(|active| {
            if active.get() {
                None
            } else {
                active.set(true);
                Some(Self)
            }
        })
    }
}
impl Drop for MenuSession {
    fn drop(&mut self) {
        MENU_ACTIVE.with(|active| active.set(false));
    }
}

impl Menu {
    fn new() -> Self {
        let handle = unsafe { CreatePopupMenu() };
        Self(handle, Vec::new(), menu_summary::Resources::new(handle), true)
    }
    fn item(&self, id: u32, label: &str, enabled: bool, checked: bool) {
        if self.2.theme != menu_theme::MenuTheme::Classic {
            menu_summary::static_item(self, id, label, enabled, checked, std::ptr::null_mut());
            return;
        }
        let flags =
            MF_STRING | if enabled { 0 } else { MF_GRAYED } | if checked { MF_CHECKED } else { 0 };
        let label = Self::label(id, label);
        unsafe {
            AppendMenuW(self.0, flags, id as usize, wide(&label).as_ptr());
        }
    }
    fn label(id: u32, label: &str) -> String {
        let shortcut = APP.with(|slot| {
            slot.try_borrow().ok().and_then(|slot| {
                slot.as_ref()?
                    .hotkeys
                    .as_ref()?
                    .label_for_command(id)
                    .map(str::to_owned)
            })
        });
        let label = label
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .replace('&', "&&");
        let label = if let Some(shortcut) = shortcut {
            format!("{label}\t{shortcut}")
        } else {
            label
        };
        label
    }
    fn line(&self) {
        if self.2.theme != menu_theme::MenuTheme::Classic {
            menu_summary::separator(self);
            return;
        }
        unsafe {
            AppendMenuW(self.0, MF_SEPARATOR, 0, std::ptr::null());
        }
    }
    fn submenu(&self, label: &str, mut menu: Menu) {
        if self.2.theme == menu_theme::MenuTheme::Classic {
            unsafe { AppendMenuW(self.0, MF_POPUP, menu.0 as usize, wide(label).as_ptr()); }
        } else {
            menu_summary::static_item(self, 0, label, true, false, menu.0);
        }
        menu.3 = false; // The parent destroys native handles; retain Rust drawing resources.
        self.2.children.borrow_mut().push(menu);
    }
}
impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            if self.3 && !self.0.is_null() {
                DestroyMenu(self.0);
            }
        }
    }
}

// Rebuild only when a submenu opens, never while the user is choosing a row.
// Keep the exact displayed network objects for dispatch after TrackPopupMenu.
fn fill_wifi_menu(handle: HMENU, view: &View) {
    let resources = menu_summary::wifi_resources().expect("Wi-Fi menu session resources");
    let wifi_menu = Menu(handle, Vec::new(), resources, false);
    unsafe {
        for position in (0..GetMenuItemCount(handle)).rev() {
            DeleteMenu(handle, position as u32, MF_BYPOSITION);
        }
    }
    wifi_menu.2.rows.borrow_mut().clear();
    WIFI_MENU_NETWORKS.with(|networks| *networks.borrow_mut() = view.networks.clone());
    wifi_menu.item(
        SCAN,
        if view.scanning {
            "正在扫描附近 Wi-Fi…"
        } else if view.busy {
            "请等待当前操作完成"
        } else {
            "刷新附近 Wi-Fi"
        },
        !view.busy && !view.scanning,
        false,
    );
    wifi_menu.line();
    if !view.scan_detail.is_empty() {
        let heading = if view.scanning {
            format!("刷新中 · 已有 {} 个网络", view.networks.len())
        } else if view.scan_cached {
            format!("{} 个网络 · 含缓存，完整刷新未确认", view.networks.len())
        } else {
            format!(
                "{} 个网络 · 已刷新 {}",
                view.networks.len(),
                view.scanned_at
            )
        };
        wifi_menu.item(0, &heading, false, false);
    }
    if !view.ssid.is_empty()
        && !view
            .networks
            .iter()
            .any(|network| network.ssid == view.ssid)
    {
        wifi_menu.item(0, &format!("{} [当前连接]", view.ssid), false, true);
    }
    if view.networks.is_empty() {
        wifi_menu.item(
            0,
            if view.scanning {
                "正在获取附近网络，请稍候…"
            } else {
                "暂无网络，可刷新或检查 Wi-Fi 设置"
            },
            false,
            false,
        );
    }
    for (index, network) in view.networks.iter().take(40).enumerate() {
        let campus = if network::is_enterprise_campus_ssid(&network.ssid) { " [802.1X · Windows 登录]" } else if network.ssid == view.ssid
            && view.network_mode == network_policy::NetworkMode::Ordinary
        {
            " [普通网络]"
        } else if network.ssid == view.ssid && view.campus {
            " [校园网络]"
        } else if network::is_campus_ssid(&network.ssid) {
            " [校园候选]"
        } else {
            ""
        };
        wifi_menu.item(
            NETWORK_BASE + index as u32,
            &format!("{}{}  {}%", network.ssid, campus, network.signal),
            !view.busy && network.ssid != view.ssid,
            network.ssid == view.ssid,
        );
    }
    wifi_menu.line();
    wifi_menu.item(
        DISCONNECT,
        "断开当前 Wi-Fi",
        !view.busy && !view.ssid.is_empty(),
        false,
    );
    wifi_menu.item(WIFI_SETTINGS, "Windows Wi-Fi 设置…", true, false);
    wifi_menu.item(WIFI_PRIVACY, "Wi-Fi 访问权限（位置设置）…", true, false);
}

fn theme_menu(selected:menu_theme::MenuTheme)->Menu {
    let menu=Menu::new();
    for (index,theme) in menu_theme::MenuTheme::ALL.into_iter().enumerate() {
        menu.item(THEME_DEFAULT+index as u32,theme.label(),true,theme==selected);
    }
    menu
}

fn show_menu(window: HWND) {
    let dialog = website_dialog::active_window();
    if !dialog.is_null() {
        unsafe {
            SetForegroundWindow(dialog);
        }
        return;
    }
    let Some(_session) = MenuSession::begin() else {
        return;
    };
    let mut view = snapshot();
    if let Ok((enabled, _)) = platform::read_proxy() {
        view.proxy = enabled;
    }
    let _theme = menu_theme::Session::new(view.menu_theme);
    let mut menu = Menu::new();
    menu_summary::header(&mut menu, &view);
    menu.line();
    menu_summary::append_actions(&mut menu, &view);
    menu.line();
    let wifi_menu = Menu::new();
    let _wifi_session = WifiMenuSession::new(&wifi_menu);
    fill_wifi_menu(wifi_menu.0, &view);
    menu.submenu("Wi-Fi 网络", wifi_menu);
    let websites_menu = Menu::new();
    if view.websites.is_empty() {
        websites_menu.item(0, "暂无收藏，可在下方新增网址", false, false);
    }
    let edit_menu = Menu::new();
    let delete_menu = Menu::new();
    for (index, website) in view
        .websites
        .iter()
        .take(websites::MAX_WEBSITES)
        .enumerate()
    {
        websites_menu.item(WEBSITE_BASE + index as u32, &website.name, true, false);
        edit_menu.item(WEBSITE_EDIT_BASE + index as u32, &website.name, true, false);
        delete_menu.item(
            WEBSITE_DELETE_BASE + index as u32,
            &website.name,
            true,
            false,
        );
    }
    websites_menu.line();
    websites_menu.item(
        WEBSITE_ADD,
        "新增网址…",
        view.websites.len() < websites::MAX_WEBSITES,
        false,
    );
    if !view.websites.is_empty() {
        websites_menu.submenu("编辑网址", edit_menu);
        websites_menu.submenu("删除网址", delete_menu);
    }
    menu.submenu("SHU 大常用网址", websites_menu);
    menu.line();
    let campus = Menu::new();
    campus.item(PORTAL, "打开校园网认证网页", true, false);
    campus.item(REFRESH, "立即查询一次", !view.busy, false);
    campus.item(IP_INFORMATION, "IP 信息与出口查询…", true, false);
    campus.item(LAN_DEVICES, "局域网与共享设备…", true, false);
    campus.item(POWER_TOOLS, "定时电源（关机／重启／休眠／唤醒）…", true, false);
    campus.line();
    let speed = speed_snapshot();
    campus.item(
        SPEED_PROXY,
        "一键测速（通过代理）",
        !view.busy && !speed.running,
        false,
    );
    campus.item(
        SPEED_DIRECT,
        "一键测速（直连）",
        !view.busy && !speed.running,
        false,
    );
    if speed.running {
        campus.item(0, &speed.progress_label(), false, false);
        campus.item(SPEED_CANCEL, "取消测速", !speed.cancelling, false);
    }
    campus.item(
        SPEED_RESULTS,
        "查看测速进度与结果…",
        speed.running || speed.results().is_some(),
        false,
    );
    campus.item(SPEED_COPY, "复制测速结果", speed.results().is_some(), false);
    menu.submenu("校园网工具", campus);
    let settings = Menu::new();
    settings.item(CONFIG, "账号与密码…", !view.busy, false);
    let operators = Menu::new();
    let mut selected_operator = OPERATOR_CAMPUS;
    for (index, operator) in operator::Operator::ALL.into_iter().enumerate() {
        let id = OPERATOR_CAMPUS + index as u32;
        operators.item(id, operator.label(), !view.busy, false);
        if view.operator == operator {
            selected_operator = id;
        }
    }
    unsafe {
        CheckMenuRadioItem(operators.0, OPERATOR_CAMPUS, OPERATOR_MOBILE, selected_operator, MF_BYCOMMAND);
    }
    settings.submenu(&format!("运营商（{}）", view.operator.label()), operators);
    settings.submenu(&format!("菜单主题（{}）", view.menu_theme.label()), theme_menu(view.menu_theme));
    settings.item(HOTKEY_SETTINGS, "快捷键…", true, false);
    let detection = Menu::new();
    detection.item(
        STATUS_DETECTION,
        "启用状态检测",
        true,
        view.status_detection,
    );
    detection.line();
    let mut selected = DETECTION_MEDIUM;
    for (id, level) in [
        (DETECTION_ADAPTIVE, detection::DetectionLevel::Adaptive),
        (DETECTION_EXTREME, detection::DetectionLevel::Extreme),
        (DETECTION_HIGH, detection::DetectionLevel::High),
        (DETECTION_MEDIUM, detection::DetectionLevel::Medium),
        (DETECTION_LOW, detection::DetectionLevel::Low),
    ] {
        detection.item(id, level.label(), true, false);
        if level == view.detection_level {
            selected = id;
        }
    }
    unsafe {
        CheckMenuRadioItem(
            detection.0,
            DETECTION_ADAPTIVE,
            DETECTION_EXTREME,
            selected,
            MF_BYCOMMAND,
        );
    }
    detection.line();
    detection.item(REFRESH, "立即查询一次", !view.busy, false);
    settings.submenu(
        &format!(
            "状态检测（{}）",
            if view.status_detection {
                view.detection_level.label()
            } else {
                "已关闭"
            }
        ),
        detection,
    );
    let kind = Menu::new();
    for (id, text, mode) in [
        (NETWORK_AUTO, "自动识别", network_policy::NetworkMode::Auto),
        (
            NETWORK_CAMPUS,
            "校园网络",
            network_policy::NetworkMode::Campus,
        ),
        (
            NETWORK_ORDINARY,
            "普通网络",
            network_policy::NetworkMode::Ordinary,
        ),
    ] {
        kind.item(
            id,
            text,
            !view.busy && !view.ssid.is_empty(),
            view.network_mode == mode,
        );
    }
    settings.submenu("当前 Wi-Fi 类型（自动保存）", kind);
    if view.auth_paused {
        settings.item(
            RESUME_CAMPUS,
            "恢复当前网络的自动策略",
            view.campus && !view.busy,
            false,
        );
    }
    let automatic = Menu::new();
    automatic.item(
        AUTO_AUTH,
        if view.auto_auth && (!view.status_detection || !view.campus || view.auth_paused) {
            "连接后自动认证（当前暂停）"
        } else {
            "连接校园 Wi-Fi 后自动认证"
        },
        true,
        view.auto_auth,
    );
    let startup = startup::status();
    let startup_label = match &startup {
        Ok(status) if status.registered && !status.current_copy => "开机启动（更新到当前副本）",
        Ok(_) => "开机启动",
        Err(_) => "开机启动（状态读取失败）",
    };
    automatic.item(
        STARTUP,
        startup_label,
        true,
        startup.as_ref().is_ok_and(|status| status.current_copy),
    );
    settings.submenu("自动与启动", automatic);
    let files = Menu::new();
    files.item(CONFIG_LOCATION, "默认配置位置…", true, false);
    files.item(CONFIG_FOLDER, "打开配置文件夹", true, false);
    files.item(CONFIG_PATH_COPY, "复制完整配置路径", true, false);
    files.line();
    files.item(IMPORT_FILE, "从文件导入配置…", !view.busy, false);
    files.item(EXPORT_FILE, "导出配置到文件…", true, false);
    files.line();
    files.item(IMPORT, "从桌面版导入账号配置", !view.busy, false);
    settings.submenu("配置文件", files);
    let windows = Menu::new();
    windows.item(WIFI_SETTINGS, "Wi-Fi", true, false);
    windows.item(PROXY_SETTINGS, "代理", true, false);
    windows.item(STARTUP_SETTINGS, "启动应用", true, false);
    settings.submenu("Windows 设置", windows);
    settings.line();
    settings.item(HELP, "帮助（使用说明）…", true, false);
    menu.submenu("运行状态与记录", menu_summary::status_menu(&view));
    menu.submenu("设置", settings);
    menu.line();
    menu.item(EXIT, "退出", true, false);
    let live_menu = menu_summary::watch(&menu, &view);
    let mut point = POINT::default();
    let id = unsafe {
        GetCursorPos(&mut point);
        SetForegroundWindow(window);
        // No RefCell borrow or mutex is held while the native menu pumps messages.
        // Keep owner notifications for WM_INITMENUPOPUP. TPM_RETURNCMD lets us
        // dispatch the selected object after the menu closes; NONOTIFY would
        // prevent the Wi-Fi submenu from receiving its initialization message.
        let id = TrackPopupMenu(
            menu.0,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            window,
            std::ptr::null(),
        ) as u32;
        PostMessageW(window, WM_NULL, 0, 0);
        id
    };
    drop(live_menu);
    view = snapshot();
    view.networks = WIFI_MENU_NETWORKS.with(|networks| std::mem::take(&mut *networks.borrow_mut()));
    drop(_wifi_session);
    execute_action(window, id, &view);
}

fn complete_account_settings(window:HWND) {
    if MENU_ACTIVE.with(Cell::get) {return;}
    let reply=APP.with(|slot| {
        let mut slot=slot.try_borrow_mut().ok()?;let app=slot.as_mut()?;
        let receiver=app.pending_credentials.as_ref()?;
        match receiver.try_recv() {
            Ok(draft)=>{app.pending_credentials=None;Some(Ok(draft))},
            Err(std::sync::mpsc::TryRecvError::Empty)=>None,
            Err(_)=>{app.pending_credentials=None;Some(Err("后台账号服务已结束，请重新打开设置。".to_owned()))}
        }
    });
    let Some(reply)=reply else {return};
    let Some(_session)=MenuSession::begin() else {return};
    let draft=match reply {Ok(draft)=>draft,Err(error)=>{alert(window,&error);return;}};
    match platform::prompt_credentials(window,&draft.username,&draft.password,draft.remember_password) {
        Ok(Some((username,password,remember)))=>{
            if username!=draft.username || password!=draft.password || remember!=draft.remember_password {
                send(Command::Credentials(username,password,remember));
            }
        }
        Ok(None)=>{},Err(error)=>alert(window,&error),
    }
}

fn execute_action(window: HWND, id: u32, view: &View) {
    unsafe {
        match id {
            POWER_TOOLS => { if let Err(error)=power_dialog::show(window) { alert(window,&error); } },
            STATUS_WINDOW => { if let Err(error) = status_dialog::show(window) { alert(window, &error); } },
            LOGIN => send(Command::Login),
            LOGOUT => send(Command::Logout),
            AUTO_RECONNECT => send(Command::ToggleReconnect),
            AUTO_AUTH => send(Command::ToggleAutoAuth),
            STATUS_DETECTION => send(Command::ToggleStatusDetection),
            DETECTION_ADAPTIVE | DETECTION_EXTREME | DETECTION_HIGH | DETECTION_MEDIUM
            | DETECTION_LOW => {
                send(Command::SetDetectionLevel(match id {
                    DETECTION_ADAPTIVE => detection::DetectionLevel::Adaptive,
                    DETECTION_EXTREME => detection::DetectionLevel::Extreme,
                    DETECTION_HIGH => detection::DetectionLevel::High,
                    DETECTION_LOW => detection::DetectionLevel::Low,
                    _ => detection::DetectionLevel::Medium,
                }));
            }
            THEME_DEFAULT | THEME_CLASSIC | THEME_MODERN => {
                send(Command::SetMenuTheme(menu_theme::MenuTheme::ALL[(id-THEME_DEFAULT) as usize]));
            }
            PROXY => send(Command::ToggleProxy),
            SCAN => send(Command::Scan),
            DISCONNECT => send(Command::Disconnect),
            OPERATOR_CAMPUS | OPERATOR_TELECOM | OPERATOR_UNICOM | OPERATOR_MOBILE => {
                let operator = operator::Operator::ALL[(id - OPERATOR_CAMPUS) as usize];
                if operator != view.operator {
                    send(Command::SetOperator(operator));
                }
            }
            REFRESH => {
                // Treat a double click / rapid key press as one manual query.
                let now = std::time::Instant::now();
                let accepted = LAST_MANUAL_QUERY.with(|last| {
                    let mut previous = last.get();
                    let accepted = accept_manual_query(&mut previous, now);
                    last.set(previous);
                    accepted
                });
                if accepted {
                    send(Command::Refresh);
                }
            }
            NETWORK_AUTO | NETWORK_CAMPUS | NETWORK_ORDINARY => {
                send(Command::SetNetworkMode(match id {
                    NETWORK_CAMPUS => network_policy::NetworkMode::Campus,
                    NETWORK_ORDINARY => network_policy::NetworkMode::Ordinary,
                    _ => network_policy::NetworkMode::Auto,
                }))
            }
            RESUME_CAMPUS => send(Command::ResumeCampus),
            IP_INFORMATION | LAN_DEVICES => {
                let kind = if id == IP_INFORMATION { network_tools_dialog::Kind::Ip } else { network_tools_dialog::Kind::Devices };
                if let Err(error) = network_tools_dialog::show(window, kind, &view.proxy_server) { alert(window, &error); }
            }
            SPEED_DIRECT | SPEED_PROXY => {
                let result = APP.with(|slot| {
                    let mut slot = slot
                        .try_borrow_mut()
                        .map_err(|_| "当前操作繁忙。".to_owned())?;
                    let app = slot.as_mut().ok_or("托盘程序尚未就绪。")?;
                    if view.busy {
                        return Err("请等待校园网操作完成后再测速。".into());
                    }
                    let mode = if id == SPEED_PROXY {
                        speedtest::Mode::Proxy
                    } else {
                        speedtest::Mode::Direct
                    };
                    app.speedtest.start(mode, &view.proxy_server)
                });
                match result {
                    Err(error) => alert(window, &error),
                    Ok(()) => {
                        if let Err(error) = speed_dialog::show(window) {
                            alert(window, &error);
                        }
                    }
                }
            }
            SPEED_CANCEL => APP.with(|slot| {
                if let Ok(slot) = slot.try_borrow() {
                    if let Some(app) = slot.as_ref() {
                        app.speedtest.cancel();
                    }
                }
            }),
            SPEED_RESULTS | SPEED_COPY => {
                if id == SPEED_RESULTS {
                    if let Err(error) = speed_dialog::show(window) {
                        alert(window, &error);
                    }
                } else if let Some(text) = speed_snapshot().results() {
                    if let Err(error) = platform::copy_text(&text) {
                        alert(window, &error);
                    }
                }
            }
            IMPORT => send(Command::ImportDesktop),
            IMPORT_FILE | EXPORT_FILE => match dialogs::choose_config(window, id == EXPORT_FILE) {
                Ok(Some(path)) => send(if id == EXPORT_FILE {
                    Command::ExportFile(path)
                } else {
                    Command::ImportFile(path)
                }),
                Ok(None) => {}
                Err(error) => alert(window, &error),
            },
            CONFIG_LOCATION => alert(
                window,
                &format!(
                    "默认配置文件：\r\n{}\r\n\r\n设置和常用网址会自动保存到这里。\r\n可通过“打开配置文件夹”访问，通过“导入 / 导出”选择备份文件。",
                    settings::SettingsStore::new().path().display()
                ),
            ),
            CONFIG_FOLDER => {
                if let Err(error) = dialogs::open_config_folder() {
                    alert(window, &error);
                }
            }
            CONFIG_PATH_COPY => {
                let path = settings::SettingsStore::new().path().display().to_string();
                match platform::copy_text(&path) {
                    Ok(()) => alert(window, "默认配置文件的完整路径已复制。"),
                    Err(error) => alert(window, &error),
                }
            }
            HELP => {
                if let Err(error) = dialogs::show_help(window) {
                    alert(window, &error);
                }
            }
            HOTKEY_SETTINGS => configure_hotkeys(window),
            ICON_LEGEND => alert(window, connection_state::LEGEND),
            PORTAL => platform::open("http://10.10.9.9/"),
            WIFI_SETTINGS => platform::open("ms-settings:network-wifi"),
            WIFI_PRIVACY => platform::open("ms-settings:privacy-location"),
            PROXY_SETTINGS => platform::open("ms-settings:network-proxy"),
            STARTUP_SETTINGS => platform::open("ms-settings:startupapps"),
            STARTUP => {
                if let Err(error) = startup::toggle() {
                    alert(window, &error);
                }
            }
            WEBSITE_ADD => match website_dialog::prompt(window, None, &view.websites) {
                Ok(Some(website)) => send(Command::AddWebsite(website)),
                Ok(None) => {}
                Err(error) => alert(window, &error),
            },
            CONFIG => {
                let result=APP.with(|slot| {
                    let mut slot=slot.try_borrow_mut().map_err(|_|"界面正忙，请稍后重试。".to_owned())?;
                    let app=slot.as_mut().ok_or("账号服务尚未启动。")?;
                    if app.pending_credentials.is_none() {app.pending_credentials=Some(app.controller.read_credentials()?);}
                    Ok::<(),String>(())
                });
                if let Err(error)=result {alert(window,&error);}
            }
            COPY_IP => {
                if let Err(error) = platform::copy_text(&view.ip) {
                    alert(window, &error);
                }
            }
            COPY_LOGS => {
                if let Err(error) =
                    platform::copy_text(&view.history.iter().map(runtime_history::Entry::line).collect::<Vec<_>>().join("\r\n"))
                {
                    alert(window, &error);
                }
            }
            EXIT => {
                send(Command::Stop);
                DestroyWindow(window);
            }
            id if (WEBSITE_BASE..WEBSITE_BASE + websites::MAX_WEBSITES as u32).contains(&id) => {
                if let Some(website) = view.websites.get((id - WEBSITE_BASE) as usize) {
                    if let Err(error) = website.open() {
                        alert(window, &error);
                    }
                }
            }
            id if (WEBSITE_EDIT_BASE..WEBSITE_EDIT_BASE + websites::MAX_WEBSITES as u32)
                .contains(&id) =>
            {
                if let Some(previous) = view.websites.get((id - WEBSITE_EDIT_BASE) as usize) {
                    match website_dialog::prompt(window, Some(previous), &view.websites) {
                        Ok(Some(updated)) => {
                            send(Command::EditWebsite {
                                previous: previous.clone(),
                                updated,
                            });
                        }
                        Ok(None) => {}
                        Err(error) => alert(window, &error),
                    }
                }
            }
            id if (WEBSITE_DELETE_BASE..WEBSITE_DELETE_BASE + websites::MAX_WEBSITES as u32)
                .contains(&id) =>
            {
                if let Some(website) = view.websites.get((id - WEBSITE_DELETE_BASE) as usize) {
                    send(Command::RemoveWebsite(website.clone()));
                }
            }
            id if (NETWORK_BASE..NETWORK_BASE + 40).contains(&id) => {
                if let Some(network) = view.networks.get((id - NETWORK_BASE) as usize) {
                    send(Command::Connect(network.clone()));
                }
            }
            _ => {}
        }
    }
}

fn action_enabled(id: u32, view: &View) -> bool {
    if (OPERATOR_CAMPUS..=OPERATOR_MOBILE).contains(&id) && view.busy {
        return false;
    }
    if matches!(id, LOGIN | LOGOUT | RESUME_CAMPUS) && !view.campus {
        return false;
    }
    if matches!(id, SPEED_DIRECT | SPEED_PROXY) && (view.busy || speed_snapshot().running) {
        return false;
    }
    if id == SPEED_CANCEL && (!speed_snapshot().running || speed_snapshot().cancelling) {
        return false;
    }
    if (id == SPEED_COPY || (id == SPEED_RESULTS && !speed_snapshot().running))
        && speed_snapshot().results().is_none()
    {
        return false;
    }
    if matches!(id, LOGIN | LOGOUT | DISCONNECT) && view.ssid.is_empty() {
        return false;
    }
    if id == SCAN && view.scanning {
        return false;
    }
    if matches!(
        id,
        LOGIN | LOGOUT | DISCONNECT | PROXY | SCAN | CONFIG | IMPORT_FILE
    ) && view.busy
    {
        return false;
    }
    if id == COPY_IP && view.ip == "—" {
        return false;
    }
    if id == COPY_LOGS && view.history.is_empty() {
        return false;
    }
    true
}

fn speed_snapshot() -> speedtest::Snapshot {
    APP.with(|slot| {
        slot.try_borrow()
            .ok()
            .and_then(|slot| slot.as_ref().map(|app| app.speedtest.snapshot()))
    })
    .unwrap_or_default()
}

fn network_busy() -> bool {
    APP.with(|slot| {
        slot.try_borrow()
            .ok()
            .and_then(|slot| slot.as_ref()?.shared.lock().ok().map(|view| view.busy))
    })
    .unwrap_or(true)
}

fn configure_hotkeys(window: HWND) {
    // Move the manager out of APP while the native modal dialog pumps messages;
    // no RefCell borrow, settings mutex or registration can reenter the editor.
    let state = APP.with(|slot| {
        slot.try_borrow_mut().ok().and_then(|mut slot| {
            let app = slot.as_mut()?;
            Some((app.hotkeys.take()?, app.controller.clone()))
        })
    });
    let Some((mut manager, controller)) = state else {
        return;
    };
    let result = hotkey_dialog::prompt(window, &snapshot().hotkeys, controller, &mut manager);
    let warning = manager.sync(&snapshot().hotkeys);
    APP.with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut() {
            if let Some(app) = slot.as_mut() {
                app.hotkeys = Some(manager);
                if warning.is_some() {
                    app.hotkey_warning = warning;
                }
            }
        }
    });
    if let Err(error) = result {
        alert(window, &error);
    }
}

fn alert(window: HWND, message: &str) {
    unsafe {
        MessageBoxW(
            window,
            wide(message).as_ptr(),
            wide("SHUNet").as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

#[cfg(test)]
mod query_feedback_tests {
    use super::*;

    #[test]
    fn transient_shell_failure_does_not_latch_into_add_only_retries() {
        let mut calls = Vec::new();
        let mut modify_calls = 0;
        let result = notify_icon_with_recovery(NIM_MODIFY, |operation| {
            calls.push(operation);
            if operation == NIM_MODIFY { modify_calls += 1; modify_calls > 1 } else { false }
        });
        assert_eq!(result, Some(NIM_MODIFY));
        assert_eq!(calls, [NIM_MODIFY, NIM_ADD, NIM_MODIFY]);

        // Previous delivery failed, but the old icon still occupies this identity.
        let mut calls = Vec::new();
        assert_eq!(notify_icon_with_recovery(NIM_ADD, |operation| {
            calls.push(operation);
            operation == NIM_MODIFY
        }), Some(NIM_MODIFY));
        assert_eq!(calls, [NIM_ADD, NIM_MODIFY]);
    }

    #[test]
    fn missing_shell_icon_is_added_and_complete_failure_is_retried_later() {
        assert_eq!(notify_icon_with_recovery(NIM_MODIFY, |operation| operation == NIM_ADD), Some(NIM_ADD));
        let mut calls = Vec::new();
        assert_eq!(notify_icon_with_recovery(NIM_MODIFY, |operation| { calls.push(operation); false }), None);
        assert_eq!(calls.len(), 3, "shell recovery must be bounded");
        assert_eq!(notify_icon_with_recovery(NIM_ADD, |_| true), Some(NIM_ADD));
        let mut calls = 0;
        assert_eq!(notify_icon_with_recovery(NIM_MODIFY, |_| { calls += 1; true }), Some(NIM_MODIFY));
        assert_eq!(calls, 1, "normal changes should make only one shell call");
    }

    #[test]
    fn repeated_manual_click_after_feedback_expires_is_always_accepted() {
        let now = std::time::Instant::now();
        let mut last = None;
        assert!(accept_manual_query(&mut last, now));
        assert!(!accept_manual_query(&mut last, now + std::time::Duration::from_millis(20)));
        assert!(!accept_manual_query(&mut last, now + std::time::Duration::from_millis(200)));
        assert!(accept_manual_query(&mut last, now + MANUAL_QUERY_DEBOUNCE));
        assert!(accept_manual_query(&mut last, now + MANUAL_QUERY_DEBOUNCE + QUERY_FEEDBACK_DURATION));
    }

    #[test]
    #[ignore = "Native isolated tray feedback test; no saved settings, network requests or credentials"]
    fn native_fast_query_feedback_and_timer_cleanup() {
        use std::time::{Duration, Instant};
        fn pump_for(duration: Duration) {
            let until = Instant::now() + duration;
            while Instant::now() < until {
                unsafe {
                    let mut message = MSG::default();
                    while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let class = wide("SHUNet.TrayFeedbackRegression");
            let definition = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: module, lpszClassName: class.as_ptr(), ..Default::default() };
            assert_ne!(RegisterClassW(&definition), 0);
            let window = CreateWindowExW(WS_EX_TOOLWINDOW, class.as_ptr(), wide("SHUNet 图标回归测试").as_ptr(), WS_OVERLAPPED,
                0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), module, std::ptr::null());
            assert!(!window.is_null());
            let (controller, _receiver) = service::Controller::isolated_test_channel();
            let shared = Arc::new(Mutex::new(View { state: connection_state::ConnectionState::Connected, ..View::default() }));
            let app = App { window, controller, shared: shared.clone(), observer: None,
                icons: icons::IconSet::for_taskbar(window).unwrap(), taskbar_created: 0,
                last_tooltip: String::new(), last_color: usize::MAX, last_query: 0,
                query_feedback_until: None, query_feedback_timer: false, icon_added: false,
                last_notice: 0, hotkeys: None, hotkey_warning: None,
                speedtest: speedtest::Manager::new(window as usize, SPEED_EVENT), last_speed_notice: 0,
                pending_credentials: None };
            struct Cleanup(HWND);
            impl Drop for Cleanup {
                fn drop(&mut self) { APP.with(|slot| { slot.borrow_mut().take(); }); unsafe { DestroyWindow(self.0); } }
            }
            let _cleanup = Cleanup(window);
            APP.with(|slot| *slot.borrow_mut() = Some(app));
            // Complete before the first UI event: the serial must still show feedback.
            shared.lock().unwrap().query = service::QueryActivity { serial: 1, manual: true, active: false, ..Default::default() };
            APP.with(|slot| {
                let mut slot = slot.borrow_mut(); let app = slot.as_mut().unwrap();
                assert!(app.notify(NIM_ADD));
                assert_eq!(app.last_color, icons::State::Connected as usize + 5);
                assert!(app.query_feedback_timer);
                let until = app.query_feedback_until;
                for _ in 0..30 { assert!(app.notify(NIM_MODIFY)); }
                assert_eq!(app.query_feedback_until, until, "ordinary events must not prolong feedback");
                // Reentrant timer dispatch must not disarm cleanup while APP is borrowed.
                handle_message(window, WM_TIMER, QUERY_FEEDBACK_TIMER, 0);
            });
            pump_for(Duration::from_millis(900));
            APP.with(|slot| {
                let slot = slot.borrow(); let app = slot.as_ref().unwrap();
                assert_eq!(app.last_color, icons::State::Connected as usize);
                assert!(!app.query_feedback_timer && app.query_feedback_until.is_none());
            });
            // A genuinely running query keeps its badge beyond the feedback minimum.
            shared.lock().unwrap().query = service::QueryActivity { serial: 2, manual: true, active: true, ..Default::default() };
            handle_message(window, SERVICE_EVENT, 0, 0);
            pump_for(Duration::from_millis(900));
            APP.with(|slot| assert!(slot.borrow().as_ref().unwrap().last_color >= 5));
            shared.lock().unwrap().query.active = false;
            handle_message(window, SERVICE_EVENT, 0, 0);
            APP.with(|slot| assert_eq!(slot.borrow().as_ref().unwrap().last_color, icons::State::Connected as usize));
            println!("fast completion / repeated updates / reentrant timer / active query / completion cleanup: passed");
        }
    }

    #[test]
    fn queued_request_is_visible_before_worker_publishes_a_query() {
        let idle = service::QueryActivity::default();
        assert!(query_badge_visible(&idle, false, false, true, false));
        // A queued read must be acknowledged even while authentication runs.
        assert!(query_badge_visible(&idle, false, true, true, false));
        assert!(query_badge_visible(&idle, false, false, false, true));
        assert!(!query_badge_visible(&idle, false, false, false, false));
    }

    #[test]
    fn manual_badge_survives_connection_change_background_stays_scoped() {
        let mut query = service::QueryActivity {
            active: true,
            manual: true,
            ..Default::default()
        };
        assert!(query_badge_visible(&query, false, false, false, false));
        query.manual = false;
        assert!(!query_badge_visible(&query, false, false, false, false));
        assert!(query_badge_visible(&query, true, false, false, false));
        assert!(!query_badge_visible(&query, true, true, false, false));
    }

    #[test]
    fn busy_worker_accepts_a_read_but_still_blocks_conflicting_actions() {
        let view = View { busy: true, ..Default::default() };
        assert!(action_enabled(REFRESH, &view));
        assert!(!action_enabled(SCAN, &view));
        assert!(!action_enabled(CONFIG, &view));
    }
}
