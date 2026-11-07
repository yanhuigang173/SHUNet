//! A modeless, on-demand native window; closing it leaves the tray task running.
use std::cell::Cell;
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
};
const REFRESH: u32 = WM_APP + 17;
thread_local! { static WINDOW: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) }; }

pub fn is_open() -> bool {
    WINDOW.with(|window| !window.get().is_null())
}
pub fn changed() {
    WINDOW.with(|window| {
        if !window.get().is_null() {
            unsafe {
                PostMessageW(window.get(), REFRESH, 0, 0);
            }
        }
    });
}
pub fn translate(message: &MSG) -> bool {
    WINDOW.with(|window| {
        !window.get().is_null() && unsafe { IsDialogMessageW(window.get(), message) != 0 }
    })
}
pub fn show(owner: HWND) -> Result<(), String> {
    if let Some(window) = WINDOW.with(|window| (!window.get().is_null()).then(|| window.get())) {
        unsafe {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
        refresh(window);
        return Ok(());
    }
    let window = unsafe {
        CreateDialogParamW(
            GetModuleHandleW(std::ptr::null()),
            104usize as *const u16,
            owner,
            Some(dialog_proc),
            0,
        )
    };
    if window.is_null() {
        return Err("无法打开测速窗口；测速仍可从托盘菜单查看或取消。".into());
    }
    unsafe {
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
    }
    Ok(())
}
unsafe extern "system" fn dialog_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    std::panic::catch_unwind(|| handle(window, message, wparam, lparam)).unwrap_or(0)
}
fn set(window: HWND, id: i32, text: &str) {
    let text = crate::wide(text);
    let control = unsafe { GetDlgItem(window, id) };
    let length = unsafe { GetWindowTextLengthW(control) }.max(0) as usize;
    let mut current = vec![0u16; length + 1];
    unsafe {
        GetWindowTextW(control, current.as_mut_ptr(), current.len() as i32);
    }
    if current != text {
        unsafe {
            SetWindowTextW(control, text.as_ptr());
        }
    }
}
fn refresh(window: HWND) {
    let state = crate::speed_snapshot();
    let report = &state.current;
    let empty = report.started_at.is_empty();
    let status = if state.cancelling {
        "正在取消"
    } else if state.running {
        "正在测速"
    } else if empty {
        "准备就绪"
    } else if report.error.is_some() {
        "未全部完成"
    } else {
        "测速完成"
    };
    set(window, 1401, status);
    set(
        window,
        1402,
        if report.route.is_empty() {
            "选择直连或代理测速，查看当前网络表现"
        } else if report.mode == crate::speedtest::Mode::Direct {
            "直连不使用 HTTP 代理；TUN / VPN / USB 共享仍由系统路由决定"
        } else {
            &report.route
        },
    );
    let stage = if state.cancelling {
        "等待当前请求结束，不再发送新请求".into()
    } else if state.running {
        report.stage.clone()
    } else if let Some(error) = &report.error {
        error.clone()
    } else if empty {
        "点击右上角的测速模式开始".into()
    } else {
        "测速完成，可切换模式对比或查看详细结果".into()
    };
    set(window, 1404, &stage.replace(['\r', '\n'], " "));
    set(
        window,
        1426,
        &if !state.running && report.error.is_some() {
            "未完成".into()
        } else {
            format!("{:.0}%", report.progress.clamp(0.0, 1.0) * 100.0)
        },
    );
    let pending = if state.running || empty {
        "—"
    } else {
        "未完成"
    };
    set(
        window,
        1405,
        &report
            .latency_ms
            .map_or_else(|| pending.into(), |n| format!("{n:.0}")),
    );
    set(window, 1420, "ms · HTTP 往返");
    for (value_id, unit_id, title_id, final_value, live_value, title) in [
        (
            1406,
            1421,
            1415,
            report.download_mbps,
            report.live_download_mbps,
            "下载速度",
        ),
        (
            1407,
            1422,
            1416,
            report.upload_mbps,
            report.live_upload_mbps,
            "上传速度",
        ),
    ] {
        let value = final_value.or(if state.running { live_value } else { None });
        set(
            window,
            value_id,
            &value.map_or_else(|| pending.into(), |n| format!("{n:.1}")),
        );
        set(
            window,
            unit_id,
            &value.map_or_else(
                || "Mbps".into(),
                |n| format!("Mbps · {:.2} MiB/s", n / 8.388608),
            ),
        );
        let label = if state.running && final_value.is_none() && live_value.is_some() {
            if value_id == 1406 {
                "下载 · 阶段均值"
            } else {
                "上传 · 阶段均值"
            }
        } else {
            title
        };
        set(window, title_id, label);
    }
    set(
        window,
        1408,
        &if empty {
            "延迟为 HTTP 往返；测速结果反映当前出口到 Cloudflare 节点的表现。".into()
        } else {
            format!(
                "已用 {:.1} 秒 · 下载 {:.1} / 上传 {:.1} MiB · HTTP 延迟不等于 Ping",
                report.elapsed,
                report.downloaded as f64 / 1048576.0,
                report.uploaded as f64 / 1048576.0
            )
        },
    );
    for (i, base) in [(0, 1430), (1, 1434)] {
        set(window, base, if i == 0 { "直连" } else { "代理" });
        if let Some(previous) = &state.reports[i] {
            let speed = |n: Option<f64>, prefix: &str| {
                n.map_or_else(|| format!("{prefix} —"), |n| format!("{prefix} {n:.1}"))
            };
            set(window, base + 1, &speed(previous.download_mbps, "↓"));
            set(window, base + 2, &speed(previous.upload_mbps, "↑"));
            let time = previous
                .finished_at
                .split_once(' ')
                .map_or(previous.finished_at.as_str(), |(_, time)| time);
            set(
                window,
                base + 3,
                &format!(
                    "{} · {}",
                    time,
                    if previous.error.is_some() {
                        "未完成"
                    } else {
                        "完成"
                    }
                ),
            );
        } else {
            set(window, base + 1, "↓ —");
            set(window, base + 2, "↑ —");
            set(window, base + 3, "尚无记录");
        }
    }
    set(
        window,
        1409,
        if state.running {
            "取消测速"
        } else {
            "详细结果"
        },
    );
    set(
        window,
        IDCANCEL,
        if state.running {
            "后台运行"
        } else {
            "完成"
        },
    );
    let has_results = state.results().is_some();
    unsafe {
        EnableWindow(
            GetDlgItem(window, 1409),
            i32::from(if state.running {
                !state.cancelling
            } else {
                has_results
            }),
        );
        let ready = !state.running && !crate::network_busy();
        for id in [1411, 1412] {
            EnableWindow(GetDlgItem(window, id), i32::from(ready));
        }
        EnableWindow(GetDlgItem(window, 1413), i32::from(has_results));
        if state.running {
            SetTimer(window, 1, 400, None);
        } else {
            KillTimer(window, 1);
        }
    }
    crate::ui::speed_state(
        window,
        report.progress,
        if state.running || empty {
            crate::ui::Tone::Working
        } else if report.error.is_some() {
            crate::ui::Tone::Warning
        } else {
            crate::ui::Tone::Success
        },
        report.mode == crate::speedtest::Mode::Proxy,
    );
}
fn handle(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    if let Some(result) = crate::ui::message(window, message, wparam, lparam) {
        return result;
    }
    unsafe {
        if message == WM_INITDIALOG {
            WINDOW.with(|active| active.set(window));
            SetWindowTextW(window, crate::wide("SHUNet · 网络测速").as_ptr());
            for (id, text) in [
                (1411, "直连测速"),
                (1412, "代理测速"),
                (1413, "复制结果"),
                (1417, "HTTP 延迟"),
                (1427, "最近对比"),
                (1428, "↓ 下载 / ↑ 上传 · Mbps"),
            ] {
                set(window, id, text);
            }
            crate::ui::init(window, crate::ui::Kind::Speed);
            refresh(window);
            return 1;
        }
        if message == WM_TIMER || message == REFRESH {
            refresh(window);
            return 1;
        }
        if message == WM_CLOSE || (message == WM_COMMAND && (wparam & 0xffff) == IDCANCEL as usize)
        {
            DestroyWindow(window);
            return 1;
        }
        if message == WM_DESTROY {
            KillTimer(window, 1);
            WINDOW.with(|active| active.set(std::ptr::null_mut()));
            return 1;
        }
        if message == WM_COMMAND {
            if (wparam & 0xffff) == 1409 && !crate::speed_snapshot().running {
                let Some(_session) = crate::MenuSession::begin() else {
                    return 1;
                };
                if let Some(text) = crate::speed_snapshot().results() {
                    if let Err(error) = crate::dialogs::show_text(window, "测速详细结果", &text)
                    {
                        crate::alert(window, &error);
                    }
                }
                return 1;
            }
            let command = match (wparam & 0xffff) as i32 {
                1409 => crate::SPEED_CANCEL,
                1411 => crate::SPEED_DIRECT,
                1412 => crate::SPEED_PROXY,
                1413 => crate::SPEED_COPY,
                _ => return 0,
            };
            let view = crate::snapshot();
            if crate::action_enabled(command, &view) {
                crate::execute_action(GetParent(window), command, &view);
            }
            refresh(window);
            return 1;
        }
    }
    0
}
