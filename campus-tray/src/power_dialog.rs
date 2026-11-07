//! Modeless configuration window and a separate, cancellable runner countdown.
use crate::power::{self, Action, Plan, Repeat, Store};
use chrono::{Datelike, Local, NaiveDate, Timelike};
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, Mutex},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::InvalidateRect,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
};

const UPDATED: u32 = WM_APP + 35;
thread_local! {static WINDOW:Cell<HWND>=const{Cell::new(std::ptr::null_mut())};}
#[derive(Default)]
struct Model {
    plans: Vec<Plan>,
    info: String,
    busy: bool,
    revision: u64,
    issues: Vec<power::PlanIssue>,
    closed: bool,
}
struct State {
    store: Store,
    model: Arc<Mutex<Model>>,
    revision: u64,
    plans: Vec<Plan>,
    issues: Vec<power::PlanIssue>,
    signature: String,
    worker: Option<std::sync::mpsc::SyncSender<Job>>,
}
struct Init(Option<State>);
pub fn translate(message: &MSG) -> bool {
    let window = WINDOW.with(Cell::get);
    !window.is_null() && unsafe { IsDialogMessageW(window, message) != 0 }
}
pub fn show(owner: HWND) -> Result<(), String> {
    show_store(owner, Store::default())
}
fn show_store(owner: HWND, store: Store) -> Result<(), String> {
    let existing = WINDOW.with(Cell::get);
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
            dwICC: ICC_DATE_CLASSES | ICC_LISTVIEW_CLASSES,
        });
    }
    let mut init = Init(Some(State {
        store,
        model: Arc::new(Mutex::new(Model::default())),
        revision: u64::MAX,
        plans: Vec::new(),
        issues: Vec::new(),
        signature: String::new(),
        worker: None,
    }));
    let window = unsafe {
        CreateDialogParamW(
            GetModuleHandleW(std::ptr::null()),
            108usize as _,
            owner,
            Some(proc),
            &mut init as *mut Init as isize,
        )
    };
    if window.is_null() {
        return Err("无法打开定时电源窗口。".into());
    }
    unsafe {
        ShowWindow(window, SW_SHOW);
        ShowWindow(window, SW_RESTORE);
        SetForegroundWindow(window);
    }
    Ok(())
}
fn set(window: HWND, id: i32, text: &str) {
    unsafe {
        let child = GetDlgItem(window, id);
        let mut buffer = vec![0u16; GetWindowTextLengthW(child).max(0) as usize + 1];
        let n = GetWindowTextW(child, buffer.as_mut_ptr(), buffer.len() as i32);
        if String::from_utf16_lossy(&buffer[..n.max(0) as usize]) != text {
            SetWindowTextW(child, crate::wide(text).as_ptr());
        }
    }
}
fn text(window: HWND, id: i32) -> String {
    unsafe {
        let child = GetDlgItem(window, id);
        let mut buffer = vec![0; GetWindowTextLengthW(child).max(0).min(1024) as usize + 1];
        let n = GetWindowTextW(child, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..n.max(0) as usize])
    }
}
fn selected(window: HWND, id: i32) -> usize {
    unsafe { SendDlgItemMessageW(window, id, CB_GETCURSEL, 0, 0).max(0) as usize }
}
fn combo(window: HWND, id: i32, values: &[&str]) {
    unsafe {
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
}
fn form(window: HWND) -> Result<Plan, String> {
    let action = Action::ALL
        .get(selected(window, 1901))
        .copied()
        .ok_or("请选择电源操作。")?;
    let relative = selected(window, 1902) == 0;
    let repeat = if relative {
        Repeat::Once
    } else {
        match selected(window, 1906) {
            1 => Repeat::Daily,
            2 => Repeat::Weekdays,
            _ => Repeat::Once,
        }
    };
    let start = if relative {
        power::relative_time(&text(window, 1903), Local::now().timestamp())?
    } else {
        let mut date = SYSTEMTIME::default();
        let mut time = SYSTEMTIME::default();
        unsafe {
            if SendDlgItemMessageW(
                window,
                1904,
                DTM_GETSYSTEMTIME,
                0,
                &mut date as *mut _ as isize,
            ) != GDT_VALID as isize
                || SendDlgItemMessageW(
                    window,
                    1905,
                    DTM_GETSYSTEMTIME,
                    0,
                    &mut time as *mut _ as isize,
                ) != GDT_VALID as isize
            {
                return Err("请选择有效日期和时间。".into());
            }
        }
        NaiveDate::from_ymd_opt(date.wYear as i32, date.wMonth as u32, date.wDay as u32)
            .and_then(|date| date.and_hms_opt(time.wHour as u32, time.wMinute as u32, 0))
            .ok_or("日期或时间无效。")?
    };
    let battery =
        unsafe { SendDlgItemMessageW(window, 1907, BM_GETCHECK, 0, 0) == BST_CHECKED as isize };
    Plan::new(action, start, repeat, battery)
}
fn preview(window: HWND) {
    // No random identifiers, file access or Windows task service calls on the UI timer.
    let relative = selected(window, 1902) == 0;
    let action = Action::ALL[selected(window, 1901).min(4)];
    let now = Local::now().timestamp();
    let info = if relative {
        match power::relative_time(&text(window, 1903), now) {
            Ok(start) => format!(
                "从点击创建起计时 · 预计{}：{}",
                action.label(),
                start.format("%m-%d %H:%M:%S")
            ),
            Err(e) => e,
        }
    } else {
        let mut date = SYSTEMTIME::default();
        let mut time = SYSTEMTIME::default();
        unsafe {
            SendDlgItemMessageW(
                window,
                1904,
                DTM_GETSYSTEMTIME,
                0,
                &mut date as *mut _ as isize,
            );
            SendDlgItemMessageW(
                window,
                1905,
                DTM_GETSYSTEMTIME,
                0,
                &mut time as *mut _ as isize,
            );
        }
        let start =
            NaiveDate::from_ymd_opt(date.wYear as i32, date.wMonth as u32, date.wDay as u32)
                .and_then(|date| date.and_hms_opt(time.wHour as u32, time.wMinute as u32, 0));
        start
            .map(|start| {
                let plan = Plan {
                    version: 1,
                    id: "0".repeat(32),
                    action,
                    start,
                    repeat: match selected(window, 1906) {
                        1 => Repeat::Daily,
                        2 => Repeat::Weekdays,
                        _ => Repeat::Once,
                    },
                    on_battery: false,
                    enabled: true,
                    last: None,
                };
                plan.next(now + 90)
                    .map(|at| format!("下次{}：{}", action.label(), power::format_time(at)))
                    .unwrap_or_else(|| "指定时间至少需晚于现在 90 秒。".into())
            })
            .unwrap_or_else(|| "请选择有效时间。".into())
    };
    set(window, 1908, &info);
    set(
        window,
        1909,
        if action == Action::Wake {
            "定时开机采用睡眠／休眠唤醒；需设备及唤醒计时器支持。完全关机后开机请查看“能力与说明”。"
        } else {
            "执行前 60 秒可取消；错过不补执行。关机／重启不强制关闭应用，请提前保存工作。"
        },
    );
}
fn mode(window: HWND) {
    let relative = selected(window, 1902) == 0;
    set(
        window,
        1933,
        if relative {
            "等待分钟"
        } else {
            "执行时间"
        },
    );
    unsafe {
        for id in [1904, 1905, 1906] {
            EnableWindow(GetDlgItem(window, id), i32::from(!relative));
        }
        EnableWindow(GetDlgItem(window, 1903), i32::from(relative));
        if relative {
            SendDlgItemMessageW(window, 1906, CB_SETCURSEL, 0, 0);
        }
    }
    preview(window);
}
fn row(window: HWND) -> Option<usize> {
    let index = unsafe {
        SendDlgItemMessageW(
            window,
            1920,
            LVM_GETNEXTITEM,
            usize::MAX,
            LVNI_SELECTED as isize,
        )
    };
    (index >= 0).then_some(index as usize)
}
fn details(window: HWND, state: &State) {
    let plan = row(window).and_then(|i| state.plans.get(i));
    let issue = row(window)
        .and_then(|i| i.checked_sub(state.plans.len()))
        .and_then(|i| state.issues.get(i));
    let detail=plan.map(|p| {
        let last=p.last.as_ref().map(|r|format!("上次结果（{}）：{}",power::format_time(r.occurrence),r.detail)).unwrap_or_else(||"尚无执行记录。".into());
        format!("{}\r\n{} · {}",p.summary(Local::now().timestamp()),if p.on_battery {"电池供电也执行"}else{"仅接通电源时执行"},last)
    }).or_else(||issue.map(|issue|format!("损坏的计划 {}\r\n{}\r\n执行器不会运行此文件。可取消对应的 Windows 任务，或移除文件。",issue.id,issue.error))).unwrap_or_else(||"选择一个计划可查看最近执行结果、取消或移除。关闭窗口后计划继续有效；需保持当前用户登录，锁屏可以。".into());
    set(window, 1914, &detail);
    let busy = state.model.lock().map_or(true, |m| m.busy);
    unsafe {
        for id in [1912, 1913, 1916] {
            EnableWindow(
                GetDlgItem(window, id),
                i32::from(!busy && (plan.is_some() || issue.is_some())),
            );
        }
    }
}
fn render(window: HWND, state: &mut State) {
    let Ok(model) = state.model.lock() else {
        return;
    };
    let signature = model
        .plans
        .iter()
        .map(|plan| plan.summary(Local::now().timestamp()))
        .collect::<Vec<_>>()
        .join("\n");
    if state.revision != model.revision || state.signature != signature {
        let selection = selected_id(window, state);
        state.plans = model.plans.clone();
        state.issues = model.issues.clone();
        state.signature = signature;
        state.revision = model.revision;
        unsafe {
            let list = GetDlgItem(window, 1920);
            SendMessageW(list, WM_SETREDRAW, 0, 0);
            SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
            let now = Local::now().timestamp();
            let rows = state
                .plans
                .iter()
                .map(|plan| {
                    let (when, status) = plan.display(now);
                    (
                        plan.id.as_str(),
                        [
                            plan.action.label().to_string(),
                            when.map(power::format_time).unwrap_or_else(|| "—".into()),
                            plan.repeat.label().to_string(),
                            status.into(),
                        ],
                    )
                })
                .chain(state.issues.iter().map(|issue| {
                    (
                        issue.id.as_str(),
                        [
                            "损坏的计划".into(),
                            "—".into(),
                            "—".into(),
                            "未执行 · 可移除".into(),
                        ],
                    )
                }));
            for (index, (id, values)) in rows.enumerate() {
                for (column, value) in values.iter().enumerate() {
                    let mut value = crate::wide(value);
                    let item = LVITEMW {
                        mask: LVIF_TEXT | if column == 0 { LVIF_STATE } else { 0 },
                        iItem: index as i32,
                        iSubItem: column as i32,
                        pszText: value.as_mut_ptr(),
                        stateMask: LVIS_SELECTED,
                        state: if selection.as_deref() == Some(id) {
                            LVIS_SELECTED
                        } else {
                            0
                        },
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
            SendMessageW(list, WM_SETREDRAW, 1, 0);
            InvalidateRect(list, std::ptr::null(), 0);
            for id in [IDOK, 1911] {
                EnableWindow(GetDlgItem(window, id), i32::from(!model.busy));
            }
        }
        set(window, 1915, &model.info);
    }
    drop(model);
    details(window, state);
}
fn selected_id(window: HWND, state: &State) -> Option<String> {
    let index = row(window)?;
    state.plans.get(index).map(|p| p.id.clone()).or_else(|| {
        index
            .checked_sub(state.plans.len())
            .and_then(|i| state.issues.get(i))
            .map(|issue| issue.id.clone())
    })
}
enum Job {
    Refresh,
    Create(Plan),
    Cancel(String),
    Remove(String),
    Inspect(String),
}
// One worker per open window. Idle checks read only metadata; JSON is parsed
// after a change or an explicit refresh. Dropping the sender stops the worker.
fn start_worker(
    window: HWND,
    store: Store,
    shared: Arc<Mutex<Model>>,
) -> Result<std::sync::mpsc::SyncSender<Job>, String> {
    use std::sync::mpsc::{self, RecvTimeoutError};
    let (sender, receiver) = mpsc::sync_channel(1);
    let address = window as usize;
    std::thread::Builder::new()
        .name("power-plans".into())
        .spawn(move || {
            let mut stamp = None;
            loop {
                let job = match receiver.recv_timeout(std::time::Duration::from_secs(1)) {
                    Ok(job) => Some(job),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                let quiet = job.is_none();
                let result = match job {
                    None => Ok(String::new()),
                    Some(Job::Refresh) => {
                        Ok("已读取本地计划；可选择计划后核对 Windows 任务。".into())
                    }
                    Some(Job::Create(plan)) => store
                        .create(plan)
                        .map(|p| format!("已保存{}计划；退出 SHUNet 后仍有效。", p.action.label())),
                    Some(Job::Cancel(id)) => store.cancel(&id),
                    Some(Job::Remove(id)) => {
                        store.remove(&id).map(|_| "计划与执行记录已移除。".into())
                    }
                    Some(Job::Inspect(id)) => store.inspect(&id),
                };
                let current = store.stamp();
                if quiet
                    && current
                        .as_ref()
                        .ok()
                        .is_some_and(|value| stamp.as_ref() == Some(value))
                {
                    continue;
                }
                let list = store.list_with_issues();
                let Ok(mut model) = shared.lock() else { break };
                if model.closed {
                    break;
                }
                let previous = (
                    model.plans.clone(),
                    model.issues.clone(),
                    model.info.clone(),
                );
                if !quiet {
                    model.info = result.unwrap_or_else(|error| format!("操作未完成：{error}"));
                    model.busy = false;
                }
                match list {
                    Ok(list) => {
                        stamp = current.ok();
                        model.plans = list.plans;
                        model.issues = list.issues;
                        if !model.issues.is_empty() {
                            let notice = format!(
                                "有 {} 个损坏计划，正常计划仍可管理；请在列表中选择后移除。",
                                model.issues.len()
                            );
                            if !quiet {
                                model.info = format!("{} {notice}", model.info);
                            } else if previous.1 != model.issues {
                                model.info = notice;
                            }
                        } else if quiet && !previous.1.is_empty() {
                            model.info = "损坏计划已清理，列表已更新。".into();
                        }
                    }
                    Err(error) => {
                        stamp = None;
                        let notice = format!("读取计划失败：{error}；已保留上次列表。");
                        model.info = if quiet {
                            notice
                        } else {
                            format!("{} {notice}", model.info)
                        };
                    }
                }
                if !quiet
                    || previous
                        != (
                            model.plans.clone(),
                            model.issues.clone(),
                            model.info.clone(),
                        )
                {
                    model.revision = model.revision.wrapping_add(1);
                    unsafe {
                        PostMessageW(address as HWND, UPDATED, 0, 0);
                    }
                }
            }
        })
        .map_err(|error| format!("无法启动后台任务：{error}"))?;
    Ok(sender)
}
fn launch(window: HWND, state: &mut State, job: Job) {
    if state.worker.is_none() {
        match start_worker(window, state.store.clone(), state.model.clone()) {
            Ok(worker) => state.worker = Some(worker),
            Err(error) => {
                crate::alert(window, &error);
                return;
            }
        }
    }
    let Ok(mut model) = state.model.lock() else {
        return;
    };
    if model.busy {
        return;
    }
    if state.worker.as_ref().unwrap().try_send(job).is_err() {
        model.info = "后台队列不可用，请关闭后重新打开窗口。".into();
    } else {
        model.busy = true;
        model.info = "正在处理 Windows 电源计划…".into();
    }
    model.revision = model.revision.wrapping_add(1);
    drop(model);
    render(window, state);
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
            let init = &mut *(lparam as *mut Init);
            let Some(state) = init.0.take() else { return 0 };
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                Box::into_raw(Box::new(RefCell::new(state))) as isize,
            );
            WINDOW.with(|cell| cell.set(window));
            SetWindowTextW(window, crate::wide("SHUNet · 定时电源").as_ptr());
            crate::ui::init(window, crate::ui::Kind::Power);
            for (id, value) in [
                (1931, "执行操作"),
                (1932, "时间方式"),
                (1933, "分钟／时间"),
                (1934, "重复方式"),
                (1935, "已保存的计划"),
                (1907, "电池供电时也执行"),
                (IDOK, "创建计划"),
                (1912, "取消计划"),
                (1913, "移除"),
                (1911, "刷新"),
                (1916, "核对 Windows"),
                (1917, "能力与说明"),
                (IDCANCEL, "关闭"),
            ] {
                set(window, id, value);
            }
            combo(window, 1901, &Action::ALL.map(Action::label));
            combo(window, 1902, &["倒计时", "指定日期与时间"]);
            combo(
                window,
                1906,
                &[
                    Repeat::Once.label(),
                    Repeat::Daily.label(),
                    Repeat::Weekdays.label(),
                ],
            );
            set(window, 1903, "60");
            SendDlgItemMessageW(window, 1903, EM_SETLIMITTEXT, 5, 0);
            SendDlgItemMessageW(
                window,
                1904,
                DTM_SETFORMATW,
                0,
                crate::wide("yyyy-MM-dd").as_ptr() as isize,
            );
            SendDlgItemMessageW(
                window,
                1905,
                DTM_SETFORMATW,
                0,
                crate::wide("HH:mm").as_ptr() as isize,
            );
            let future = Local::now() + chrono::Duration::hours(1);
            let time = SYSTEMTIME {
                wYear: future.year() as u16,
                wMonth: future.month() as u16,
                wDay: future.day() as u16,
                wHour: future.hour() as u16,
                wMinute: future.minute() as u16,
                ..Default::default()
            };
            for id in [1904, 1905] {
                SendDlgItemMessageW(
                    window,
                    id,
                    DTM_SETSYSTEMTIME,
                    GDT_VALID as usize,
                    &time as *const _ as isize,
                );
            }
            let list = GetDlgItem(window, 1920);
            SendMessageW(
                list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                0,
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
            );
            let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window).max(96);
            for (index, (name, width)) in [
                ("操作", 160),
                ("下次执行（本地时间）", 210),
                ("重复", 182),
                ("计划状态", 140),
            ]
            .into_iter()
            .enumerate()
            {
                let mut name = crate::wide(name);
                let column = LVCOLUMNW {
                    mask: LVCF_TEXT | LVCF_WIDTH,
                    pszText: name.as_mut_ptr(),
                    cx: width * dpi as i32 / 96,
                    ..Default::default()
                };
                SendMessageW(list, LVM_INSERTCOLUMNW, index, &column as *const _ as isize);
            }
            mode(window);
            SetTimer(window, 1, 1000, None);
            let ptr = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut RefCell<State>;
            launch(window, &mut (&*ptr).borrow_mut(), Job::Refresh);
            return 1;
        }
        if let Some(result) = crate::ui::message(window, message, wparam, lparam) {
            return result;
        }
        let ptr = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut RefCell<State>;
        if ptr.is_null() {
            return 0;
        }
        if message == WM_DESTROY {
            KillTimer(window, 1);
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            WINDOW.with(|cell| cell.set(std::ptr::null_mut()));
            if let Ok(mut model) = (&*ptr).borrow().model.lock() {
                model.closed = true;
            }
            drop(Box::from_raw(ptr));
            return 1;
        }
        if message == WM_CLOSE || message == WM_COMMAND && wparam & 0xffff == IDCANCEL as usize {
            DestroyWindow(window);
            return 1;
        }
        if message == WM_COMMAND && wparam & 0xffff == 1917 {
            crate::alert(
                window,
                &format!("{}\r\n\r\n{}", power::capabilities().text, power::HELP),
            );
            return 1;
        }
        if message == WM_COMMAND && wparam & 0xffff == IDOK as usize {
            match form(window) {
                Ok(plan) => launch(window, &mut (&*ptr).borrow_mut(), Job::Create(plan)),
                Err(error) => crate::alert(window, &error),
            }
            return 1;
        }
        let Ok(mut state) = (&*ptr).try_borrow_mut() else {
            return 0;
        };
        if message == UPDATED || message == WM_TIMER {
            preview(window);
            render(window, &mut state);
            return 1;
        }
        if message == WM_COMMAND {
            match (wparam & 0xffff) as i32 {
                1901 | 1902 | 1906 => mode(window),
                1903 => preview(window),
                1911 => launch(window, &mut state, Job::Refresh),
                id @ (1912 | 1913 | 1916) => {
                    if let Some(plan_id) = selected_id(window, &state) {
                        launch(
                            window,
                            &mut state,
                            match id {
                                1912 => Job::Cancel(plan_id),
                                1913 => Job::Remove(plan_id),
                                _ => Job::Inspect(plan_id),
                            },
                        );
                    }
                }
                _ => return 0,
            }
            return 1;
        }
        if message == WM_NOTIFY {
            details(window, &state);
            preview(window);
            return 0;
        }
    }
    0
}

struct Countdown {
    store: Store,
    plan: Plan,
    at: i64,
    go: bool,
    done: bool,
}
unsafe extern "system" fn countdown_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    std::panic::catch_unwind(|| unsafe {
        if message == WM_INITDIALOG {
            SetWindowLongPtrW(window, GWLP_USERDATA, lparam);
            crate::ui::init(window, crate::ui::Kind::PowerRun);
            SetWindowTextW(window, crate::wide("SHUNet · 电源操作倒计时").as_ptr());
            set(window, IDCANCEL, "取消本次操作");
            SetTimer(window, 1, 200, None);
            return 1;
        }
        if let Some(result) = crate::ui::message(window, message, wparam, lparam) {
            return result;
        }
        let ptr = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Countdown;
        if ptr.is_null() {
            return 0;
        }
        let state = &mut *ptr;
        if message == WM_CLOSE || message == WM_COMMAND && wparam & 0xffff == IDCANCEL as usize {
            state.go = false;
            DestroyWindow(window);
            return 1;
        }
        if message == WM_TIMER {
            let now = Local::now().timestamp();
            let remaining = state.at - now;
            set(
                window,
                1941,
                &format!(
                    "{} · 剩余 {} 秒",
                    state.plan.action.label(),
                    remaining.max(0)
                ),
            );
            set(
                window,
                1942,
                &format!(
                    "执行时间：{}\r\n关闭此窗口也会取消本次操作。",
                    power::format_time(state.at)
                ),
            );
            if !state.store.permitted(&state.plan.id, state.at)
                || remaining > 65
                || now > state.at + 15
            {
                state.go = false;
                DestroyWindow(window);
            } else if remaining <= 0 {
                state.go = true;
                DestroyWindow(window);
            }
            return 1;
        }
        if message == WM_DESTROY {
            KillTimer(window, 1);
            state.done = true;
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            return 1;
        }
        0
    })
    .unwrap_or(0)
}
pub fn countdown(store: &Store, plan: &Plan, at: i64) -> bool {
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        let mut state = Box::new(Countdown {
            store: store.clone(),
            plan: plan.clone(),
            at,
            go: false,
            done: false,
        });
        let window = CreateDialogParamW(
            GetModuleHandleW(std::ptr::null()),
            109usize as _,
            std::ptr::null_mut(),
            Some(countdown_proc),
            &mut *state as *mut Countdown as isize,
        );
        if window.is_null() {
            return false;
        }
        ShowWindow(window, SW_SHOW);
        ShowWindow(window, SW_RESTORE);
        SetWindowPos(window, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        SetForegroundWindow(window);
        let mut message = MSG::default();
        while !state.done && GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if IsWindow(window) != 0 {
            DestroyWindow(window);
        }
        state.go
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        Graphics::Gdi::MapWindowPoints, UI::Input::KeyboardAndMouse::IsWindowEnabled,
    };
    #[test]
    fn corrupt_file_notice_does_not_hide_action_failure() {
        let temp = power::tests::Temporary::new();
        std::fs::create_dir_all(&temp.0.folder).unwrap();
        std::fs::write(
            temp.0.folder.join(format!("{}.json", "1".repeat(32))),
            "bad JSON",
        )
        .unwrap();
        let model = Arc::new(Mutex::new(Model {
            busy: true,
            ..Model::default()
        }));
        let weak = Arc::downgrade(&model);
        let sender = start_worker(std::ptr::null_mut(), temp.0.clone(), model.clone()).unwrap();
        // Invalid ID is rejected before any Windows task service request.
        sender.send(Job::Cancel("invalid-id".into())).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while model.lock().unwrap().busy && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        {
            let mut state = model.lock().unwrap();
            assert!(!state.busy);
            assert!(state.info.contains("操作未完成"));
            assert!(state.info.contains("损坏计划"));
            state.closed = true;
        }
        drop(sender);
        drop(model);
        for _ in 0..100 {
            if weak.upgrade().is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(weak.upgrade().is_none());
    }
    fn pump() {
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
    }
    #[test]
    #[ignore = "Uses only own windows and temporary local plan fixtures on an invisible test desktop; no scheduled task or power action"]
    fn native_power_configuration_window() {
        let _desktop = crate::menu_summary::tests::TestDesktop::new();
        unsafe {
            windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            );
        }
        let temp = power::tests::Temporary::new();
        let now = Local::now();
        for (index, action) in [Action::Shutdown, Action::Hibernate, Action::Wake]
            .into_iter()
            .enumerate()
        {
            let mut plan = Plan::new(
                action,
                (now + chrono::Duration::hours(index as i64 + 2)).naive_local(),
                if index == 0 {
                    Repeat::Once
                } else {
                    Repeat::Daily
                },
                false,
            )
            .unwrap();
            plan.enabled = true;
            temp.save(&plan);
        }
        show_store(std::ptr::null_mut(), temp.0.clone()).unwrap();
        let window = WINDOW.with(Cell::get);
        for _ in 0..30 {
            pump();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_ne!(unsafe { IsWindowVisible(window) }, 0);
        assert_eq!(
            unsafe { SendDlgItemMessageW(window, 1920, LVM_GETITEMCOUNT, 0, 0) },
            3
        );
        unsafe {
            SendDlgItemMessageW(window, 1907, BM_CLICK, 0, 0);
        }
        assert!(
            form(window).unwrap().on_battery,
            "battery checkbox must preserve native toggle semantics"
        );
        unsafe {
            SendDlgItemMessageW(window, 1907, BM_CLICK, 0, 0);
        }
        assert!(!form(window).unwrap().on_battery);
        assert_eq!(form(window).unwrap().action, Action::Shutdown);
        assert_eq!(form(window).unwrap().repeat, Repeat::Once);
        unsafe {
            assert_eq!(IsWindowEnabled(GetDlgItem(window, 1904)), 0);
            SendDlgItemMessageW(window, 1902, CB_SETCURSEL, 1, 0);
            SendMessageW(
                window,
                WM_COMMAND,
                1902 | ((CBN_SELCHANGE as usize) << 16),
                0,
            );
            assert_ne!(IsWindowEnabled(GetDlgItem(window, 1904)), 0);
            SendDlgItemMessageW(window, 1906, CB_SETCURSEL, 2, 0);
        }
        assert_eq!(form(window).unwrap().repeat, Repeat::Weekdays);
        for (index, action) in Action::ALL.into_iter().enumerate() {
            unsafe {
                SendDlgItemMessageW(window, 1901, CB_SETCURSEL, index, 0);
            }
            assert_eq!(form(window).unwrap().action, action);
        }
        unsafe {
            SendDlgItemMessageW(window, 1901, CB_SETCURSEL, 0, 0);
        }
        set(window, 1903, "0");
        unsafe {
            SendDlgItemMessageW(window, 1902, CB_SETCURSEL, 0, 0);
        }
        mode(window);
        assert!(form(window).is_err());
        set(window, 1903, "60");
        // Select a fixture, inspect bounds, and capture the real native configuration UI.
        unsafe {
            let item = LVITEMW {
                stateMask: LVIS_SELECTED,
                state: LVIS_SELECTED,
                ..Default::default()
            };
            SendDlgItemMessageW(
                window,
                1920,
                LVM_SETITEMSTATE,
                0,
                &item as *const _ as isize,
            );
        }
        for _ in 0..8 {
            pump();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        for id in [
            1901, 1902, 1903, 1904, 1905, 1906, 1907, IDOK, 1920, 1912, IDCANCEL,
        ] {
            unsafe {
                let mut child = RECT::default();
                let mut client = RECT::default();
                GetWindowRect(GetDlgItem(window, id), &mut child);
                MapWindowPoints(std::ptr::null_mut(), window, &mut child as *mut _ as _, 2);
                GetClientRect(window, &mut client);
                assert!(
                    child.left >= 0
                        && child.top >= 0
                        && child.right <= client.right
                        && child.bottom <= client.bottom,
                    "control {id} outside client"
                );
            }
        }
        crate::network_tools_dialog::tests::capture(window, "power-configuration");
        let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut RefCell<State> };
        let shared = Arc::downgrade(&unsafe { &*pointer }.borrow().model);
        let selection = selected_id(window, &unsafe { &*pointer }.borrow()).unwrap();
        // Timer ticks keep one worker; a changed fixture appears without losing selection.
        for _ in 0..20 {
            unsafe {
                SendMessageW(window, WM_TIMER, 1, 0);
            }
        }
        assert_eq!(shared.strong_count(), 2);
        let bad_id = "2".repeat(32);
        std::fs::write(temp.0.folder.join(format!("{bad_id}.json")), "invalid JSON").unwrap();
        for _ in 0..120 {
            pump();
            if unsafe { SendDlgItemMessageW(window, 1920, LVM_GETITEMCOUNT, 0, 0) } == 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            unsafe { SendDlgItemMessageW(window, 1920, LVM_GETITEMCOUNT, 0, 0) },
            4
        );
        assert_eq!(
            selected_id(window, &unsafe { &*pointer }.borrow()).as_deref(),
            Some(selection.as_str())
        );
        unsafe {
            SendMessageW(window, WM_CLOSE, 0, 0);
        }
        assert!(WINDOW.with(Cell::get).is_null());
        for _ in 0..60 {
            if shared.upgrade().is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            shared.upgrade().is_none(),
            "closed window must release its worker and model"
        );
        show_store(std::ptr::null_mut(), temp.0.clone()).unwrap();
        let reopened = WINDOW.with(Cell::get);
        assert!(!reopened.is_null());
        unsafe {
            DestroyWindow(reopened);
        }
    }
    unsafe extern "system" fn close_countdown(_: HWND, _: u32, id: usize, _: u32) {
        unsafe {
            KillTimer(std::ptr::null_mut(), id);
            let window = FindWindowW(
                std::ptr::null(),
                crate::wide("SHUNet · 电源操作倒计时").as_ptr(),
            );
            if !window.is_null() {
                crate::network_tools_dialog::tests::capture(window, "power-countdown");
                PostMessageW(window, WM_CLOSE, 0, 0);
            }
        }
    }
    #[test]
    #[ignore = "Opens only an own cancellable countdown on an invisible test desktop and automatically cancels it; never calls the power executor"]
    fn native_power_countdown_cancel() {
        let _desktop = crate::menu_summary::tests::TestDesktop::new();
        let temp = power::tests::Temporary::new();
        let at = Local::now().timestamp() + 55;
        let mut plan = Plan::new(
            Action::Shutdown,
            power::relative_time("2", at - 120).unwrap(),
            Repeat::Once,
            false,
        )
        .unwrap();
        plan.enabled = true;
        temp.save(&plan);
        let (claimed, occurrence) = temp.0.claim(&plan.id, at - 55).unwrap().unwrap();
        unsafe {
            assert_ne!(
                SetTimer(std::ptr::null_mut(), 0, 500, Some(close_countdown)),
                0
            );
        }
        assert!(!countdown(&temp.0, &claimed, occurrence));
        temp.0
            .record(&plan.id, occurrence, "cancelled", "test cancellation")
            .unwrap();
        assert!(!temp.0.permitted(&plan.id, occurrence));
        assert!(!temp.0.commit(&plan.id, occurrence, occurrence).unwrap());
    }
}
