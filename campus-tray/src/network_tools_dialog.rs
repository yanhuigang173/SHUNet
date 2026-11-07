//! Two modeless native windows with bounded, on-demand background readers.
use crate::network_tools::{self, Adapter, Device, PublicIp};
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::*, HiDpi::GetDpiForWindow, Input::KeyboardAndMouse::EnableWindow,
        WindowsAndMessaging::*,
    },
};

const UPDATED: u32 = WM_APP + 27;
const TITLES: [&str; 8] = [
    "内网 IPv4",
    "本机 IPv6",
    "公网 IP（直连）",
    "直连国家／地区",
    "公网 IP（代理）",
    "代理国家／地区",
    "网关",
    "DNS",
];
static IP_RUNNING: AtomicBool = AtomicBool::new(false);
static DEVICES_RUNNING: AtomicBool = AtomicBool::new(false);
thread_local! {
    static IP_WINDOW: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
    static DEVICES_WINDOW: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ip,
    Devices,
}
impl Kind {
    fn running(self) -> &'static AtomicBool {
        if self == Self::Ip {
            &IP_RUNNING
        } else {
            &DEVICES_RUNNING
        }
    }
    fn window(self) -> HWND {
        match self {
            Self::Ip => IP_WINDOW.with(Cell::get),
            Self::Devices => DEVICES_WINDOW.with(Cell::get),
        }
    }
    fn set_window(self, value: HWND) {
        match self {
            Self::Ip => IP_WINDOW.with(|w| w.set(value)),
            Self::Devices => DEVICES_WINDOW.with(|w| w.set(value)),
        }
    }
}
#[derive(Default, Clone)]
struct Report {
    adapters: Vec<Adapter>,
    devices: Vec<Device>,
    notes: Vec<String>,
    public: [Option<Result<PublicIp, String>>; 2],
    error: Option<String>,
    pending: bool,
    serial: u64,
    updated: String,
}
struct State {
    kind: Kind,
    proxy: String,
    report: Arc<Mutex<Report>>,
    cancelled: Arc<AtomicBool>,
    selected: Option<u32>,
    all_interfaces: bool,
    shared_only: bool,
    serial: u64,
    visible: Vec<Device>,
    values: [String; 8],
}
struct Init(Option<Box<State>>);

