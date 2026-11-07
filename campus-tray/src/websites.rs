use serde::{Deserialize, Serialize};

pub const MAX_WEBSITES: usize = 64;
pub const MAX_NAME: usize = 64;
pub const MAX_URL: usize = 2048;
pub const WELCOME_URL: &str = "https://newsso.shu.edu.cn/oauth2/login/eyJyZXNwb25zZVR5cGUiOiJjb2RlIiwiY2xpZW50SWQiOiJMMzc5NlQ0WjEzM2pZdjY0eTFNMzNaMjY2azc4TElqaiIsImNsaWVudE5hbWUiOiLov47mlrDns7vnu58iLCJzY29wZSI6IiIsInJlZGlyZWN0VXJpIjoiaHR0cHM6Ly95eHh0LnNodS5lZHUuY24vc3NvTG9naW4iLCJzdGF0ZSI6IiJ9";

pub fn migrate_welcome(websites:&mut [Website])->bool {
    let mut changed=false;
    for website in websites {
        if website.name=="上海大学迎新系统" && website.url=="https://yxxt.shu.edu.cn/" {
            website.url=WELCOME_URL.into();changed=true;
        }
    }
    changed
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Website {
    pub name: String,
    pub url: String,
}

impl Website {
    pub fn new(name: &str, address: &str) -> Result<Self, String> {
        let name = name.trim();
        let address = address.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control)
        {
            return Err(format!(
                "名称不能为空，最多 {MAX_NAME} 个字，且不能包含换行或制表符。"
            ));
        }
        if address.is_empty()
            || address.len() > MAX_URL * 4
            || address.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("请输入有效网址，不能包含空白字符。".into());
        }
        let address = if address.contains("://") {
            address.to_owned()
        } else {
            format!("https://{address}")
        };
        let url = reqwest::Url::parse(&address).map_err(|_| "网址格式不正确。".to_owned())?;
        if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
            return Err("仅支持 http:// 或 https:// 网页地址。".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("请勿在网址中包含账号密码，请在浏览器中登录。".into());
        }
        if url.as_str().encode_utf16().count() > MAX_URL {
            return Err(format!("网址过长，最多 {MAX_URL} 个字符。"));
        }
        Ok(Self {
            name: name.to_owned(),
            url: url.into(),
        })
    }

    pub fn open(&self) -> Result<(), String> {
        // Configuration is editable outside the app; validate again before ShellExecute.
        let website = Self::new(&self.name, &self.url)?;
        crate::platform::try_open(&website.url)
    }
}

pub fn defaults() -> Vec<Website> {
    [
        ("上海大学迎新系统", WELCOME_URL),
        ("上海大学研究生招生信息网", "https://yjszs.shu.edu.cn/"),
        ("上海大学迎新网站", "https://yingxin.shu.edu.cn/"),
        ("上海大学邮件系统", "https://newmail.shu.edu.cn/"),
        (
            "上海大学个人信息门户（一网通办）",
            "https://eportal.shu.edu.cn/",
        ),
        ("研究生综合服务平台", "https://yjs.shu.edu.cn/"),
        ("研究生选课系统（本研一体化）", "https://byxk.shu.edu.cn/"),
        (
            "本硕博全生命周期统一管理平台",
            "https://xuegong.shu.edu.cn/xsfw/sys/xggzptapp/index.do",
        ),
        ("上海大学机电工程与自动化学院", "https://auto.shu.edu.cn/"),
    ]
    .into_iter()
    .map(|(name, url)| Website {
        name: name.into(),
        url: url.into(),
    })
    .collect()
}

// A damaged bookmark must not make otherwise valid saved credentials unreadable.
// An explicit empty array stays empty; only a missing field gets the defaults.
pub fn from_stored(value: Option<serde_json::Value>) -> (Vec<Website>, Option<String>) {
    let Some(value) = value else {
        return (defaults(), None);
    };
    let Some(entries) = value.as_array() else {
        return (
            defaults(),
            Some("常用网址格式无效，已恢复默认网址；账号设置已保留。".into()),
        );
    };
    let mut websites = Vec::new();
    for entry in entries {
        if websites.len() >= MAX_WEBSITES {
            break;
        }
        if let Ok(website) = serde_json::from_value::<Website>(entry.clone()) {
            if let Ok(website) = Website::new(&website.name, &website.url) {
                if !websites.iter().any(|old: &Website| old.url == website.url) {
                    websites.push(website);
                }
            }
        }
    }
    let warning = (websites.len() != entries.len())
        .then(|| "已跳过无效、重复或超出数量限制的常用网址；其他设置已保留。".into());
    (websites, warning)
}

pub fn validate_all(websites: &[Website]) -> Result<(), String> {
    if websites.len() > MAX_WEBSITES {
        return Err(format!("最多保存 {MAX_WEBSITES} 个网址。"));
    }
    for (index, website) in websites.iter().enumerate() {
        let normalized = Website::new(&website.name, &website.url)?;
        if normalized != *website {
            return Err("网址尚未规范化，请重新编辑后保存。".into());
        }
        if websites[..index].iter().any(|old| old.url == website.url) {
            return Err("这个网址已经在收藏中。".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn welcome_migrates_old_default_without_overwriting_custom_bookmarks() {
        let mut links=vec![Website::new("上海大学迎新系统","https://yxxt.shu.edu.cn/").unwrap(),Website::new("上海大学迎新系统","https://example.org/custom").unwrap()];
        assert!(migrate_welcome(&mut links));assert_eq!(links[0].url,WELCOME_URL);
        assert_eq!(links[1].url,"https://example.org/custom");assert!(!migrate_welcome(&mut links));
        assert!(!migrate_welcome(&mut []));validate_all(&defaults()).unwrap();
        let url=reqwest::Url::parse(WELCOME_URL).unwrap();assert_eq!(url.host_str(),Some("newsso.shu.edu.cn"));
        use base64::Engine;
        let payload=base64::engine::general_purpose::STANDARD.decode(url.path().rsplit('/').next().unwrap()).unwrap();
        let payload:serde_json::Value=serde_json::from_slice(&payload).unwrap();
        assert_eq!(payload["redirectUri"],"https://yxxt.shu.edu.cn/ssoLogin");
    }
}
