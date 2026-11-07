use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    #[default]
    Campus,
    Telecom,
    Unicom,
    Mobile,
}

impl Operator {
    pub const ALL: [Self; 4] = [Self::Campus, Self::Telecom, Self::Unicom, Self::Mobile];

    pub fn label(self) -> &'static str {
        match self {
            Self::Campus => "校园网",
            Self::Telecom => "中国电信",
            Self::Unicom => "中国联通",
            Self::Mobile => "中国移动",
        }
    }

    /// Values verified against the saved campus portal's selectService calls.
    pub fn service_name(self) -> &'static str {
        match self {
            Self::Campus => "shu",
            _ => self.label(),
        }
    }

    pub fn matches(self, value: &str, display: &str) -> bool {
        value == self.service_name() || value == self.label() || display == self.label()
    }
}
