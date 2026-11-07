use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::reconnect::OperationGuard;
use crate::operator::Operator;
use crate::auth_trace::{self, PortalRequest};
use reqwest::Url;
use reqwest::blocking::Client;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Clone, Debug)]
pub struct WifiNetwork {
    pub ssid: String,
    pub authentication: String,
    pub encryption: String,
    pub signal: u8,
    pub hidden: bool,
}

impl WifiNetwork {
    pub fn security(&self) -> String {
        if self.is_open() {
            "开放网络".to_owned()
        } else if self.authentication.is_empty() && self.encryption.is_empty() {
            "未知".to_owned()
        } else if self.encryption.is_empty() {
            self.authentication.clone()
        } else {
            format!("{}/{}", self.authentication, self.encryption)
        }
    }

    pub fn is_open(&self) -> bool {
        self.authentication.to_lowercase().contains("open") || self.authentication.contains("开放")
    }

    pub fn is_enterprise(&self) -> bool {
        let auth = self.authentication.to_lowercase();
        auth.contains("enterprise")
            || auth.contains("802.1x")
            || self.authentication.contains("企业")
    }

    pub fn is_personal(&self) -> bool {
        let auth = self.authentication.to_lowercase();
        !self.is_open()
            && !self.is_enterprise()
            && ["personal", "psk", "个人", "wpa"]
                .iter()
                .any(|key| auth.contains(key))
    }
}

#[derive(Debug)]
pub enum ConnectStatus {
    Connected(String),
    NeedsPassword,
}

pub(crate) fn decode_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.trim_start_matches('\u{feff}').to_owned();
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::{CP_ACP, CP_OEMCP, MultiByteToWideChar};
        for code_page in [CP_OEMCP, CP_ACP] {
            let length = unsafe {
                MultiByteToWideChar(
                    code_page as u32,
                    0,
                    bytes.as_ptr(),
                    bytes.len() as i32,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if length > 0 {
                let mut wide = vec![0u16; length as usize];
                unsafe {
                    MultiByteToWideChar(
                        code_page as u32,
                        0,
                        bytes.as_ptr(),
                        bytes.len() as i32,
                        wide.as_mut_ptr(),
                        length,
                    );
                }
                return String::from_utf16_lossy(&wide);
            }
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn run_netsh(arguments: &[&str]) -> Result<String, String> {
    let mut command = Command::new("netsh");
    command.args(arguments);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = command
        .output()
        .map_err(|err| format!("无法启动 netsh：{err}"))?;
    let stdout = decode_output(&output.stdout);
    let stderr = decode_output(&output.stderr);
    let combined = format!("{stdout}{stderr}").trim().to_owned();
    if output.status.success() {
        Ok(combined)
    } else {
        Err(if combined.is_empty() {
            format!("netsh 命令失败：{}", output.status)
        } else {
            combined
        })
    }
}

pub fn connected_ssid() -> String {
    #[cfg(windows)]
    {
        return crate::wifi::current_ssid(None).unwrap_or_default();
    }
    #[cfg(not(windows))]
    {
        let Ok(output) = run_netsh(&["wlan", "show", "interfaces"]) else {
            return String::new();
        };
        for line in output.lines() {
            let line = line.trim();
            if let Some(value) = line
                .strip_prefix("SSID")
                .and_then(|v| v.trim_start().strip_prefix(':'))
            {
                let value = value.trim();
                if !value.is_empty() && !value.eq_ignore_ascii_case("none") {
                    return value.to_owned();
                }
            }
        }
        String::new()
    }
}

pub fn is_campus_ssid(ssid: &str) -> bool {
    let name = ssid.to_lowercase();
    name.contains("shu") || name.contains("cmcc")
}

pub fn is_enterprise_campus_ssid(ssid:&str)->bool { ssid.trim().eq_ignore_ascii_case("ShuWlan-1X") }

pub fn supports_campus_portal(ssid: &str) -> bool {
    let name = ssid.to_lowercase();
    is_campus_ssid(ssid) && !name.contains("1x")
}

pub fn current_wifi_connection() -> Result<(String, String, Option<u32>), String> {
    let interface = wifi_interface().ok();
    #[cfg(windows)]
    let ssid = crate::wifi::cached_current_ssid(interface.as_ref().map(|interface| interface.index))?;
    #[cfg(not(windows))]
    let ssid = connected_ssid();
    let index = interface.as_ref().map(|interface| interface.index);
    let ip = if ssid.is_empty() {
        "—".to_owned()
    } else {
        interface.map_or_else(|| "—".to_owned(), |interface| interface.address.to_string())
    };
    Ok((ssid, ip, index))
}

/// Local link information only; an active adapter does not prove Internet access.
pub fn other_connection_address() -> Option<String> {
    use windows_sys::Win32::{
        Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS},
        NetworkManagement::{IpHelper::*, Ndis::IfOperStatusUp},
        Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6},
    };
    let mut bytes = 15_000u32;
    for _ in 0..3 {
        let count = (bytes as usize).div_ceil(std::mem::size_of::<IP_ADAPTER_ADDRESSES_LH>());
        let mut buffer = vec![IP_ADAPTER_ADDRESSES_LH::default(); count];
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER,
                std::ptr::null(),
                buffer.as_mut_ptr(),
                &mut bytes,
            )
        };
        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if result != ERROR_SUCCESS {
            return None;
        }
        let mut pointer = buffer.as_ptr();
        while !pointer.is_null() {
            let adapter = unsafe { &*pointer };
            if adapter.OperStatus == IfOperStatusUp
                && adapter.IfType != IF_TYPE_IEEE80211
                && adapter.IfType != IF_TYPE_SOFTWARE_LOOPBACK
            {
                let mut unicast = adapter.FirstUnicastAddress;
                while !unicast.is_null() {
                    let socket = unsafe { (*unicast).Address };
                    if !socket.lpSockaddr.is_null()
                        && socket.iSockaddrLength as usize >= std::mem::size_of::<SOCKADDR_IN6>()
                        && unsafe { (*socket.lpSockaddr).sa_family } == AF_INET6
                    {
                        let socket = unsafe { &*(socket.lpSockaddr.cast::<SOCKADDR_IN6>()) };
                        let ip = std::net::Ipv6Addr::from(unsafe { socket.sin6_addr.u.Byte });
                        if !ip.is_unspecified()
                            && !ip.is_loopback()
                            && !ip.is_unicast_link_local()
                            && !ip.is_multicast()
                        {
                            return Some(ip.to_string());
                        }
                    }
                    if !socket.lpSockaddr.is_null()
                        && socket.iSockaddrLength as usize >= std::mem::size_of::<SOCKADDR_IN>()
                        && unsafe { (*socket.lpSockaddr).sa_family } == AF_INET
                    {
                        let socket = unsafe { &*(socket.lpSockaddr.cast::<SOCKADDR_IN>()) };
                        let b = unsafe { socket.sin_addr.S_un.S_un_b };
                        let ip = Ipv4Addr::new(b.s_b1, b.s_b2, b.s_b3, b.s_b4);
                        if !ip.is_unspecified() && !ip.is_loopback() && !ip.is_link_local() {
                            return Some(ip.to_string());
                        }
                    }
                    unicast = unsafe { (*unicast).Next };
                }
            }
            pointer = adapter.Next;
        }
        return None;
    }
    None
}

pub fn scan_wifi() -> Result<(Vec<WifiNetwork>, String), String> {
    #[cfg(windows)]
    {
        crate::wifi::scan()
    }
    #[cfg(not(windows))]
    {
        Err("Wi-Fi 扫描目前仅支持 Windows。".to_owned())
    }
}

fn saved_profiles() -> Result<Vec<String>, String> {
    let output = run_netsh(&["wlan", "show", "profiles"])?;
    let mut profiles = Vec::new();
    for line in output.lines() {
        let lower = line.to_lowercase();
        if let Some((_, profile)) = line.split_once(':') {
            if lower.contains("all user profile")
                || lower.contains("current user profile")
                || line.contains("所有用户配置文件")
                || line.contains("当前用户配置文件")
            {
                profiles.push(profile.trim().to_owned());
            }
        }
    }
    Ok(profiles)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn profile_xml(network: &WifiNetwork, password: Option<&str>) -> String {
    let ssid = xml_escape(&network.ssid);
    let (authentication, encryption, shared_key) = if let Some(password) = password {
        let auth_text = network.authentication.to_lowercase();
        let auth = if auth_text.contains("wpa3") && !auth_text.contains("wpa2") {
            "WPA3SAE"
        } else if auth_text.contains("wpa-personal") || auth_text.contains("wpa personal") {
            "WPAPSK"
        } else {
            "WPA2PSK"
        };
        let encryption = if network.encryption.to_lowercase().contains("tkip") {
            "TKIP"
        } else {
            "AES"
        };
        (
            auth.to_owned(),
            encryption.to_owned(),
            format!(
                "<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>",
                xml_escape(password)
            ),
        )
    } else {
        ("open".to_owned(), "none".to_owned(), String::new())
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{ssid}</name><SSIDConfig><SSID><name>{ssid}</name></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM><security><authEncryption><authentication>{authentication}</authentication><encryption>{encryption}</encryption><useOneX>false</useOneX></authEncryption>{shared_key}</security></MSM></WLANProfile>"
    )
}

pub fn connect_wifi(
    network: &WifiNetwork,
    password: Option<&str>,
) -> Result<ConnectStatus, String> {
    if network.hidden {
        return Err("隐藏网络需要先在 Windows Wi-Fi 设置中输入网络名称和密码。".to_owned());
    }
    if network.is_enterprise() || is_enterprise_campus_ssid(&network.ssid) {
        return Err("这是 802.1X 企业网络，请先在 Windows Wi-Fi 设置中完成认证。".to_owned());
    }
    if !network.is_open() && !network.is_personal() {
        return Err("无法识别该 Wi-Fi 的安全类型，请在 Windows Wi-Fi 设置中连接。".to_owned());
    }

    if network.is_personal() {
        if let Some(password) = password {
            if password.len() < 8 || password.len() > 64 {
                return Err("Wi-Fi 密码长度应为 8 到 64 个字符。".to_owned());
            }
        }
    }

    let profiles = saved_profiles()?;
    let has_profile = profiles.iter().any(|profile| profile == &network.ssid);
    if network.is_personal() && password.is_none() && !has_profile {
        return Ok(ConnectStatus::NeedsPassword);
    }
    // Re-add a profile with the supplied key without deleting the existing one first.
    // If Windows rejects the replacement, its previous saved profile remains intact.
    if !has_profile || (network.is_personal() && password.is_some()) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path =
            std::env::temp_dir().join(format!("campus-link-{}-{stamp}.xml", std::process::id()));
        std::fs::write(
            &path,
            profile_xml(
                network,
                if network.is_personal() {
                    password
                } else {
                    None
                },
            ),
        )
        .map_err(|err| format!("无法创建临时 Wi-Fi 配置：{err}"))?;
        let path_string = path.to_string_lossy().into_owned();
        let filename = format!("filename=\"{path_string}\"");
        let result = run_netsh(&["wlan", "add", "profile", &filename, "user=current"]);
        let _ = std::fs::remove_file(&path);
        result?;
    }

    let name = format!("name=\"{}\"", network.ssid);
    let ssid = format!("ssid=\"{}\"", network.ssid);
    run_netsh(&["wlan", "connect", &name, &ssid])?;

    let deadline = Instant::now() + Duration::from_secs(18);
    while Instant::now() < deadline {
        if connected_ssid() == network.ssid {
            return Ok(ConnectStatus::Connected(format!(
                "已连接到 {}",
                network.ssid
            )));
        }
        thread::sleep(Duration::from_millis(800));
    }
    Err("Windows 未能在限定时间内连接。请检查密码，或尝试在系统 Wi-Fi 设置中连接。".to_owned())
}

pub fn disconnect_wifi() -> Result<String, String> {
    run_netsh(&["wlan", "disconnect"]).map(|output| {
        if output.is_empty() {
            "已断开 Wi-Fi。".to_owned()
        } else {
            output
        }
    })
}

#[derive(Clone, Debug)]
struct WifiInterface {
    name: String,
    index: u32,
    address: Ipv4Addr,
}

