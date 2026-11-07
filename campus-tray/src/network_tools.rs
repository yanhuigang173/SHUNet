//! On-demand network information; no login, DHCP changes, or subnet scanning.
use std::{
    collections::BTreeSet,
    io::Read,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NOT_FOUND, ERROR_SUCCESS},
    NetworkManagement::{IpHelper::*, Ndis::IfOperStatusUp},
    Networking::WinSock::*,
};

#[derive(Clone, Debug)]
pub struct Adapter {
    pub index: u32,
    pub guid: String,
    pub name: String,
    pub description: String,
    pub addresses: Vec<(IpAddr, u8)>,
    pub gateways: Vec<IpAddr>,
    pub dns: Vec<IpAddr>,
    pub metric: u32,
    pub shared: bool,
    pub hotspot: bool,
}
impl Adapter {
    pub fn usb_network(&self) -> bool {
        let description = self.description.to_ascii_lowercase();
        ["usb", "rndis", "remote ndis", "gadget", "cdc ethernet"]
            .iter()
            .any(|word| description.contains(word))
    }
    fn virtual_exit(&self) -> bool {
        let name = format!("{} {}", self.name, self.description).to_ascii_lowercase();
        [
            "tun",
            "tunnel",
            "tap-windows",
            "wireguard",
            "vpn",
            "atrust",
            "virtual",
        ]
        .iter()
        .any(|word| name.contains(word))
    }
    pub fn label(&self) -> String {
        format!(
            "{}{} · {}{}",
            self.name,
            if self.usb_network() {
                " · USB/RNDIS"
            } else {
                ""
            },
            self.addresses
                .iter()
                .find(|(ip, _)| ip.is_ipv4())
                .or_else(|| self.addresses.first())
                .map_or_else(|| "无地址".into(), |(ip, _)| ip.to_string()),
            if self.shared {
                " · 本机共享下游"
            } else if self.hotspot {
                " · 热点虚拟接口"
            } else {
                ""
            }
        )
    }
    pub fn values(&self) -> [String; 4] {
        [
            self.addresses
                .iter()
                .filter(|(ip, _)| ip.is_ipv4())
                .map(|(ip, _)| ip.to_string())
                .collect::<Vec<_>>()
                .join("；"),
            self.addresses
                .iter()
                .filter(|(ip, _)| ip.is_ipv6())
                .map(|(ip, _)| scoped_ip(*ip, self.index))
                .collect::<Vec<_>>()
                .join("；"),
            self.gateways
                .iter()
                .map(|ip| scoped_ip(*ip, self.index))
                .collect::<Vec<_>>()
                .join("；"),
            self.dns
                .iter()
                .map(|ip| scoped_ip(*ip, self.index))
                .collect::<Vec<_>>()
                .join("；"),
        ]
    }
}

fn scoped_ip(ip: IpAddr, index: u32) -> String {
    match ip {
        IpAddr::V6(ip) if ip.is_unicast_link_local() => format!("{ip}%{index}"),
        _ => ip.to_string(),
    }
}
unsafe fn socket_ip(address: SOCKET_ADDRESS) -> Option<IpAddr> {
    if address.lpSockaddr.is_null() {
        return None;
    }
    unsafe {
        match (*address.lpSockaddr).sa_family {
            AF_INET if address.iSockaddrLength as usize >= std::mem::size_of::<SOCKADDR_IN>() => {
                let ip = &*(address.lpSockaddr as *const SOCKADDR_IN);
                let bytes = ip.sin_addr.S_un.S_un_b;
                Some(Ipv4Addr::new(bytes.s_b1, bytes.s_b2, bytes.s_b3, bytes.s_b4).into())
            }
            AF_INET6 if address.iSockaddrLength as usize >= std::mem::size_of::<SOCKADDR_IN6>() => {
                let ip = &*(address.lpSockaddr as *const SOCKADDR_IN6);
                Some(Ipv6Addr::from(ip.sin6_addr.u.Byte).into())
            }
            _ => None,
        }
    }
}
unsafe fn wide_string(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0;
    unsafe {
        while length < 512 && *pointer.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
    }
}
fn normalize_guid(value: &str) -> String {
    value.trim_matches(['{', '}']).to_ascii_lowercase()
}

