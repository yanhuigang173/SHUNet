//! On-demand Internet measurements. These clients never carry campus credentials
//! or change Windows proxy settings, and do not reuse the portal connection pool.
use std::{
    io::{self, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::{
    blocking::{Body, Client, Response},
    header::{ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_TYPE},
};

const DOWNLOAD_URL: &str = "https://speed.cloudflare.com/__down";
const UPLOAD_URL: &str = "https://speed.cloudflare.com/__up";
const MIB: u64 = 1024 * 1024;
const PHASE_TIME: Duration = Duration::from_secs(8);
const TOTAL_TIME: Duration = Duration::from_secs(45);
const CANCELLED: &str = "测速已取消（手动取消或网络变化）。";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Direct,
    Proxy,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "直连（不使用 HTTP 代理）",
            Self::Proxy => "通过代理",
        }
    }
    fn index(self) -> usize {
        match self {
            Self::Direct => 0,
            Self::Proxy => 1,
        }
    }
}

pub(crate) struct Route {
    proxy: Option<reqwest::Url>,
}
impl Route {
    pub(crate) fn capture(mode: Mode, fallback: &str) -> Result<Self, String> {
        if mode == Mode::Direct {
            return Ok(Self { proxy: None });
        }
        // Explicit proxy measurements also work when the Windows toggle is off.
        // Never silently fall back to a direct connection on missing/failed proxy.
        let (_, server) = crate::platform::read_proxy()
            .map_err(|_| "无法读取系统代理配置，请先在 Windows 代理设置中配置地址。".to_owned())?;
        let server = if server.trim().is_empty() {
            fallback
        } else {
            &server
        };
        let uniform_url = server.trim().split_once("://").is_some_and(|(scheme, _)| {
            matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "socks5" | "socks5h"
            )
        });
        let address = if !uniform_url && server.contains('=') {
            server
                .split(';')
                .find_map(|entry| {
                    let (protocol, address) = entry.split_once('=')?;
                    (protocol.trim().eq_ignore_ascii_case("https") && !address.trim().is_empty())
                        .then_some(address.trim())
                })
                .ok_or("当前分协议代理没有 HTTPS 地址，无法执行代理测速。")?
        } else {
            server.trim()
        };
        if address.is_empty() {
            return Err("没有可用的代理地址，请先配置并启动代理软件。".into());
        }
        let address = if address.contains("://") {
            address.to_owned()
        } else {
            format!("http://{address}")
        };
        let url = reqwest::Url::parse(&address).map_err(|_| "代理地址格式无效。")?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
            || url.host_str().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err("代理地址需为 HTTP / HTTPS / SOCKS5 地址，例如 127.0.0.1:7897。".into());
        }
        Ok(Self { proxy: Some(url) })
    }

    pub(crate) fn client(&self) -> Result<Client, String> {
        let mut builder = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("SHUNet/", env!("CARGO_PKG_VERSION")))
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(ACCEPT_ENCODING, "identity".parse().unwrap());
                headers.insert(CACHE_CONTROL, "no-cache, no-store".parse().unwrap());
                headers
            });
        if let Some(address) = &self.proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(address.as_str()).map_err(|_| "无法使用配置的代理地址。")?,
            );
        }
        builder
            .build()
            .map_err(|_| "无法创建 HTTPS 测速客户端。".into())
    }
    pub(crate) fn label(&self) -> String {
        match &self.proxy {
            Some(url) => format!(
                "代理：{}://{}:{}（代理软件仍可按规则分流）",
                url.scheme(),
                url.host_str().unwrap_or("—"),
                url.port_or_known_default().unwrap_or(1080)
            ),
            None => "直连：跳过 HTTP 代理；TUN / VPN / USB 共享仍由系统路由决定".into(),
        }
    }
}

