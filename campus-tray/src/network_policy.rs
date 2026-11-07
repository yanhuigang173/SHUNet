use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    #[default]
    Auto,
    Campus,
    Ordinary,
}

impl NetworkMode {
    pub fn allows_campus(self, ssid: &str) -> bool {
        !ssid.is_empty()
            && !crate::network::is_enterprise_campus_ssid(ssid)
            && match self {
                Self::Auto => crate::network::supports_campus_portal(ssid),
                Self::Campus => true,
                Self::Ordinary => false,
            }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkPreference {
    pub mode: NetworkMode,
    pub paused: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enterprise_exclusion_precedes_manual_override() {
        for ssid in ["ShuWlan-1X","shuwlan-1x","ShuWlan-1x"] {
            assert!(crate::network::is_enterprise_campus_ssid(ssid));
            for mode in [NetworkMode::Auto,NetworkMode::Campus,NetworkMode::Ordinary] {assert!(!mode.allows_campus(ssid));}
        }
        assert!(NetworkMode::Auto.allows_campus("Shu(ForAll)"));
        assert!(NetworkMode::Campus.allows_campus("OtherCampusSSID"));
    }
}