pub fn adapters() -> Result<Vec<Adapter>, String> {
    let mut bytes = 16_384u32;
    for _ in 0..3 {
        let count = (bytes as usize).div_ceil(std::mem::size_of::<IP_ADAPTER_ADDRESSES_LH>());
        let mut memory = vec![IP_ADAPTER_ADDRESSES_LH::default(); count];
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST,
                std::ptr::null(),
                memory.as_mut_ptr(),
                &mut bytes,
            )
        };
        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if result != ERROR_SUCCESS {
            return Err(format!("读取网络接口失败（Windows {result}）。"));
        }
        let mut found = Vec::new();
        let mut pointer = memory.as_ptr();
        unsafe {
            while !pointer.is_null() && found.len() < 256 {
                let item = &*pointer;
                pointer = item.Next;
                if item.OperStatus != IfOperStatusUp || item.IfType == IF_TYPE_SOFTWARE_LOOPBACK {
                    continue;
                }
                let mut adapter = Adapter {
                    index: item.Anonymous1.Anonymous.IfIndex,
                    guid: if item.AdapterName.is_null() {
                        String::new()
                    } else {
                        normalize_guid(
                            &std::ffi::CStr::from_ptr(item.AdapterName.cast()).to_string_lossy(),
                        )
                    },
                    name: wide_string(item.FriendlyName),
                    description: wide_string(item.Description),
                    addresses: Vec::new(),
                    gateways: Vec::new(),
                    dns: Vec::new(),
                    metric: item.Ipv4Metric,
                    shared: false,
                    hotspot: false,
                };
                let mut unicast = item.FirstUnicastAddress;
                while !unicast.is_null() && adapter.addresses.len() < 128 {
                    if let Some(ip) = socket_ip((*unicast).Address)
                        .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
                    {
                        adapter.addresses.push((ip, (*unicast).OnLinkPrefixLength));
                    }
                    unicast = (*unicast).Next;
                }
                let mut gateway = item.FirstGatewayAddress;
                while !gateway.is_null() && adapter.gateways.len() < 64 {
                    if let Some(ip) =
                        socket_ip((*gateway).Address).filter(|ip| !ip.is_unspecified())
                    {
                        adapter.gateways.push(ip);
                    }
                    gateway = (*gateway).Next;
                }
                let mut dns = item.FirstDnsServerAddress;
                while !dns.is_null() && adapter.dns.len() < 64 {
                    if let Some(ip) = socket_ip((*dns).Address) {
                        adapter.dns.push(ip);
                    }
                    dns = (*dns).Next;
                }
                adapter.hotspot = adapter
                    .description
                    .to_ascii_lowercase()
                    .contains("wi-fi direct");
                if !adapter.addresses.is_empty() {
                    found.push(adapter);
                }
            }
        }
        found.sort_by_key(|adapter| {
            (
                adapter.gateways.is_empty(),
                adapter.virtual_exit(),
                adapter.metric,
                adapter.index,
            )
        });
        return Ok(found);
    }
    Err("网络接口正在变化，请刷新重试。".into())
}