pub fn translate(message: &MSG) -> bool {
    [Kind::Ip.window(), Kind::Devices.window()]
        .into_iter()
        .any(|window| !window.is_null() && unsafe { IsDialogMessageW(window, message) != 0 })
}
pub fn show(owner: HWND, kind: Kind, proxy: &str) -> Result<(), String> {
    let existing = kind.window();
    if !existing.is_null() {
        unsafe {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
        }
        return Ok(());
    }
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        });
    }
    let mut init = Init(Some(Box::new(State {
        kind,
        proxy: proxy.into(),
        report: Arc::new(Mutex::new(Report::default())),
        cancelled: Arc::new(AtomicBool::new(false)),
        selected: None,
        all_interfaces: false,
        shared_only: kind == Kind::Devices,
        serial: u64::MAX,
        visible: Vec::new(),
        values: Default::default(),
    })));
    let window = unsafe {
        CreateDialogParamW(
            GetModuleHandleW(std::ptr::null()),
            if kind == Kind::Ip {
                105usize as _
            } else {
                106usize as _
            },
            owner,
            Some(dialog_proc),
            &mut init as *mut Init as isize,
        )
    };
    if window.is_null() {
        return Err("无法打开网络工具窗口。".into());
    }
    unsafe {
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
    }
    Ok(())
}
fn set(window: HWND, id: i32, text: &str) {
    unsafe {
        SetDlgItemTextW(window, id, crate::wide(text).as_ptr());
    }
}
fn enabled(window: HWND, id: i32, enable: bool) {
    unsafe {
        EnableWindow(GetDlgItem(window, id), i32::from(enable));
    }
}
fn notify(window: usize) {
    unsafe {
        PostMessageW(window as HWND, UPDATED, 0, 0);
    }
}
fn edit_report(
    report: &Arc<Mutex<Report>>,
    window: usize,
    cancelled: &AtomicBool,
    change: impl FnOnce(&mut Report),
) {
    if cancelled.load(Ordering::Acquire) {
        return;
    }
    if let Ok(mut data) = report.lock() {
        change(&mut data);
        data.serial = data.serial.wrapping_add(1);
    }
    notify(window);
}
fn fingerprint(adapters: &[Adapter]) -> Vec<(u32, String)> {
    let mut keys: Vec<_> = adapters
        .iter()
        .map(|a| {
            (
                a.index,
                format!("{:?}|{:?}|{:?}", a.addresses, a.gateways, a.dns),
            )
        })
        .collect();
    keys.sort();
    keys
}
fn refresh(window: HWND, state: &mut State) {
    if state
        .kind
        .running()
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        set(window, 1703, "上次查询正在结束，稍后可刷新。");
        return;
    }
    state.cancelled.store(true, Ordering::Release);
    state.cancelled = Arc::new(AtomicBool::new(false));
    if let Ok(mut report) = state.report.lock() {
        let serial = report.serial.wrapping_add(1);
        *report = Report {
            pending: true,
            serial,
            ..Report::default()
        };
    }
    state.serial = u64::MAX;
    render(window, state);
    let kind = state.kind;
    let cancelled = state.cancelled.clone();
    let report = state.report.clone();
    let proxy = state.proxy.clone();
    let target = window as usize;
    let spawned = std::thread::Builder::new()
        .name("shunet-network-tools".into())
        .spawn(move || {
            struct Finish(Kind);
            impl Drop for Finish {
                fn drop(&mut self) {
                    self.0.running().store(false, Ordering::Release);
                }
            }
            let _finish = Finish(kind);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if kind == Kind::Devices {
                    match network_tools::devices() {
                        Ok(data) => edit_report(&report, target, &cancelled, |r| {
                            r.adapters = data.adapters;
                            r.devices = data.devices;
                            r.notes = data.notes;
                        }),
                        Err(error) => {
                            edit_report(&report, target, &cancelled, |r| r.error = Some(error))
                        }
                    }
                } else {
                    let local = match network_tools::adapters() {
                        Ok(adapters) => adapters,
                        Err(error) => {
                            edit_report(&report, target, &cancelled, |r| r.error = Some(error));
                            return;
                        }
                    };
                    let before = fingerprint(&local);
                    let proxy_before = crate::platform::read_proxy().ok();
                    edit_report(&report, target, &cancelled, |r| r.adapters = local);
                    for (index, mode) in [
                        crate::speedtest::Mode::Direct,
                        crate::speedtest::Mode::Proxy,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if cancelled.load(Ordering::Acquire) {
                            return;
                        }
                        let result = network_tools::public_ip(mode, &proxy, &cancelled);
                        edit_report(&report, target, &cancelled, |r| {
                            r.public[index] = Some(result)
                        });
                    }
                    let changed = network_tools::adapters()
                        .is_ok_and(|after| fingerprint(&after) != before)
                        || crate::platform::read_proxy().ok() != proxy_before;
                    if changed {
                        edit_report(&report, target, &cancelled, |r| {
                            r.public = [None, None];
                            r.error = Some(
                                "查询期间接口或代理设置发生变化，请刷新后查看当前出口。".into(),
                            );
                        });
                    }
                }
            }));
            edit_report(&report, target, &cancelled, |r| {
                if outcome.is_err() {
                    r.error = Some("查询任务异常，可刷新重试。".into());
                }
                r.pending = false;
                r.updated = chrono::Local::now().format("%H:%M:%S").to_string();
            });
            // Clear the process-wide job flag before notifying the button state.
            drop(_finish);
            if !cancelled.load(Ordering::Acquire) {
                notify(target);
            }
        });
    if spawned.is_err() {
        kind.running().store(false, Ordering::Release);
        if let Ok(mut data) = state.report.lock() {
            data.pending = false;
            data.error = Some("无法创建查询任务，请稍后重试。".into());
            data.serial += 1;
        }
        render(window, state);
    }
}
fn fill_adapters(window: HWND, state: &mut State, report: &Report) {
    enabled(window, 1702, !report.adapters.is_empty());
    if report.adapters.is_empty() && report.pending {
        return;
    }
    let selected = state.selected;
    let preferred = if state.kind == Kind::Devices {
        report
            .adapters
            .iter()
            .find(|a| a.shared || a.hotspot)
            .or_else(|| {
                report
                    .adapters
                    .iter()
                    .find(|a| a.usb_network() && a.gateways.is_empty())
            })
    } else {
        None
    };
    let selected = selected
        .filter(|id| report.adapters.iter().any(|a| a.index == *id))
        .or_else(|| preferred.map(|a| a.index))
        .or_else(|| {
            if state.kind == Kind::Ip {
                report.adapters.first().map(|a| a.index)
            } else {
                None
            }
        });
    let selected = if state.kind == Kind::Devices && (state.all_interfaces || state.shared_only) {
        None
    } else {
        selected
    };
    state.selected = selected;
    unsafe {
        let combo = GetDlgItem(window, 1702);
        SendMessageW(combo, CB_RESETCONTENT, 0, 0);
        let mut active = usize::from(state.kind == Kind::Devices && state.all_interfaces);
        let offset = if state.kind == Kind::Devices {
            SendMessageW(
                combo,
                CB_ADDSTRING,
                0,
                crate::wide("热点 / USB 共享设备（推荐）").as_ptr() as isize,
            );
            SendMessageW(
                combo,
                CB_ADDSTRING,
                0,
                crate::wide("全部活动接口（含 USB / VPN）").as_ptr() as isize,
            );
            2
        } else {
            0
        };
        for (index, adapter) in report.adapters.iter().enumerate() {
            SendMessageW(
                combo,
                CB_ADDSTRING,
                0,
                crate::wide(&adapter.label()).as_ptr() as isize,
            );
            if Some(adapter.index) == selected {
                active = index + offset;
            }
        }
        SendMessageW(combo, CB_SETCURSEL, active, 0);
    }
}
fn visible_devices(report: &Report, selected: Option<u32>, shared_only: bool) -> Vec<Device> {
    report
        .devices
        .iter()
        .filter(|device| {
            if shared_only {
                return device.source == "Windows 热点客户端"
                    || report.adapters.iter().any(|adapter| {
                        Some(adapter.index) == device.adapter_index
                            && (adapter.shared
                                || adapter.hotspot
                                || (adapter.usb_network() && adapter.gateways.is_empty()))
                    });
            }
            selected.is_none()
                || device.adapter_index == selected
                || (device.adapter_index.is_none()
                    && device.source == "Windows 热点客户端"
                    && report
                        .adapters
                        .iter()
                        .any(|a| Some(a.index) == selected && (a.hotspot || a.shared)))
        })
        .cloned()
        .collect()
}
fn render(window: HWND, state: &mut State) {
    let report = match state.report.lock() {
        Ok(data) => data.clone(),
        Err(_) => return,
    };
    let running = state.kind.running().load(Ordering::Acquire);
    enabled(window, 1705, !running);
    if state.serial == report.serial {
        return;
    }
    let first = state.serial == u64::MAX;
    state.serial = report.serial;
    if first || !report.adapters.is_empty() {
        fill_adapters(window, state, &report);
    }
    if state.kind == Kind::Ip {
        state.values = Default::default();
        if let Some(adapter) = report
            .adapters
            .iter()
            .find(|a| Some(a.index) == state.selected)
        {
            let [v4, v6, gateway, dns] = adapter.values();
            state.values[0] = v4;
            state.values[1] = v6;
            state.values[6] = gateway;
            state.values[7] = dns;
        }
        for index in 0..2 {
            let row = 2 + index * 2;
            match &report.public[index] {
                Some(Ok(value)) => {
                    state.values[row] = value.ip.to_string();
                    state.values[row + 1] = value.country.clone();
                }
                Some(Err(error)) => {
                    set(window, 1710 + row as i32, error);
                    set(window, 1711 + row as i32, "未获取");
                }
                None => {
                    set(
                        window,
                        1710 + row as i32,
                        if report.pending {
                            "正在查询…"
                        } else {
                            "未获取"
                        },
                    );
                    set(window, 1711 + row as i32, "—");
                }
            }
        }
        for (index, value) in state.values.iter().enumerate() {
            if !value.is_empty() || [0, 1, 6, 7].contains(&index) {
                set(
                    window,
                    1710 + index as i32,
                    if value.is_empty() { "—" } else { value },
                );
            }
            enabled(window, 1750 + index as i32, !value.is_empty());
        }
        set(
            window,
            1704,
            "直连跳过 HTTP/SOCKS 代理，TUN 仍可能接管；代理软件也可能按规则直出。公网查询使用系统出口，与本地接口选择独立。国家/地区为 IP 归属地。查询服务：ipwho.is。",
        );
    } else {
        state.visible = visible_devices(&report, state.selected, state.shared_only);
        fill_list(window, state, &report);
        let notes = report.notes.join(" ");
        set(
            window,
            1704,
            &if notes.is_empty() {
                "读取本机热点客户端与接口邻居缓存；不扫描整个子网。缓存不是完整 DHCP 租约表。"
                    .into()
            } else {
                notes
            },
        );
    }
    enabled(window, 1706, !report.adapters.is_empty());
    let status = if let Some(error) = &report.error {
        error.clone()
    } else if report.pending {
        if state.kind == Kind::Ip {
            "正在查询直连与代理出口，本地信息可先查看和复制…".into()
        } else {
            "正在读取共享接口和设备记录…".into()
        }
    } else if state.kind == Kind::Devices {
        format!(
            "{} 更新 · 当前筛选 {} 条记录{}",
            report.updated,
            state.visible.len(),
            if state.visible.is_empty() {
                "；暂无设备记录，请连接开发板/热点设备，通信后刷新"
            } else {
                ""
            }
        )
    } else {
        format!(
            "{} 更新 · 直连与代理的结果分别显示，失败项可稍后刷新",
            report.updated
        )
    };
    set(window, 1703, &status);
}
fn fill_list(window: HWND, state: &State, report: &Report) {
    unsafe {
        let list = GetDlgItem(window, 1720);
        SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
        for (index, device) in state.visible.iter().enumerate() {
            let adapter = report
                .adapters
                .iter()
                .find(|a| Some(a.index) == device.adapter_index)
                .map_or("热点（接口待确认）", |a| a.name.as_str());
            let values = [
                device.ip_text().unwrap_or_else(|| "IP 暂未解析".into()),
                device.mac.clone(),
                device.status.clone(),
                adapter.into(),
                device.source.clone(),
                device.name.clone(),
            ];
            for (column, value) in values.iter().enumerate() {
                let mut text = crate::wide(value);
                let item = LVITEMW {
                    mask: LVIF_TEXT,
                    iItem: index as i32,
                    iSubItem: column as i32,
                    pszText: text.as_mut_ptr(),
                    ..Default::default()
                };
                SendMessageW(
                    list,
                    if column == 0 {
                        LVM_INSERTITEMW
                    } else {
                        LVM_SETITEMW
                    },
                    0,
                    &item as *const _ as isize,
                );
            }
        }
    }
}
fn copy_all(state: &State) -> String {
    let report = state
        .report
        .lock()
        .ok()
        .map(|r| r.clone())
        .unwrap_or_default();
    let mut text = format!(
        "SHUNet · {}\r\n查询时间：{}\r\n",
        if state.kind == Kind::Ip {
            "IP 信息"
        } else {
            "局域网与共享设备"
        },
        report.updated
    );
    if state.kind == Kind::Ip {
        for (title, value) in TITLES.into_iter().zip(&state.values) {
            text.push_str(&format!(
                "{title}：{}\r\n",
                if value.is_empty() { "未获取" } else { value }
            ));
        }
        if let Some(adapter) = report
            .adapters
            .iter()
            .find(|a| Some(a.index) == state.selected)
        {
            text.push_str(&format!(
                "本地接口：{} · {}\r\n",
                adapter.name, adapter.description
            ));
        }
        for index in 0..2 {
            if let Some(Err(error)) = &report.public[index] {
                text.push_str(&format!(
                    "{}查询：{error}\r\n",
                    if index == 0 { "直连" } else { "代理" }
                ));
            }
        }
        text.push_str("直连跳过 HTTP/SOCKS 代理，TUN/VPN 仍可能接管；公网查询与本地接口选择独立。归属地来源：ipwho.is。\r\n");
    } else {
        text.push_str("IP\tMAC\t状态\t接口\t来源\t名称\r\n");
        for device in &state.visible {
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\r\n",
                device.ip_text().unwrap_or_else(|| "未解析".into()),
                device.mac,
                device.status,
                report
                    .adapters
                    .iter()
                    .find(|a| Some(a.index) == device.adapter_index)
                    .map_or("待确认", |a| a.name.as_str()),
                device.source,
                device.name
            ));
        }
        text.push_str(&report.notes.join("\r\n"));
    }
    if let Some(error) = report.error {
        text.push_str(&format!("\r\n{error}"));
    }
    text
}
unsafe extern "system" fn dialog_proc(
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
            let init = &mut *(lparam as *mut Init);
            let Some(state) = init.0.take() else {
                return 0;
            };
            let kind = state.kind;
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                Box::into_raw(Box::new(RefCell::new(*state))) as isize,
            );
            kind.set_window(window);
            let title = if kind == Kind::Ip {
                "IP 信息与出口查询"
            } else {
                "局域网与共享设备"
            };
            SetWindowTextW(window, crate::wide(&format!("SHUNet · {title}")).as_ptr());
            set(window, 1600, title);
            set(
                window,
                1601,
                if kind == Kind::Ip {
                    "查看本地接口、直连和代理出口；支持逐项复制"
                } else {
                    "查看本机热点、共享下游及其他接口的设备记录"
                },
            );
            set(
                window,
                1701,
                if kind == Kind::Ip {
                    "本地接口"
                } else {
                    "查看接口"
                },
            );
            for (id, label) in [
                (1705, "刷新查询"),
                (1706, "复制全部"),
                (1707, "复制所选 IP"),
                (IDCANCEL, "关闭"),
            ] {
                set(window, id, label);
            }
            if kind == Kind::Ip {
                for (index, title) in TITLES.into_iter().enumerate() {
                    set(window, 1730 + index as i32, title);
                    set(window, 1750 + index as i32, "复制");
                }
            }
            crate::ui::init(
                window,
                if kind == Kind::Ip {
                    crate::ui::Kind::IpInfo
                } else {
                    crate::ui::Kind::Devices
                },
            );
            if kind == Kind::Devices {
                let list = GetDlgItem(window, 1720);
                SendMessageW(
                    list,
                    LVM_SETEXTENDEDLISTVIEWSTYLE,
                    0,
                    (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP) as isize,
                );
                let dpi = GetDpiForWindow(window).max(96);
                for (index, (name, width)) in [
                    ("IP 地址", 150),
                    ("MAC 地址", 145),
                    ("状态", 130),
                    ("接口", 100),
                    ("来源", 150),
                    ("名称", 140),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut text = crate::wide(name);
                    let column = LVCOLUMNW {
                        mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM,
                        cx: width * dpi as i32 / 96,
                        pszText: text.as_mut_ptr(),
                        iSubItem: index as i32,
                        ..Default::default()
                    };
                    SendMessageW(list, LVM_INSERTCOLUMNW, index, &column as *const _ as isize);
                }
            }
            SetTimer(window, 1, 300, None);
            let state = &*(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut RefCell<State>);
            if let Ok(mut state) = state.try_borrow_mut() {
                refresh(window, &mut state);
            }
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
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            let state = Box::from_raw(pointer);
            let state = state.into_inner();
            state.cancelled.store(true, Ordering::Release);
            state.kind.set_window(std::ptr::null_mut());
            KillTimer(window, 1);
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
        let Ok(mut state) = (&*pointer).try_borrow_mut() else {
            return 0;
        };
        if message == WM_TIMER || message == UPDATED {
            render(window, &mut state);
            return 1;
        }
        if message == WM_COMMAND {
            let id = (wparam & 0xffff) as i32;
            if id == 1705 {
                refresh(window, &mut state);
                return 1;
            }
            if id == 1702 && (wparam >> 16) == CBN_SELCHANGE as usize {
                let index = SendDlgItemMessageW(window, 1702, CB_GETCURSEL, 0, 0);
                let report = state
                    .report
                    .lock()
                    .ok()
                    .map(|r| r.clone())
                    .unwrap_or_default();
                state.shared_only = state.kind == Kind::Devices && index == 0;
                state.all_interfaces = state.kind == Kind::Devices && index == 1;
                state.selected = if state.kind == Kind::Devices && index < 2 {
                    None
                } else {
                    report
                        .adapters
                        .get(
                            (index - if state.kind == Kind::Devices { 2 } else { 0 }).max(0)
                                as usize,
                        )
                        .map(|a| a.index)
                };
                state.serial = report.serial.wrapping_sub(1);
                render(window, &mut state);
                return 1;
            }
            let text = if id == 1706 {
                Some(copy_all(&state))
            } else if (1750..1758).contains(&id) {
                state
                    .values
                    .get((id - 1750) as usize)
                    .filter(|s| !s.is_empty())
                    .cloned()
            } else if id == 1707 {
                let selected = SendDlgItemMessageW(
                    window,
                    1720,
                    LVM_GETNEXTITEM,
                    usize::MAX,
                    LVNI_SELECTED as isize,
                );
                if selected < 0 {
                    set(window, 1703, "请先选择一条设备记录。");
                    None
                } else {
                    state
                        .visible
                        .get(selected as usize)
                        .and_then(Device::ip_text)
                }
            } else {
                None
            };
            if let Some(text) = text {
                match crate::platform::copy_text(&text) {
                    Ok(()) => set(window, 1703, "已复制到剪贴板。"),
                    Err(error) => set(window, 1703, &error),
                }
                return 1;
            }
        }
    }
    0
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        net::Ipv4Addr,
        time::{Duration, Instant},
    };

    fn state(kind: Kind) -> State {
        State {
            kind,
            proxy: String::new(),
            report: Arc::new(Mutex::new(Report::default())),
            cancelled: Arc::new(AtomicBool::new(false)),
            selected: None,
            all_interfaces: false,
            shared_only: kind == Kind::Devices,
            serial: 0,
            visible: Vec::new(),
            values: Default::default(),
        }
    }
    fn adapter(index: u32, shared: bool) -> Adapter {
        Adapter {
            index,
            guid: format!("adapter-{index}"),
            name: format!("接口 {index}"),
            description: "test".into(),
            addresses: vec![(Ipv4Addr::new(192, 168, 137, 1).into(), 24)],
            gateways: Vec::new(),
            dns: Vec::new(),
            metric: 1,
            shared,
            hotspot: shared,
        }
    }
    #[test]
    fn device_filter_keeps_hotspot_clients_with_unresolved_interface_out_of_other_lans() {
        let report = Report {
            adapters: vec![adapter(42, true), adapter(15, false)],
            devices: vec![Device {
                ip: None,
                mac: "AA:BB:CC:DD:EE:FF".into(),
                adapter_index: None,
                name: "phone".into(),
                status: "热点已连接".into(),
                source: "Windows 热点客户端".into(),
            }],
            ..Report::default()
        };
        assert_eq!(visible_devices(&report, Some(42), false).len(), 1);
        assert!(visible_devices(&report, Some(15), false).is_empty());
        assert_eq!(visible_devices(&report, None, false).len(), 1);
        assert_eq!(visible_devices(&report, None, true).len(), 1);
    }
    #[test]
    fn copy_all_preserves_partial_success_and_proxy_failure_without_fallback() {
        let mut state = state(Kind::Ip);
        state.values[0] = "192.168.137.1".into();
        state.values[2] = "8.8.8.8".into();
        state.values[3] = "美国 (US)".into();
        state.report.lock().unwrap().public[1] = Some(Err("代理未启动".into()));
        let copied = copy_all(&state);
        assert!(copied.contains("公网 IP（直连）：8.8.8.8"));
        assert!(copied.contains("公网 IP（代理）：未获取"));
        assert!(copied.contains("代理查询：代理未启动"));
    }
    #[test]
    fn shared_view_includes_usb_board_neighbors_without_showing_campus_neighbors() {
        let mut usb = adapter(42, false);
        usb.description = "USB Ethernet/RNDIS Gadget".into();
        let board = Device {
            ip: Some("192.168.137.2".parse().unwrap()),
            mac: "02:11:22:33:44:55".into(),
            adapter_index: Some(42),
            name: String::new(),
            status: "近期可达".into(),
            source: "接口邻居缓存".into(),
        };
        let mut campus = board.clone();
        campus.adapter_index = Some(15);
        campus.ip = Some("10.1.2.3".parse().unwrap());
        let report = Report {
            adapters: vec![usb, adapter(15, false)],
            devices: vec![board, campus],
            ..Report::default()
        };
        let filtered = visible_devices(&report, None, true);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].ip_text().as_deref(), Some("192.168.137.2"));
        assert_eq!(visible_devices(&report, None, false).len(), 2);
    }
    #[test]
    fn cancelled_report_is_not_published_to_a_reused_window() {
        let report = Arc::new(Mutex::new(Report::default()));
        edit_report(&report, 0, &AtomicBool::new(true), |r| {
            r.error = Some("should not publish".into())
        });
        let report = report.lock().unwrap();
        assert_eq!(report.serial, 0);
        assert!(report.error.is_none());
    }

    fn pump_until(condition: impl Fn() -> bool, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !condition() {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if !translate(&message) {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "native tool window did not complete within the test budget"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    pub(crate) fn capture(window: HWND, name: &str) {
        use windows_sys::Win32::Graphics::Gdi::*;
        #[link(name = "user32")]
        unsafe extern "system" {
            fn PrintWindow(window: HWND, dc: HDC, flags: u32) -> i32;
        }
        unsafe {
            let mut rect = RECT::default();
            GetWindowRect(window, &mut rect);
            let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
            assert!(width > 0 && height > 0 && width < 5000 && height < 5000);
            let dc = GetDC(window);
            let memory = CreateCompatibleDC(dc);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            assert!(!bitmap.is_null() && !bits.is_null());
            let previous = SelectObject(memory, bitmap as _);
            let rendered = PrintWindow(window, memory, 2);
            let size = (width * height * 4) as usize;
            let mut file = Vec::new();
            file.extend_from_slice(b"BM");
            file.extend_from_slice(&(54u32 + size as u32).to_le_bytes());
            file.extend_from_slice(&[0u8; 4]);
            file.extend_from_slice(&54u32.to_le_bytes());
            file.extend_from_slice(std::slice::from_raw_parts(
                (&info.bmiHeader as *const BITMAPINFOHEADER).cast::<u8>(),
                40,
            ));
            file.extend_from_slice(std::slice::from_raw_parts(bits.cast::<u8>(), size));
            SelectObject(memory, previous);
            DeleteObject(bitmap as _);
            DeleteDC(memory);
            ReleaseDC(window, dc);
            assert_ne!(rendered, 0, "own dialog screenshot failed");
            std::fs::write(format!("benchmarks/{name}.bmp"), file).unwrap();
        }
    }
    #[test]
    #[ignore = "Opens the new native windows and performs two authorized public-IP queries; no login or LAN scanning"]
    fn native_tools_window_smoke() {
        let (_, settings_warning) = crate::settings::SettingsStore::new().load();
        assert!(settings_warning.is_none());
        let (settings, _) = crate::settings::SettingsStore::new().load();
        for (kind, name) in [
            (Kind::Ip, "ip-information-window"),
            (Kind::Devices, "lan-devices-window"),
        ] {
            show(std::ptr::null_mut(), kind, &settings.proxy_server).unwrap();
            pump_until(
                || !kind.running().load(Ordering::Acquire),
                Duration::from_secs(28),
            );
            let window = kind.window();
            assert!(!window.is_null());
            let pointer =
                unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *const RefCell<State> };
            let state = unsafe { &*pointer }.borrow();
            let report = state.report.lock().unwrap();
            assert!(
                !report.adapters.is_empty(),
                "active local adapters required"
            );
            println!(
                "window={name}, adapters={}, devices={}, direct_ok={}, proxy_ok={}, error={}",
                report.adapters.len(),
                report.devices.len(),
                report.public[0].as_ref().is_some_and(|r| r.is_ok()),
                report.public[1].as_ref().is_some_and(|r| r.is_ok()),
                report.error.as_deref().unwrap_or("none")
            );
            for error in report
                .public
                .iter()
                .flatten()
                .filter_map(|r| r.as_ref().err())
            {
                println!("public lookup: {error}");
            }
            drop(report);
            drop(state);
            unsafe {
                SetForegroundWindow(window);
                ShowWindow(window, SW_RESTORE);
            }
            let ready = Instant::now();
            pump_until(
                || ready.elapsed() > Duration::from_millis(180),
                Duration::from_secs(2),
            );
            capture(window, name);
            unsafe {
                SendMessageW(window, WM_CLOSE, 0, 0);
            }
            assert!(kind.window().is_null());
        }
        println!(
            "Both native windows opened, updated, captured and closed successfully; no authentication or DHCP changes"
        );
    }
}