#[cfg(windows)]
fn wifi_interface() -> Result<WifiInterface, String> {
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
        GetAdaptersAddresses, IF_TYPE_IEEE80211, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    let mut byte_count = 15_000u32;
    for _ in 0..3 {
        // Typed POD storage keeps the buffer correctly aligned for the linked
        // adapter/address structures returned by the Windows IP Helper API.
        let count = (byte_count as usize).div_ceil(std::mem::size_of::<IP_ADAPTER_ADDRESSES_LH>());
        let mut buffer = vec![IP_ADAPTER_ADDRESSES_LH::default(); count];
        let result = unsafe {
            GetAdaptersAddresses(
                AF_INET as u32,
                GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER,
                std::ptr::null(),
                buffer.as_mut_ptr(),
                &mut byte_count,
            )
        };
        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if result != ERROR_SUCCESS {
            return Err(format!(
                "读取 Wi-Fi 网卡地址失败（Windows 错误 {result}）。"
            ));
        }

        let mut candidates = Vec::new();
        let mut adapter_ptr = buffer.as_ptr();
        while !adapter_ptr.is_null() {
            // All pointers stay inside buffer, which remains alive throughout
            // this traversal. Only active IEEE 802.11 adapters are eligible;
            // USB tethering and TUN adapters must never supply the login IP.
            let adapter = unsafe { &*adapter_ptr };
            if adapter.IfType == IF_TYPE_IEEE80211 && adapter.OperStatus == IfOperStatusUp {
                let mut address_ptr = adapter.FirstUnicastAddress;
                while !address_ptr.is_null() {
                    let unicast = unsafe { &*address_ptr };
                    let socket = unicast.Address;
                    if !socket.lpSockaddr.is_null()
                        && socket.iSockaddrLength as usize >= std::mem::size_of::<SOCKADDR_IN>()
                        && unsafe { (*socket.lpSockaddr).sa_family } == AF_INET
                    {
                        let socket = unsafe { &*(socket.lpSockaddr.cast::<SOCKADDR_IN>()) };
                        let octets = unsafe { socket.sin_addr.S_un.S_un_b };
                        let address =
                            Ipv4Addr::new(octets.s_b1, octets.s_b2, octets.s_b3, octets.s_b4);
                        if !address.is_unspecified()
                            && !address.is_loopback()
                            && !address.is_link_local()
                        {
                            let name = if adapter.FriendlyName.is_null() {
                                "Wi-Fi".to_owned()
                            } else {
                                let mut length = 0;
                                while length < 256
                                    && unsafe { *adapter.FriendlyName.add(length) } != 0
                                {
                                    length += 1;
                                }
                                String::from_utf16_lossy(unsafe {
                                    std::slice::from_raw_parts(adapter.FriendlyName, length)
                                })
                            };
                            candidates.push((
                                adapter.Ipv4Metric,
                                WifiInterface {
                                    name,
                                    index: unsafe { adapter.Anonymous1.Anonymous.IfIndex },
                                    address,
                                },
                            ));
                        }
                    }
                    address_ptr = unicast.Next;
                }
            }
            adapter_ptr = adapter.Next;
        }
        candidates.sort_by_key(|(metric, interface)| (*metric, interface.index));
        return candidates
            .into_iter()
            .next()
            .map(|(_, interface)| interface)
            .ok_or_else(|| {
                "未找到具有有效 IPv4 地址的已连接 Wi-Fi 网卡。请先连接校园 Wi-Fi。".to_owned()
            });
    }
    Err("读取 Wi-Fi 网卡时网络配置连续变化，请稍后重试。".to_owned())
}

#[cfg(not(windows))]
fn wifi_interface() -> Result<WifiInterface, String> {
    Err("校园 Wi-Fi 网卡选择目前仅支持 Windows。".to_owned())
}

pub fn local_ip_address() -> String {
    if let Ok(interface) = wifi_interface() {
        return interface.address.to_string();
    }
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("10.10.9.9:80")?;
            socket.local_addr()
        })
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|_| "—".to_owned())
}

fn extract_attribute(tag: &str, name: &str) -> Option<String> {
    let tag_lower = tag.to_ascii_lowercase();
    let name_lower = name.to_ascii_lowercase();
    let bytes = tag_lower.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let relative = tag_lower[cursor..].find(&name_lower)?;
        let start = cursor + relative;
        let end = start + name_lower.len();
        if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'-') {
            cursor = end;
            continue;
        }
        let mut value_start = end;
        while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
            value_start += 1;
        }
        if bytes.get(value_start) != Some(&b'=') {
            cursor = end;
            continue;
        }
        value_start += 1;
        while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
            value_start += 1;
        }
        let quote = *bytes.get(value_start)?;
        if quote == b'\'' || quote == b'"' {
            let content_start = value_start + 1;
            let content_end = tag.as_bytes()[content_start..]
                .iter()
                .position(|byte| *byte == quote)?
                + content_start;
            return Some(tag[content_start..content_end].to_owned());
        }
        let content_end = tag[value_start..]
            .find(|ch: char| ch.is_ascii_whitespace() || ch == '>')
            .map(|offset| value_start + offset)
            .unwrap_or(tag.len());
        return Some(tag[value_start..content_end].to_owned());
    }
    None
}

fn extract_html_value(html: &str, name: &str) -> Option<String> {
    // Handle the common ePortal JavaScript form: `var queryString = "..."`.
    let lower = html.to_ascii_lowercase();
    let marker = name.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(&marker) {
        let start = cursor + relative;
        let mut value_start = start + marker.len();
        let bytes = html.as_bytes();
        while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
            value_start += 1;
        }
        if bytes.get(value_start) == Some(&b'=') || bytes.get(value_start) == Some(&b':') {
            value_start += 1;
            while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
                value_start += 1;
            }
            if let Some(quote @ (b'\'' | b'"')) = bytes.get(value_start).copied() {
                let content_start = value_start + 1;
                if let Some(offset) = bytes[content_start..]
                    .iter()
                    .position(|byte| *byte == quote)
                {
                    return Some(html[content_start..content_start + offset].to_owned());
                }
            }
        }
        cursor = start + marker.len();
    }

    // Also accept a matching hidden input field.
    for tag in html
        .split('<')
        .filter(|part| part.starts_with("input") || part.starts_with("INPUT"))
    {
        if extract_attribute(tag, "name").is_some_and(|field| field.eq_ignore_ascii_case(name)) {
            if let Some(value) = extract_attribute(tag, "value") {
                return Some(value);
            }
        }
    }
    None
}

fn portal_query_string(page_url: &Url, _html: &str) -> String {
    // The portal JavaScript submits the complete, still-escaped location.search
    // (without the leading '?'). Decoding individual pairs here changes the
    // data that the portal expects for its double-encoded login request.
    page_url.query().unwrap_or_default().to_owned()
}

fn portal_page_url(response_url: &Url) -> Result<Url, String> {
    // Do not manufacture a page URL from an unrelated redirect: its missing
    // controller parameters make a valid API report "device not registered".
    if response_url.scheme() == "http"
        && response_url.host_str() == Some("10.10.9.9")
        && response_url
            .path()
            .to_ascii_lowercase()
            .starts_with("/eportal/")
    {
        return Ok(response_url.clone());
    }
    Err(format!(
        "未取得真实校园网认证页（收到 {}{}）。请检查 Wi-Fi 出口或代理/TUN 设置后重试。",
        response_url.host_str().unwrap_or("未知主机"),
        response_url.path()
    ))
}

fn portal_script_redirect(current_url: &Url, html: &str) -> Option<Url> {
    // The gateway bootstrap assigns a literal location.href. Treat it as data,
    // and follow only a link back to the configured ePortal login page.
    for marker in [
        "location.href",
        "window.location",
        "document.location",
        "location",
    ] {
        if let Some(target) = extract_html_value(html, marker) {
            let target = target.replace("\\/", "/");
            if let Ok(next) = current_url.join(&target) {
                if portal_page_url(&next).is_ok() && next.path().ends_with("/index.jsp") {
                    return Some(next);
                }
            }
        }
    }
    None
}

fn validate_portal_context(page_url: &Url) -> Result<(), String> {
    let path = page_url.path().to_ascii_lowercase();
    if !path.ends_with("/index.jsp") {
        return Err(
            "当前返回的页面不是登录页，已停止重复认证。请先确认校园网在线状态。".to_owned(),
        );
    }
    let pairs: Vec<_> = page_url.query_pairs().collect();
    let required = ["wlanuserip", "wlanacname", "ssid", "nasip", "mac", "t"];
    let missing: Vec<_> = required
        .into_iter()
        .filter(|name| {
            !pairs
                .iter()
                .any(|(key, value)| key.eq_ignore_ascii_case(name) && !value.is_empty())
        })
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "门户跳转缺少当前 Wi-Fi 的认证参数：{}。已停止提交，避免门户误判为设备未注册。",
            missing.join("、")
        ));
    }
    Ok(())
}

/// Accept the two address representations used by the supplied SHU gateway.
/// `wlanuserip` is often a 32-character hexadecimal gateway token, not an IPv4
/// literal. Its original query bytes must be passed back without decoding or
/// inventing an IP. A token is not evidence of a matching/decrypted address:
/// callers must discover it afresh through the Wi-Fi-bound, proxy-free client,
/// require an explicit offline API result, and recheck the adapter fingerprint.
fn validate_login_page_address(page_url: &Url, expected: Ipv4Addr) -> Result<(), String> {
    validate_portal_context(page_url)?;
    let value = page_url
        .query_pairs()
        .find(|(key, _)| key.eq_ignore_ascii_case("wlanuserip"))
        .map(|(_, value)| value.into_owned())
        .ok_or("门户登录页缺少网卡地址参数，状态待确认。")?;
    let value = value.trim();
    if let Ok(address) = value.parse::<Ipv4Addr>() {
        if address != expected {
            return Err("门户登录页的明文 IPv4 地址与当前 Wi-Fi 不一致，状态待确认。".into());
        }
        return Ok(());
    }
    if value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    Err("门户登录页的地址参数格式无法识别，状态待确认；未提交账号密码。".into())
}

fn uri_component_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                *byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            encoded.push(*byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn uri_component_encode_twice(value: &str) -> String {
    uri_component_encode(&uri_component_encode(value))
}

fn portal_endpoint(page_url: &Url, method: &str) -> Result<Url, String> {
    let mut endpoint = portal_page_url(page_url)?
        .join("./InterFace.do")
        .map_err(|error| format!("无法生成门户接口地址：{error}"))?;
    endpoint.set_query(None);
    endpoint.query_pairs_mut().append_pair("method", method);
    Ok(endpoint)
}

fn response_field<'a>(value: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    value
        .get(name)
        .or_else(|| value.get("data").and_then(|nested| nested.get(name)))
        .or_else(|| value.get("pageInfo").and_then(|nested| nested.get(name)))
}

fn json_value_as_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[derive(Debug)]
struct PortalPageInfo {
    password_encrypt: bool,
    public_key_exponent: String,
    public_key_modulus: String,
}

fn portal_page_info(body: &str, page_html: &str) -> PortalPageInfo {
    let json = serde_json::from_str::<serde_json::Value>(body).unwrap_or(serde_json::Value::Null);
    let encrypt = response_field(&json, "passwordEncrypt")
        .and_then(json_value_as_string)
        .or_else(|| extract_html_value(page_html, "passwordEncrypt"))
        .is_some_and(|value| value.eq_ignore_ascii_case("true") || value == "1");
    let public_key_exponent = response_field(&json, "publicKeyExponent")
        .and_then(json_value_as_string)
        .filter(|value| !value.is_empty())
        .or_else(|| extract_html_value(page_html, "publicKeyExponent"))
        .unwrap_or_default();
    let public_key_modulus = response_field(&json, "publicKeyModulus")
        .and_then(json_value_as_string)
        .filter(|value| !value.is_empty())
        .or_else(|| extract_html_value(page_html, "publicKeyModulus"))
        .unwrap_or_default();
    PortalPageInfo {
        password_encrypt: encrypt,
        public_key_exponent,
        public_key_modulus,
    }
}