#[derive(Clone, Default)]
pub struct Report {
    pub mode: Mode,
    pub stage: String,
    pub progress: f32,
    pub latency_ms: Option<f64>,
    pub download_mbps: Option<f64>,
    pub upload_mbps: Option<f64>,
    pub downloaded: u64,
    pub uploaded: u64,
    pub finished_at: String,
    pub elapsed: f64,
    pub error: Option<String>,
    pub route: String,
    pub started_at: String,
    pub live_download_mbps: Option<f64>,
    pub live_upload_mbps: Option<f64>,
    upload_progress: Option<Arc<UploadProgress>>,
}
struct UploadProgress {
    start: Instant,
    base: u64,
    sent: Arc<AtomicU64>,
}
impl Report {
    fn refresh_live(&mut self) {
        if let Some(progress) = &self.upload_progress {
            self.uploaded = progress.base + progress.sent.load(Ordering::Relaxed);
            self.live_upload_mbps = Some(
                self.uploaded as f64 * 8.0
                    / progress.start.elapsed().as_secs_f64().max(0.001)
                    / 1_000_000.0,
            );
            self.progress = 0.6
                + (progress.start.elapsed().as_secs_f32() / 8.0)
                    .max(self.uploaded as f32 / (8 * MIB) as f32)
                    .min(0.99)
                    * 0.38;
        }
    }
    pub fn compact(&self) -> String {
        let speed = |value: Option<f64>| value.map_or("—".into(), |n| format!("{n:.1}"));
        format!(
            "{} {}  下载 {} / 上传 {} Mbps  [{}]",
            self.finished_at,
            self.mode.label(),
            speed(self.download_mbps),
            speed(self.upload_mbps),
            if self.error.is_some() {
                "未完成"
            } else {
                "完成"
            }
        )
    }
    pub fn text(&self) -> String {
        let latency = self
            .latency_ms
            .map_or_else(|| "未完成".into(), |n| format!("{n:.0} ms"));
        let speed = |n: Option<f64>| n.map_or_else(|| "未完成".into(), |n| format!("{n:.1} Mbps"));
        format!(
            "{} · {}\r\n{}\r\nHTTP 延迟：{}\r\n下载：{}\r\n上传：{}\r\n载荷流量：下载 {:.1} / 上传 {:.1} MiB\r\n耗时：{:.1} 秒\r\n{}\r\n\r\n测速服务：Cloudflare。结果反映当前出口到测速节点的表现。直连仍受 TUN / 系统路由影响。",
            self.mode.label(),
            self.finished_at,
            self.route,
            latency,
            speed(self.download_mbps),
            speed(self.upload_mbps),
            self.downloaded as f64 / MIB as f64,
            self.uploaded as f64 / MIB as f64,
            self.elapsed,
            self.error.as_deref().unwrap_or("测速完成")
        )
    }
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub running: bool,
    pub cancelling: bool,
    pub current: Report,
    pub reports: [Option<Report>; 2],
    pub notice: Option<(u64, String)>,
    pub started: Option<Instant>,
    pub cancel_reason: Option<String>,
}
impl Snapshot {
    pub fn progress_label(&self) -> String {
        if self.cancelling {
            "正在取消测速，等待当前请求结束…".into()
        } else {
            format!(
                "{} · {} {:.0}%",
                self.current.mode.label(),
                self.current.stage,
                self.current.progress * 100.0
            )
        }
    }
    pub fn results(&self) -> Option<String> {
        let reports: Vec<_> = self.reports.iter().flatten().map(Report::text).collect();
        (!reports.is_empty()).then(|| reports.join("\r\n\r\n─────────────────\r\n\r\n"))
    }
}

