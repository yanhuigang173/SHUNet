use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::detection::DetectionLevel;

#[derive(Clone, Copy)]
enum Phase {
    Online,
    OfflineConfirm,
    Idle,
    Error,
    Transient,
    Cooldown,
}

/// Shared with workers: invalidate before changing a connection or credentials.
#[derive(Clone)]
pub struct OperationGuard {
    revision: Arc<AtomicU64>,
    expected: u64,
    deadline: Option<Instant>,
}
impl OperationGuard {
    pub fn check(&self) -> Result<(), String> {
        if self.revision.load(Ordering::Acquire) == self.expected && self.deadline.is_none_or(|deadline| Instant::now() < deadline) {
            Ok(())
        } else {
            Err("自动重连任务已取消。".to_owned())
        }
    }
    pub fn with_deadline(&self, deadline: Instant) -> Self { Self { deadline: Some(deadline), ..self.clone() } }
    pub fn remaining(&self) -> Option<Duration> { self.deadline.map(|deadline| deadline.saturating_duration_since(Instant::now())) }
}

pub struct ReconnectMonitor {
    pub status: String,
    pub generation: u64,
    revision: Arc<AtomicU64>,
    in_flight: bool,
    next_check: Instant,
    next_auth: Instant,
    offline_samples: u8,
    offline_confirmed: bool,
    attempts: u8,
    probe_errors: u8,
    blocked_credentials: bool,
    waiting_samples: u8,
    warned: bool,
    was_online: bool,
    level: DetectionLevel,
    stable_since: Option<Instant>,
    online_samples: u8,
    observed_online: bool,
    last_dropout: Option<Instant>,
    rapid_pending: bool,
    rapid_until: Option<Instant>,
    phase: Phase,
    interval: Duration,
    reason: &'static str,
}

impl ReconnectMonitor {
    pub fn new(enabled: bool) -> Self {
        Self {
            status: if enabled { "等待检测" } else { "已关闭" }.to_owned(),
            generation: 0,
            revision: Arc::new(AtomicU64::new(0)),
            in_flight: false,
            next_check: Instant::now(),
            next_auth: Instant::now(),
            offline_samples: 0,
            offline_confirmed: false,
            attempts: 0,
            probe_errors: 0,
            blocked_credentials: false,
            waiting_samples: 0,
            warned: false,
            was_online: false,
            level: DetectionLevel::default(),
            stable_since: None,
            online_samples: 0,
            observed_online: false,
            last_dropout: None,
            rapid_pending: false,
            rapid_until: None,
            phase: Phase::Transient,
            interval: Duration::ZERO,
            reason: "启动后首次查询",
        }
    }

    pub fn guard(&self) -> OperationGuard {
        OperationGuard {
            revision: self.revision.clone(),
            expected: self.generation,
            deadline: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.reset_stability();
        self.waiting_samples = 0;
        self.generation = self.revision.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        self.offline_samples = 0;
        self.offline_confirmed = false;
        self.probe_errors = 0;
        self.schedule(
            Duration::from_secs(3),
            "等待网络或设置变化稳定",
            Phase::Transient,
        );
        // Keep in_flight until the bounded worker really returns.
    }

    pub fn cancellation_counter(&self) -> Arc<AtomicU64> {
        self.revision.clone()
    }

    pub fn next_delay(&self) -> Duration {
        self.next_check
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(200))
            .min(Duration::from_secs(300))
    }

    pub fn enable(&mut self, enabled: bool) {
        self.invalidate();
        self.attempts = 0;
        self.probe_errors = 0;
        self.blocked_credentials = false;
        self.warned = false;
        self.was_online = false;
        self.observed_online = false;
        self.schedule(Duration::from_secs(1), "等待首次只读确认", Phase::Transient);
        self.next_auth = Instant::now();
        self.status = if enabled {
            "等待检测"
        } else {
            "自动认证已关闭 · 等待只读检测"
        }
        .to_owned();
    }