fn service_from_html(html: &str, operator: Operator) -> Option<String> {
    let mut values = Vec::new();
    for part in html.split('<') {
        let tag = part.trim_start();
        if tag
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("option"))
        {
            if let Some(value) = extract_attribute(tag, "value") {
                let display = tag.split_once('>').map(|(_, text)| text.trim()).unwrap_or("");
                if !value.is_empty() && operator.matches(&value, display) {
                    values.push(value);
                }
            }
        }
        // The saved SHU page renders DIV choices rather than SELECT options.
        // Parse only the choice's two string arguments, never execute page code.
        let Some(id) = extract_attribute(tag, "id") else { continue; };
        if id == "net_access_type" {
            if let Some(value) = extract_attribute(tag, "value").filter(|value| operator.matches(value, "")) {
                values.push(value);
            }
        }
        if !id.starts_with("bch_service_") { continue; }
        if let Some(call) = extract_attribute(tag, "onclick") {
            let call = call.replace("&#39;", "'").replace("&#x27;", "'")
                .replace("&apos;", "'").replace("&quot;", "\"");
            if let Some(args) = call.trim().strip_prefix("selectService(") {
                let mut args = args.split(',').take(2).map(|arg| arg.trim().trim_matches(['\'', '"']));
                if let (Some(value), Some(display)) = (args.next(), args.next()) {
                    if !value.is_empty() && operator.matches(value, display) {
                        values.push(value.to_owned());
                    }
                }
            }
        }
    }
    if let Some(value) = extract_html_value(html, "net_access_type").filter(|value| operator.matches(value, "")) { values.push(value); }
    values.sort();
    values.dedup();
    (values.len() == 1).then(|| values.remove(0))
}

fn service_from_response(body: &str, operator: Operator) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(body).ok()?;
    let services_value = root.get("serviceJson")?;
    let services: serde_json::Value = match services_value {
        serde_json::Value::String(json) => serde_json::from_str(json).ok()?,
        value => value.clone(),
    };
    let list = services.as_array()?;
    let mut values: Vec<String> = list.iter().filter_map(|item| {
        let value = item.get("serviceName").or_else(|| item.get("id"))?.as_str()?;
        let display = item.get("serviceShowName").and_then(|value| value.as_str()).unwrap_or("");
        (!value.is_empty() && operator.matches(value, display)).then(|| value.to_owned())
    }).collect();
    values.sort(); values.dedup();
    (values.len() == 1).then(|| values.remove(0))
}

fn portal_login_body(username: &str, password: &str, service: &str, query: &str, encrypt: &str) -> String {
    // Match AuthInterFace.js: each field is encoded twice before sending the body.
    format!(
        "userId={}&password={}&service={}&queryString={}&operatorPwd=&operatorUserId=&validcode=&passwordEncrypt={}",
        uri_component_encode_twice(username.trim()),
        uri_component_encode_twice(password),
        uri_component_encode_twice(service),
        uri_component_encode_twice(query),
        uri_component_encode_twice(encrypt),
    )
}

// The campus page uses RSAUtils' textbook RSA format: reverse the UTF-16
// string, pack character pairs little-endian into 16-bit words, exponentiate,
// then emit each result word as four lowercase hex digits. Keeping this small
// implementation local avoids depending on a native crypto DLL on Windows.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PortalBigUint {
    words: Vec<u32>,
}

impl PortalBigUint {
    fn normalize(&mut self) {
        while self.words.last() == Some(&0) {
            self.words.pop();
        }
    }

    fn from_hex(hex: &str) -> Option<Self> {
        let mut result = Self::default();
        for ch in hex.trim().trim_start_matches("0x").chars() {
            let digit = ch.to_digit(16)?;
            let mut carry = digit;
            for word in &mut result.words {
                let value = (*word << 4) | carry;
                *word = value & 0xffff;
                carry = value >> 16;
            }
            if carry != 0 {
                result.words.push(carry);
            }
            if result.words.is_empty() && digit != 0 {
                result.words.push(digit);
            }
        }
        result.normalize();
        Some(result)
    }

    fn from_little_endian_bytes(bytes: &[u8]) -> Self {
        let mut words = Vec::with_capacity(bytes.len().div_ceil(2));
        for pair in bytes.chunks(2) {
            words.push(u32::from(pair[0]) | (u32::from(*pair.get(1).unwrap_or(&0)) << 8));
        }
        let mut result = Self { words };
        result.normalize();
        result
    }

    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        self.words
            .len()
            .cmp(&other.words.len())
            .then_with(|| self.words.iter().rev().cmp(other.words.iter().rev()))
    }

    fn bit_len(&self) -> usize {
        self.words.last().map_or(0, |word| {
            (self.words.len() - 1) * 16 + (32 - word.leading_zeros()) as usize
        })
    }

    fn bit(&self, index: usize) -> bool {
        self.words
            .get(index / 16)
            .is_some_and(|word| (word & (1 << (index % 16))) != 0)
    }

    fn add_mod(left: &Self, right: &Self, modulus: &Self) -> Self {
        let length = left.words.len().max(right.words.len());
        let mut words = Vec::with_capacity(length + 1);
        let mut carry = 0u32;
        for index in 0..length {
            let value = left.words.get(index).copied().unwrap_or(0)
                + right.words.get(index).copied().unwrap_or(0)
                + carry;
            words.push(value & 0xffff);
            carry = value >> 16;
        }
        if carry != 0 {
            words.push(carry);
        }
        let mut result = Self { words };
        result.normalize();
        if result.compare(modulus) != std::cmp::Ordering::Less {
            result.subtract_assign(modulus);
        }
        result
    }

    fn subtract_assign(&mut self, other: &Self) {
        let mut borrow = 0i32;
        for index in 0..self.words.len() {
            let value = self.words[index] as i32
                - other.words.get(index).copied().unwrap_or(0) as i32
                - borrow;
            if value < 0 {
                self.words[index] = (value + 0x1_0000) as u32;
                borrow = 1;
            } else {
                self.words[index] = value as u32;
                borrow = 0;
            }
        }
        self.normalize();
    }

    fn multiply_mod(left: &Self, right: &Self, modulus: &Self) -> Self {
        let mut result = Self::default();
        let mut addend = left.clone();
        for bit in 0..right.bit_len() {
            if right.bit(bit) {
                result = Self::add_mod(&result, &addend, modulus);
            }
            if bit + 1 < right.bit_len() {
                addend = Self::add_mod(&addend, &addend, modulus);
            }
        }
        result
    }

    fn pow_mod(&self, exponent: u32, modulus: &Self) -> Self {
        let mut result = Self { words: vec![1] };
        let mut base = self.clone();
        let mut power = exponent;
        while power != 0 {
            if power & 1 != 0 {
                result = Self::multiply_mod(&result, &base, modulus);
            }
            power >>= 1;
            if power != 0 {
                base = Self::multiply_mod(&base, &base, modulus);
            }
        }
        result
    }

    fn to_hex_words(&self) -> String {
        self.words
            .iter()
            .rev()
            .map(|word| format!("{word:04x}"))
            .collect()
    }
}

fn portal_rsa_encrypt(text: &str, exponent_hex: &str, modulus_hex: &str) -> Result<String, String> {
    let modulus = PortalBigUint::from_hex(modulus_hex)
        .filter(|value| value.words.len() >= 2)
        .ok_or_else(|| "门户返回的 RSA 公钥无效。".to_owned())?;
    let exponent = u32::from_str_radix(exponent_hex.trim().trim_start_matches("0x"), 16)
        .map_err(|_| "门户返回的 RSA 指数无效。".to_owned())?;
    let chunk_size = 2 * (modulus.words.len() - 1);
    if chunk_size == 0 {
        return Err("门户返回的 RSA 公钥长度无效。".to_owned());
    }

    let mut chars: Vec<u16> = text.encode_utf16().collect();
    chars.reverse();
    let mut blocks = Vec::new();
    for chunk in chars.chunks(chunk_size) {
        let mut bytes = Vec::with_capacity(chunk_size);
        for pair in chunk.chunks(2) {
            bytes.push((pair[0] & 0xff) as u8);
            bytes.push((pair.get(1).copied().unwrap_or(0) & 0xff) as u8);
        }
        while bytes.len() < chunk_size {
            bytes.push(0);
        }
        let block = PortalBigUint::from_little_endian_bytes(&bytes);
        if block.compare(&modulus) != std::cmp::Ordering::Less {
            return Err("校园网密码过长，无法按门户 RSA 规则加密。".to_owned());
        }
        blocks.push(block.pow_mod(exponent, &modulus).to_hex_words());
    }
    if blocks.is_empty() {
        return Ok(String::new());
    }
    Ok(blocks.join(" "))
}

fn portal_json_result(body: &str) -> Option<(bool, String)> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let result = json
        .get("result")
        .or_else(|| json.get("code"))
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    let message = json
        .get("message")
        .or_else(|| json.get("msg"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    if result.is_empty() {
        return None;
    }
    let success = matches!(result.as_str(), "success" | "ok" | "1");
    let message = if message.is_empty() {
        if success {
            "校园网认证成功。".to_owned()
        } else {
            format!("门户认证未成功（{result}）。")
        }
    } else {
        message
    };
    Some((success, message))
}

fn online_response_matches_session(json: &serde_json::Value, requested_index: &str) -> bool {
    if json.get("result").and_then(|value| value.as_str()) != Some("success") {
        return false;
    }
    let returned = json
        .get("userIndex")
        .and_then(|value| value.as_str())
        .filter(|index| !index.trim().is_empty());
    if requested_index.is_empty() {
        return returned.is_some();
    }
    returned.is_none_or(|index| index == requested_index)
}

struct PortalOnlineSession {
    online: bool,
    waiting: bool,
    offline: bool,
    detail: String,
    user_index: String,
    user_ip: Option<String>,
    operator: Option<Operator>,
}

fn online_operator(json: &serde_json::Value) -> Option<Operator> {
    ["realServiceName", "service", "serviceName", "serviceShowName"].into_iter().find_map(|key| {
        let value = json.get(key)?.as_str()?.trim();
        Operator::ALL.into_iter().find(|operator| operator.matches(value, value))
    })
}

fn portal_online_status_once(
    client: &Client,
    page_url: &Url,
) -> Result<PortalOnlineSession, String> {
    portal_online_status_bounded(client, page_url, Duration::from_secs(4))
}

// The portal itself retries `wait`: user information can be incomplete for a
// moment even while this session remains online. Keep the same client and index
// for these read-only retries; never retry authentication or an explicit failure.
fn monitor_online_status(client: &Client, page_url: &Url, guard: &OperationGuard) -> Result<PortalOnlineSession, String> {
    read_status_with_wait_retry(guard, |timeout| portal_online_status_bounded(client, page_url, timeout))
}

fn read_status_with_wait_retry(
    guard: &OperationGuard,
    mut read: impl FnMut(Duration) -> Result<PortalOnlineSession, String>,
) -> Result<PortalOnlineSession, String> {
    guard.check()?;
    let budget = guard.remaining().unwrap_or(Duration::from_secs(4)).min(Duration::from_secs(4));
    if budget.is_zero() { return Err("状态查询已超时。".into()); }
    let deadline = Instant::now() + budget;
    let mut session = read(budget)?;
    guard.check()?;
    for delay in [Duration::from_millis(250), Duration::from_millis(750)] {
        if !session.waiting || deadline.saturating_duration_since(Instant::now()) <= delay {
            break;
        }
        let wake = Instant::now() + delay;
        while Instant::now() < wake {
            guard.check()?;
            thread::sleep(wake.saturating_duration_since(Instant::now()).min(Duration::from_millis(25)));
        }
        guard.check()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() { break; }
        session = read(remaining)?;
        guard.check()?;
    }
    Ok(session)
}

fn portal_online_status_bounded(client: &Client, page_url: &Url, timeout: Duration) -> Result<PortalOnlineSession, String> {
    let _timing = auth_trace::phase("online_status");
    let url = portal_endpoint(page_url, "getOnlineUserInfo")?;
    let user_index = page_url
        .query_pairs()
        .find(|(key, _)| key == "userIndex")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let response = client
        .post(url)
        .timeout(timeout.min(Duration::from_secs(4)))
        .header(reqwest::header::REFERER, page_url.as_str())
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded; charset=UTF-8",
        )
        .body(format!("userIndex={}", uri_component_encode(&user_index)))
        .send_traced("online_status")
        .map_err(|error| format!("在线查询连接失败：{}", error.without_url()))?;
    let status = response.status();
    let body = bounded_portal_text(response, 64 * 1024)?;
    if !status.is_success() {
        return Err(format!(
            "在线查询返回 HTTP {}，认证状态未知。",
            status.as_u16()
        ));
    }
    let json: serde_json::Value = serde_json::from_str(body.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("在线查询返回 HTTP {status}，JSON 解析失败：{error}"))?;
    let result = json
        .get("result")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let has_index = json
        .get("userIndex")
        .is_some_and(|value| !value.as_str().unwrap_or_default().is_empty());
    let online = status.is_success() && online_response_matches_session(&json, &user_index);
    let result_label = if result.is_empty() {
        if json.get("result").is_some() {
            "非字符串或空值"
        } else {
            "缺失"
        }
    } else if result.len() <= 32
        && result
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        result
    } else {
        "其他"
    };
    let session_index = if online {
        json.get("userIndex")
            .and_then(|value| value.as_str())
            .filter(|index| !index.trim().is_empty())
            .unwrap_or(&user_index)
            .to_owned()
    } else {
        String::new()
    };
    Ok(PortalOnlineSession {
        online,
        waiting: status.is_success() && result == "wait",
        offline: status.is_success() && result == "fail",
        detail: format!("HTTP {status}，result={result_label}，响应包含会话标识={has_index}"),
        user_index: session_index,
        operator: online_operator(&json),
        user_ip: json
            .get("userIp")
            .and_then(|value| value.as_str())
            .filter(|ip| !ip.trim().is_empty())
            .map(str::to_owned),
    })
}