fn shared_interfaces() -> windows::core::Result<(BTreeSet<String>, bool)> {
    use windows::{
        Win32::{
            NetworkManagement::WindowsFirewall::*,
            System::{Com::*, Ole::IEnumVARIANT, Variant::VARIANT},
        },
        core::{IUnknown, Interface},
    };
    let _apartment = crate::wifi::WinrtApartment::new()?;
    unsafe {
        let manager: INetSharingManager =
            CoCreateInstance(&NetSharingManager, None, CLSCTX_INPROC_SERVER)?;
        let collection = manager.EnumEveryConnection()?;
        let iterator: IEnumVARIANT = collection._NewEnum()?.cast()?;
        let mut ids = BTreeSet::new();
        let mut partial = false;
        for _ in 0..256 {
            let mut value = [VARIANT::default()];
            let mut fetched = 0;
            iterator.Next(&mut value, &mut fetched).ok()?;
            if fetched == 0 {
                break;
            }
            let connection: INetConnection =
                match IUnknown::try_from(&value[0]).and_then(|value| value.cast()) {
                    Ok(value) => value,
                    Err(_) => match IDispatch::try_from(&value[0]).and_then(|value| value.cast()) {
                        Ok(connection) => connection,
                        Err(_) => {
                            partial = true;
                            continue;
                        }
                    },
                };
            let Ok(configuration) =
                manager.get_INetSharingConfigurationForINetConnection(&connection)
            else {
                partial = true;
                continue;
            };
            if configuration.SharingEnabled()?.as_bool()
                && configuration.SharingConnectionType()? == ICSSHARINGTYPE_PRIVATE
            {
                ids.insert(normalize_guid(
                    &manager
                        .get_NetConnectionProps(&connection)?
                        .Guid()?
                        .to_string(),
                ));
            }
        }
        Ok((ids, partial))
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub ip: Option<IpAddr>,
    pub mac: String,
    pub adapter_index: Option<u32>,
    pub name: String,
    pub status: String,
    pub source: String,
}
impl Device {
    pub fn ip_text(&self) -> Option<String> {
        self.ip.map(|ip| match self.adapter_index {
            Some(index) => scoped_ip(ip, index),
            None => ip.to_string(),
        })
    }
}
pub struct Devices {
    pub adapters: Vec<Adapter>,
    pub devices: Vec<Device>,
    pub notes: Vec<String>,
}
fn mac_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
fn same_subnet(ip: IpAddr, base: IpAddr, prefix: u8) -> bool {
    match (ip, base) {
        (IpAddr::V4(ip), IpAddr::V4(base)) if (1..=32).contains(&prefix) => {
            u32::from(ip) >> (32 - prefix) == u32::from(base) >> (32 - prefix)
        }
        (IpAddr::V6(ip), IpAddr::V6(base)) if (1..=128).contains(&prefix) => {
            u128::from(ip) >> (128 - prefix) == u128::from(base) >> (128 - prefix)
        }
        _ => false,
    }
}
fn usable_neighbor(ip: IpAddr, adapter: &Adapter) -> bool {
    if ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || adapter.addresses.iter().any(|(own, _)| *own == ip)
    {
        return false;
    }
    if let IpAddr::V4(ip) = ip {
        if ip == Ipv4Addr::BROADCAST {
            return false;
        }
        // Do not render subnet/broadcast addresses as devices.
        for (base, prefix) in &adapter.addresses {
            if let IpAddr::V4(base) = base {
                if (1..=30).contains(prefix) && same_subnet(ip.into(), (*base).into(), *prefix) {
                    let host = u32::from(ip) & (u32::MAX >> prefix);
                    if host == 0 || host == u32::MAX >> prefix {
                        return false;
                    }
                }
            }
        }
    }
    adapter
        .addresses
        .iter()
        .any(|(base, prefix)| same_subnet(ip, *base, *prefix))
}

fn neighbors(adapters: &[Adapter]) -> Result<Vec<Device>, String> {
    let mut pointer = std::ptr::null_mut();
    let result = unsafe { GetIpNetTable2(AF_UNSPEC, &mut pointer) };
    if result == ERROR_NOT_FOUND {
        return Ok(Vec::new());
    }
    if result != ERROR_SUCCESS || pointer.is_null() {
        return Err(format!("读取邻居缓存失败（Windows {result}）。"));
    }
    struct Table(*mut MIB_IPNET_TABLE2);
    impl Drop for Table {
        fn drop(&mut self) {
            unsafe {
                FreeMibTable(self.0.cast());
            }
        }
    }
    let table = Table(pointer);
    let mut devices = Vec::new();
    unsafe {
        let length = (*table.0).NumEntries as usize;
        if length > 65536 {
            return Err("邻居缓存记录过多，请稍后重试。".into());
        }
        for row in std::slice::from_raw_parts(
            std::ptr::addr_of!((*table.0).Table).cast::<MIB_IPNET_ROW2>(),
            length,
        ) {
            if row.PhysicalAddressLength == 0
                || row.PhysicalAddressLength as usize > row.PhysicalAddress.len()
                || row.State == NlnsUnreachable
                || row.State == NlnsIncomplete
            {
                continue;
            }
            let Some(adapter) = adapters
                .iter()
                .find(|adapter| adapter.index == row.InterfaceIndex)
            else {
                continue;
            };
            let socket = SOCKET_ADDRESS {
                lpSockaddr: std::ptr::addr_of!(row.Address).cast_mut().cast(),
                iSockaddrLength: std::mem::size_of::<SOCKADDR_INET>() as i32,
            };
            let Some(ip) = socket_ip(socket).filter(|ip| usable_neighbor(*ip, adapter)) else {
                continue;
            };
            let bytes = &row.PhysicalAddress[..row.PhysicalAddressLength as usize];
            if bytes.iter().all(|byte| *byte == 0) || bytes[0] & 1 != 0 {
                continue;
            }
            devices.push(Device {
                ip: Some(ip),
                mac: mac_text(bytes),
                adapter_index: Some(adapter.index),
                name: if adapter.gateways.contains(&ip) {
                    "网关".into()
                } else {
                    String::new()
                },
                status: if row.State == NlnsReachable {
                    "近期可达"
                } else if row.State == NlnsPermanent {
                    "静态记录，在线待确认"
                } else {
                    "缓存，在线待确认"
                }
                .into(),
                source: if adapter.shared {
                    "共享下游邻居"
                } else {
                    "接口邻居缓存"
                }
                .into(),
            });
        }
    }
    Ok(devices)
}

fn normalized_mac(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(char::to_uppercase)
        .collect()
}
fn tethering_clients() -> windows::core::Result<Vec<(String, Vec<String>)>> {
    use windows::Networking::{
        Connectivity::NetworkInformation,
        NetworkOperators::{NetworkOperatorTetheringManager, TetheringOperationalState},
    };
    let _apartment = crate::wifi::WinrtApartment::new()?;
    let profiles = NetworkInformation::GetConnectionProfiles()?;
    let mut clients: Vec<(String, Vec<String>)> = Vec::new();
    let mut succeeded = false;
    for index in 0..profiles.Size()?.min(128) {
        let profile = profiles.GetAt(index)?;
        let Ok(manager) = NetworkOperatorTetheringManager::CreateFromConnectionProfile(&profile)
        else {
            continue;
        };
        let Ok(state) = manager.TetheringOperationalState() else {
            continue;
        };
        succeeded = true;
        if state != TetheringOperationalState::On {
            continue;
        }
        let entries = manager.GetTetheringClients()?;
        for client in entries {
            let mac = client.MacAddress()?.to_string();
            let mut names = Vec::new();
            for name in client.HostNames()? {
                names.push(name.CanonicalName()?.to_string());
            }
            if !clients
                .iter()
                .any(|(known, _)| normalized_mac(known) == normalized_mac(&mac))
            {
                clients.push((mac, names));
            }
        }
    }
    if !succeeded {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80070005u32 as i32,
        )));
    }
    Ok(clients)
}

