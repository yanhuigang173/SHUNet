use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::NetworkManagement::WiFi::*;
use windows_sys::core::GUID;

use crate::network::{WifiNetwork, decode_output};

type InterfaceKey = (u32, u16, u16, [u8; 8]);

static CONNECTION_GENERATION: AtomicU64 = AtomicU64::new(0);
static OBSERVING_CONNECTION: AtomicBool = AtomicBool::new(false);
static SSID_CACHE: Mutex<Option<SsidCache>> = Mutex::new(None);

struct SsidCache {
    interface: InterfaceKey,
    generation: u64,
    read_at: Instant,
    ssid: String,
}

impl SsidCache {
    fn matches(&self, interface: InterfaceKey, generation: u64, lifetime: Duration) -> bool {
        self.interface == interface
            && self.generation == generation
            && self.read_at.elapsed() < lifetime
    }
}

pub fn connection_generation() -> u64 {
    CONNECTION_GENERATION.load(Ordering::Acquire)
}

/// Safe in WLAN callbacks: no I/O or mutex acquisition.
pub fn invalidate_ssid_cache() {
    CONNECTION_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Routine checks enumerate adapter state, but reuse the SSID when possible.
/// Periodic revalidation bounds staleness if a Windows notification is missed.
pub fn cached_current_ssid(index: Option<u32>) -> Result<String, String> {
    read_current_ssid(index, true)
}

fn interface_key(guid: &GUID) -> InterfaceKey {
    (guid.data1, guid.data2, guid.data3, guid.data4)
}

#[derive(Default)]
struct ScanCompletion {
    results: Mutex<Vec<(InterfaceKey, ScanState)>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Pending,
    Complete,
    Failed(Option<u32>),
    Rejected(u32),
}

struct WlanSession {
    handle: HANDLE,
    completion: Box<ScanCompletion>,
}

impl Drop for WlanSession {
    fn drop(&mut self) {
        // CloseHandle waits for any running callback before the boxed context
        // is dropped. Never close while holding the callback's mutex.
        unsafe {
            WlanCloseHandle(self.handle, std::ptr::null());
        }
    }
}

struct WlanBuffer<T>(*mut T);

impl<T> Drop for WlanBuffer<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                WlanFreeMemory(self.0.cast());
            }
        }
    }
}

/// Read only the connected SSID through WinRT, without requesting BSSID/location.
/// An optional index ensures the SSID belongs to the IPv4 adapter used by HTTP.
pub fn current_ssid(index: Option<u32>) -> Result<String, String> {
    read_current_ssid(index, false)
}

fn read_current_ssid(index: Option<u32>, cached: bool) -> Result<String, String> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        ConvertInterfaceGuidToLuid, ConvertInterfaceLuidToIndex,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
    let generation = connection_generation();
    let mut handle = std::ptr::null_mut();
    let mut version = 0;
    let code = unsafe { WlanOpenHandle(2, std::ptr::null(), &mut version, &mut handle) };
    if code != ERROR_SUCCESS {
        return Err(wlan_error("读取无线连接", code));
    }
    let session = WlanSession {
        handle,
        completion: Box::default(),
    };
    let mut interfaces = WlanBuffer::<WLAN_INTERFACE_INFO_LIST>(std::ptr::null_mut());
    let code = unsafe { WlanEnumInterfaces(session.handle, std::ptr::null(), &mut interfaces.0) };
    if code != ERROR_SUCCESS {
        return Err(wlan_error("读取无线网卡", code));
    }
    if interfaces.0.is_null() {
        *SSID_CACHE.lock().unwrap_or_else(|error| error.into_inner()) = None;
        return Ok(String::new());
    }
    let count = unsafe { (*interfaces.0).dwNumberOfItems } as usize;
    if count > 256 {
        return Err("无线网卡数量异常。".to_owned());
    }
    for interface in
        unsafe { std::slice::from_raw_parts((*interfaces.0).InterfaceInfo.as_ptr(), count) }
    {
        if interface.isState != wlan_interface_state_connected {
            continue;
        }
        if let Some(expected) = index {
            let mut luid = NET_LUID_LH::default();
            let mut actual = 0;
            if unsafe { ConvertInterfaceGuidToLuid(&interface.InterfaceGuid, &mut luid) }
                != ERROR_SUCCESS
                || unsafe { ConvertInterfaceLuidToIndex(&luid, &mut actual) } != ERROR_SUCCESS
                || actual != expected
            {
                continue;
            }
        }
        let key = interface_key(&interface.InterfaceGuid);
        let lifetime = Duration::from_secs(if OBSERVING_CONNECTION.load(Ordering::Acquire) {
            600
        } else {
            60
        });
        if cached {
            let cache = SSID_CACHE.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(entry) = cache.as_ref().filter(|entry| entry.matches(key, generation, lifetime)) {
                if connection_generation() != generation {
                    return Err("读取期间 Wi-Fi 已变化，请稍后重试。".into());
                }
                return Ok(entry.ssid.clone());
            }
        }
        let ssid = connected_ssid_for_adapter(key)
            .map_err(|error| format!("读取 Wi-Fi 名称失败（GetConnectedSsid）：{error}"))?;
        if connection_generation() != generation {
            return Err("读取期间 Wi-Fi 已变化，请稍后重试。".into());
        }
        *SSID_CACHE.lock().unwrap_or_else(|error| error.into_inner()) = Some(SsidCache {
            interface: key,
            generation,
            read_at: Instant::now(),
            ssid: ssid.clone(),
        });
        return Ok(ssid);
    }
    // A subsequent reconnect must never reuse a pre-disconnection SSID.
    *SSID_CACHE.lock().unwrap_or_else(|error| error.into_inner()) = None;
    Ok(String::new())
}