fn portal_online_session(client: &Client, page_url: &Url) -> Result<PortalOnlineSession, String> {
    // success.jsp retries result=wait with 1s, 2s, 3s, 4s delays. This only
    // refreshes session information; no login request is resubmitted.
    for attempt in 1..=5 {
        let session = portal_online_status_once(client, page_url)?;
        if !session.waiting {
            return Ok(session);
        }
        if attempt < 5 {
            thread::sleep(Duration::from_secs(attempt));
        }
    }
    Err("门户在线信息仍在同步（result=wait），请稍后重试；认证请求未重复提交。".to_owned())
}

fn portal_online_status(client: &Client, page_url: &Url) -> Result<(bool, String), String> {
    let session = portal_online_session(client, page_url)?;
    Ok((session.online, session.detail))
}

fn portal_is_online(client: &Client, page_url: &Url) -> bool {
    portal_online_status(client, page_url).is_ok_and(|(online, _)| online)
}

fn campus_portal_client() -> Result<(Client, WifiInterface), String> {
    let interface = wifi_interface()?;
    let client = campus_client_for_interface(&interface, false)?;
    Ok((client, interface))
}

fn campus_client_for_interface(interface: &WifiInterface, monitor: bool) -> Result<Client, String> {
    let client = Client::builder()
        .cookie_store(true)
        .no_proxy()
        .local_address(IpAddr::V4(interface.address))
        // The campus interception appliance drops requests with lowercase
        // `host:` before returning its bootstrap login redirect. Match browser
        // HTTP/1.1 wire casing for every discovery and credential request.
        .http1_only()
        .http1_title_case_headers()
        .connect_timeout(Duration::from_secs(if monitor { 3 } else { 8 }))
        .timeout(Duration::from_secs(if monitor { 4 } else { 12 }))
        .pool_max_idle_per_host(1)
        .pool_idle_timeout(Duration::from_secs(40))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/132.0 Safari/537.36")
        // Discovery follows GET redirects explicitly. Credential POSTs do not
        // follow redirects to any bootstrap/external destination.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("初始化门户会话失败：{}", error.without_url()))?;
    Ok(client)
}

fn bounded_portal_text(
    response: reqwest::blocking::Response,
    limit: usize,
) -> Result<String, String> {
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("").to_owned();
    let bytes = bounded_portal_bytes(response, limit)?;
    Ok(decode_portal_bytes(&bytes, &content_type))
}
fn decode_portal_bytes(bytes: &[u8], content_type: &str) -> String {
    let mime = content_type.parse::<mime::Mime>().ok();
    let encoding = mime.as_ref().and_then(|value| value.get_param("charset"))
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_str().as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}
fn bounded_portal_bytes(response: impl std::io::Read, limit: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::with_capacity(4096);
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取门户响应失败或超时。".to_owned())?;
    if bytes.len() > limit {
        return Err("门户响应超出限制，已停止读取。".to_owned());
    }
    Ok(bytes)
}

fn load_portal_page(client: &Client) -> Result<(Url, String), String> {
    load_portal_page_guarded(client, None)
}

fn load_portal_page_guarded(
    client: &Client,
    guard: Option<&OperationGuard>,
) -> Result<(Url, String), String> {
    let _timing = auth_trace::phase("discover_login");
    let deadline = Instant::now() + Duration::from_secs(24);
    let mut current_url = Url::parse("http://10.10.9.9/").map_err(|error| error.to_string())?;
    let mut visited = Vec::new();
    for _ in 0..10 {
        if let Some(guard) = guard {
            guard.check()?;
        }
        if visited.contains(&current_url) {
            return Err("校园网门户出现循环跳转，请稍后重试。".to_owned());
        }
        visited.push(current_url.clone());
        let mut request = client
            .get(current_url.clone())
            .header(reqwest::header::CACHE_CONTROL, "no-cache, no-store")
            .header(reqwest::header::PRAGMA, "no-cache");
        if guard.is_some() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("认证发现超时，将稍后重试。".to_owned());
            }
            request = request.timeout(remaining.min(Duration::from_secs(4)));
        }
        let response = request
            .send_traced("discovery")
            .map_err(|error| format!("无法通过校园 Wi-Fi 打开门户：{}", error.without_url()))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "门户跳转未返回 Location 地址。".to_owned())?;
            let next = current_url
                .join(location)
                .map_err(|_| "门户返回了无效的跳转地址。".to_owned())?;
            if next.scheme() != "http" || !next.username().is_empty() || next.password().is_some() {
                return Err("门户返回了不支持的认证引导地址。".to_owned());
            }
            current_url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("门户页面返回 HTTP {}。", response.status()));
        }
        let page_html = bounded_portal_text(response, 256 * 1024)?;
        if let Ok(page_url) = portal_page_url(&current_url) {
            return Ok((page_url, page_html));
        }
        if let Some(next) = portal_script_redirect(&current_url, &page_html) {
            current_url = next;
            continue;
        }
        return Err(portal_page_url(&current_url).unwrap_err());
    }
    Err("校园网门户重定向次数过多。".to_owned())
}

/// Read-only diagnostic entry point. It neither reads saved credentials nor
/// calls the login/logout methods. URLs are reported without query values.
pub fn portal_diagnostics() -> Result<Vec<String>, String> {
    let (client, interface) = campus_portal_client()?;
    let mut report = vec![format!(
        "认证网卡：{}（ifIndex={}，IPv4={}）",
        interface.name, interface.index, interface.address
    )];
    let (page_url, _) = load_portal_page(&client)?;
    report.push(format!(
        "门户页面：{}{}",
        page_url.host_str().unwrap_or_default(),
        page_url.path()
    ));
    let keys: Vec<_> = page_url
        .query_pairs()
        .map(|(key, _)| key.into_owned())
        .collect();
    report.push(format!("查询参数名：{}", keys.join(", ")));
    let (online, detail) = match portal_online_status(&client, &page_url) {
        Ok(check) => check,
        Err(error) => (false, error),
    };
    report.push(format!("在线查询详情：{detail}"));
    report.push(format!(
        "门户在线状态：{}",
        if online {
            "已认证"
        } else {
            "未确认在线"
        }
    ));
    if !online && page_url.path().ends_with("/index.jsp") {
        report.push(
            match validate_login_page_address(&page_url, interface.address) {
                Ok(()) => {
                    "登录页网关参数完整；明文地址冲突检查通过，支持网关编码地址参数。".to_owned()
                }
                Err(error) => error,
            },
        );
    }
    Ok(report)
}

#[derive(Clone, Copy, Debug)]
pub enum CampusPortalState {
    Online,
    Offline,
    NotCampus,
    WaitingAddress,
    Verifying,
}

pub struct CampusConnectionCheck {
    pub ssid: String,
    pub ip: String,
    pub state: CampusPortalState,
    pub session: Option<crate::online_session::SessionEvidence>,
}

/// Read-only check of the current physical Wi-Fi and its portal. A complete
/// freshly discovered gateway login URL plus an explicit offline API reply
/// supplies evidence of being unauthenticated. Encoded gateway parameters are
/// kept opaque; only actual plaintext IPv4 addresses can be compared directly.
/// Timeouts, unknown pages and API errors never count as an offline sample.
#[derive(Default)]
pub struct CampusProbe {
    cached: Option<ProbeSession>,
    prepared: Option<AuthAttemptContext>,
}

/// Consumed by one authentication only; never reused by periodic monitoring.
pub struct AuthAttemptContext {
    client: Client,
    interface: WifiInterface,
    ssid: String,
    operator: Operator,
    wifi_generation: u64,
    guard: OperationGuard,
    created: Instant,
    page_url: Url,
    page_html: String,
}

impl AuthAttemptContext {
    fn valid(&self, ssid: Option<&str>, operator: Operator, fingerprint: (u32, Ipv4Addr), generation: u64) -> bool {
        self.guard.check().is_ok() && self.created.elapsed() < Duration::from_secs(15)
            && ssid == Some(self.ssid.as_str()) && operator == self.operator
            && fingerprint == (self.interface.index, self.interface.address)
            && generation == self.wifi_generation
    }
}

pub enum AuthOutcome {
    AlreadyOnline(crate::online_session::SessionEvidence),
    Accepted,
    Rejected(String),
    Unknown(String),
}

struct ProbeSession {
    ssid: String,
    index: u32,
    address: Ipv4Addr,
    created: Instant,
    client: Client,
}

impl CampusProbe {
    pub fn prepare_auth(&mut self, guard: &OperationGuard, operator: Operator) -> Result<CampusConnectionCheck, String> {
        self.prepared = None;
        let result = self.check_inner(guard, true, Some(operator));
        self.complete_check(result)
    }

    pub fn take_auth_context(&mut self) -> Option<AuthAttemptContext> { self.prepared.take() }

    pub fn check(
        &mut self,
        guard: &OperationGuard,
        campus: bool,
    ) -> Result<CampusConnectionCheck, String> {
        let result = self.check_inner(guard, campus, None);
        self.complete_check(result)
    }

    fn complete_check(&mut self, result: Result<CampusConnectionCheck, String>) -> Result<CampusConnectionCheck, String> {
        // Only an unresolved wait reaches here after the short monitoring retry.
        // Refresh the HTTP session on the next scheduled read as a fallback.
        // Do not retry credentials or infer that an ambiguous reply is offline.
        if result.is_err() || result.as_ref().is_ok_and(|check| matches!(check.state, CampusPortalState::Verifying)) {
            self.cached = None;
            self.prepared = None;
        }
        result
    }