    /// Reschedule a read without forgetting a completed login or bad credentials.
    pub fn request_check(&mut self) {
        self.invalidate();
        self.schedule(
            Duration::from_secs(1),
            "已请求一次只读查询",
            Phase::Transient,
        );
    }

    /// Manual reads keep existing evidence, stability and credential cooldowns.
    pub fn request_readonly_check(&mut self) {
        self.schedule(Duration::ZERO, "已请求立即只读查询", Phase::Transient);
    }

    pub fn settle_connection(&mut self) {
        self.schedule(
            Duration::from_secs(3),
            "等待当前 Wi-Fi 连接稳定",
            Phase::Transient,
        );
    }

    pub fn is_checking(&self) -> bool {
        self.in_flight
    }

    fn reset_stability(&mut self) {
        self.stable_since = None;
        self.online_samples = 0;
    }
    fn schedule(&mut self, delay: Duration, reason: &'static str, phase: Phase) {
        self.next_check = Instant::now() + delay;
        self.interval = delay;
        self.reason = reason;
        self.phase = phase;
    }
    fn healthy_policy(&self) -> (Duration, &'static str) {
        if self.level != DetectionLevel::Adaptive {
            return (self.level.interval(), "按所选固定档位查询");
        }
        if self.rapid_until.is_some_and(|until| until > Instant::now()) {
            return (
                Duration::from_secs(10),
                "近期多次明确掉线，恢复后加强观察 5 分钟",
            );
        }
        if self.online_samples < 3 {
            return (Duration::from_secs(10), "刚确认在线，等待连续 3 次正常");
        }
        if self
            .stable_since
            .is_some_and(|since| since.elapsed() >= Duration::from_secs(300))
        {
            (Duration::from_secs(60), "已连续稳定在线超过 5 分钟")
        } else {
            (Duration::from_secs(30), "连续确认在线，进入均衡观察")
        }
    }
    fn idle_interval(&self) -> Duration {
        if self.level == DetectionLevel::Adaptive {
            Duration::from_secs(60)
        } else {
            self.level.interval()
        }
    }
    /// Change only future scheduling. Keep credentials protection, cooldown,
    /// offline evidence and the already-completed first-login opportunity.
    pub fn set_level(&mut self, level: DetectionLevel) {
        if self.level == level {
            return;
        }
        self.level = level;
        match self.phase {
            Phase::Online => {
                let (delay, reason) = self.healthy_policy();
                self.schedule(delay, reason, Phase::Online);
            }
            Phase::Error => self.schedule(
                self.level.failure_delay(self.probe_errors),
                "查询异常，逐步降低重试频率",
                Phase::Error,
            ),
            Phase::Idle => self.schedule(
                self.idle_interval(),
                "自动提交暂停，继续只读观察",
                Phase::Idle,
            ),
            Phase::OfflineConfirm => {
                let remaining = self.next_check.saturating_duration_since(Instant::now());
                self.schedule(
                    remaining.min(self.level.confirmation_delay()),
                    "疑似下线，等待第二次独立确认",
                    Phase::OfflineConfirm,
                );
            }
            Phase::Transient | Phase::Cooldown => {}
        }
    }
    pub fn reset_network_history(&mut self) {
        self.reset_stability();
        self.observed_online = false;
        self.last_dropout = None;
        self.rapid_pending = false;
        self.rapid_until = None;
    }
    pub fn schedule_info(&self) -> (u64, &'static str, Instant) {
        (
            self.interval.as_secs_f64().ceil() as u64,
            self.reason,
            self.next_check,
        )
    }

    pub fn set_status(&mut self, text: &str) -> bool {
        if self.status == text {
            return false;
        }
        self.status = text.to_owned();
        true
    }