pub fn devices() -> Result<Devices, String> {
    let mut adapters = adapters()?;
    let mut notes = Vec::new();
    match shared_interfaces() {
        Ok((ids, partial)) => {
            for adapter in &mut adapters {
                adapter.shared = ids.contains(&adapter.guid);
            }
            if partial {
                notes.push("部分接口未提供共享状态；未标记为共享的接口仍可手动查看。".into());
            }
        }
        Err(_) => notes.push("Windows 未提供共享接口状态；可按网卡查看邻居缓存。".into()),
    }
    let mut devices = neighbors(&adapters)?;
    match tethering_clients() {
        Ok(clients) => {
            for (mac, names) in clients {
                let mut matched = false;
                for device in &mut devices {
                    if normalized_mac(&device.mac) == normalized_mac(&mac)
                        && adapters.iter().any(|a| {
                            Some(a.index) == device.adapter_index && (a.shared || a.hotspot)
                        })
                    {
                        device.status = "热点已连接".into();
                        device.source = "Windows 热点客户端".into();
                        device.name = names
                            .iter()
                            .find(|name| name.parse::<IpAddr>().is_err())
                            .cloned()
                            .unwrap_or_default();
                        matched = true;
                    }
                }
                if !matched {
                    let ip = names.iter().find_map(|name| name.parse::<IpAddr>().ok());
                    let index = ip.and_then(|ip| {
                        adapters
                            .iter()
                            .find(|a| (a.shared || a.hotspot) && usable_neighbor(ip, a))
                            .map(|a| a.index)
                    });
                    devices.push(Device {
                        ip,
                        mac,
                        adapter_index: index,
                        name: names
                            .iter()
                            .find(|name| name.parse::<IpAddr>().is_err())
                            .cloned()
                            .unwrap_or_default(),
                        status: "热点已连接".into(),
                        source: "Windows 热点客户端".into(),
                    });
                }
            }
        }
        Err(_) => {
            notes.push("热点客户端接口不可用，当前结果来自邻居缓存；并非完整 DHCP 租约表。".into())
        }
    }
    devices.sort_by_key(|d| (d.adapter_index, d.ip, d.mac.clone()));
    devices.dedup_by(|a, b| a.adapter_index == b.adapter_index && a.ip == b.ip && a.mac == b.mac);
    notes.push(
        "来源为 Windows 热点客户端及邻居缓存，并非完整 DHCP 租约表；缓存记录的在线状态待确认。"
            .into(),
    );
    notes.push("未显示的休眠或尚未通信设备，不代表未连接。手机给电脑 USB 共享上网时，其他手机客户端不属于电脑分配的地址。".into());
    Ok(Devices {
        adapters,
        devices,
        notes,
    })
}