    fn check_inner(&mut self, guard: &OperationGuard, campus: bool, prepare: Option<Operator>) -> Result<CampusConnectionCheck, String> {
        guard.check()?;
        let connection_generation = crate::wifi::connection_generation();
        let interface = wifi_interface().ok();
        let index = interface.as_ref().map(|interface| interface.index);
        #[cfg(windows)]
        let ssid = crate::wifi::cached_current_ssid(index)?;
        #[cfg(not(windows))]
        let ssid = connected_ssid();
        let ip = if ssid.is_empty() {
            "—".to_owned()
        } else {
            interface
                .as_ref()
                .map_or_else(|| "—".to_owned(), |interface| interface.address.to_string())
        };
        let mut online_evidence = None;
        let state =
            if ssid.is_empty() || !campus || is_enterprise_campus_ssid(&ssid) {
                self.cached = None;
                CampusPortalState::NotCampus
            } else if interface.is_none() {
                self.cached = None;
                CampusPortalState::WaitingAddress
            } else {
                let interface = interface.as_ref().unwrap();
                if self.cached.as_ref().is_none_or(|session| {
                    session.ssid != ssid
                        || session.index != interface.index
                        || session.address != interface.address
                        || session.created.elapsed() > Duration::from_secs(600)
                }) {
                    self.cached = Some(ProbeSession {
                        ssid: ssid.clone(),
                        index: interface.index,
                        address: interface.address,
                        created: Instant::now(),
                        client: campus_client_for_interface(&interface, true)?,
                    });
                }
                let client = &self.cached.as_ref().unwrap().client;
                let created = Instant::now();
                let (page_url, page_html) = if prepare.is_some() {
                    load_portal_page_guarded(client, Some(guard))?
                } else { (load_monitor_page(client, guard)?, String::new()) };
                guard.check()?;
                // Query session evidence before treating a bootstrap login page as
                // offline. Some gateways keep serving this page after browser login.
                guard.check()?;
                let session = if prepare.is_some() {
                    portal_online_status_bounded(client, &page_url, guard.remaining().unwrap_or(Duration::from_secs(4)))
                } else {
                    monitor_online_status(client, &page_url, guard)
                };
                if session.as_ref().is_ok_and(|session| session.online) {
                    let session = session.as_ref().unwrap();
                    if session
                        .user_ip
                        .as_deref()
                        .is_some_and(|ip| ip.parse::<Ipv4Addr>().ok() != Some(interface.address))
                    {
                        return Err("门户在线会话与当前 Wi-Fi 地址不一致，状态待确认。".into());
                    }
                    online_evidence = Some(crate::online_session::SessionEvidence { id: session.user_index.clone(), operator: session.operator });
                    CampusPortalState::Online
                } else if session.as_ref().is_ok_and(|session| session.waiting) {
                    CampusPortalState::Verifying
                } else if validate_portal_context(&page_url).is_ok() {
                    // A failed status query is unknown, never evidence to send credentials.
                    if !session?.offline {
                        return Err("在线查询未返回明确结果，保持认证状态待确认。".into());
                    }
                    validate_login_page_address(&page_url, interface.address)?;
                    if let Some(operator) = prepare {
                        self.prepared = Some(AuthAttemptContext {
                            client: client.clone(),
                            interface: WifiInterface { name: interface.name.clone(), index: interface.index, address: interface.address },
                            ssid: ssid.clone(), operator, wifi_generation: connection_generation,
                            guard: guard.clone(), created, page_url, page_html,
                        });
                    }
                    CampusPortalState::Offline
                } else {
                    let session = session?;
                    if session.online {
                        if session.user_ip.as_deref().is_some_and(|ip| {
                            ip.parse::<Ipv4Addr>().ok() != Some(interface.address)
                        }) {
                            return Err("门户在线会话与当前 Wi-Fi 地址不一致，状态待确认。".into());
                        }
                        online_evidence = Some(crate::online_session::SessionEvidence { id: session.user_index.clone(), operator: session.operator });
                        CampusPortalState::Online
                    } else if session.waiting {
                        CampusPortalState::Verifying
                    } else {
                        return Err("门户尚未确认在线状态，将稍后重新检测。".to_owned());
                    }
                }
            };
        guard.check()?;
        #[cfg(windows)]
        let latest_ssid = crate::wifi::cached_current_ssid(index)?;
        #[cfg(not(windows))]
        let latest_ssid = connected_ssid();
        if crate::wifi::connection_generation() != connection_generation
            || latest_ssid != ssid
            || interface_fingerprint()
                != interface
                    .as_ref()
                    .map(|interface| (interface.index, interface.address))
        {
            self.cached = None;
            return Err("检测期间 Wi-Fi 已变化，将稍后重新检测。".to_owned());
        }
        Ok(CampusConnectionCheck { ssid, ip, state, session: online_evidence })
    }
}

fn interface_fingerprint() -> Option<(u32, Ipv4Addr)> {
    wifi_interface()
        .ok()
        .map(|interface| (interface.index, interface.address))
}

fn portal_request_error(error: reqwest::Error) -> String {
    use std::error::Error;
    let error = error.without_url();
    let category = if error.is_timeout() {
        "请求超时"
    } else if error.is_connect() {
        "连接失败"
    } else {
        "请求失败"
    };
    let mut cause = error.source();
    while let Some(source) = cause {
        if source.to_string().contains("connection closed before message completed") {
            return "校园 Wi-Fi 认证引导连接被提前关闭，未收到完整 HTTP 响应。".into();
        }
        if let Some(io) = source.downcast_ref::<std::io::Error>() {
            return match io.raw_os_error() {
                Some(code) => format!("校园 Wi-Fi 访问认证网关失败：{category}，系统错误 {code}（{io}）。"),
                None => format!("校园 Wi-Fi 访问认证网关失败：{category}（{io}）。"),
            };
        }
        cause = source.source();
    }
    format!("校园 Wi-Fi 访问认证网关失败：{category}。")
}

/// Read only the small bootstrap pages. Once the real portal URL is known,
/// its HTML, CSS, JS and images are unnecessary for detecting authentication.
fn load_monitor_page(client: &Client, guard: &OperationGuard) -> Result<Url, String> {
    let _timing = auth_trace::phase("discover_monitor");
    let deadline = Instant::now() + guard.remaining().unwrap_or(Duration::from_secs(12)).min(Duration::from_secs(12));
    let mut current = Url::parse("http://10.10.9.9/").unwrap();
    let mut visited = Vec::new();
    for _ in 0..10 {
        guard.check()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("门户检测超时，将在后台重试。".to_owned());
        }
        if visited.contains(&current) {
            return Err("门户暂时循环跳转。".to_owned());
        }
        visited.push(current.clone());
        let response = client
            .get(current.clone())
            .header(reqwest::header::CACHE_CONTROL, "no-cache, no-store")
            .header(reqwest::header::PRAGMA, "no-cache")
            .timeout(remaining.min(Duration::from_secs(4)))
            .send_traced("discovery")
            .map_err(|error| format!("{}（目标 {}，第 {} 跳）", portal_request_error(error), current.host_str().unwrap_or("未知"), visited.len()))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("门户跳转地址缺失。")?;
            let next = current
                .join(location)
                .map_err(|_| "门户跳转地址无效。".to_owned())?;
            if next.scheme() != "http" || !next.username().is_empty() || next.password().is_some() {
                return Err("门户跳转协议不受支持。".to_owned());
            }
            current = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!(
                "门户检测返回 HTTP {}。",
                response.status().as_u16()
            ));
        }
        if let Ok(page) = portal_page_url(&current) {
            return Ok(page);
        }
        let html = bounded_portal_text(response, 256 * 1024)?;
        current = portal_script_redirect(&current, &html).ok_or("未取得有效校园门户地址。")?;
    }
    Err("门户检测跳转次数过多。".to_owned())
}

fn portal_logout_confirmed(client: &Client, attempts: u64) -> bool {
    // Re-discover through the gateway rather than querying the old userIndex:
    // the success page may retain cached session data after a successful logout.
    for attempt in 0..attempts {
        if attempt > 0 {
            thread::sleep(Duration::from_secs(attempt));
        }
        if load_portal_page(client).is_ok_and(|(url, _)| validate_portal_context(&url).is_ok()
            && portal_online_session(client, &url).is_ok_and(|session| session.offline)) {
            return true;
        }
    }
    false
}

/// Log out only the current Wi-Fi portal session. No credentials are needed,
/// and WLAN association and Windows proxy settings are not changed.
#[cfg(test)]
pub fn benchmark_logout_online_session() -> Result<(), String> {
    let (client, interface) = campus_portal_client()?;
    let (page, _) = load_portal_page(&client)?;
    let session = portal_online_session(&client, &page)?;
    if session.offline { return Ok(()); }
    if !session.online || session.user_index.is_empty()
        || session.user_ip.as_deref().is_some_and(|ip| ip.parse::<Ipv4Addr>().ok() != Some(interface.address)) {
        return Err("benchmark requires a matching online session before logout".into());
    }
    let reply = client.post(portal_endpoint(&page, "logout")?)
        .header(reqwest::header::REFERER, page.as_str())
        .header(reqwest::header::ORIGIN, "http://10.10.9.9")
        .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
        .body(format!("userIndex={}", uri_component_encode(&session.user_index)))
        .send().map_err(|_| "benchmark logout transport failure")?;
    let body = reply.text().map_err(|_| "benchmark logout body failure")?;
    if !portal_json_result(&body).is_some_and(|result| result.0) {
        return Err("benchmark logout not accepted; response omitted".into());
    }
    Ok(())
}

pub fn portal_logout() -> Result<(bool, String), String> {
    let (client, _) = campus_portal_client()?;
    let (page_url, _) = load_portal_page(&client)?;
    let session = portal_online_session(&client, &page_url)?;
    if session.offline && validate_portal_context(&page_url).is_ok() {
        return Ok((
            true,
            "当前校园网已处于下线状态，Wi-Fi 连接保持不变。".to_owned(),
        ));
    }
    if !session.online || session.user_index.trim().is_empty() {
        return Ok((
            false,
            "未取得当前在线会话，未提交下线请求。请打开门户查看状态。".to_owned(),
        ));
    }
    let endpoint = portal_endpoint(&page_url, "logout")?;
    let reply = client
        .post(endpoint)
        .header(reqwest::header::REFERER, page_url.as_str())
        .header(reqwest::header::ORIGIN, "http://10.10.9.9")
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded; charset=UTF-8",
        )
        .body(format!(
            "userIndex={}",
            uri_component_encode(&session.user_index)
        ))
        .send()
        .map_err(|error| format!("校园网下线请求连接失败：{}", error.without_url()))
        .and_then(|response| {
            let status = response.status();
            bounded_portal_text(response, 64 * 1024).map(|body| (status, body))
        });
    let accepted = reply.as_ref().is_ok_and(|(status, body)| {
        status.is_success()
            && serde_json::from_str::<serde_json::Value>(body.trim_start_matches('\u{feff}'))
                .is_ok_and(|json| {
                    json.get("result").and_then(|value| value.as_str()) == Some("success")
                })
    });
    // Send exactly one logout POST. Even a lost response can mean that the
    // gateway logged us out, so confirm using read-only discovery afterwards.
    if portal_logout_confirmed(&client, if accepted { 3 } else { 1 }) {
        return Ok((
            true,
            "校园网已下线，已确认门户返回登录页；Wi-Fi 连接保持不变。".to_owned(),
        ));
    }
    match reply {
        Ok(_) if accepted => Ok((
            false,
            "门户已接受下线请求，但尚未确认下线。自动认证已暂停，请稍后重试或打开门户查看。"
                .to_owned(),
        )),
        Ok((status, _)) if !status.is_success() => {
            Err(format!("下线请求返回 HTTP {status}，尚未确认下线。"))
        }
        Ok(_) => Ok((
            false,
            "门户未确认下线成功。自动认证已暂停，请打开门户查看在线状态。".to_owned(),
        )),
        Err(error) => Err(error),
    }
}

pub fn portal_login(username: &str, password: &str) -> Result<(bool, String), String> {
    portal_login_for_connection(username, password, Operator::default(), None, None)
}

pub fn portal_login_guarded(
    expected_ssid: &str,
    username: &str,
    password: &str,
    operator: Operator,
    guard: &OperationGuard,
) -> Result<(bool, String), String> {
    if expected_ssid.is_empty() {
        return Err("请先连接校园 Wi-Fi。".to_owned());
    }
    portal_login_for_connection(username, password, operator, Some(expected_ssid), Some(guard))
}

pub fn portal_reconnect(
    expected_ssid: &str,
    username: &str,
    password: &str,
    operator: Operator,
    guard: &OperationGuard,
) -> Result<(bool, String), String> {
    if !supports_campus_portal(expected_ssid) {
        return Err("当前不是受支持的校园 Wi-Fi，自动重连已取消。".to_owned());
    }
    portal_login_for_connection(username, password, operator, Some(expected_ssid), Some(guard))
}