    pub fn begin_if_due(&mut self) -> Option<u64> {
        if self.in_flight || Instant::now() < self.next_check {
            return None;
        }
        self.in_flight = true;
        // Healthy checks do not flash the status, write logs or open a browser.
        if !self.was_online {
            self.set_status("后台检查中…");
        }
        Some(self.generation)
    }

    pub fn finish_probe(&mut self, generation: u64) -> bool {
        self.in_flight = false;
        generation == self.generation
    }

    pub fn online(&mut self) {
        if self.stable_since.is_none() {
            self.stable_since = Some(Instant::now());
        }
        self.online_samples = self.online_samples.saturating_add(1);
        self.observed_online = true;
        if self.rapid_pending {
            self.rapid_until = Some(Instant::now() + Duration::from_secs(300));
            self.rapid_pending = false;
        }
        if self
            .rapid_until
            .is_some_and(|until| until <= Instant::now())
        {
            self.rapid_until = None;
        }
        self.waiting_samples = 0;
        self.offline_samples = 0;
        self.offline_confirmed = false;
        self.attempts = 0;
        self.probe_errors = 0;
        self.warned = false;
        self.was_online = true;
        self.next_auth = Instant::now();
        let (delay, reason) = self.healthy_policy();
        self.schedule(delay, reason, Phase::Online);
        self.set_status("门户已确认校园网认证在线");
    }