#[derive(Clone, Debug)]
pub struct PublicIp {
    pub ip: IpAddr,
    pub country: String,
}
pub fn parse_public_ip(body: &str) -> Result<PublicIp, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "查询服务未返回有效数据。")?;
    if value.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return Err("公网查询服务未完成请求或已达到限额。".into());
    }
    let ip: IpAddr = value
        .get("ip")
        .and_then(|v| v.as_str())
        .ok_or("查询服务未返回 IP。")?
        .parse()
        .map_err(|_| "查询服务返回了无效 IP。")?;
    if !is_public_ip(ip) {
        return Err("查询服务未返回公网地址。".into());
    }
    let country = value
        .get("country")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("未知");
    let code = value
        .get("country_code")
        .and_then(|v| v.as_str())
        .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic()));
    Ok(PublicIp {
        ip,
        country: match code {
            Some(code) => format!(
                "{} ({})",
                country
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(80)
                    .collect::<String>(),
                code.to_ascii_uppercase()
            ),
            None => country
                .chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect(),
        },
    })
}
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_documentation()
                && ip != Ipv4Addr::BROADCAST
                && ip.octets()[0] != 0
                && ip.octets()[0] < 240
                && !(ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                && !(ip.octets()[0] == 198 && (18..=19).contains(&ip.octets()[1]))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                is_public_ip(v4.into())
            } else {
                !ip.is_unspecified()
                    && !ip.is_loopback()
                    && !ip.is_multicast()
                    && !ip.is_unicast_link_local()
                    && ip.segments()[0] & 0xfe00 != 0xfc00
                    && !(ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8)
            }
        }
    }
}