/// Pair WinRT initialization on the calling thread, including early returns.
/// An existing STA apartment is already usable and must not be uninitialized here.
pub(crate) struct WinrtApartment(bool);

// windows-rs caches agile activation factories for the lifetime of the process.
// Keep their MTA alive for that same lifetime, even between per-call apartments
// and after short-lived callers exit. Otherwise the cached factory can point to
// freed memory on the next refresh. One cookie is retained, never one per query;
// Windows releases it at process exit, together with the static factory cache.
static WINRT_MTA: OnceLock<Result<usize, windows::core::HRESULT>> = OnceLock::new();

fn retain_winrt_mta() -> windows::core::Result<()> {
    let result = WINRT_MTA.get_or_init(|| {
        unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() }
            .map(|cookie| cookie.0 as usize)
            .map_err(|error| error.code())
    });
    match result {
        Ok(_) => Ok(()),
        Err(code) => Err(windows::core::Error::from_hresult(*code)),
    }
}

impl WinrtApartment {
    pub(crate) fn new() -> windows::core::Result<Self> {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        retain_winrt_mta()?;
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => Ok(Self(true)),
            Err(error) if error.code().0 == windows_sys::Win32::Foundation::RPC_E_CHANGED_MODE => {
                Ok(Self(false))
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for WinrtApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows::Win32::System::WinRT::RoUninitialize(); }
        }
    }
}

fn connected_ssid_for_adapter(key: InterfaceKey) -> windows::core::Result<String> {
    use windows::Networking::Connectivity::NetworkInformation;
    let _apartment = WinrtApartment::new()?;
    // Enumerate all profiles: the Internet/default route may belong to a VPN,
    // Ethernet, or another Wi-Fi adapter, including before campus authentication.
    let profiles = NetworkInformation::GetConnectionProfiles()?;
    for index in 0..profiles.Size()? {
        let profile = profiles.GetAt(index)?;
        if !profile.IsWlanConnectionProfile()? {
            continue;
        }
        let adapter = profile.NetworkAdapter()?.NetworkAdapterId()?;
        if (adapter.data1, adapter.data2, adapter.data3, adapter.data4) != key {
            continue;
        }
        let ssid = profile.WlanConnectionProfileDetails()?.GetConnectedSsid()?.to_string();
        if !ssid.is_empty() {
            return Ok(ssid);
        }
    }
    // Do not silently fall back to the location-sensitive WlanQueryInterface.
    Err(windows::core::Error::new(
        windows::core::HRESULT(0x80004005u32 as i32),
        "已连接的 Wi-Fi 网卡尚未提供名称，请稍后重试。",
    ))
}

/// Callbacks only coalesce a flag. They perform no I/O and take no UI locks.
pub struct ConnectionObserver {
    handle: HANDLE,
    ip_handle: HANDLE,
    changed: Box<AtomicBool>,
}