pub struct Manager {
    shared: Arc<Mutex<Snapshot>>,
    cancel: Arc<AtomicBool>,
    serial: u64,
    window: usize,
    event: u32,
}
impl Manager {
    pub fn new(window: usize, event: u32) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Snapshot::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            serial: 0,
            window,
            event,
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = self
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if snapshot.running {
            snapshot.current.refresh_live();
            if let Some(started) = snapshot.started {
                snapshot.current.elapsed = started.elapsed().as_secs_f64();
            }
        }
        snapshot
    }
    pub fn cancel(&self) {
        self.cancel_for("测速已手动取消。");
    }
    pub fn cancel_for(&self, reason: &str) {
        let mut state = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if state.running {
            self.cancel.store(true, Ordering::Release);
            state.cancelling = true;
            if state.cancel_reason.is_none() {
                state.cancel_reason = Some(reason.to_owned());
            }
        }
    }
    pub fn start(&mut self, mode: Mode, fallback: &str) -> Result<(), String> {
        if self.snapshot().running {
            return Err("正在测速，请先取消或等待当前测速完成。".into());
        }
        let route = Route::capture(mode, fallback)?;
        self.serial = self.serial.wrapping_add(1);
        let serial = self.serial;
        self.cancel.store(false, Ordering::Release);
        let initial = Report {
            mode,
            stage: "连接测速服务…".into(),
            route: route.label(),
            started_at: chrono::Local::now().format("%H:%M:%S").to_string(),
            ..Default::default()
        };
        {
            let mut state = self.shared.lock().unwrap_or_else(|p| p.into_inner());
            state.running = true;
            state.cancelling = false;
            state.current = initial.clone();
            state.notice = None;
            state.started = Some(Instant::now());
            state.cancel_reason = None;
        }
        let shared = self.shared.clone();
        let cancel = self.cancel.clone();
        let (window, event) = (self.window, self.event);
        let spawned = std::thread::Builder::new()
            .name("shunet-speedtest".into())
            .stack_size(1024 * 1024)
            .spawn(move || {
                let started = Instant::now();
                let mut report = initial;
                let mut last_emit = Instant::now();
                let mut last_stage = String::new();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    measure(&route, &cancel, &mut report, |report| {
                        if report.stage == last_stage
                            && report.upload_progress.is_none()
                            && last_emit.elapsed() < Duration::from_millis(400)
                        {
                            return;
                        }
                        shared.lock().unwrap_or_else(|p| p.into_inner()).current = report.clone();
                        wake(window, event);
                        last_emit = Instant::now();
                        last_stage = report.stage.clone();
                    })
                }))
                .unwrap_or_else(|_| Err("测速任务异常，已结束；可稍后重新测速。".into()));
                report.elapsed = started.elapsed().as_secs_f64();
                report.refresh_live();
                report.upload_progress = None;
                report.finished_at = chrono::Local::now().format("%m-%d %H:%M:%S").to_string();
                report.error = result.err();
                {
                    let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
                    // Serialize completion with cancel(), including the final
                    // instant after the last response has been read.
                    if cancel.load(Ordering::Acquire) {
                        report.error = Some(
                            state
                                .cancel_reason
                                .clone()
                                .unwrap_or_else(|| CANCELLED.into()),
                        );
                    }
                    report.stage = if report.error.is_some() {
                        "测速未完成"
                    } else {
                        "测速完成"
                    }
                    .into();
                    let text = report.text();
                    state.running = false;
                    state.cancelling = false;
                    state.reports[mode.index()] = Some(report.clone());
                    state.current = report;
                    state.notice = Some((serial, text));
                }
                wake(window, event);
                // The thread and its HTTPS client are released after this run.
            });
        if spawned.is_err() {
            self.shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .running = false;
            return Err("无法创建测速任务，请稍后重试。".into());
        }
        Ok(())
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn wake(window: usize, event: u32) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(window as _, event, 0, 0);
    }
}

fn remaining(cancel: &AtomicBool, deadline: Instant) -> Result<Duration, String> {
    if cancel.load(Ordering::Acquire) {
        return Err(CANCELLED.into());
    }
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err("测速超过 45 秒，已停止；已完成阶段保留。".into());
    }
    Ok(left.min(Duration::from_secs(8)))
}

fn measure(
    route: &Route,
    cancel: &Arc<AtomicBool>,
    report: &mut Report,
    mut update: impl FnMut(&Report),
) -> Result<(), String> {
    let deadline = Instant::now() + TOTAL_TIME;
    let client = route.client()?;
    report.stage = "HTTP 延迟".into();
    let mut samples = Vec::with_capacity(5);
    // Warm up once; use the median of five reused-connection HTTP round trips.
    for index in 0..6 {
        let timeout = remaining(cancel, deadline)?;
        let start = Instant::now();
        let response = download_request(&client, 0, timeout)?;
        let mut bytes = Vec::new();
        response
            .take(1025)
            .read_to_end(&mut bytes)
            .map_err(|_| "读取延迟响应失败。")?;
        if !bytes.is_empty() {
            return Err("测速服务返回异常内容，无法测量延迟。".into());
        }
        if index > 0 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        report.progress = (index + 1) as f32 / 6.0 * 0.15;
        update(report);
    }
    samples.sort_by(f64::total_cmp);
    report.latency_ms = Some(samples[2]);
    for upload in [false, true] {
        let limit = if upload { 8 * MIB } else { 32 * MIB };
        let start = Instant::now();
        let mut completed = 0;
        let mut chunk: u64 = 64 * 1024;
        report.stage = if upload { "上传" } else { "下载" }.into();
        update(report);
        while completed < limit && start.elapsed() < PHASE_TIME {
            let timeout = remaining(cancel, deadline)?;
            let size = chunk.min(limit - completed);
            let request_start = Instant::now();
            if upload {
                let sent = Arc::new(AtomicU64::new(0));
                report.upload_progress = Some(Arc::new(UploadProgress {
                    start,
                    base: completed,
                    sent: sent.clone(),
                }));
                update(report);
                let body = UploadBody {
                    remaining: size,
                    random: nonce(),
                    cancel: cancel.clone(),
                    sent: sent.clone(),
                };
                let response = client
                    .post(UPLOAD_URL)
                    .timeout(timeout)
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(Body::sized(body, size))
                    .send();
                report.uploaded = completed + sent.load(Ordering::Relaxed);
                report.refresh_live();
                report.upload_progress = None;
                remaining(cancel, deadline)?;
                let response = response.map_err(|e| request_error("上传测速", &e))?;
                check_response(&response)?;
                let mut ack = Vec::new();
                response
                    .take(4097)
                    .read_to_end(&mut ack)
                    .map_err(|_| "读取上传确认失败。")?;
                if ack.len() > 4096 || sent.load(Ordering::Relaxed) != size {
                    return Err("上传响应异常，本次上传速度无效。".into());
                }
            } else {
                let mut response = download_request(&client, size, timeout)?;
                let mut received = 0;
                let mut buffer = [0u8; 64 * 1024];
                while received < size {
                    remaining(cancel, deadline)?;
                    let wanted = buffer.len().min((size - received) as usize);
                    let count = response
                        .read(&mut buffer[..wanted])
                        .map_err(|_| "下载测速中断或超时，本次下载速度无效。")?;
                    if count == 0 {
                        return Err("下载数据不完整，本次下载速度无效。".into());
                    }
                    received += count as u64;
                    report.downloaded = completed + received;
                    phase_progress(report, upload, start, completed + received, limit);
                    update(report);
                }
            }
            completed += size;
            if upload {
                report.uploaded = completed;
            } else {
                report.downloaded = completed;
            }
            phase_progress(report, upload, start, completed, limit);
            update(report);
            chunk = ((size as f64 / request_start.elapsed().as_secs_f64().max(0.001)) as u64)
                .clamp(64 * 1024, if upload { 2 * MIB } else { 8 * MIB });
        }
        remaining(cancel, deadline)?;
        let mbps = completed as f64 * 8.0 / start.elapsed().as_secs_f64().max(0.001) / 1_000_000.0;
        if upload {
            report.upload_mbps = Some(mbps);
        } else {
            report.download_mbps = Some(mbps);
        }
        update(report);
    }
    remaining(cancel, deadline)?;
    report.progress = 1.0;
    Ok(())
}