pub fn public_ip(
    mode: crate::speedtest::Mode,
    proxy: &str,
    cancelled: &AtomicBool,
) -> Result<PublicIp, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("已取消查询。".into());
    }
    let route = crate::speedtest::Route::capture(mode, proxy)?;
    let client = route.client()?;
    let response = client
        .get("https://ipwho.is/?lang=zh-CN")
        .timeout(Duration::from_secs(8))
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                "公网 IP 查询超时，请稍后刷新。"
            } else {
                "无法连接公网查询服务，请检查该出口的网络或代理。"
            }
        })?;
    if cancelled.load(Ordering::Acquire) {
        return Err("已取消查询。".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "公网查询服务返回 HTTP {}。",
            response.status().as_u16()
        ));
    }
    let mut body = String::new();
    response
        .take(65_537)
        .read_to_string(&mut body)
        .map_err(|_| "读取公网查询结果失败。")?;
    if body.len() > 65_536 {
        return Err("公网查询响应过大。".into());
    }
    parse_public_ip(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn adapter() -> Adapter {
        Adapter {
            index: 42,
            guid: "test".into(),
            name: "热点".into(),
            description: "Wi-Fi Direct".into(),
            addresses: vec![(Ipv4Addr::new(192, 168, 137, 1).into(), 24)],
            gateways: Vec::new(),
            dns: Vec::new(),
            metric: 1,
            shared: true,
            hotspot: true,
        }
    }
    #[test]
    fn neighbor_filter_omits_self_broadcast_outside_subnet_and_multicast() {
        let adapter = adapter();
        assert!(usable_neighbor("192.168.137.2".parse().unwrap(), &adapter));
        for ip in [
            "192.168.137.1",
            "192.168.137.0",
            "192.168.137.255",
            "192.168.138.2",
            "255.255.255.255",
            "224.0.0.1",
            "0.0.0.0",
            "127.0.0.1",
        ] {
            assert!(!usable_neighbor(ip.parse().unwrap(), &adapter), "{ip}");
        }
        assert!(!same_subnet(
            "10.0.0.1".parse().unwrap(),
            "10.0.0.2".parse().unwrap(),
            0
        ));
        assert!(!same_subnet(
            "::1".parse().unwrap(),
            "10.0.0.2".parse().unwrap(),
            24
        ));
    }
    #[test]
    fn public_ip_response_requires_real_address_and_retains_country() {
        let response = r#"{"success":true,"ip":"8.8.8.8","country":"美国","country_code":"US"}"#;
        let parsed = parse_public_ip(response).unwrap();
        assert_eq!(parsed.ip.to_string(), "8.8.8.8");
        assert_eq!(parsed.country, "美国 (US)");
        for ip in [
            "10.1.2.3",
            "198.18.0.1",
            "100.64.0.1",
            "192.0.2.1",
            "::1",
            "fc00::1",
            "2001:db8::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(
                parse_public_ip(&serde_json::json!({"ip":ip,"success":true}).to_string()).is_err(),
                "{ip}"
            );
        }
        assert!(parse_public_ip(r#"{"success":false,"message":"rate limit"}"#).is_err());
        assert!(parse_public_ip("<html>login page</html>").is_err());
    }
    #[test]
    fn adapter_values_copy_plain_ips_and_scope_ipv6_to_the_selected_adapter() {
        let mut adapter = adapter();
        adapter.addresses.push(("fe80::abcd".parse().unwrap(), 64));
        adapter.gateways.push("fe80::1".parse().unwrap());
        assert_eq!(adapter.values()[0], "192.168.137.1");
        assert_eq!(adapter.values()[1], "fe80::abcd%42");
        assert_eq!(adapter.values()[2], "fe80::1%42");
    }
    #[test]
    fn usb_board_adapter_is_labelled_and_prefers_its_ipv4_address() {
        let mut adapter = adapter();
        adapter.description = "USB Ethernet/RNDIS Gadget".into();
        adapter
            .addresses
            .insert(0, ("fe80::1234".parse().unwrap(), 64));
        assert!(adapter.usb_network());
        assert!(adapter.label().contains("USB/RNDIS"));
        assert!(adapter.label().contains("192.168.137.1"));
    }
    #[test]
    fn closed_window_cancel_does_not_send_any_public_request() {
        let cancelled = AtomicBool::new(true);
        assert!(public_ip(crate::speedtest::Mode::Direct, "", &cancelled).is_err());
        assert!(public_ip(crate::speedtest::Mode::Proxy, "", &cancelled).is_err());
    }
    #[test]
    #[ignore = "Read-only live local adapters and hotspot/neighbor list; no scanning or DHCP changes"]
    fn live_local_information() {
        match shared_interfaces() {
            Ok((ids, partial)) => println!(
                "ICS private interface count={}, partial={partial}",
                ids.len()
            ),
            Err(error) => println!("ICS query unavailable: {:?}", error.code()),
        }
        let report = devices().expect("read Windows local network information");
        assert!(!report.adapters.is_empty());
        println!(
            "adapters={}, neighbor/client records={}, shared downstream={}, hotspot interfaces={}",
            report.adapters.len(),
            report.devices.len(),
            report.adapters.iter().filter(|a| a.shared).count(),
            report.adapters.iter().filter(|a| a.hotspot).count()
        );
        for note in report.notes {
            println!("{note}");
        }
    }
}