impl ConnectionObserver {
    pub fn new() -> Result<Self, String> {
        use windows_sys::Win32::NetworkManagement::IpHelper::NotifyUnicastIpAddressChange;
        use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;
        let mut observer = Self {
            handle: std::ptr::null_mut(),
            ip_handle: std::ptr::null_mut(),
            changed: Box::new(AtomicBool::new(false)),
        };
        let mut version = 0;
        let code =
            unsafe { WlanOpenHandle(2, std::ptr::null(), &mut version, &mut observer.handle) };
        if code != ERROR_SUCCESS {
            return Err(wlan_error("监听无线连接", code));
        }
        let context = (&*observer.changed as *const AtomicBool).cast();
        let code = unsafe {
            WlanRegisterNotification(
                observer.handle,
                WLAN_NOTIFICATION_SOURCE_ACM,
                1,
                Some(connection_notification),
                context,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        if code != ERROR_SUCCESS {
            return Err(wlan_error("监听无线连接", code));
        }
        // WLAN notifications still work if IP Helper subscription is unavailable.
        unsafe {
            NotifyUnicastIpAddressChange(
                AF_UNSPEC,
                Some(ip_notification),
                context,
                false,
                &mut observer.ip_handle,
            );
        }
        invalidate_ssid_cache();
        OBSERVING_CONNECTION.store(true, Ordering::Release);
        Ok(observer)
    }
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }
}

impl Drop for ConnectionObserver {
    fn drop(&mut self) {
        OBSERVING_CONNECTION.store(false, Ordering::Release);
        invalidate_ssid_cache();
        // Both calls wait for callbacks before the boxed atomic is freed.
        unsafe {
            if !self.ip_handle.is_null() {
                windows_sys::Win32::NetworkManagement::IpHelper::CancelMibChangeNotify2(
                    self.ip_handle,
                );
            }
            if !self.handle.is_null() {
                WlanRegisterNotification(
                    self.handle,
                    WLAN_NOTIFICATION_SOURCE_NONE,
                    0,
                    None,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                );
                WlanCloseHandle(self.handle, std::ptr::null());
            }
        }
    }
}

unsafe extern "system" fn connection_notification(
    data: *mut L2_NOTIFICATION_DATA,
    context: *mut c_void,
) {
    if data.is_null() || context.is_null() {
        return;
    }
    let data = unsafe { &*data };
    if data.NotificationSource & WLAN_NOTIFICATION_SOURCE_ACM != 0
        && [
            wlan_notification_acm_connection_complete,
            wlan_notification_acm_disconnected,
            wlan_notification_acm_interface_arrival,
            wlan_notification_acm_interface_removal,
        ]
        .contains(&(data.NotificationCode as i32))
    {
        invalidate_ssid_cache();
        unsafe { &*context.cast::<AtomicBool>() }.store(true, Ordering::Release);
    }
}

unsafe extern "system" fn ip_notification(
    context: *const c_void,
    _row: *const windows_sys::Win32::NetworkManagement::IpHelper::MIB_UNICASTIPADDRESS_ROW,
    _kind: windows_sys::Win32::NetworkManagement::IpHelper::MIB_NOTIFICATION_TYPE,
) {
    if !context.is_null() {
        unsafe { &*context.cast::<AtomicBool>() }.store(true, Ordering::Release);
    }
}

fn wlan_error(action: &str, code: u32) -> String {
    if code == ERROR_ACCESS_DENIED {
        return format!(
            "Windows 拒绝{action}（错误 5）。请在“设置 → 隐私和安全性 → 位置”中检查位置服务及桌面应用访问权限，然后刷新。"
        );
    }
    format!(
        "{action}失败：{}（{code}）。",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

unsafe extern "system" fn scan_notification(data: *mut L2_NOTIFICATION_DATA, context: *mut c_void) {
    if data.is_null() || context.is_null() {
        return;
    }
    // Windows owns data for this call; WlanSession keeps context alive until
    // CloseHandle has completed. Do not unwind or call WLAN APIs here.
    let data = unsafe { &*data };
    if data.NotificationSource & WLAN_NOTIFICATION_SOURCE_ACM == 0 {
        return;
    }
    let code = data.NotificationCode as i32;
    let outcome = if code == wlan_notification_acm_scan_complete {
        ScanState::Complete
    } else if code == wlan_notification_acm_scan_fail {
        let reason = if !data.pData.is_null() && data.dwDataSize >= 4 {
            Some(unsafe { std::ptr::read_unaligned(data.pData.cast::<u32>()) })
        } else {
            None
        };
        ScanState::Failed(reason)
    } else {
        return;
    };
    let completion = unsafe { &*context.cast::<ScanCompletion>() };
    if let Ok(mut results) = completion.results.lock() {
        if let Some((_, result)) = results
            .iter_mut()
            .find(|(key, _)| *key == interface_key(&data.InterfaceGuid))
        {
            if *result == ScanState::Pending {
                *result = outcome;
            }
        }
    }
}

fn authentication(network: &WLAN_AVAILABLE_NETWORK) -> String {
    if network.bSecurityEnabled == 0 {
        return "Open".to_owned();
    }
    match network.dot11DefaultAuthAlgorithm {
        DOT11_AUTH_ALGO_80211_OPEN | DOT11_AUTH_ALGO_80211_SHARED_KEY => "WEP",
        DOT11_AUTH_ALGO_WPA => "WPA-Enterprise",
        DOT11_AUTH_ALGO_WPA_PSK => "WPA-Personal",
        DOT11_AUTH_ALGO_RSNA => "WPA2-Enterprise",
        DOT11_AUTH_ALGO_RSNA_PSK => "WPA2-Personal",
        DOT11_AUTH_ALGO_WPA3_SAE => "WPA3-Personal",
        DOT11_AUTH_ALGO_WPA3_ENT => "WPA3-Enterprise",
        DOT11_AUTH_ALGO_WPA3_ENT_192 => "WPA3-Enterprise-192",
        DOT11_AUTH_ALGO_OWE => "OWE",
        _ => "其他安全类型",
    }
    .to_owned()
}

fn encryption(network: &WLAN_AVAILABLE_NETWORK) -> String {
    match network.dot11DefaultCipherAlgorithm {
        DOT11_CIPHER_ALGO_NONE => "None",
        DOT11_CIPHER_ALGO_CCMP => "AES",
        DOT11_CIPHER_ALGO_CCMP_256 => "AES-256",
        DOT11_CIPHER_ALGO_GCMP => "GCMP",
        DOT11_CIPHER_ALGO_GCMP_256 => "GCMP-256",
        DOT11_CIPHER_ALGO_TKIP => "TKIP",
        DOT11_CIPHER_ALGO_WEP | DOT11_CIPHER_ALGO_WEP40 | DOT11_CIPHER_ALGO_WEP104 => "WEP",
        _ => "未知加密",
    }
    .to_owned()
}

/// A bounded scan held by the existing service worker. WLAN performs the radio
/// work asynchronously; waiting never blocks credential/configuration commands.
pub struct ScanTask {
    session: WlanSession,
    interfaces: Vec<WLAN_INTERFACE_INFO>,
    before: Vec<(InterfaceKey, NetworkList)>,
    deadline: Instant,
    subscription_error: Option<u32>,
}

#[derive(Clone, Default)]
struct NetworkList {
    networks: Vec<WifiNetwork>,
    active: String,
}

pub struct ScanReport {
    pub networks: Vec<WifiNetwork>,
    pub cached: bool,
    pub detail: String,
}

impl ScanTask {
    pub fn start() -> Result<Self, String> {
        let mut version = 0;
        let mut handle = std::ptr::null_mut();
        let code = unsafe { WlanOpenHandle(2, std::ptr::null(), &mut version, &mut handle) };
        if code != ERROR_SUCCESS {
            return Err(wlan_error("打开无线服务", code));
        }
        let session = WlanSession {
            handle,
            completion: Box::default(),
        };
        let mut buffer = WlanBuffer::<WLAN_INTERFACE_INFO_LIST>(std::ptr::null_mut());
        let code = unsafe { WlanEnumInterfaces(handle, std::ptr::null(), &mut buffer.0) };
        if code != ERROR_SUCCESS {
            return Err(wlan_error("枚举无线网卡", code));
        }
        if buffer.0.is_null() {
            return Err("Windows 未返回无线网卡信息。".into());
        }
        let count = unsafe { (*buffer.0).dwNumberOfItems } as usize;
        if count > 64 {
            return Err("Windows 返回的无线网卡数量异常。".into());
        }
        let interfaces: Vec<_> =
            unsafe { std::slice::from_raw_parts((*buffer.0).InterfaceInfo.as_ptr(), count) }
                .iter()
                .filter(|interface| interface.isState != wlan_interface_state_not_ready)
                .copied()
                .collect();
        if interfaces.is_empty() {
            return Err("没有可用的无线网卡，请开启 Wi-Fi 并关闭飞行模式。".into());
        }
        // WlanScan may flush the OS list. Retain a snapshot BEFORE requesting it.
        let before = interfaces
            .iter()
            .filter_map(|interface| {
                read_network_list(handle, &interface.InterfaceGuid)
                    .ok()
                    .map(|list| (interface_key(&interface.InterfaceGuid), list))
            })
            .collect();
        *session
            .completion
            .results
            .lock()
            .map_err(|_| "无法初始化 Wi-Fi 扫描。")? = interfaces
            .iter()
            .map(|interface| (interface_key(&interface.InterfaceGuid), ScanState::Pending))
            .collect();
        let code = unsafe {
            WlanRegisterNotification(
                handle,
                WLAN_NOTIFICATION_SOURCE_ACM,
                0,
                Some(scan_notification),
                (&*session.completion as *const ScanCompletion).cast(),
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        let subscription_error = (code != ERROR_SUCCESS).then_some(code);
        for interface in &interfaces {
            let code = unsafe {
                WlanScan(
                    handle,
                    &interface.InterfaceGuid,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            if code != ERROR_SUCCESS {
                if let Ok(mut results) = session.completion.results.lock() {
                    if let Some((_, state)) = results
                        .iter_mut()
                        .find(|(key, _)| *key == interface_key(&interface.InterfaceGuid))
                    {
                        *state = ScanState::Rejected(code);
                    }
                }
            }
        }
        Ok(Self {
            session,
            interfaces,
            before,
            deadline: Instant::now() + Duration::from_secs(5),
            subscription_error,
        })
    }

    pub fn initial_networks(&self) -> Vec<WifiNetwork> {
        let mut combined = NetworkList::default();
        for (_, list) in &self.before {
            merge_list(&mut combined, list.clone());
        }
        sort_networks(&mut combined);
        combined.networks
    }

    pub fn is_ready(&self) -> bool {
        Instant::now() >= self.deadline
            || self
                .session
                .completion
                .results
                .lock()
                .map_or(true, |results| {
                    results
                        .iter()
                        .all(|(_, state)| *state != ScanState::Pending)
                })
    }

    pub fn finish(&self) -> Result<ScanReport, String> {
        let results = self
            .session
            .completion
            .results
            .lock()
            .map_err(|_| "读取扫描状态失败。")?
            .clone();
        let mut combined = NetworkList::default();
        let mut notes = Vec::new();
        let mut cached = false;
        let mut readable = 0;
        if let Some(code) = self.subscription_error {
            notes.push(wlan_error("订阅扫描通知", code));
        }
        for interface in &self.interfaces {
            let key = interface_key(&interface.InterfaceGuid);
            let state = results
                .iter()
                .find(|(entry, _)| *entry == key)
                .map_or(ScanState::Pending, |(_, state)| *state);
            let fresh = state == ScanState::Complete;
            match state {
                ScanState::Pending => notes.push("扫描等待 5 秒，未收到驱动完成通知".into()),
                ScanState::Failed(reason) => notes.push(scan_reason(reason)),
                ScanState::Rejected(code) => notes.push(wlan_error("请求无线扫描", code)),
                ScanState::Complete => {}
            }
            match read_network_list(self.session.handle, &interface.InterfaceGuid) {
                Ok(list) => {
                    readable += 1;
                    merge_list(&mut combined, list);
                    if !fresh {
                        cached = true;
                        if let Some((_, before)) =
                            self.before.iter().find(|(entry, _)| *entry == key)
                        {
                            merge_list(&mut combined, before.clone());
                        }
                    }
                }
                Err(error) => {
                    cached = true;
                    notes.push(error);
                    if let Some((_, before)) = self.before.iter().find(|(entry, _)| *entry == key) {
                        readable += 1;
                        merge_list(&mut combined, before.clone());
                    }
                }
            }
        }
        notes.sort();
        notes.dedup();
        notes.truncate(6);
        if readable == 0 {
            return Err(if notes.is_empty() {
                "无法读取附近 Wi-Fi 列表。".into()
            } else {
                notes.join("；")
            });
        }
        if cached && combined.networks.is_empty() {
            return Err(format!(
                "{}；系统暂无可显示的网络，请从 Windows Wi-Fi 菜单刷新后重试。",
                if notes.is_empty() {
                    "扫描尚未取得可用结果".into()
                } else {
                    notes.join("；")
                }
            ));
        }
        sort_networks(&mut combined);
        let detail = if cached {
            format!(
                "显示系统已有的 {} 个 Wi-Fi（含缓存，未确认完整刷新）；{}",
                combined.networks.len(),
                notes.join("；")
            )
        } else {
            format!("扫描完成，发现 {} 个 Wi-Fi。", combined.networks.len())
        };
        Ok(ScanReport {
            networks: combined.networks,
            cached,
            detail,
        })
    }
}

fn scan_reason(reason: Option<u32>) -> String {
    let Some(reason) = reason else {
        return "驱动通知扫描失败，未提供原因码".into();
    };
    let mut buffer = [0u16; 512];
    let result = unsafe {
        WlanReasonCodeToString(
            reason,
            buffer.len() as u32,
            buffer.as_mut_ptr(),
            std::ptr::null(),
        )
    };
    let length = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    let detail = String::from_utf16_lossy(&buffer[..length])
        .trim()
        .to_owned();
    if result == ERROR_SUCCESS && !detail.is_empty() {
        format!("驱动扫描失败（原因码 {reason}）：{detail}")
    } else {
        format!("驱动扫描失败（原因码 {reason}）")
    }
}

fn read_network_list(handle: HANDLE, guid: &GUID) -> Result<NetworkList, String> {
    let mut list = WlanBuffer::<WLAN_AVAILABLE_NETWORK_LIST>(std::ptr::null_mut());
    let code =
        unsafe { WlanGetAvailableNetworkList(handle, guid, 0, std::ptr::null(), &mut list.0) };
    if code != ERROR_SUCCESS {
        return Err(wlan_error("读取附近 Wi-Fi 列表", code));
    }
    if list.0.is_null() {
        return Err("Windows 未返回有效的 Wi-Fi 列表。".into());
    }
    let count = unsafe { (*list.0).dwNumberOfItems } as usize;
    if count > 16384 {
        return Err("Windows 返回的 Wi-Fi 数量异常。".into());
    }
    let mut result = NetworkList::default();
    for entry in unsafe { std::slice::from_raw_parts((*list.0).Network.as_ptr(), count) } {
        let connected = entry.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0;
        if entry.uNumberOfBssids == 0 && !connected {
            continue;
        }
        let length = (entry.dot11Ssid.uSSIDLength as usize).min(32);
        let hidden = length == 0;
        let ssid = if hidden {
            "隐藏网络".into()
        } else {
            decode_output(&entry.dot11Ssid.ucSSID[..length])
        };
        if connected && !hidden {
            result.active = ssid.clone();
        }
        merge_network(
            &mut result,
            WifiNetwork {
                ssid,
                hidden,
                authentication: authentication(entry),
                encryption: encryption(entry),
                signal: entry.wlanSignalQuality.min(100) as u8,
            },
            connected,
        );
    }
    Ok(result)
}

fn merge_network(list: &mut NetworkList, network: WifiNetwork, connected: bool) {
    if let Some(existing) = list
        .networks
        .iter_mut()
        .find(|item| item.ssid == network.ssid && item.hidden == network.hidden)
    {
        let signal = existing.signal.max(network.signal);
        if connected || (network.signal > existing.signal && network.ssid != list.active) {
            *existing = network;
        }
        existing.signal = signal;
    } else {
        list.networks.push(network);
    }
}
fn merge_list(destination: &mut NetworkList, source: NetworkList) {
    if !source.active.is_empty() {
        destination.active = source.active.clone();
    }
    for network in source.networks {
        let connected = network.ssid == source.active;
        merge_network(destination, network, connected);
    }
}
fn sort_networks(list: &mut NetworkList) {
    list.networks.sort_by(|a, b| {
        (b.ssid == list.active)
            .cmp(&(a.ssid == list.active))
            .then_with(|| b.signal.cmp(&a.signal))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
}

/// Compatibility entry point; the tray service uses ScanTask without waiting.
pub fn scan() -> Result<(Vec<WifiNetwork>, String), String> {
    let task = ScanTask::start()?;
    while !task.is_ready() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let report = task.finish()?;
    let active = current_ssid(None).unwrap_or_default();
    Ok((report.networks, active))
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    #[ignore = "Read-only integration check using the host's active Wi-Fi"]
    fn winrt_ssid_live_read() {
        let _apartment = WinrtApartment::new().expect("initialize WinRT");
        windows::Networking::Connectivity::NetworkInformation::GetConnectionProfiles()
            .expect("enumerate connection profiles");
        let ssid = current_ssid(None).expect("read SSID for active Wi-Fi via WinRT");
        assert!(!ssid.is_empty(), "live check requires a connected Wi-Fi adapter");
        assert_eq!(cached_current_ssid(None).expect("read cached SSID"), ssid);
        // Do not include the user's network name in diagnostic output.
    }

    #[test]
    #[ignore = "Read-only regression check using the host's active Wi-Fi"]
    fn winrt_ssid_repeated_refresh() {
        // Match the worker: no enclosing apartment guard around individual queries.
        for _ in 0..100 {
            invalidate_ssid_cache();
            let ssid = current_ssid(None).expect("repeat fresh WinRT SSID read");
            assert!(!ssid.is_empty(), "requires an active Wi-Fi connection");
        }
        // Exercise callers exiting between queries while the factory is cached.
        for _ in 0..10 {
            std::thread::spawn(|| {
                assert!(!current_ssid(None).expect("read on a new thread").is_empty());
            }).join().expect("SSID query thread must survive");
        }
    }

    #[test]
    fn connection_events_invalidate_cache_but_scan_and_ip_events_do_not() {
        let changed = AtomicBool::new(false);
        let context = (&changed as *const AtomicBool).cast_mut().cast();
        let mut data = L2_NOTIFICATION_DATA {
            NotificationSource: WLAN_NOTIFICATION_SOURCE_ACM,
            ..Default::default()
        };
        let before = connection_generation();
        data.NotificationCode = wlan_notification_acm_scan_complete as u32;
        unsafe { connection_notification(&mut data, context); }
        assert_eq!(connection_generation(), before);
        assert!(!changed.load(Ordering::Acquire));
        for event in [
            wlan_notification_acm_connection_complete,
            wlan_notification_acm_disconnected,
            wlan_notification_acm_interface_arrival,
            wlan_notification_acm_interface_removal,
        ] {
            let before = connection_generation();
            data.NotificationCode = event as u32;
            unsafe { connection_notification(&mut data, context); }
            assert_ne!(connection_generation(), before);
            assert!(changed.swap(false, Ordering::AcqRel));
        }
        let before = connection_generation();
        unsafe { ip_notification(context, std::ptr::null(), 0); }
        assert_eq!(connection_generation(), before);
        assert!(changed.load(Ordering::Acquire));
    }

    #[test]
    fn cached_ssid_requires_same_adapter_generation_and_unexpired_age() {
        let key = (1, 2, 3, [4; 8]);
        let mut entry = SsidCache {
            interface: key,
            generation: 7,
            read_at: Instant::now(),
            ssid: "SHU".into(),
        };
        let lifetime = Duration::from_secs(600);
        assert!(entry.matches(key, 7, lifetime));
        // Switching networks on the same adapter/IP must reject the old SSID.
        assert!(!entry.matches(key, 8, lifetime));
        assert!(!entry.matches((9, 2, 3, [4; 8]), 7, lifetime));
        entry.read_at = Instant::now() - Duration::from_secs(61);
        assert!(entry.matches(key, 7, lifetime));
        assert!(!entry.matches(key, 7, Duration::from_secs(60)));
        entry.read_at = Instant::now() - Duration::from_secs(601);
        assert!(!entry.matches(key, 7, lifetime));
    }
}