fn ensure_reconnect_network(expected_ssid: Option<&str>) -> Result<(), String> {
    let current=connected_ssid();
    if is_enterprise_campus_ssid(&current) { return Err("ShuWlan-1X 由 Windows Wi-Fi 进行 802.1X 认证，无需网页认证。".into()); }
    if expected_ssid.is_some_and(|expected| current != expected) {
        return Err("Wi-Fi 已变化，已取消本次自动重连。".to_owned());
    }
    Ok(())
}

fn portal_login_for_connection(
    username: &str,
    password: &str,
    operator: Operator,
    expected_ssid: Option<&str>,
    guard: Option<&OperationGuard>,
) -> Result<(bool, String), String> {
    portal_login_attempt(username, password, operator, expected_ssid, guard, None).map(|outcome| match outcome {
        AuthOutcome::AlreadyOnline(_) => (true, "校园网已经认证，无需重复登录。".into()),
        AuthOutcome::Accepted => (true, "门户已接受认证，等待确认在线。".into()),
        AuthOutcome::Rejected(message) | AuthOutcome::Unknown(message) => (false, message),
    })
}

pub fn portal_login_prepared(expected_ssid: &str, username: &str, password: &str, operator: Operator,
    guard: &OperationGuard, prepared: Option<AuthAttemptContext>) -> Result<AuthOutcome, String> {
    portal_login_attempt(username, password, operator, Some(expected_ssid), Some(guard), prepared)
}

fn portal_login_attempt(username: &str, password: &str, operator: Operator, expected_ssid: Option<&str>,
    guard: Option<&OperationGuard>, prepared: Option<AuthAttemptContext>) -> Result<AuthOutcome, String> {
    if username.trim().is_empty() || password.is_empty() {
        return Ok(AuthOutcome::Rejected("请先填写校园网账号和密码。".into()));
    }
    ensure_reconnect_network(expected_ssid)?;
    if let Some(guard) = guard {
        guard.check()?;
    }
    let prepared = if let Some(context) = prepared {
        context.guard.check()?;
        if expected_ssid != Some(context.ssid.as_str()) || operator != context.operator
            || interface_fingerprint() != Some((context.interface.index, context.interface.address))
            || crate::wifi::connection_generation() != context.wifi_generation {
            return Err("预检查后 Wi-Fi 或运营商已变化，已取消本次认证。".into());
        }
        // Expiry alone permits one fresh discovery. Identity/revision changes abort.
        if context.created.elapsed() < Duration::from_secs(15) { Some(context) } else { None }
    } else { None };
    let (client, interface, prepared_page, context_created, wifi_generation) = if let Some(context) = prepared {
        (context.client, context.interface, Some((context.page_url, context.page_html)), context.created, context.wifi_generation)
    } else if guard.is_some() {
        let interface = wifi_interface()?;
        (campus_client_for_interface(&interface, true)?, interface, None, Instant::now(), crate::wifi::connection_generation())
    } else {
        let (client, interface) = campus_portal_client()?;
        (client, interface, None, Instant::now(), crate::wifi::connection_generation())
    };
    let fingerprint = (interface.index, interface.address);
    #[cfg(windows)]
    if let Some(expected) = expected_ssid {
        if crate::wifi::current_ssid(Some(interface.index))? != expected {
            return Err("认证网卡与当前校园 Wi-Fi 不一致，已停止提交。".to_owned());
        }
    }
    let reused_preflight = prepared_page.is_some();
    let (page_url, page_html) = match prepared_page { Some(page) => page, None => load_portal_page_guarded(&client, guard)? };
    if let Some(guard) = guard {
        guard.check()?;
    }
    // A reused context already has explicit offline evidence. Keep the final
    // fresh session check immediately before credentials instead of querying twice here.
    if !reused_preflight {
    let session = portal_online_session(&client, &page_url)?;
    if session.online {
        if session
            .user_ip
            .as_deref()
            .is_some_and(|ip| ip.parse::<Ipv4Addr>().ok() != Some(interface.address))
        {
            return Err("门户在线会话与当前 Wi-Fi 地址不一致，状态待确认。".into());
        }
        if let Some(guard) = guard {
            guard.check()?;
        }
        ensure_reconnect_network(expected_ssid)?;
        if interface_fingerprint() != Some(fingerprint) {
            return Err("检测期间 Wi-Fi 地址已变化，将重新检测认证状态。".into());
        }
        return Ok(AuthOutcome::AlreadyOnline(crate::online_session::SessionEvidence { id: session.user_index, operator: session.operator }));
    }
    if !session.offline {
        return Err("门户在线查询未明确确认下线，未提交账号密码。".into());
    }
    }
    validate_login_page_address(&page_url, interface.address)?;
    let query_string = portal_query_string(&page_url, &page_html);

    let page_info_url = portal_endpoint(&page_url, "pageInfo")?;
    let page_info_timing = auth_trace::phase("page_info");
    let page_info_body = format!("queryString={}", uri_component_encode(&query_string));
    let page_info_response = client
        .post(page_info_url)
        .header(reqwest::header::REFERER, page_url.as_str())
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded; charset=UTF-8",
        )
        .body(page_info_body)
        .send_traced("page_info")
        .map_err(|error| format!("读取认证页面参数失败：{}", error.without_url()))?;
    let page_info_status = page_info_response.status();
    let page_info_response_body = bounded_portal_text(page_info_response, 64 * 1024)?;
    let page_info = portal_page_info(&page_info_response_body, &page_html);
    if !page_info_status.is_success() && page_info.public_key_modulus.is_empty() {
        return Err(format!("门户页面参数读取失败（HTTP {page_info_status}）。"));
    }
    if page_info.password_encrypt
        && (page_info.public_key_exponent.is_empty() || page_info.public_key_modulus.is_empty())
    {
        return Err("门户要求加密密码，但没有返回 RSA 公钥。请刷新门户后重试。".into());
    }

    let mut services_url = portal_endpoint(&page_url, "getServices")?;
    drop(page_info_timing);
    let service_timing = auth_trace::phase("services");
    if let Some(guard) = guard {
        guard.check()?;
    }
    if !query_string.is_empty() {
        services_url
            .query_pairs_mut()
            .append_pair("queryString", &query_string);
    }
    let service = service_from_html(&page_html, operator).or_else(|| client
        .post(services_url)
        .header(reqwest::header::REFERER, page_url.as_str())
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded; charset=UTF-8",
        )
        .body("")
        .send_traced("services")
        .ok()
        .and_then(|response| bounded_portal_text(response, 64 * 1024).ok())
        .and_then(|body| service_from_response(&body, operator)))
        .ok_or_else(|| format!("门户未提供所选运营商“{}”，请检查选择或打开认证网页确认。", operator.label()))?;

    let mac = page_url
        .query_pairs()
        .find(|(key, _)| key.eq_ignore_ascii_case("mac"))
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "111111111".to_owned());
    drop(service_timing);
    let encryption_timing = auth_trace::phase("encryption");
    let (password_for_portal, encrypt_flag) =
        if page_info.password_encrypt && password.encode_utf16().count() < 150 {
            let cleartext = format!("{password}>{mac}");
            (
                portal_rsa_encrypt(
                    &cleartext,
                    &page_info.public_key_exponent,
                    &page_info.public_key_modulus,
                )?,
                "true",
            )
        } else {
            (
                password.to_owned(),
                if page_info.password_encrypt {
                    "true"
                } else {
                    "false"
                },
            )
        };
    // AuthInterFace.js assembles an already URI-encoded body string and sends
    // it as-is. Encoding each value twice reproduces doauthen() exactly.
    drop(encryption_timing);
    let login_body = portal_login_body(username, &password_for_portal, &service, &query_string, encrypt_flag);
    let login_url = portal_endpoint(&page_url, "login")?;
    let origin = format!(
        "{}://{}",
        page_url.scheme(),
        page_url.host_str().unwrap_or("10.10.9.9")
    );
    let session = portal_online_status_once(&client, &page_url)?;
    if session.online {
        if session.user_ip.as_deref().is_some_and(|ip| ip.parse::<Ipv4Addr>().ok() != Some(interface.address)) {
            return Err("门户在线会话与当前 Wi-Fi 地址不一致，已停止提交。".into());
        }
        ensure_reconnect_network(expected_ssid)?;
        if let Some(guard) = guard { guard.check()?; }
        if interface_fingerprint() != Some(fingerprint) || crate::wifi::connection_generation() != wifi_generation {
            return Err("Wi-Fi 已变化，已取消旧认证。".into());
        }
        return Ok(AuthOutcome::AlreadyOnline(crate::online_session::SessionEvidence { id: session.user_index, operator: session.operator }));
    }
    if !session.offline { return Err("提交前门户未明确确认下线，未发送账号密码。".into()); }
    // A monitor result can become stale while pageInfo/RSA are being prepared.
    // Check the physical SSID again immediately before posting credentials.
    ensure_reconnect_network(expected_ssid)?;
    if context_created.elapsed() >= Duration::from_secs(15)
        || crate::wifi::connection_generation() != wifi_generation {
        return Err("本次门户上下文已过期或连接已变化，请重新认证。".into());
    }
    if let Some(guard) = guard {
        guard.check()?;
        #[cfg(windows)]
        if expected_ssid.is_some_and(|expected| {
            crate::wifi::current_ssid(Some(interface.index))
                .ok()
                .as_deref()
                != Some(expected)
        }) {
            return Err("校园 Wi-Fi 已变化，已取消旧认证。".to_owned());
        }
        if interface_fingerprint() != Some(fingerprint) {
            return Err("Wi-Fi 地址已变化，已取消旧认证。".to_owned());
        }
    }
    let submit_timing = auth_trace::phase("submit");
    let outcome = send_login_once(client
        .post(login_url)
        .header(reqwest::header::REFERER, page_url.as_str())
        .header(reqwest::header::ORIGIN, origin)
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded; charset=UTF-8",
        )
        .body(login_body));
    drop(submit_timing);
    Ok(outcome)
}

fn send_login_once(request: reqwest::blocking::RequestBuilder) -> AuthOutcome {
    let login_response = match request.send_traced("login") {
            Ok(response) => response,
            Err(_) => return AuthOutcome::Unknown("登录请求结果未确认，正在只读核对；不会重复提交。".into()),
        };
    let status = login_response.status();
    let body = match bounded_portal_text(login_response, 64 * 1024) {
        Ok(body) => body,
        Err(_) => return AuthOutcome::Unknown("读取登录结果未完成，正在只读核对；不会重复提交。".into()),
    };

    classify_auth_response(status, &body)
}