fn phase_progress(report: &mut Report, upload: bool, start: Instant, bytes: u64, limit: u64) {
    let rate = bytes as f64 * 8.0 / start.elapsed().as_secs_f64().max(0.001) / 1_000_000.0;
    if upload {
        report.live_upload_mbps = Some(rate);
    } else {
        report.live_download_mbps = Some(rate);
    }
    let fraction = (bytes as f32 / limit as f32)
        .max(start.elapsed().as_secs_f32() / PHASE_TIME.as_secs_f32())
        .min(0.99);
    report.progress = if upload {
        0.6 + fraction * 0.38
    } else {
        0.15 + fraction * 0.45
    };
}
fn download_request(client: &Client, bytes: u64, timeout: Duration) -> Result<Response, String> {
    let response = client
        .get(DOWNLOAD_URL)
        .timeout(timeout)
        .query(&[("bytes", bytes), ("t", nonce())])
        .send()
        .map_err(|e| request_error("下载测速", &e))?;
    check_response(&response)?;
    if response.content_length().is_some_and(|n| n != bytes)
        || response
            .headers()
            .get("content-encoding")
            .is_some_and(|v| v != "identity")
    {
        return Err("测速响应大小或编码异常，请检查认证状态和代理。".into());
    }
    Ok(response)
}
fn check_response(response: &Response) -> Result<(), String> {
    if !response.status().is_success() {
        return Err(format!(
            "测速服务返回 HTTP {}，请稍后重试。",
            response.status().as_u16()
        ));
    }
    if response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("text/html"))
    {
        return Err("测速服务返回网页，无法产生有效测速结果。".into());
    }
    Ok(())
}
fn request_error(stage: &str, error: &reqwest::Error) -> String {
    let reason = if error.is_timeout() {
        "超时"
    } else if error.is_connect() {
        "连接失败"
    } else {
        "请求失败"
    };
    format!("{stage}{reason}；请检查认证、代理软件或外部测速服务。测速失败不代表校园网已下线。")
}
fn nonce() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        | 1
}
struct UploadBody {
    remaining: u64,
    random: u64,
    cancel: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
}
impl Read for UploadBody {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(io::Error::other(CANCELLED));
        }
        let count = buffer.len().min(self.remaining as usize);
        for chunk in buffer[..count].chunks_mut(8) {
            self.random ^= self.random << 13;
            self.random ^= self.random >> 7;
            self.random ^= self.random << 17;
            chunk.copy_from_slice(&self.random.to_le_bytes()[..chunk.len()]);
        }
        self.remaining -= count as u64;
        self.sent.fetch_add(count as u64, Ordering::Relaxed);
        Ok(count)
    }
}
