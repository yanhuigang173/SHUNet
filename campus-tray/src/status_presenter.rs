use crate::{connection_state::ConnectionState as State, service::View};

pub fn state_label(view: &View) -> &'static str {
    if view.query.active && view.query.ssid == view.ssid && view.query.ip == view.ip {
        return "正在核对连接";
    }
    match view.state {
        State::Online => "已认证在线",
        State::OnlineRechecking => "最近在线 · 正在复核",
        State::EnterpriseConnected => "802.1X 已连接",
        State::Disconnected => "Wi-Fi 未连接",
        State::Connected => "普通 Wi-Fi 已连接",
        State::OtherConnection => "其他接口已连接",
        State::Authenticating => "正在认证",
        State::Verifying => "等待在线确认",
        State::SigningOut => "正在下线",
        State::DetectionDisabled => "定期检测已关闭",
        State::Paused => "已下线 · 自动认证暂停",
        State::Offline | State::NeedsAuthentication => "等待校园认证",
        State::AuthenticationFailed => "认证未成功",
        State::GatewayUnavailable => "网关暂不可达",
        State::WaitingAddress => "等待分配地址",
        State::Checking => "连接状态待确认",
    }
}
pub fn network_label(view: &View) -> String {
    view.network_type_text()
        .trim_start_matches("网络类型：")
        .to_owned()
}
pub fn duration(view: &View) -> String {
    if crate::network::is_enterprise_campus_ssid(&view.ssid) {
        "由 Windows 完成 Wi-Fi 认证".into()
    } else {
        view.online_duration_text()
            .replace("本次在线时长：", "在线 ")
    }
}
pub fn next_check(view: &View) -> String {
    if crate::network::is_enterprise_campus_ssid(&view.ssid) {
        return "无需网页认证与门户检测".into();
    }
    if !view.status_detection {
        return "定期检测已关闭".into();
    }
    if !view.campus {
        return "当前网络无需门户检测".into();
    }
    if view.query.active {
        return "正在只读查询".into();
    }
    if view.busy {
        return "当前操作完成后继续".into();
    }
    view.next_detection.map_or_else(
        || "等待调度".into(),
        |next| {
            let seconds = next
                .saturating_duration_since(std::time::Instant::now())
                .as_secs_f64()
                .ceil() as u64;
            if seconds == 0 {
                "即将进行下次检测".into()
            } else {
                format!("下次检测约 {seconds} 秒后")
            }
        },
    )
}
pub fn details(view: &View) -> String {
    let stats =
        view.history.stats(crate::runtime_history::now_ms());
    format!(
        "SHUNet v{}\r\nWi-Fi：{}\r\n状态：{}\r\n{}\r\n{}\r\n本机 IP：{}\r\n下次认证运营商：{}\r\n今日自动重连：{} 次；已确认恢复：{} 次\r\n最近检测：{}\r\n上次确认在线：{}\r\n检测策略：{}\r\n{}\r\n调度原因：{}\r\n{}{}",
        env!("CARGO_PKG_VERSION"),
        if view.ssid.is_empty() {
            "未连接"
        } else {
            &view.ssid
        },
        state_label(view),
        view.network_type_text(),
        duration(view),
        view.ip,
        view.operator.label(),
        stats.attempts,
        stats.recovered,
        view.checked_at,
        if view.confirmed_at.is_empty() {
            "—"
        } else {
            &view.confirmed_at
        },
        view.detection_level.label(),
        next_check(view),
        view.detection_reason,
        view.state_detail,
        view.history_warning
            .as_ref()
            .map_or_else(String::new, |warning| format!("\r\n日志提示：{warning}"))
    )
}
