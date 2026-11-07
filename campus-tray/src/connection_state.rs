use crate::icons;

/// The source of truth for the tray colour and its network label. A query badge
/// is presentation only and retains this state until an actual result arrives.
/// Operation logs and automation preferences do not determine authentication.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectionState {
    #[default]
    Checking,
    DetectionDisabled,
    Disconnected,
    Connected,
    EnterpriseConnected,
    OtherConnection,
    WaitingAddress,
    NeedsAuthentication,
    Online,
    OnlineRechecking,
    Authenticating,
    Verifying,
    SigningOut,
    Paused,
    Offline,
    AuthenticationFailed,
    GatewayUnavailable,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Checking => "校园网认证状态待确认",
            Self::DetectionDisabled => "状态检测已关闭 · 认证状态未知",
            Self::Disconnected => "Wi-Fi 未连接",
            Self::Connected => "普通网络 · Wi-Fi 已连接",
            Self::EnterpriseConnected => "802.1X 已连接 · 无需网页认证",
            Self::OtherConnection => "其他网络接口已连接 · 未连接 Wi-Fi",
            Self::WaitingAddress => "Wi-Fi 已连接 · 等待分配 IPv4 地址",
            Self::NeedsAuthentication => "校园网未认证",
            Self::Online => "校园网已认证 · 在线",
            Self::OnlineRechecking => "最近已认证在线 · 正在复核",
            Self::Authenticating => "正在提交校园网认证",
            Self::Verifying => "正在确认校园网认证结果",
            Self::SigningOut => "正在下线校园网",
            Self::Paused => "校园网已下线 · 自动认证已暂停",
            Self::Offline => "已确认校园网认证下线",
            Self::AuthenticationFailed => "校园网认证未成功 · 请查看运行记录",
            Self::GatewayUnavailable => "校园网状态连续检测异常 · 认证情况未知",
        }
    }

    pub fn icon(self) -> icons::State {
        match self {
            Self::Disconnected => icons::State::Disconnected,
            Self::Online => icons::State::Authenticated,
            Self::Authenticating | Self::Verifying | Self::SigningOut => {
                icons::State::Authenticating
            }
            Self::Offline | Self::AuthenticationFailed | Self::GatewayUnavailable => {
                icons::State::Error
            }
            _ => icons::State::Connected,
        }
    }
}

pub const LEGEND: &str = concat!(
    "灰色：未发现已连接 Wi-Fi，也未发现其他具有有效地址的活动接口。\r\n\r\n",
    "蓝色：普通网络 / 其他接口已连接、校园检测关闭，或状态待复核、等待地址、手动下线等。“最近已认证在线 · 正在复核”表示保留最近的在线记录，门户本次返回同步中，不表示重新认证。\r\n\r\n",
    "绿色勾选：本机校园 Wi-Fi 的门户接口已确认认证在线。普通网络不使用校园已认证状态。\r\n\r\n",
    "橙色：正在提交认证、只读确认本轮已提交的认证结果，或执行校园网下线。普通状态查询返回 wait 不会变成正在认证。\r\n\r\n",
    "红色：两次确认已在线会话下线、认证返回失败，或已识别的校园门户连续三次检测异常。检测异常只表示状态未知，不直接认定掉线。\r\n\r\n",
    "同色圆点加载标记：正在只读查询状态，保留查询前的颜色；查询中不等于认证中或下线。结果返回后再更新状态。左键单击查询至少短暂显示约 0.7 秒反馈，快速完成也能看见；后台查询不强制延长显示。\r\n\r\n",
    "已在线时不重复登录。自动认证仅负责首次登录；自动重连仅恢复本次曾在线的意外下线会话。手动下线暂停按网络保存，重启仍保留。\r\n\r\n",
    "“设置 → 当前 Wi-Fi 类型”可覆盖名称识别。普通网络不定期访问校园门户。“状态检测”关闭时暂停定期查询和自动提交，保留手动操作；旧的绿色 / 红色不继续作为当前状态。\r\n\r\n",
    "ShuWlan-1X 使用 Windows Wi-Fi 的 802.1X 账号密码登录，不执行网页认证，也不受校园网类型覆盖设置影响。蓝色表示 Wi-Fi 已连接。\r\n\r\n",
    "校园状态与代理、测速结果独立；代理或 USB 可上网不能证明校园 Wi-Fi 已认证。"
);