fn classify_auth_response(status: reqwest::StatusCode, body: &str) -> AuthOutcome {
    if !status.is_success() {
        return AuthOutcome::Unknown(format!("门户返回 HTTP {status}，正在只读确认结果。"));
    }
    if let Some(result) = portal_json_result(&body) {
        if result.0 {
            return AuthOutcome::Accepted;
        }
        let json: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let code = json.get("result").or_else(|| json.get("code")).map(|value| value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())).unwrap_or_default().to_ascii_lowercase();
        if matches!(code.as_str(), "fail" | "failed" | "error" | "0" | "false") { return AuthOutcome::Rejected(result.1); }
    }
    AuthOutcome::Unknown("门户未返回明确认证结果，正在只读核对；不会重复提交。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_network_type_uses_current_portal_service_not_available_service_list() {
        for operator in Operator::ALL {
            let actual = serde_json::json!({"realServiceName":operator.service_name(), "service":"unknown", "serviceList":["中国移动"]});
            assert_eq!(online_operator(&actual), Some(operator));
            assert_eq!(online_operator(&serde_json::json!({"service":operator.label()})), Some(operator));
        }
        assert_eq!(online_operator(&serde_json::json!({"serviceList":["校园网","中国移动"]})), None);
        assert_eq!(online_operator(&serde_json::json!({"service":"unknown"})), None);
    }

    #[test]
    fn auth_context_expires_and_rejects_identity_operator_or_revision_changes() {
        let monitor = crate::reconnect::ReconnectMonitor::new(false);
        let mut context = AuthAttemptContext {
            client: Client::builder().no_proxy().build().unwrap(),
            interface: WifiInterface { name: "test".into(), index: 15, address: Ipv4Addr::new(10, 1, 2, 3) },
            ssid: "Shu(ForAll)".into(), operator: Operator::Campus,
            wifi_generation: 7, guard: monitor.guard(), created: Instant::now(),
            page_url: Url::parse("http://10.10.9.9/eportal/index.jsp").unwrap(), page_html: String::new(),
        };
        let fingerprint = (15, Ipv4Addr::new(10, 1, 2, 3));
        assert!(context.valid(Some("Shu(ForAll)"), Operator::Campus, fingerprint, 7));
        assert!(!context.valid(Some("ShuOther"), Operator::Campus, fingerprint, 7));
        assert!(!context.valid(Some("Shu(ForAll)"), Operator::Mobile, fingerprint, 7));
        assert!(!context.valid(Some("Shu(ForAll)"), Operator::Campus, (16, fingerprint.1), 7));
        assert!(!context.valid(Some("Shu(ForAll)"), Operator::Campus, (15, Ipv4Addr::new(10, 1, 2, 4)), 7));
        assert!(!context.valid(Some("Shu(ForAll)"), Operator::Campus, fingerprint, 8));
        context.created -= Duration::from_secs(16);
        assert!(!context.valid(Some("Shu(ForAll)"), Operator::Campus, fingerprint, 7));
        context.created = Instant::now();
        monitor.cancellation_counter().fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        assert!(!context.valid(Some("Shu(ForAll)"), Operator::Campus, fingerprint, 7));
    }

    #[test]
    fn ambiguous_services_require_fallback_instead_of_first_match() {
        assert_eq!(service_from_html("<option value='a'>校园网</option><option value='b'>校园网</option>", Operator::Campus), None);
        let body = r#"{"serviceJson":[{"serviceName":"a","serviceShowName":"校园网"},{"serviceName":"b","serviceShowName":"校园网"}]}"#;
        assert_eq!(service_from_response(body, Operator::Campus), None);
        assert_eq!(service_from_html("<option value='shu'>校园网</option><option value='shu'>校园网</option>", Operator::Campus).as_deref(), Some("shu"));
    }

    #[test]
    fn auth_acceptance_unknown_and_permission_rejection_are_distinct() {
        let ok = reqwest::StatusCode::OK;
        assert!(matches!(classify_auth_response(ok, r#"{"result":"success"}"#), AuthOutcome::Accepted));
        assert!(matches!(classify_auth_response(ok, r#"{"result":"wait"}"#), AuthOutcome::Unknown(_)));
        assert!(matches!(classify_auth_response(ok, "<html>gateway</html>"), AuthOutcome::Unknown(_)));
        assert!(matches!(classify_auth_response(reqwest::StatusCode::BAD_GATEWAY, r#"{"result":"success"}"#), AuthOutcome::Unknown(_)));
        match classify_auth_response(ok, r#"{"result":"fail","message":"您未绑定服务对应的运营商!"}"#) {
            AuthOutcome::Rejected(message) => assert_eq!(message, "您未绑定服务对应的运营商!"),
            _ => panic!("permission rejection must retain the portal message"),
        }
    }

    #[test]
    fn transient_wait_recovers_with_same_cookie_and_no_login() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (expected_cookie, waiting) in [(false, false), (true, true), (true, false)] {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let size = socket.read(&mut buffer).unwrap();
                    assert!(size > 0 && request.len() < 16384);
                    request.extend_from_slice(&buffer[..size]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        assert!(headers.starts_with("post http://10.10.9.9/eportal/interface.do?method=getonlineuserinfo "));
                        assert_eq!(headers.contains("jsessionid=stale"), expected_cookie);
                        let length: usize = headers.lines().find_map(|line| line.strip_prefix("content-length: ")).unwrap().parse().unwrap();
                        if request.len() >= end + 4 + length {
                            assert_eq!(&request[end + 4..end + 4 + length], b"userIndex=synthetic-session");
                            break;
                        }
                    }
                }
                let body = if waiting { r#"{"result":"wait"}"# } else { r#"{"result":"success","userIndex":"synthetic-session","userIp":"127.0.0.1"}"# };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nSet-Cookie: JSESSIONID=stale; Path=/\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        // The loopback proxy intercepts every synthetic portal URL; this test
        // never reaches the campus gateway and never invokes the login endpoint.
        let make_client = || Client::builder().cookie_store(true)
            .proxy(reqwest::Proxy::http(format!("http://{address}")).unwrap())
            .timeout(Duration::from_secs(3)).build().unwrap();
        let client = make_client();
        let page = Url::parse("http://10.10.9.9/eportal/success.jsp?userIndex=synthetic-session").unwrap();
        let mut probe = CampusProbe { cached: Some(ProbeSession { ssid: "synthetic".into(), index: 0, address: Ipv4Addr::LOCALHOST, created: Instant::now(), client: client.clone() }), prepared: None };
        auth_trace::start();
        assert!(portal_online_status_once(&client, &page).unwrap().online);
        let guard = crate::reconnect::ReconnectMonitor::new(true).guard();
        assert!(monitor_online_status(&client, &page, &guard).unwrap().online);
        let check = CampusConnectionCheck { ssid: "synthetic".into(), ip: "127.0.0.1".into(), state: CampusPortalState::Online, session: None };
        assert!(matches!(probe.complete_check(Ok(check)).unwrap().state, CampusPortalState::Online));
        assert!(probe.cached.is_some(), "a transient wait must retain the HTTP session");
        assert!(probe.prepared.is_none());
        let trace = auth_trace::finish().unwrap();
        assert_eq!(trace.requests.get("online_status"), Some(&3));
        assert!(!trace.requests.contains_key("login"));
        server.join().unwrap();
    }

    fn synthetic_status(waiting: bool, online: bool, offline: bool) -> PortalOnlineSession {
        PortalOnlineSession { online, waiting, offline, detail: String::new(), user_index: String::new(), user_ip: None, operator: None }
    }

    #[test]
    fn status_retry_is_limited_to_wait_and_keeps_failure_unknown_or_offline() {
        let guard = crate::reconnect::ReconnectMonitor::new(true).guard();
        for (waiting, online, offline, expected_reads) in [(true, false, false, 3), (false, true, false, 1), (false, false, true, 1)] {
            let mut reads = 0;
            let status = read_status_with_wait_retry(&guard, |timeout| {
                assert!(timeout > Duration::ZERO && timeout <= Duration::from_secs(4));
                reads += 1;
                Ok(synthetic_status(waiting, online, offline))
            }).unwrap();
            assert_eq!(reads, expected_reads);
            assert_eq!((status.waiting, status.online, status.offline), (waiting, online, offline));
        }
        let mut reads = 0;
        let status = read_status_with_wait_retry(&guard, |_| {
            reads += 1;
            Ok(synthetic_status(reads == 1, false, reads > 1))
        }).unwrap();
        assert_eq!(reads, 2);
        assert!(status.offline && !status.online && !status.waiting);
        for fail_at in [1, 2] {
            let mut reads = 0;
            let result = read_status_with_wait_retry(&guard, |_| {
                reads += 1;
                if reads == fail_at { Err("synthetic transport error".into()) }
                else { Ok(synthetic_status(true, false, false)) }
            });
            assert_eq!(result.err().as_deref(), Some("synthetic transport error"));
            assert_eq!(reads, fail_at, "transport errors must not retry or become online");
        }
        let mut probe = CampusProbe::default();
        let check = CampusConnectionCheck { ssid: "synthetic".into(), ip: "127.0.0.1".into(), state: CampusPortalState::Verifying, session: None };
        assert!(matches!(probe.complete_check(Ok(check)).unwrap().state, CampusPortalState::Verifying));
        assert!(probe.cached.is_none() && probe.prepared.is_none());
    }

    #[test]
    fn status_wait_retry_obeys_deadline_and_cancellation_during_delay() {
        let monitor = crate::reconnect::ReconnectMonitor::new(true);
        let guard = monitor.guard().with_deadline(Instant::now() + Duration::from_millis(80));
        let started = Instant::now();
        let mut reads = 0;
        assert!(read_status_with_wait_retry(&guard, |timeout| {
            assert!(timeout <= Duration::from_millis(80));
            reads += 1;
            Ok(synthetic_status(true, false, false))
        }).unwrap().waiting);
        assert_eq!(reads, 1);
        assert!(started.elapsed() < Duration::from_millis(250));

        let guard = monitor.guard();
        let counter = monitor.cancellation_counter();
        let mut cancel = None;
        let mut reads = 0;
        let started = Instant::now();
        let result = read_status_with_wait_retry(&guard, |_| {
            reads += 1;
            let counter = counter.clone();
            cancel = Some(thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                counter.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }));
            Ok(synthetic_status(true, false, false))
        });
        cancel.unwrap().join().unwrap();
        assert!(result.is_err());
        assert_eq!(reads, 1);
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(read_status_with_wait_retry(&guard, |_| panic!("cancelled work must not issue HTTP reads")).is_err());
    }

    #[test]
    fn portal_read_limits_and_legacy_chinese_charset_are_preserved() {
        let limit = 64 * 1024;
        let mut reader = std::io::Cursor::new(vec![b'x'; limit * 4]);
        assert!(bounded_portal_bytes(&mut reader, limit).is_err());
        assert_eq!(reader.position(), (limit + 1) as u64);
        assert_eq!(bounded_portal_bytes(std::io::Cursor::new(vec![b'x'; limit]), limit).unwrap().len(), limit);
        let (gbk, _, _) = encoding_rs::GBK.encode("校园网");
        assert_eq!(decode_portal_bytes(&gbk, "text/html; charset=\"gbk\""), "校园网");
        assert_eq!(decode_portal_bytes("校园网".as_bytes(), "text/html"), "校园网");
    }

    #[test]
    fn oversized_chunked_login_reply_is_unknown_and_never_retried() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let size = socket.read(&mut buffer).unwrap();
                assert!(size > 0 && request.len() < 16384);
                request.extend_from_slice(&buffer[..size]);
            }
            assert!(request.starts_with(b"POST "));
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n10001\r\n").unwrap();
            let _ = socket.write_all(&vec![b'x'; 64 * 1024 + 1]);
            let _ = socket.write_all(b"\r\n0\r\n\r\n");
            drop(socket);
            listener.set_nonblocking(true).unwrap();
            let until = Instant::now() + Duration::from_millis(200);
            while Instant::now() < until {
                assert!(listener.accept().is_err(), "oversized login reply must not retry credentials");
                thread::sleep(Duration::from_millis(10));
            }
        });
        let client = reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap();
        auth_trace::start();
        assert!(matches!(send_login_once(client.post(format!("http://{address}/login")).body("")), AuthOutcome::Unknown(_)));
        assert_eq!(auth_trace::finish().unwrap().requests.get("login"), Some(&1));
        server.join().unwrap();
    }

    #[test]
    fn lost_login_reply_sends_one_post_and_returns_unknown() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let size = socket.read(&mut buffer).unwrap();
                assert!(size > 0 && request.len() < 16384);
                request.extend_from_slice(&buffer[..size]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    assert!(headers.starts_with("POST "));
                    let length: usize = headers.lines().find_map(|line| line.strip_prefix("Content-Length: ")).unwrap().parse().unwrap();
                    if request.len() >= end + 4 + length { break; }
                }
            }
            drop(socket); // The server may have accepted credentials; lose the response.
            listener.set_nonblocking(true).unwrap();
            let until = Instant::now() + Duration::from_millis(400);
            while Instant::now() < until {
                assert!(listener.accept().is_err(), "credential POST was retried");
                thread::sleep(Duration::from_millis(10));
            }
        });
        let interface = WifiInterface { name: "test".into(), index: 0, address: Ipv4Addr::LOCALHOST };
        let client = campus_client_for_interface(&interface, true).unwrap();
        auth_trace::start();
        let result = send_login_once(client.post(format!("http://{address}/login")).body("dummy=not-a-real-account"));
        assert!(matches!(result, AuthOutcome::Unknown(_)));
        assert_eq!(auth_trace::finish().unwrap().requests.get("login"), Some(&1));
        server.join().unwrap();
    }

    #[test]
    fn campus_http_uses_appliance_compatible_headers_for_get_and_post() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let size = socket.read(&mut chunk).unwrap();
                    assert!(size > 0 && request.len() + size <= 16384);
                    request.extend_from_slice(&chunk[..size]);
                }
                let headers = String::from_utf8_lossy(&request);
                // Reproduce the real appliance's non-standard case-sensitive
                // Host handling: close the socket without a response otherwise.
                if !headers.contains("\r\nHost: ") { continue; }
                assert!(!headers.contains("\r\nhost:"));
                if headers.starts_with("POST ") {
                    assert!(headers.contains("\r\nContent-Type: "));
                }
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
            }
        });
        let interface = WifiInterface { name: "test".into(), index: 0, address: Ipv4Addr::LOCALHOST };
        let client = campus_client_for_interface(&interface, true).unwrap();
        let url = format!("http://{address}/");
        assert_eq!(client.get(&url).send().unwrap().text().unwrap(), "ok");
        assert_eq!(client.post(&url).header("Content-Type", "application/x-www-form-urlencoded")
            .body("service=shu").send().unwrap().text().unwrap(), "ok");
        server.join().unwrap();
    }

    #[test]
    #[ignore = "Explicit live campus login using saved campus-only credentials; never logs out"]
    fn live_saved_campus_authentication() {
        if std::env::var("SHUNET_LIVE_AUTH").as_deref() != Ok("1") {
            println!("SKIPPED: explicit SHUNET_LIVE_AUTH=1 required; no login sent");
            return;
        }
        let (settings, warning) = crate::settings::SettingsStore::new().load();
        if warning.is_some() || settings.operator != Operator::Campus
            || settings.username.is_empty() || settings.password.is_empty() {
            println!("SKIPPED: saved campus credentials are not available; no login sent");
            return;
        }
        let ssid = crate::wifi::current_ssid(None).expect("read current Wi-Fi");
        assert!(supports_campus_portal(&ssid), "recognized campus Wi-Fi required; phone sharing alone is insufficient");
        let monitor = crate::reconnect::ReconnectMonitor::new(false);
        let (success, _) = portal_login_guarded(&ssid, &settings.username, &settings.password, Operator::Campus, &monitor.guard())
            .expect("campus authentication request should complete");
        assert!(success, "portal did not accept saved campus credentials; response omitted for privacy");
        let mut probe = CampusProbe::default();
        // A successful login reply is acceptance, not immediate session evidence.
        // Match the application's verification phase using bounded read-only polls.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if probe.check(&monitor.guard(), true).is_ok_and(|checked| matches!(checked.state, CampusPortalState::Online)) {
                break;
            }
            assert!(Instant::now() < deadline, "portal did not confirm the session as online within 30s");
            thread::sleep(Duration::from_secs(2));
        }
        println!("Campus authentication and independent online confirmation passed; no logout sent");
    }

    #[test]
    #[ignore = "Read-only live campus portal diagnosis; never sends credentials"]
    fn live_portal_readonly_diagnostic() {
        let interface = wifi_interface().expect("active physical Wi-Fi");
        println!("Wi-Fi index={}, source={}", interface.index, interface.address);
        let ssid = crate::wifi::current_ssid(Some(interface.index)).expect("current physical Wi-Fi");
        println!("Recognized campus Wi-Fi={}", supports_campus_portal(&ssid));
        let monitor = crate::reconnect::ReconnectMonitor::new(false);
        let client = campus_client_for_interface(&interface, true).unwrap();
        let page = load_monitor_page(&client, &monitor.guard()).expect("discover current portal");
        let session = portal_online_status_once(&client, &page).expect("query current session");
        println!("Session: online={}, waiting={}, offline={}, {}, IP supplied={}, IP matches={}",
            session.online, session.waiting, session.offline, session.detail, session.user_ip.is_some(),
            session.user_ip.as_deref().is_some_and(|ip| ip.parse::<Ipv4Addr>().ok() == Some(interface.address)));
        let mut probe = CampusProbe::default();
        for attempt in 1..=2 {
            match probe.check(&monitor.guard(), true) {
                Ok(check) => println!("attempt={attempt}, state={:?}", check.state),
                Err(error) => panic!("attempt={attempt}, {error}"),
            }
        }
    }

    #[test]
    fn operator_selection_uses_requested_service_instead_of_portal_default() {
        let list = serde_json::json!([
            {"serviceName":"中国移动","serviceShowName":"中国移动","serviceDefault":true},
            {"serviceName":"shu","serviceShowName":"校园网"},
            {"serviceName":"中国电信","serviceShowName":"中国电信"},
            {"serviceName":"中国联通","serviceShowName":"中国联通"}
        ]);
        for services in [list.clone(), serde_json::Value::String(list.to_string())] {
            let body = serde_json::json!({"serviceJson":services}).to_string();
            for operator in Operator::ALL {
                assert_eq!(service_from_response(&body, operator).unwrap(), operator.service_name());
            }
        }
        let only_campus = r#"{"serviceJson":[{"serviceName":"shu","serviceDefault":true}]}"#;
        assert!(service_from_response(only_campus, Operator::Telecom).is_none());
        let opaque = r#"{"serviceJson":[{"serviceName":"opaque-id","serviceShowName":"中国联通"}]}"#;
        assert_eq!(service_from_response(opaque, Operator::Unicom).as_deref(), Some("opaque-id"));
    }

    #[test]
    fn html_operator_choices_match_saved_shu_page_values() {
        let html = r#"
            <input id="net_access_type" value="shu">
            <div id="bch_service_0" onclick="selectService(&#39;shu&#39;,&#39;校园网&#39;,&#39;0&#39;)"></div>
            <div id="bch_service_1" onclick="selectService(&#39;中国电信&#39;,&#39;中国电信&#39;,&#39;1&#39;)"></div>
            <div id="bch_service_2" onclick="selectService(&#39;中国联通&#39;,&#39;中国联通&#39;,&#39;2&#39;)"></div>
            <div id="bch_service_3" onclick="selectService(&#39;中国移动&#39;,&#39;中国移动&#39;,&#39;3&#39;)"></div>
        "#;
        for operator in Operator::ALL {
            assert_eq!(service_from_html(html, operator).unwrap(), operator.service_name());
        }
        let hidden = r#"<input id="net_access_type" value="shu">"#;
        assert!(service_from_html(hidden, Operator::Mobile).is_none());
        assert_eq!(service_from_html(hidden, Operator::Campus).as_deref(), Some("shu"));
        let options = r#"<select><option value="shu" selected>校园网</option><option value="telecom-id">中国电信</option></select>"#;
        assert_eq!(service_from_html(options, Operator::Telecom).as_deref(), Some("telecom-id"));
    }

    #[test]
    fn selected_operator_reaches_login_body_with_portal_double_encoding() {
        for operator in Operator::ALL {
            let body = portal_login_body("demo", "test-password", operator.service_name(), "ssid=campus&token=a%2Bb", "true");
            let first = Url::parse(&format!("http://example.test/?{body}")).unwrap();
            let service_once = first.query_pairs().find(|(key, _)| key == "service").unwrap().1.into_owned();
            let second = Url::parse(&format!("http://example.test/?service={service_once}")).unwrap();
            assert_eq!(second.query_pairs().next().unwrap().1, operator.service_name());
            if operator != Operator::Campus {
                assert!(body.contains("service=%25E4%25B8%25AD"));
            }
        }
    }

    fn login_url() -> Url {
        Url::parse("http://10.10.9.9/eportal/index.jsp?wlanuserip=fake%252Bip&wlanacname=controller&ssid=campus&nasip=nas&mac=station&t=wireless-v2&url=target").unwrap()
    }

    #[test]
    fn unrelated_redirect_does_not_become_a_portal_page() {
        let unrelated = Url::parse("http://123.123.123.123/?userIndex=private-session").unwrap();
        let error = portal_page_url(&unrelated).unwrap_err();
        assert!(!error.contains("private-session"));
        assert!(!error.contains("userIndex"));
        assert!(portal_endpoint(&unrelated, "pageInfo").is_err());
        assert!(portal_page_url(&login_url()).is_ok());
    }

    #[test]
    fn gateway_javascript_redirect_supplies_the_missing_wireless_context() {
        let bootstrap = Url::parse("http://123.123.123.123/").unwrap();
        let actual = login_url();
        let html = format!(
            "<script>window.location.href='{}';</script>",
            actual.as_str()
        );
        let resolved = portal_script_redirect(&bootstrap, &html).unwrap();
        assert_eq!(resolved, actual);
        assert_eq!(resolved.query(), actual.query());
        assert!(validate_portal_context(&resolved).is_ok());
    }

    #[test]
    fn gateway_script_is_read_as_a_url_and_does_not_follow_other_destinations() {
        let bootstrap = Url::parse("http://123.123.123.123/").unwrap();
        assert!(
            portal_script_redirect(
                &bootstrap,
                "<script>location.href='http://example.com/eportal/index.jsp';</script>"
            )
            .is_none()
        );
        assert!(
            portal_script_redirect(
                &bootstrap,
                "<script>location.href='http://10.10.9.9/eportal/logout.jsp';</script>"
            )
            .is_none()
        );
    }

    #[test]
    fn online_page_is_never_used_as_wireless_login_context() {
        let online =
            Url::parse("http://10.10.9.9/eportal/success.jsp?userIndex=private-session").unwrap();
        assert!(validate_portal_context(&online).is_err());
    }

    #[test]
    fn missing_controller_parameters_stop_login_before_submission() {
        let incomplete =
            Url::parse("http://10.10.9.9/eportal/index.jsp?mac=private-station").unwrap();
        let error = validate_portal_context(&incomplete).unwrap_err();
        assert!(error.contains("wlanacname"));
        assert!(error.contains("nasip"));
        assert!(!error.contains("private-station"));
        assert!(validate_portal_context(&login_url()).is_ok());
    }

    #[test]
    fn portal_query_retains_original_escaping_and_all_gateway_fields() {
        let url = login_url();
        let query = portal_query_string(&url, "");
        assert_eq!(query, url.query().unwrap());
        assert!(query.starts_with("wlanuserip=fake%252Bip&"));
        assert!(query.contains("&t=wireless-v2&url=target"));
        let body_value = uri_component_encode_twice("mac=a%3Ab&nasip=n");
        assert_eq!(body_value, "mac%253Da%25253Ab%2526nasip%253Dn");
    }

    #[test]
    fn portal_api_preserves_the_real_eportal_directory() {
        let endpoint = portal_endpoint(&login_url(), "pageInfo").unwrap();
        assert_eq!(
            endpoint.as_str(),
            "http://10.10.9.9/eportal/InterFace.do?method=pageInfo"
        );
    }

    #[test]
    fn online_status_requires_success_and_a_known_session() {
        assert!(online_response_matches_session(
            &serde_json::json!({"result":"success","userIndex":"session"}),
            ""
        ));
        assert!(online_response_matches_session(
            &serde_json::json!({"result":"success"}),
            "session"
        ));
        assert!(!online_response_matches_session(
            &serde_json::json!({"result":"fail","userIndex":"session"}),
            "session"
        ));
        assert!(!online_response_matches_session(
            &serde_json::json!({"result":"success"}),
            ""
        ));
    }

    #[test]
    fn rsa_ciphertext_matches_the_supplied_portal_javascript() {
        // Golden value generated by the supplied security.js RSAUtils using
        // this public test modulus and non-secret sample text.
        let modulus = format!("{}93", "f".repeat(254));
        let actual = portal_rsa_encrypt("Demo0123>00-11-22-33-44-55", "10001", &modulus).unwrap();
        assert_eq!(
            actual,
            "7571e5b9190330bb85ecd84d9e158f1f430512ee356590da0f61935156057e2252b5dee1b3b73c805e980c29b1aa0f895e2bfb4d0e6b0b59f0cc9602cd79edc6068a3beba04649cdb8d9d7fbb417dde20af64339ecfeaa93d96c147aed41b165f8fc25562551d730f74d9faa76eff58251f00f32f4b8b54fb2f6655318d6906d"
        );
    }
}
