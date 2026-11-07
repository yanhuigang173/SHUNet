//! Detection policies change scheduling, never the evidence required to log in.
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionLevel {
    Adaptive,
    Extreme,
    High,
    #[default]
    Medium,
    Low,
}

impl DetectionLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Adaptive => "自适应 · 10→30→60 秒",
            Self::Extreme => "极高 · 3 秒",
            Self::High => "高 · 10 秒",
            Self::Medium => "中 · 30 秒",
            Self::Low => "低 · 120 秒",
        }
    }
    pub fn interval(self) -> Duration {
        Duration::from_secs(match self {
            Self::Extreme => 3,
            Self::Adaptive | Self::High => 10,
            Self::Medium => 30,
            Self::Low => 120,
        })
    }
    pub fn confirmation_delay(self) -> Duration {
        Duration::from_secs(match self {
            Self::Adaptive | Self::Extreme | Self::High => 2,
            Self::Medium => 3,
            Self::Low => 5,
        })
    }
    pub fn failure_delay(self, failures: u8) -> Duration {
        let delays = match self {
            Self::Adaptive => [3, 6, 12, 30, 60, 120],
            Self::Extreme | Self::High => [3, 6, 12, 24, 48, 60],
            Self::Medium => [5, 10, 20, 40, 80, 120],
            Self::Low => [10, 20, 40, 80, 160, 300],
        };
        Duration::from_secs(delays[failures.saturating_sub(1).min(5) as usize])
    }
}