    pub fn offline(&mut self, may_authenticate: bool) -> bool {
        self.reset_stability();
        self.waiting_samples = 0;
        self.was_online = false;
        self.probe_errors = 0;
        self.offline_samples = self.offline_samples.saturating_add(1);
        if self.offline_samples < 2 {
            self.schedule(
                self.level.confirmation_delay(),
                "疑似下线，等待第二次独立确认",
                Phase::OfflineConfirm,
            );
            self.set_status("疑似下线 · 正在复查");
            return false;
        }
        if !self.offline_confirmed && self.observed_online {
            let now = Instant::now();
            if self
                .last_dropout
                .is_some_and(|last| now.duration_since(last) <= Duration::from_secs(600))
            {
                self.rapid_pending = true;
            }
            self.last_dropout = Some(now);
            self.observed_online = false;
        }
        self.offline_confirmed = true;
        if !may_authenticate || self.blocked_credentials {
            self.schedule(
                self.idle_interval(),
                "已确认未认证，自动提交未启用或需处理账号",
                Phase::Idle,
            );
            self.set_status(if self.blocked_credentials {
                "校园网未认证 · 账号问题需处理"
            } else {
                "校园网未认证 · 可手动认证"
            });
            return false;
        }
        let remaining = self.next_auth.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            self.schedule(remaining, "认证处于独立冷却期", Phase::Cooldown);
            self.set_status("恢复中 · 等待下一次重试");
            return false;
        }
        self.offline_samples = 0;
        self.attempts = self.attempts.saturating_add(1);
        self.next_auth = Instant::now() + retry_delay(self.attempts, 5, 120);
        self.set_status("正在静默重连…");
        true
    }

    pub fn authentication_submitted(&mut self) {
        self.reset_stability();
        self.waiting_samples = 0;
        self.offline_samples = 0;
        self.offline_confirmed = false;
        self.schedule(
            Duration::from_secs(1),
            "已提交认证，等待门户确认",
            Phase::Transient,
        );
        self.set_status("已提交认证 · 等待在线确认");
    }

    pub fn verification_after(&mut self, delay: Duration) {
        self.offline_samples = 0;
        self.offline_confirmed = false;
        self.schedule(delay, "只读确认认证结果，不重复提交", Phase::Transient);
        self.set_status("认证结果待确认 · 正在只读核对");
    }

    pub fn wait_for_network(&mut self, text: &str) {
        self.reset_stability();
        self.observed_online = false;
        self.offline_samples = 0;
        self.offline_confirmed = false;
        self.probe_errors = 0;
        self.was_online = false;
        self.schedule(
            self.idle_interval(),
            "自动提交暂停，继续只读观察",
            Phase::Idle,
        );
        self.set_status(text);
    }
    pub fn portal_waiting(&mut self) -> bool {
        self.reset_stability();
        self.waiting_samples = self.waiting_samples.saturating_add(1);
        if self.waiting_samples <= 5 {
            self.schedule(
                Duration::from_secs(3),
                "门户正在同步认证结果",
                Phase::Transient,
            );
            self.set_status("门户同步中 · 正在确认认证");
            true
        } else {
            self.check_failed();
            self.set_status("门户同步时间较长 · 已降低检测频率，不重复提交认证");
            false
        }
    }

    pub fn wait_for_address(&mut self) {
        self.reset_stability();
        self.offline_samples = 0;
        self.offline_confirmed = false;
        self.probe_errors = 0;
        self.was_online = false;
        self.schedule(
            Duration::from_secs(3),
            "等待 Wi-Fi 分配 IPv4 地址",
            Phase::Transient,
        );
        self.set_status("等待校园 Wi-Fi 分配地址…");
    }

    pub fn check_failed(&mut self) -> bool {
        self.reset_stability();
        self.offline_samples = 0;
        self.offline_confirmed = false;
        self.was_online = false;
        self.probe_errors = self.probe_errors.saturating_add(1);
        self.schedule(
            self.level.failure_delay(self.probe_errors),
            "查询异常，逐步降低重试频率",
            Phase::Error,
        );
        self.set_status("校园门户检测未成功 · 认证状态待确认");
        let notify = !self.warned;
        self.warned = true;
        notify
    }

    pub fn authentication_failed(&mut self, message: &str) {
        self.reset_stability();
        let message_lower = message.to_lowercase();
        self.offline_samples = 0;
        self.offline_confirmed = false;
        self.was_online = false;
        self.blocked_credentials = [
            "密码错误",
            "密码不正确",
            "用户名或密码",
            "账号或密码",
            "帐号或密码",
            "账户锁定",
            "账号锁定",
            "账号不存在",
            "用户不存在",
            "验证码",
            "欠费",
            "密码过期",
            "未绑定服务对应的运营商",
            "未开通该服务",
            "未开通所选运营商",
            "无权使用该服务",
            "invalid password",
            "incorrect password",
            "invalid username",
            "account locked",
            "insufficient balance",
            "captcha",
        ]
        .iter()
        .any(|word| message_lower.contains(word));
        if self.blocked_credentials {
            self.schedule(
                self.idle_interval(),
                "账号问题需处理，仅做只读查询",
                Phase::Idle,
            );
            self.set_status("认证需处理 · 检查账号或运营商设置");
        } else {
            let delay = self
                .next_auth
                .saturating_duration_since(Instant::now())
                .max(Duration::from_secs(5));
            self.schedule(delay, "认证重试冷却中", Phase::Cooldown);
            self.set_status("重连未完成 · 将自动重试");
        }
    }

    pub fn needs_attention(&self) -> bool {
        self.blocked_credentials
    }

    pub fn offline_confirmed(&self) -> bool {
        self.offline_confirmed
    }
    pub fn probe_failures(&self) -> u8 {
        self.probe_errors
    }
}

fn retry_delay(attempt: u8, base: u64, max: u64) -> Duration {
    let seconds = (base * (1u64 << attempt.saturating_sub(1).min(6))).min(max);
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_millis() as u64
        % 500;
    Duration::from_millis(seconds * 1000 + jitter_ms)
}

#[cfg(test)]
mod operator_permission_tests {
    use super::*;

    #[test]
    fn unbound_operator_pauses_submissions_until_settings_change() {
        let mut monitor = ReconnectMonitor::new(true);
        monitor.authentication_failed("未绑定服务对应的运营商");
        assert!(monitor.needs_attention());
        assert!(!monitor.offline(true));
        assert!(!monitor.offline(true));
        monitor.enable(true);
        assert!(!monitor.needs_attention());
    }
}
