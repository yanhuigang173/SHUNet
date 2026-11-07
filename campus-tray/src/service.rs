use crate::{
    connection_state::ConnectionState,
    detection::DetectionLevel,
    hotkeys::{self, Binding},
    network::{self, CampusPortalState, ConnectStatus, WifiNetwork},
    network_policy::NetworkMode,
    operator::Operator,
    platform,
    reconnect::ReconnectMonitor,
    settings::{Settings, SettingsStore},
    websites::{self, Website},
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicU64, Ordering},
    mpsc::{self, RecvTimeoutError, SyncSender},
};

/// Presentation of a read, independent from its eventual authentication result.
#[derive(Clone, Default)]
pub struct QueryActivity {
    pub serial: u64,
    pub active: bool,
    pub manual: bool,
    pub previous_state: ConnectionState,
    pub ssid: String,
    pub ip: String,
}

#[derive(Clone)]
pub struct View {
    pub operator: Operator,
    pub menu_theme: crate::menu_theme::MenuTheme,
    pub ssid: String,
    pub ip: String,
    pub status: String,
    pub state: ConnectionState,
    pub online_session: crate::online_session::OnlineSession,
    pub state_detail: String,
    pub query: QueryActivity,
    pub checked_at: String,
    pub confirmed_at: String,
    pub busy: bool,
    pub scanning: bool,
    pub scan_detail: String,
    pub scan_cached: bool,
    pub scanned_at: String,
    pub auto_auth: bool,
    pub auto_reconnect: bool,
    pub status_detection: bool,
    pub detection_level: DetectionLevel,
    pub detection_interval: u64,
    pub detection_reason: String,
    pub next_detection: Option<std::time::Instant>,
    pub proxy: bool,
    pub proxy_server: String,
    pub network_mode: NetworkMode,
    pub campus: bool,
    pub auth_paused: bool,
    pub username: String,
    pub networks: Vec<WifiNetwork>,
    pub websites: Vec<Website>,
    pub hotkeys: Vec<Binding>,
    pub logs: VecDeque<String>,
    pub history: Arc<crate::runtime_history::Entries>,
    pub reconnect_today: crate::runtime_history::DailyStats,
    pub history_warning: Option<String>,
    pub notice: Option<(u64, String)>,
}
impl View {
    pub fn network_type_text(&self) -> String {
        if network::is_enterprise_campus_ssid(&self.ssid) { return "网络类型：校园网 · 802.1X".into(); }
        if self.ssid.is_empty() { return "网络类型：未连接校园 Wi-Fi".into(); }
        if !self.campus { return "网络类型：普通网络".into(); }
        if let Some(operator) = self.online_session.operator() {
            format!("网络类型：{}", operator.label())
        } else if self.online_session.has_session() {
            "网络类型：校园门户（运营商待确认）".into()
        } else {
            format!("网络类型：{}（待认证）", self.operator.label())
        }
    }

    pub fn online_duration_text(&self) -> String {
        if !self.campus || self.ssid.is_empty() { return "本次在线时长：—".into(); }
        let live = self.status_detection && (self.state == ConnectionState::Online
            || (self.query.active && self.query.ssid == self.ssid && self.query.ip == self.ip
                && self.query.previous_state == ConnectionState::Online));
        self.online_session.duration_text(live)
    }
}

impl Default for View {
    fn default() -> Self {
        Self {
            operator: Operator::default(),
            menu_theme: Default::default(),
            ssid: String::new(),
            ip: "—".to_owned(),
            status: "正在读取网络状态…".to_owned(),
            state: ConnectionState::Checking,
            online_session: crate::online_session::OnlineSession::default(),
            state_detail: "等待读取物理 Wi-Fi 与校园门户状态".into(),
            query: QueryActivity::default(),
            checked_at: "尚未检测".into(),
            confirmed_at: String::new(),
            busy: false,
            scanning: true,
            scan_detail: "等待启动 Wi-Fi 扫描".into(),
            scan_cached: true,
            scanned_at: String::new(),
            auto_auth: false,
            auto_reconnect: false,
            status_detection: true,
            detection_level: DetectionLevel::default(),
            detection_interval: 0,
            detection_reason: "等待首次查询".into(),
            next_detection: None,
            proxy: false,
            proxy_server: String::new(),
            network_mode: NetworkMode::Auto,
            campus: false,
            auth_paused: false,
            username: String::new(),
            networks: Vec::new(),
            websites: Vec::new(),
            hotkeys: Vec::new(),
            logs: VecDeque::new(),
            history: Arc::default(),
            reconnect_today: crate::runtime_history::DailyStats::default(),
            history_warning: None,
            notice: None,
        }
    }
}

/// Ephemeral reply for the account editor; never part of View, logs or history.
pub struct CredentialDraft {
    pub username:String,
    pub password:String,
    pub remember_password:bool,
}
impl Drop for CredentialDraft {
    fn drop(&mut self) {
        unsafe {for byte in self.password.as_mut_vec() {std::ptr::write_volatile(byte,0);}}
    }
}

pub enum Command {
    ReadCredentials {reply:SyncSender<CredentialDraft>},
    SetMenuTheme(crate::menu_theme::MenuTheme),
    SetOperator(Operator),
    Wake,
    NetworkChanged,
    Refresh,
    Scan,
    Connect(WifiNetwork),
    Disconnect,
    Login,
    Logout,
    ToggleReconnect,
    ToggleAutoAuth,
    ToggleStatusDetection,
    SetDetectionLevel(DetectionLevel),
    ToggleProxy,
    SetNetworkMode(NetworkMode),
    ResumeCampus,
    Credentials(String, String, bool),
    ImportDesktop,
    ImportFile(PathBuf),
    ExportFile(PathBuf),
    AddWebsite(Website),
    EditWebsite {
        previous: Website,
        updated: Website,
    },
    RemoveWebsite(Website),
    SaveHotkeys {
        bindings: Vec<Binding>,
        reply: SyncSender<Result<(), String>>,
    },
    Stop,
}

impl Command {
    fn changes_network(&self) -> bool {
        !matches!(
            self,
            Self::Wake
                | Self::ReadCredentials { .. }
                | Self::SetMenuTheme(_)
                | Self::NetworkChanged
                | Self::SetDetectionLevel(_)
                | Self::Scan
                | Self::Refresh
                | Self::Login
                | Self::AddWebsite(_)
                | Self::EditWebsite { .. }
                | Self::RemoveWebsite(_)
                | Self::ImportDesktop
                | Self::ImportFile(_)
                | Self::ExportFile(_)
                | Self::SaveHotkeys { .. }
        )
    }
}

#[derive(Clone)]
pub struct Controller {
    sender: SyncSender<Command>,
    cancel: Arc<AtomicU64>,
    pending_events: Arc<AtomicU8>,
    admission: Arc<Mutex<()>>,
}

#[cfg(test)]
mod refresh_queue_tests {
    use super::*;

    fn controller() -> (Controller, mpsc::Receiver<Command>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Controller {
            sender,
            cancel: Arc::new(AtomicU64::new(0)),
            pending_events: Arc::new(AtomicU8::new(0)),
            admission: Arc::new(Mutex::new(())),
        }, receiver)
    }

    #[test]
    fn rejected_network_commands_do_not_cancel_active_work() {
        let (controller, receiver) = controller();
        assert!(controller.send(Command::Scan));
        assert!(!controller.send(Command::Stop));
        assert_eq!(controller.cancel.load(Ordering::Acquire), 0);
        receiver.recv().unwrap();
        assert!(controller.send(Command::Stop));
        assert_eq!(controller.cancel.load(Ordering::Acquire), 1);
        receiver.recv().unwrap();
        drop(receiver);
        assert!(!controller.send(Command::Stop));
        assert_eq!(controller.cancel.load(Ordering::Acquire), 1);
    }

    #[test]
    fn admitted_revision_is_visible_before_worker_starts_command() {
        let (controller, receiver) = controller();
        let worker_controller = controller.clone();
        let worker = std::thread::spawn(move || {
            receiver.recv().unwrap();
            let _barrier = worker_controller.admission.lock().unwrap();
            worker_controller.cancel.load(Ordering::Acquire)
        });
        assert!(controller.send(Command::Stop));
        assert_eq!(worker.join().unwrap(), 1);
    }

    #[test]
    fn repeated_clicks_keep_one_pending_refresh() {
        let (controller, receiver) = controller();
        for _ in 0..20 {
            assert!(controller.send(Command::Refresh));
        }
        assert!(controller.refresh_queued());
        assert!(matches!(receiver.try_recv(), Ok(Command::NetworkChanged)));
        assert!(receiver.try_recv().is_err());
        assert_eq!(controller.pending_events.swap(0, Ordering::AcqRel), 4);
        assert!(!controller.refresh_queued());
    }

    #[test]
    fn full_command_queue_does_not_lose_the_refresh_request() {
        let (controller, receiver) = controller();
        controller.sender.try_send(Command::Scan).ok().unwrap();
        assert!(controller.send(Command::Refresh));
        assert!(controller.refresh_queued());
        assert!(matches!(receiver.try_recv(), Ok(Command::Scan)));
        assert_eq!(controller.pending_events.swap(0, Ordering::AcqRel), 4);
    }

    #[test]
    fn stopped_worker_does_not_report_a_request_as_accepted() {
        let (controller, receiver) = controller();
        drop(receiver);
        assert!(!controller.send(Command::Refresh));
        assert!(!controller.refresh_queued());
    }
}

impl Controller {
    #[cfg(test)]
    pub fn isolated_test_channel() -> (Self, mpsc::Receiver<Command>) {
        let (sender, receiver) = mpsc::sync_channel(8);
        (Self { sender, cancel: Arc::new(AtomicU64::new(0)), pending_events: Arc::new(AtomicU8::new(0)), admission: Arc::new(Mutex::new(())) }, receiver)
    }

    pub fn read_credentials(&self)->Result<mpsc::Receiver<CredentialDraft>,String> {
        let (reply,receiver)=mpsc::sync_channel(1);
        if self.send(Command::ReadCredentials {reply}) {Ok(receiver)} else {Err("账号读取请求未加入后台队列，请稍后重试。".into())}
    }

    pub fn refresh_queued(&self) -> bool {
        self.pending_events.load(Ordering::Acquire) & 4 != 0
    }

    pub fn send(&self, command: Command) -> bool {
        if matches!(
            &command,
            Command::Wake | Command::NetworkChanged | Command::Refresh
        ) {
            let bit = match &command {
                Command::Wake => 2,
                Command::Refresh => 4,
                _ => 1,
            };
            if self.pending_events.fetch_or(bit, Ordering::AcqRel) == 0 {
                // The flag survives a full queue; any queued command wakes the worker.
                if matches!(self.sender.try_send(Command::NetworkChanged), Err(mpsc::TrySendError::Disconnected(_))) {
                    self.pending_events.fetch_and(!bit, Ordering::AcqRel);
                    return false;
                }
            }
            return true;
        }
        // Publish cancellation only after admission. The worker crosses the same
        // short barrier before processing a received command, so it cannot start
        // that command before its cancellation revision has been published.
        let Ok(_admission) = self.admission.lock() else { return false };
        let changes_network = command.changes_network();
        match self.sender.try_send(command) {
            Ok(()) => {
                if changes_network { self.cancel.fetch_add(1, Ordering::AcqRel); }
                true
            },
            Err(_) => false,
        }
    }
}

pub fn start(view: Arc<Mutex<View>>, window: usize, event: u32) -> Result<Controller, String> {
    let (sender, receiver) = mpsc::sync_channel(8);
    // Perform exactly one startup scan through the normal worker path, including
    // its panic recovery. No scan loop or additional background thread is needed.
    sender
        .try_send(Command::Scan)
        .map_err(|_| "无法安排启动 Wi-Fi 扫描。".to_owned())?;
    let monitor = ReconnectMonitor::new(false);
    let cancel = monitor.cancellation_counter();
    let pending_events = Arc::new(AtomicU8::new(0));
    let worker_events = pending_events.clone();
    let admission = Arc::new(Mutex::new(()));
    let worker_admission = admission.clone();
    std::thread::Builder::new()
        .name("campus-tray-worker".into())
        .stack_size(1024 * 1024)
        .spawn(move || {
            let store = SettingsStore::new();
            let (mut settings, warning) = if store.exists() {
                store.load()
            } else {
                let (mut settings, warning) = SettingsStore::desktop().load();
                settings.auto_reconnect = false;
                let _ = store.save(&settings);
                (settings, warning)
            };
            // The tray instance owns its preferences independently after the first import.
            if settings.proxy_server.trim().is_empty() {
                settings.proxy_server = "127.0.0.1:7890".to_owned();
            }
            let mut service = Service {
                store,
                settings,
                monitor,
                probe: network::CampusProbe::default(),
                view: View::default(),
                shared: view,
                window,
                event,
                paused_ssid: String::new(),
                auto_auth_ssid: String::new(),
                recovery: false,
                had_online: false,
                portal_seen: false,
                manual_check_pending: false,
                session_ssid: String::new(),
                initial_attempts: 0,
                scan_task: None,
                wifi_index: None,
                next_local_check: std::time::Instant::now() + std::time::Duration::from_secs(60),
                verification: None,
                owns_auth_trace: false,
                history: crate::runtime_history::History::open(crate::runtime_history::folder()),
                pending_reconnect: None,
            };
            service.monitor.set_level(service.settings.detection_level);
            service
                .monitor
                .enable(service.settings.auto_auth || service.settings.auto_reconnect);
            service.log("SHUNet 已启动。", false);
            if let Some(warning) = warning {
                service.log(&warning, false);
            }
            service.refresh_connection();
            service.publish();
            loop {
                // With detection off, block until a command or native network event.
                // Only an explicitly requested single check may wake the probe timer.
                let monitoring = service.should_monitor();
                let mut delay = if monitoring {
                    service.monitor.next_delay()
                } else {
                    // Local-only fallback if Windows notifications are missed.
                    std::time::Duration::from_secs(60)
                };
                delay = delay.min(
                    service
                        .next_local_check
                        .saturating_duration_since(std::time::Instant::now())
                        .max(std::time::Duration::from_millis(200)),
                );
                if service.scan_task.is_some() {
                    delay = delay.min(std::time::Duration::from_millis(250));
                }
                let command = receiver.recv_timeout(delay);
                // Never hold admission while doing network or file operations.
                drop(worker_admission.lock().unwrap_or_else(|error| error.into_inner()));
                let events = worker_events.swap(0, Ordering::AcqRel);
                if events != 0
                    && !matches!(
                        &command,
                        Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected)
                    )
                {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if events & 3 != 0 {
                            service.command(if events & 2 != 0 {
                                Command::Wake
                            } else {
                                Command::NetworkChanged
                            });
                        }
                        if events & 4 != 0 {
                            service.command(Command::Refresh);
                        }
                    }));
                    if outcome.is_err() {
                        service.view.query.active = false;
                        service.manual_check_pending = false;
                        service.monitor.finish_probe(service.monitor.generation);
                        service.monitor.check_failed();
                        service.log("状态刷新任务异常，将重新查询。", false);
                    }
                }
                match command {
                    Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                    Ok(Command::NetworkChanged) => {}
                    Ok(command) => {
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                service.command(command)
                            }));
                        if outcome.is_err() {
                            service.view.query.active = false;
                            service.view.busy = false;
                            service.view.scanning = false;
                            service.scan_task = None;
                            service.view.state = ConnectionState::Checking;
                            service.view.state_detail = "后台任务已复位，等待重新检测".into();
                            service.monitor.finish_probe(service.monitor.generation);
                            service.probe = network::CampusProbe::default();
                            service.manual_check_pending = false;
                            service.monitor.check_failed();
                            service.log("后台任务已复位，稍后自动重试。", true);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if std::time::Instant::now() >= service.next_local_check {
                    service.refresh_connection();
                }
                service.history.maintain(crate::runtime_history::now_ms());
                if service
                    .scan_task
                    .as_ref()
                    .is_some_and(crate::wifi::ScanTask::is_ready)
                {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        service.complete_scan()
                    }));
                    if outcome.is_err() {
                        service.scan_task = None;
                        service.view.scanning = false;
                        service.view.scan_cached = true;
                        service.view.scan_detail =
                            "扫描任务异常，已保留原列表，可重新刷新。".into();
                        service.log("扫描任务异常，已保留原列表，可重新刷新。", false);
                    }
                }
                if !service.view.busy && service.scan_task.is_none() && service.should_monitor() {
                    if service.monitor.guard().check().is_err() {
                        service.monitor.invalidate();
                    }
                    let outcome =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| service.poll()));
                    if outcome.is_err() {
                        service.view.query.active = false;
                        service.view.busy = false;
                        service.view.scanning = false;
                        service.view.state = ConnectionState::Checking;
                        service.view.state_detail = "检测任务已复位，等待重新检测".into();
                        service.monitor.finish_probe(service.monitor.generation);
                        service.probe = network::CampusProbe::default();
                        service.manual_check_pending = false;
                        service.monitor.check_failed();
                        service.log("守护任务已复位，稍后自动重试。", true);
                    }
                }
                if !service.settings.status_detection && !service.manual_check_pending {
                    // Drop cookies, sockets and the cached HTTP client while suspended.
                    service.probe = network::CampusProbe::default();
                }
                service.publish();
            }
        })
        .map_err(|_| "无法创建网络后台线程。".to_owned())?;
    Ok(Controller {
        sender,
        cancel,
        pending_events,
        admission,
    })
}

#[cfg(test)]
mod authentication_benchmark {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "Explicit before/after live campus benchmark: may log out campus, then logs in once"]
    fn live_full_authentication_benchmark() {
        assert_eq!(std::env::var("SHUNET_BENCH").as_deref(), Ok("1"));
        let output = PathBuf::from(std::env::var_os("SHUNET_BENCH_OUTPUT").expect("report path"));
        let store = SettingsStore::new();
        let (mut settings, warning) = store.load();
        assert!(warning.is_none(), "settings warning; benchmark cancelled");
        assert!(settings.operator == Operator::Campus && !settings.username.is_empty() && !settings.password.is_empty(), "saved campus-only credentials required");
        let (ssid, ip, index) = network::current_wifi_connection().expect("physical Wi-Fi");
        assert!(network::supports_campus_portal(&ssid));
        let status_detection = settings.status_detection;
        let level = settings.detection_level;
        let mut probe = network::CampusProbe::default();
        let monitor = ReconnectMonitor::new(false);
        let initial = probe.check(&monitor.guard(), true).expect("initial portal state");
        if matches!(initial.state, CampusPortalState::Online | CampusPortalState::Verifying) {
            network::benchmark_logout_online_session().expect("campus logout");
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if probe.check(&monitor.guard(), true).is_ok_and(|check| matches!(check.state, CampusPortalState::Offline)) { break; }
            assert!(Instant::now() < deadline, "confirmed offline required before timing");
            std::thread::sleep(Duration::from_secs(2));
        }
        // Isolate a single manual login without changing saved preferences.
        settings.auto_auth = false;
        settings.auto_reconnect = false;
        if let Some(preference) = settings.network_preferences.get_mut(&ssid) { preference.paused = false; }
        let mut service = Service {
            store, settings, monitor: ReconnectMonitor::new(false),
            probe: network::CampusProbe::default(), view: View::default(),
            shared: Arc::new(Mutex::new(View::default())), window: 0, event: 0,
            paused_ssid: String::new(), auto_auth_ssid: String::new(), recovery: false,
            had_online: false, portal_seen: false, manual_check_pending: false,
            session_ssid: String::new(), initial_attempts: 0, scan_task: None,
            wifi_index: None, next_local_check: Instant::now(),
            verification: None, owns_auth_trace: false,
            history: crate::runtime_history::History::memory(), pending_reconnect: None,
        };
        service.view.scanning = false;
        service.monitor.set_level(level);
        crate::auth_trace::start();
        let started = Instant::now();
        assert!(service.refresh_connection());
        service.monitor.enable(true);
        service.authenticate(false);
        let accepted_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut rounds = 0;
        let confirmation_timing = crate::auth_trace::phase("confirmation");
        while service.view.state != ConnectionState::Online && started.elapsed() < Duration::from_secs(90) {
            if service.view.state == ConnectionState::AuthenticationFailed || !service.should_monitor() { break; }
            std::thread::sleep(service.monitor.next_delay().min(Duration::from_secs(5)));
            service.poll();
            rounds += 1;
        }
        drop(confirmation_timing);
        let timing = crate::auth_trace::finish().expect("timing active");
        let online = service.view.state == ConnectionState::Online;
        let unchanged = network::current_wifi_connection().is_ok_and(|(now_ssid, now_ip, now_index)| now_ssid == ssid && now_ip == ip && now_index == index);
        let report = serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"), "timestamp": chrono::Local::now().to_rfc3339(),
            "measurement": "serial service manual authentication, release build; logout/setup excluded",
            "source_ip": ip, "wifi_index": index, "identity_unchanged": unchanged,
            "status_detection": status_detection, "operator": "campus", "confirmed_offline_start": true,
            "online": online, "confirmation_iterations": rounds, "accepted_ms": accepted_ms, "timing": timing,
        });
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).expect("save sanitized benchmark");
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert!(online && unchanged, "authentication did not finish on the same physical campus interface");
        assert_eq!(report["timing"]["requests"]["login"], 1, "exactly one credential POST expected");
    }
}

struct Verification {
    started: std::time::Instant,
    guard: crate::reconnect::OperationGuard,
    wifi_generation: u64,
    round: usize,
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn service() -> Service {
        let monitor = ReconnectMonitor::new(false);
        let verification = Verification { started: Instant::now(), guard: monitor.guard(), wifi_generation: crate::wifi::connection_generation(), round: 0 };
        Service {
            store: SettingsStore::new(), settings: Settings { status_detection: false, auto_auth: false, auto_reconnect: false, ..Settings::default() },
            monitor, probe: network::CampusProbe::default(), view: View::default(),
            shared: Arc::new(Mutex::new(View::default())), window: 0, event: 0,
            paused_ssid: String::new(), auto_auth_ssid: String::new(), recovery: false,
            had_online: false, portal_seen: false, manual_check_pending: false,
            session_ssid: String::new(), initial_attempts: 0, scan_task: None,
            wifi_index: None, next_local_check: Instant::now(),
            verification: Some(verification), owns_auth_trace: false,
            history: crate::runtime_history::History::memory(), pending_reconnect: None,
        }
    }

    #[test]
    fn credential_editor_reply_uses_current_memory_without_touching_network() {
        let mut service=service();
        service.settings.username="synthetic-account".into();service.settings.password="temporary-test-secret".into();service.settings.remember_password=false;
        let generation=service.monitor.generation;
        let started=service.verification.as_ref().unwrap().started;
        let logs=service.history.snapshot().len();
        let (reply,receiver)=mpsc::sync_channel(1);
        let command=Command::ReadCredentials {reply};assert!(!command.changes_network());
        service.command(command);
        let draft=receiver.try_recv().unwrap();
        assert!(draft.username=="synthetic-account" && draft.password=="temporary-test-secret" && !draft.remember_password);
        assert_eq!(service.monitor.generation,generation);assert_eq!(service.verification.as_ref().unwrap().started,started);
        assert_eq!(service.history.snapshot().len(),logs);
        drop(receiver);
        let (reply,receiver)=mpsc::sync_channel(1);drop(receiver);service.command(Command::ReadCredentials {reply});
        assert!(service.settings.password=="temporary-test-secret");
    }

    #[test]
    fn credential_save_round_trip_and_failure_keep_existing_credentials() {
        let mut service=service();
        let folder=std::env::temp_dir().join(format!("SHUNetCredentialsTest-{}",std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();let path=folder.join("settings.json");
        service.store=SettingsStore::test_at(path.clone());
        service.command(Command::Credentials("synthetic-account".into(),"synthetic-secret".into(),true));
        let saved=service.store.load().0;
        assert!(saved.password=="synthetic-secret" && saved.remember_password);
        let json=std::fs::read_to_string(&path).unwrap();assert!(!json.contains("synthetic-secret"));
        service.command(Command::Credentials("synthetic-account".into(),"synthetic-secret".into(),false));
        assert!(service.settings.password=="synthetic-secret");assert!(service.store.load().0.password.is_empty());
        let blocker=folder.join("not-a-folder");std::fs::write(&blocker,"block").unwrap();service.store=SettingsStore::test_at(blocker.join("settings.json"));
        service.command(Command::Credentials("replacement".into(),"replacement-secret".into(),true));
        assert!(service.settings.username=="synthetic-account" && service.settings.password=="synthetic-secret" && !service.settings.remember_password);
        std::fs::remove_file(blocker).unwrap();std::fs::remove_file(path).unwrap();std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn next_operator_setting_does_not_relabel_current_online_session() {
        let mut view = View { ssid: "Shu(ForAll)".into(), ip: "10.1.2.3".into(), campus: true,
            state: ConnectionState::Online, operator: Operator::Mobile, ..View::default() };
        view.online_session.submitted(Operator::Campus, true);
        view.online_session.confirm(Some(&crate::online_session::SessionEvidence { id: "current".into(), operator: Some(Operator::Campus) }));
        assert_eq!(view.network_type_text(), "网络类型：校园网");
        assert!(!view.online_duration_text().contains("待确认"));
        view.status_detection = false;
        assert!(view.online_duration_text().contains("待确认"));
        view.online_session.clear();
        assert_eq!(view.network_type_text(), "网络类型：中国移动（待认证）");
        assert_eq!(view.online_duration_text(), "本次在线时长：—");
    }

    #[test]
    fn reconnect_counters_only_count_sent_automatic_recoveries_once() {
        use network::AuthOutcome as Outcome;
        let mut service=service();
        service.settings.username="test-account".into();service.settings.password="test-password".into();
        assert!(service.record_reconnect_request(true,&Ok(Outcome::Accepted)).is_none(),"first auto login is not a reconnect");
        service.had_online=true;
        for result in [Err("not sent".into()),Ok(Outcome::AlreadyOnline(crate::online_session::SessionEvidence{id:"private".into(),operator:None}))] {
            assert!(service.record_reconnect_request(true,&result).is_none());
        }
        assert!(service.record_reconnect_request(false,&Ok(Outcome::Accepted)).is_none(),"manual login is excluded");
        service.pending_reconnect=service.record_reconnect_request(true,&Ok(Outcome::Accepted));
        assert_eq!(service.history.stats().attempts,1);assert_eq!(service.history.stats().recovered,0);
        service.mark_online(None);service.mark_online(None);
        assert_eq!(service.history.stats().recovered,1,"repeated online observations must not double-count");
        service.record_reconnect_request(true,&Ok(Outcome::Rejected("not entitled".into())));
        assert_eq!(service.history.stats().attempts,2);assert_eq!(service.history.stats().recovered,1);
    }

    #[test]
    fn enterprise_campus_never_authenticates_even_with_forced_campus_mode() {
        let mut service=service();service.verification=None;
        service.settings.status_detection=true;
        service.settings.network_preferences.insert("ShuWlan-1X".into(),crate::network_policy::NetworkPreference{mode:NetworkMode::Campus,paused:false});
        service.update_connection("ShuWlan-1X".into(),"10.1.2.3".into());
        assert_eq!(service.view.state,ConnectionState::EnterpriseConnected);
        assert!(!service.is_campus());assert!(!service.should_monitor());
        crate::auth_trace::start();service.authenticate(true);service.authenticate(false);
        assert!(crate::auth_trace::finish().unwrap().requests.is_empty());
        assert_eq!(service.history.stats().attempts,0);
        assert!(service.view.network_type_text().contains("802.1X"));
    }

    #[test]
    fn persisted_log_text_hides_credentials_and_control_characters() {
        let mut service=service();
        service.settings.username="test-account".into();service.settings.password="test-password".into();
        service.log("test-account\r\ntest-password\tquery failed",false);
        let snapshot=service.history.snapshot();let entry=&snapshot[0];
        assert!(!entry.text.contains("test-account")&&!entry.text.contains("test-password"));
        assert!(!entry.text.chars().any(char::is_control));
    }

    #[test]
    fn same_ssid_address_change_clears_previous_session_clock() {
        let mut service = service();
        service.verification = None;
        service.view.ssid = "Shu(ForAll)".into();
        service.view.ip = "10.1.2.3".into();
        service.view.online_session.confirm(Some(&crate::online_session::SessionEvidence { id: "old".into(), operator: Some(Operator::Campus) }));
        service.update_connection("Shu(ForAll)".into(), "10.1.2.4".into());
        assert!(!service.view.online_session.has_session());
    }

    #[test]
    #[ignore = "Read-only live session display check; never sends credentials or logout"]
    fn live_online_session_display() {
        let monitor = ReconnectMonitor::new(false);
        let mut probe = network::CampusProbe::default();
        let check = probe.check(&monitor.guard(), true).expect("physical campus session check");
        assert!(matches!(check.state, CampusPortalState::Online), "online session required for display validation");
        let mut view = View { ssid: check.ssid, ip: check.ip, campus: true, state: ConnectionState::Online, ..View::default() };
        view.online_session.confirm(check.session.as_ref());
        println!("{}", view.network_type_text());
        println!("{}", view.online_duration_text());
        assert!(view.online_session.operator().is_some(), "current portal operator should be recognized");
        std::thread::sleep(Duration::from_millis(1100));
        println!("{}", view.online_duration_text());
        assert!(!view.online_duration_text().contains("00:00:00"));
        let next = probe.check(&monitor.guard(), true).expect("repeat read");
        assert!(matches!(next.state, CampusPortalState::Online));
        view.online_session.confirm(next.session.as_ref());
        assert!(!view.online_duration_text().contains("00:00:00"), "read-only refresh must retain the session clock");
    }

    #[test]
    fn confirmed_online_wait_is_readonly_recheck_not_authentication() {
        let mut service = service();
        service.verification = None;
        service.settings.status_detection = true;
        service.update_connection("Shu(ForAll)".into(), "10.1.2.3".into());
        let evidence = crate::online_session::SessionEvidence { id: "synthetic-session".into(), operator: Some(Operator::Campus) };
        service.mark_online(Some(&evidence));
        service.view.confirmed_at = "11:15:20".into();
        let clock = service.view.online_session.duration_text(false);
        crate::auth_trace::start();
        for _ in 0..8 {
            service.apply_portal_wait();
            assert_eq!(service.view.state, ConnectionState::OnlineRechecking);
            assert!(matches!(service.view.state.icon(), crate::icons::State::Connected));
            assert!(service.verification.is_none());
            assert!(service.view.online_session.has_session());
            assert_eq!(service.view.online_session.duration_text(false), clock);
            assert_eq!(service.view.confirmed_at, "11:15:20");
            assert!(!service.monitor.offline_confirmed());
        }
        assert!(service.view.state_detail.contains("11:15:20"));
        assert_eq!(service.history.snapshot().len(), 1, "same wait episode must not flood the log");
        assert_eq!(service.history.stats().attempts, 0);
        service.mark_online(Some(&evidence));
        assert_eq!(service.view.state, ConnectionState::Online);
        assert!(matches!(service.view.state.icon(), crate::icons::State::Authenticated));
        assert_eq!(service.history.snapshot().len(), 2, "a persistent wait episode needs one recovery log");
        assert_eq!(service.view.online_session.duration_text(false), clock);
        service.mark_online(Some(&evidence));
        assert_eq!(service.history.snapshot().len(), 2, "normal online polls must not repeat recovery logs");
        assert!(crate::auth_trace::finish().unwrap().requests.is_empty());
    }

    #[test]
    fn wait_before_auth_or_after_connection_change_never_looks_like_a_submission() {
        let mut service = service();
        service.verification = None;
        service.update_connection("Shu(ForAll)".into(), "10.1.2.3".into());
        service.apply_portal_wait();
        assert_eq!(service.view.state, ConnectionState::Checking);
        assert!(matches!(service.view.state.icon(), crate::icons::State::Connected));
        service.mark_online(Some(&crate::online_session::SessionEvidence { id: "old".into(), operator: None }));
        service.update_connection("Shu(ForAll)".into(), "10.1.2.4".into());
        service.apply_portal_wait();
        assert_eq!(service.view.state, ConnectionState::Checking);
        assert!(!service.view.online_session.has_session());
        assert!(service.verification.is_none());
    }

    #[test]
    fn transient_offline_and_errors_keep_readonly_grace_without_authentication() {
        let mut service = service();
        crate::auth_trace::start();
        for state in [CampusPortalState::Offline, CampusPortalState::Verifying, CampusPortalState::WaitingAddress] {
            let result = Ok(network::CampusConnectionCheck { ssid: "Shu(ForAll)".into(), ip: "10.1.2.3".into(), state, session: None });
            assert!(service.continue_verification(&result));
            assert_eq!(service.view.state, ConnectionState::Verifying);
            assert!(!service.monitor.offline_confirmed());
            service.authenticate(false);
        }
        assert!(service.continue_verification(&Err("timeout".into())));
        assert!(service.should_monitor());
        assert!(!service.settings.status_detection);
        assert!(crate::auth_trace::finish().unwrap().requests.is_empty());
        let online = Ok(network::CampusConnectionCheck { ssid: "Shu(ForAll)".into(), ip: "10.1.2.3".into(), state: CampusPortalState::Online, session: None });
        assert!(!service.continue_verification(&online));
        service.mark_online(None);
        assert_eq!(service.view.state, ConnectionState::Online);
        assert!(!service.should_monitor());
        assert!(!service.settings.status_detection);
    }

    #[test]
    fn confirmation_timeout_stops_when_periodic_detection_is_off() {
        let mut service = service();
        service.verification.as_mut().unwrap().started -= Duration::from_secs(31);
        crate::auth_trace::start();
        service.poll();
        assert!(service.verification.is_none());
        assert!(!service.should_monitor());
        assert_eq!(service.view.state, ConnectionState::Checking);
        assert!(crate::auth_trace::finish().unwrap().requests.is_empty());
    }

    #[test]
    fn changed_revision_cancels_confirmation_without_network_requests() {
        let mut service = service();
        service.monitor.invalidate();
        crate::auth_trace::start();
        service.poll();
        assert!(service.verification.is_none());
        assert!(!service.should_monitor());
        assert!(crate::auth_trace::finish().unwrap().requests.is_empty());
    }

    #[test]
    fn theme_switch_persists_without_touching_network_or_verification() {
        let mut service=service();
        let path=std::env::temp_dir().join(format!("shunet-theme-test-{}.json",std::process::id()));
        service.store=SettingsStore::test_at(path.clone());
        service.view.online_session.confirm(None);
        let started=service.verification.as_ref().unwrap().started;
        let generation=service.monitor.generation;
        crate::auth_trace::start();
        for theme in crate::menu_theme::MenuTheme::ALL {
            assert!(!Command::SetMenuTheme(theme).changes_network());
            service.command(Command::SetMenuTheme(theme));
            service.publish();
            assert_eq!(service.view.menu_theme,theme);
            assert_eq!(service.verification.as_ref().unwrap().started,started);
            assert_eq!(service.monitor.generation,generation);
            assert!(service.view.online_session.has_session());
            if path.exists() {assert_eq!(service.store.load().0.menu_theme,theme);}
        }
        assert!(crate::auth_trace::finish().unwrap().requests.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn repeated_login_clicks_keep_same_confirmation_and_never_post_credentials() {
        let mut service = service();
        let started = service.verification.as_ref().unwrap().started;
        let generation = service.monitor.generation;
        crate::auth_trace::start();
        for _ in 0..8 { service.command(Command::Login); }
        assert_eq!(service.verification.as_ref().unwrap().started, started);
        assert_eq!(service.monitor.generation, generation);
        assert!(crate::auth_trace::finish().unwrap().requests.is_empty());
    }
}

struct Service {
    store: SettingsStore,
    settings: Settings,
    monitor: ReconnectMonitor,
    probe: network::CampusProbe,
    view: View,
    shared: Arc<Mutex<View>>,
    window: usize,
    event: u32,
    paused_ssid: String,
    auto_auth_ssid: String,
    recovery: bool,
    had_online: bool,
    portal_seen: bool,
    manual_check_pending: bool,
    session_ssid: String,
    initial_attempts: u8,
    scan_task: Option<crate::wifi::ScanTask>,
    wifi_index: Option<u32>,
    next_local_check: std::time::Instant,
    verification: Option<Verification>,
    owns_auth_trace: bool,
    history: crate::runtime_history::History,
    pending_reconnect: Option<u64>,
}
impl Service {
    fn network_mode(&self) -> NetworkMode {
        self.settings
            .network_preferences
            .get(&self.view.ssid)
            .map_or(NetworkMode::Auto, |preference| preference.mode)
    }
    fn is_campus(&self) -> bool {
        self.network_mode().allows_campus(&self.view.ssid)
    }
    fn is_paused(&self) -> bool {
        !self.view.ssid.is_empty()
            && (self.paused_ssid == self.view.ssid
                || self
                    .settings
                    .network_preferences
                    .get(&self.view.ssid)
                    .is_some_and(|p| p.paused))
    }
    fn should_monitor(&self) -> bool {
        self.verification.is_some() || self.manual_check_pending || (self.settings.status_detection && self.is_campus())
    }
    fn set_paused(&mut self, paused: bool) -> bool {
        if self.view.ssid.is_empty() {
            return false;
        }
        if !paused
            && !self
                .settings
                .network_preferences
                .contains_key(&self.view.ssid)
        {
            self.paused_ssid.clear();
            return true;
        }
        let mut candidate = self.settings.clone();
        candidate
            .network_preferences
            .entry(self.view.ssid.clone())
            .or_default()
            .paused = paused;
        if !paused
            && candidate
                .network_preferences
                .get(&self.view.ssid)
                .is_some_and(|p| p.mode == NetworkMode::Auto)
        {
            candidate.network_preferences.remove(&self.view.ssid);
        }
        if let Err(error) = self.store.save(&candidate) {
            self.notice(&format!(
                "无法保存自动认证暂停设置，本次操作未执行：{error}"
            ));
            return false;
        }
        self.settings = candidate;
        self.paused_ssid = if paused {
            self.view.ssid.clone()
        } else {
            String::new()
        };
        true
    }
    fn mark_online(&mut self, evidence: Option<&crate::online_session::SessionEvidence>) {
        let finished_recheck = self.view.state == ConnectionState::OnlineRechecking;
        if let Some(attempt)=self.pending_reconnect.take() {
            self.write_log(crate::runtime_history::Event::AutoReconnectOnline,Some(attempt),"自动重连已通过门户确认恢复在线。",false);
        }
        self.view.online_session.confirm(evidence);
        self.view.state = ConnectionState::Online;
        self.view.checked_at = chrono::Local::now().format("%H:%M:%S").to_string();
        self.view.confirmed_at = self.view.checked_at.clone();
        self.had_online = true;
        self.portal_seen = true;
        // Observing an existing browser session completes first authentication too.
        self.auto_auth_ssid = self.view.ssid.clone();
        self.monitor.online();
        self.view.state_detail = if self.is_paused() {
            "门户已确认在线；该网络的自动认证仍为手动暂停".into()
        } else if self.monitor.needs_attention() {
            "门户已确认在线；保存的凭据存在问题，自动认证暂停".into()
        } else {
            "已通过当前物理 Wi-Fi 查询并确认本机会话在线；无需重复登录".into()
        };
        self.finish_auth_trace("已确认在线");
        self.verification = None;
        if finished_recheck {
            self.log("在线状态复核完成：门户已确认认证在线。", false);
        }
    }
    fn record_reconnect_request(&mut self, automatic: bool, result: &Result<network::AuthOutcome,String>) -> Option<u64> {
        if automatic && self.had_online && !self.settings.username.trim().is_empty() && !self.settings.password.is_empty()
            && matches!(result, Ok(network::AuthOutcome::Accepted | network::AuthOutcome::Unknown(_) | network::AuthOutcome::Rejected(_))) {
            Some(self.write_log(crate::runtime_history::Event::AutoReconnectRequest,None,"自动重连：已发送认证请求，结果见后续记录。",false))
        } else { None }
    }
    fn begin_query(&mut self, manual: bool) {
        if self.view.query.active {
            return;
        }
        let previous_state = if !self.settings.status_detection && self.is_campus() {
            ConnectionState::DetectionDisabled
        } else {
            self.view.state
        };
        self.view.query = QueryActivity {
            serial: self.view.query.serial.wrapping_add(1).max(1),
            active: true,
            manual,
            previous_state,
            ssid: self.view.ssid.clone(),
            ip: self.view.ip.clone(),
        };
        self.publish();
    }
    fn publish(&mut self) {
        self.view.history=self.history.snapshot();
        self.view.reconnect_today=self.history.stats();
        self.view.history_warning=self.history.warning.clone();
        self.view.operator = self.settings.operator;
        self.view.menu_theme = self.settings.menu_theme;
        self.view.auto_auth = self.settings.auto_auth;
        self.view.auto_reconnect = self.settings.auto_reconnect;
        self.view.status_detection = self.settings.status_detection;
        self.view.detection_level = self.settings.detection_level;
        let (interval, reason, next) = self.monitor.schedule_info();
        self.view.detection_interval = interval;
        self.view.detection_reason = reason.into();
        self.view.next_detection = Some(next);
        if !self.settings.status_detection && self.verification.is_some() {
            self.view.detection_reason = "仅确认本次手动认证；定期检测仍关闭，最多 30 秒".into();
        } else if !self.settings.status_detection {
            self.view.next_detection = None;
            self.view.detection_reason = if self.view.query.active {
                "正在手动查询一次；定期检测仍关闭"
            } else {
                "定期检测已关闭，可手动查询一次"
            }
            .into();
        } else if !self.is_campus() {
            self.view.next_detection = None;
            self.view.detection_reason = if network::is_enterprise_campus_ssid(&self.view.ssid) {"802.1X 由 Windows 管理，无需门户查询"} else {"当前未连接需要检测的校园 Wi-Fi"}.into();
        } else if self.scan_task.is_some() {
            self.view.next_detection = None;
            self.view.detection_reason = "Wi-Fi 扫描结束后继续状态查询".into();
        } else if self.view.query.active {
            self.view.next_detection = None;
            self.view.detection_reason = "正在通过当前校园 Wi-Fi 只读查询".into();
        }
        self.view.username = self.settings.username.clone();
        self.view.proxy_server = self.settings.proxy_server.clone();
        self.view.network_mode = self.network_mode();
        self.view.campus = self.is_campus();
        self.view.auth_paused = self.is_paused();
        self.view.websites = self.settings.websites.clone();
        self.view.hotkeys = self.settings.hotkeys.clone();
        self.view.proxy = platform::read_proxy()
            .map(|(enabled, _)| enabled)
            .unwrap_or(false);
        let mut published = self.view.clone();
        if !self.settings.status_detection && self.is_campus() {
            if published.state == ConnectionState::Disconnected {
                published.state_detail = "状态检测已关闭；物理 Wi-Fi 未连接".into();
            } else if self.verification.is_none() && !(published.busy
                && matches!(
                    published.state,
                    ConnectionState::Authenticating | ConnectionState::SigningOut
                ))
            {
                // Keep prior evidence only as timestamped history, never as a live
                // green/red state while periodic monitoring is disabled.
                published.state = ConnectionState::DetectionDisabled;
                published.state_detail =
                    "定期检测、自动认证和自动重连已暂停；可手动认证、下线或单次检测".into();
            }
        }
        if let Ok(mut view) = self.shared.lock() {
            *view = published;
        }
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                self.window as _,
                self.event,
                0,
                0,
            );
        }
    }
    fn log(&mut self, message: &str, important: bool) {
        self.write_log(crate::runtime_history::Event::Message,None,message,important);
    }
    fn write_log(&mut self, event:crate::runtime_history::Event, attempt:Option<u64>, message:&str,important:bool)->u64 {
        let mut safe = message.to_owned();
        for secret in [&self.settings.username, &self.settings.password] {
            if !secret.is_empty() {
                safe = safe.replace(secret.as_str(), "[已隐藏]");
            }
        }
        let safe: String = safe.chars().filter(|c| !c.is_control()).take(400).collect();
        self.view.logs.push_back(format!(
            "[{}] {safe}",
            chrono::Local::now().format("%H:%M:%S")
        ));
        while self.view.logs.len() > 48 {
            self.view.logs.pop_front();
        }
        if important {
            self.view.status = safe.clone();
        }
        let id=self.history.record(event,attempt,&safe);
        self.view.history=self.history.snapshot();self.view.reconnect_today=self.history.stats();
        id
    }
    fn save(&mut self) {
        if let Err(error) = self.store.save(&self.settings) {
            self.log(&format!("设置保存失败：{error}"), true);
        }
    }
    fn notice(&mut self, message: &str) {
        self.log(message, true);
        let serial = self
            .view
            .notice
            .as_ref()
            .map_or(1, |(serial, _)| serial.wrapping_add(1));
        self.view.notice = Some((serial, message.to_owned()));
        self.publish();
    }
    fn command(&mut self, command: Command) {
        if command.changes_network() {
            self.pending_reconnect=None;
            if self.verification.is_some() { self.finish_auth_trace("连接或设置变化，已取消"); self.verification = None; }
            self.monitor.invalidate();
            self.manual_check_pending = false;
            self.view.query.active = false;
        }
        match command {
            Command::ReadCredentials {reply} => {
                let _=reply.try_send(CredentialDraft {username:self.settings.username.clone(),password:self.settings.password.clone(),remember_password:self.settings.remember_password});
                unsafe {windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(self.window as _,self.event,0,0);}
            }
            Command::SetMenuTheme(theme) => {
                if self.settings.menu_theme == theme { return; }
                let mut candidate = self.settings.clone();
                candidate.menu_theme = theme;
                if let Err(error) = self.store.save(&candidate) {
                    self.notice(&format!("菜单主题保存失败，原选择已保留：{error}"));
                    return;
                }
                self.settings = candidate;
                self.log(&format!("菜单主题已保存为{}，下次打开菜单生效。", theme.label()), false);
            }
            Command::SetOperator(operator) => {
                if self.settings.operator == operator { return; }
                let mut candidate = self.settings.clone();
                candidate.operator = operator;
                if let Err(error) = self.store.save(&candidate) {
                    self.notice(&format!("运营商保存失败，原选择已保留：{error}"));
                    return;
                }
                self.settings = candidate;
                self.probe = network::CampusProbe::default();
                self.monitor.enable(self.settings.auto_auth || self.settings.auto_reconnect);
                self.auto_auth_ssid.clear();
                self.initial_attempts = 0;
                self.log(&format!("运营商已保存为{}，下次认证生效；当前在线连接不会自动下线。", operator.label()), true);
            }
            Command::Wake => {
                crate::wifi::invalidate_ssid_cache();
                self.monitor.invalidate();
                self.probe=network::CampusProbe::default();
                self.refresh_connection();
                self.view.status = "网络变化，等待连接稳定…".into();
                if self.is_campus() {
                    self.view.state = ConnectionState::Checking;
                    self.view.state_detail = "网络发生变化，等待稳定后重新确认认证".into();
                }
            }
            Command::NetworkChanged => {
                // Most IP notifications concern USB/TUN/IPv6 or duplicate events.
                // refresh_connection invalidates evidence only on an actual change.
                self.refresh_connection();
            }
            Command::Refresh => {
                if self.manual_check_pending && self.view.query.active { return; }
                crate::wifi::invalidate_ssid_cache();
                // Publish before the native read, which can itself take time.
                // This worker is serial, so an inactive pending request is safe to retry.
                self.view.query.active = false;
                self.begin_query(true);
                let known = self.refresh_connection();
                if self.view.query.ssid != self.view.ssid || self.view.query.ip != self.view.ip {
                    // Rebind after roaming without retaining the old network's colour.
                    self.view.query.ssid = self.view.ssid.clone();
                    self.view.query.ip = self.view.ip.clone();
                    self.view.query.previous_state = self.view.state;
                }
                self.log("手动查询已接收，正在核对当前连接。", false);
                if known && !self.is_campus() {
                    self.manual_check_pending = false;
                    self.view.query.active = false;
                    self.log("手动查询完成：已更新本机连接信息；当前无需查询校园门户。", false);
                } else {
                    self.manual_check_pending = true;
                    self.monitor.request_readonly_check();
                }
            }
            Command::Scan => {
                if self.scan_task.is_some() { return; }
                self.view.scanning = true;
                self.view.scan_detail = "正在刷新，先显示系统已有的网络列表".into();
                match crate::wifi::ScanTask::start() {
                    Ok(task) => {
                        let existing=task.initial_networks();
                        if !existing.is_empty() || self.view.networks.is_empty() {self.view.networks=existing;}
                        self.view.scan_cached=true;
                        self.scan_task=Some(task);
                    }
                    Err(error) => {
                        self.view.scanning=false;
                        self.view.scan_cached=true;
                        self.view.scan_detail=error.clone();
                        self.log(&error,false);
                    }
                }
            }
            Command::Connect(network) => {
                if network.is_enterprise() || network::is_enterprise_campus_ssid(&network.ssid) {
                    platform::open("ms-settings:network-wifi");
                    self.notice("该 Wi-Fi 使用 802.1X，请在 Windows Wi-Fi 界面输入校园账号密码连接；无需网页认证。");
                    return;
                }
                self.view.busy = true;
                self.view.status = format!("正在连接 {}…", network.ssid);
                self.view.state = ConnectionState::Checking;
                self.view.state_detail = self.view.status.clone();
                self.publish();
                match network::connect_wifi(&network, None) {
                    Ok(ConnectStatus::Connected(_)) => {
                        crate::wifi::invalidate_ssid_cache();
                        self.paused_ssid.clear();
                        self.refresh_connection();
                        self.monitor.request_check();
                    }
                    Ok(ConnectStatus::NeedsPassword) => {
                        self.log("此网络需要配置密码，请在 Windows Wi-Fi 设置中连接。", true);
                        platform::open("ms-settings:network-wifi");
                    }
                    Err(error) => self.log(&error, true),
                }
                self.view.busy = false;
            }
            Command::Disconnect => {
                self.paused_ssid = self.view.ssid.clone();
                self.auto_auth_ssid = self.view.ssid.clone();
                match network::disconnect_wifi() {
                    Ok(_) => {
                        crate::wifi::invalidate_ssid_cache();
                        self.refresh_connection();
                        self.log("已断开 Wi-Fi，自动认证保持暂停。", true);
                    }
                    Err(error) => self.log(&error, true),
                }
            }
            Command::Login => {
                if self.verification.as_ref().is_some_and(|verification| verification.guard.check().is_ok()
                    && verification.wifi_generation == crate::wifi::connection_generation()
                    && verification.started.elapsed() < std::time::Duration::from_secs(30)) {
                    self.log("已有认证请求正在确认，未重复提交账号密码。", false);
                    return;
                }
                self.verification = None;
                self.refresh_connection();
                if network::is_enterprise_campus_ssid(&self.view.ssid) {
                    platform::open("ms-settings:network-wifi");
                    self.notice("ShuWlan-1X 由 Windows Wi-Fi 完成账号认证，无需网页认证。");return;
                }
                if !self.is_campus() {
                    self.log("当前为普通网络，不执行校园认证。校园网络可在设置中手动标记。", true);
                    return;
                }
                if !self.set_paused(false) { return; }
                self.monitor.enable(true);
                // Manual retry must not inherit cookies from a previous login/logout.
                self.probe = network::CampusProbe::default();
                self.authenticate(false);
            }
            Command::Logout => {
                self.refresh_connection();
                if !self.is_campus() { self.log("当前不是校园网络，无需校园网下线。", true); return; }
                if !self.set_paused(true) { return; }
                self.auto_auth_ssid = self.view.ssid.clone();
                self.view.busy = true;
                self.view.state = ConnectionState::SigningOut;
                self.view.state_detail = "正在向当前 Wi-Fi 的校园门户请求下线".into();
                self.view.status = "正在下线校园网…".into();
                self.publish();
                match network::portal_logout() {
                    Ok((confirmed, message)) => {
                        self.view.state = if confirmed { ConnectionState::Paused } else { ConnectionState::Checking };
                        self.view.state_detail = message.clone();
                        if confirmed {
                            self.view.online_session.clear();
                            self.view.checked_at = chrono::Local::now().format("%H:%M:%S").to_string();
                            self.monitor.wait_for_network("网关已确认下线 · 自动认证暂停");
                        }
                        self.log(&message, true);
                    }
                    Err(message) => {
                        self.view.state = ConnectionState::Checking;
                        self.view.state_detail = "下线结果未确认，将只读检测；自动认证已暂停".into();
                        self.log(&message, true);
                    }
                }
                self.view.busy = false;
                self.probe = network::CampusProbe::default();
            }
            Command::ToggleReconnect => {
                self.settings.auto_reconnect = !self.settings.auto_reconnect;
                self.monitor.request_check();
                self.save();
                self.log(
                    if self.settings.auto_reconnect {
                        if self.settings.status_detection {
                            "自动重连已开启。"
                        } else {
                            "自动重连偏好已开启；状态检测关闭期间暂停运行。"
                        }
                    } else {
                        "自动重连已关闭。"
                    },
                    true,
                );
            }
            Command::ToggleAutoAuth => {
                self.settings.auto_auth = !self.settings.auto_auth;
                self.monitor.request_check();
                self.save();
                self.log(
                    if self.settings.auto_auth {
                        if self.settings.status_detection {
                            "连接校园 Wi-Fi 后自动认证已开启。"
                        } else {
                            "自动认证偏好已开启；状态检测关闭期间暂停运行。"
                        }
                    } else {
                        "连接后自动认证已关闭。"
                    },
                    true,
                );
            }
            Command::ToggleStatusDetection => {
                let mut candidate = self.settings.clone();
                candidate.status_detection = !candidate.status_detection;
                if let Err(error) = self.store.save(&candidate) {
                    self.notice(&format!("状态检测设置保存失败，原设置已保留：{error}"));
                    return;
                }
                self.settings.status_detection = candidate.status_detection;
                self.manual_check_pending = false;
                self.recovery = false;
                self.probe = network::CampusProbe::default();
                self.monitor.request_check();
                self.refresh_connection();
                if self.settings.status_detection {
                    if self.is_campus() {
                        self.view.state = ConnectionState::Checking;
                        self.view.state_detail = "状态检测已开启，等待重新确认校园网认证".into();
                    }
                    self.log("状态检测已开启，将重新确认校园网状态。", true);
                } else {
                    self.log("状态检测已关闭，自动认证和自动重连已暂停；原开关设置已保留，手动操作仍可用。", true);
                }
            }
            Command::SetDetectionLevel(level) => {
                if level==self.settings.detection_level {return;}
                let mut candidate=self.settings.clone();
                candidate.detection_level=level;
                if let Err(error)=self.store.save(&candidate) {
                    self.notice(&format!("检测档位保存失败，原档位继续生效：{error}"));return;
                }
                self.settings.detection_level=level;
                self.monitor.set_level(level);
                self.log(&format!("状态检测已设置为{}。{}",level.label(),if self.settings.status_detection {"后续查询按新策略调度。"}else{"偏好已保存，开启状态检测后生效。"}),false);
            }
            Command::ToggleProxy => {
                let (enabled, current) = platform::read_proxy().unwrap_or((false, String::new()));
                let server = if current.is_empty() {
                    &self.settings.proxy_server
                } else {
                    &current
                };
                match platform::set_proxy(!enabled, server) {
                    Ok(message) => self.log(&message, true),
                    Err(error) => self.log(&error, true),
                }
            }
            Command::SetNetworkMode(mode) => {
                self.refresh_connection();
                if self.view.ssid.is_empty() { return; }
                let mut candidate = self.settings.clone();
                candidate.network_preferences.entry(self.view.ssid.clone()).or_default().mode = mode;
                if mode == NetworkMode::Auto && candidate.network_preferences.get(&self.view.ssid)
                    .is_some_and(|p| !p.paused) {
                    candidate.network_preferences.remove(&self.view.ssid);
                }
                if let Err(error) = self.store.save(&candidate) {
                    self.notice(&format!("网络类型保存失败，原设置已保留：{error}"));
                    return;
                }
                self.settings = candidate;
                self.manual_check_pending = false;
                self.probe = network::CampusProbe::default();
                self.monitor.request_check();
                if self.is_campus() {
                    self.view.state = ConnectionState::Checking;
                    self.view.state_detail = "网络类型已更新，将先查询认证状态".into();
                } else {
                    self.view.state = ConnectionState::Connected;
                    self.view.state_detail = "当前作为普通网络使用，不访问校园门户".into();
                }
                self.log("当前网络类型已保存。", true);
            }
            Command::ResumeCampus => {
                self.refresh_connection();
                if self.is_campus() && self.set_paused(false) {
                    self.monitor.enable(true);
                    self.log("已恢复该网络的自动策略，将先只读确认状态。", true);
                }
            }
            Command::Credentials(username, password, remember) => {
                if username.trim().is_empty() || password.is_empty() {
                    self.notice("账号和密码不能为空，原设置已保留。");return;
                }
                let mut candidate=self.settings.clone();
                candidate.username=username;candidate.password=password;candidate.remember_password=remember;
                if let Err(error)=self.store.save(&candidate) {
                    self.notice(&format!("账号设置保存失败，原设置已保留：{error}"));return;
                }
                self.settings=candidate;
                self.monitor.enable(self.settings.auto_auth || self.settings.auto_reconnect);
                if !self.had_online {self.auto_auth_ssid.clear();self.initial_attempts=0;}
                self.log("校园账号设置已更新。",true);
            }
            Command::ImportDesktop => {
                let desktop = SettingsStore::desktop();
                if !desktop.exists() {
                    self.notice("未找到桌面版配置，现有设置已保留。");
                    return;
                }
                match SettingsStore::import_file(desktop.path()) {
                    Ok(mut settings) => {
                        settings.operator = self.settings.operator;
                        settings.menu_theme = self.settings.menu_theme;
                        settings.auto_reconnect = self.settings.auto_reconnect;
                        settings.status_detection = self.settings.status_detection;
                        settings.detection_level = self.settings.detection_level;
                        settings.websites = self.settings.websites.clone();
                        settings.hotkeys = self.settings.hotkeys.clone();
                        settings.network_preferences = self.settings.network_preferences.clone();
                        self.apply_import(settings);
                    }
                    Err(error) => self.notice(&format!("导入失败，现有设置已保留。\r\n{error}")),
                }
            }
            Command::ImportFile(path) => {
                match SettingsStore::import_file(&path) {
                    Ok(settings) => self.apply_import(settings),
                    Err(error) => self.notice(&format!("导入失败，现有设置已保留。\r\n{error}")),
                }
            }
            Command::ExportFile(path) => {
                match self.store.export_file(&path, &self.settings) {
                    Ok(()) => self.notice(&format!("配置已导出到：\r\n{}\r\n\r\n记住的密码仍为 Windows 加密数据，仅限原电脑的同一 Windows 用户恢复。未记住的密码不包含在备份中。", path.display())),
                    Err(error) => self.notice(&format!("导出失败：\r\n{error}")),
                }
            }
            Command::AddWebsite(website) => {
                let mut websites = self.settings.websites.clone();
                websites.push(website);
                self.save_websites(websites);
            }
            Command::EditWebsite { previous, updated } => {
                let mut websites = self.settings.websites.clone();
                if let Some(entry) = websites.iter_mut().find(|entry| **entry == previous) {
                    *entry = updated;
                    self.save_websites(websites);
                } else {
                    self.log("该网址已发生变化，请重新打开菜单编辑。", true);
                }
            }
            Command::RemoveWebsite(website) => {
                let mut websites = self.settings.websites.clone();
                websites.retain(|entry| entry != &website);
                self.save_websites(websites);
            }
            Command::SaveHotkeys { bindings, reply } => {
                let result = (|| {
                    hotkeys::validate(&bindings)?;
                    let mut candidate = self.settings.clone();
                    candidate.hotkeys = bindings;
                    self.store.save(&candidate)?;
                    self.settings.hotkeys = candidate.hotkeys;
                    self.log("快捷键已保存。", true);
                    Ok(())
                })();
                // Publish before acknowledging; the UI can safely reconcile the
                // registered keys from the committed settings when its dialog closes.
                self.publish();
                let _ = reply.try_send(result);
            }
            Command::Stop => {}
        }
    }
    fn apply_import(&mut self, settings: Settings) {
        let result = (|| {
            let backup = self.store.backup_before_import()?;
            self.store.save(&settings)?;
            Ok::<_, String>(backup)
        })();
        match result {
            Ok(backup) => {
                self.settings = settings;
                self.monitor.set_level(self.settings.detection_level);
                self.paused_ssid.clear();
                if !self.had_online {
                    self.auto_auth_ssid.clear();
                    self.initial_attempts = 0;
                }
                if !self.view.ssid.is_empty() {
                    self.view.state = ConnectionState::Checking;
                    self.view.state_detail = "配置已更新，等待重新确认校园网状态".into();
                }
                self.recovery = false;
                self.manual_check_pending = false;
                self.probe = network::CampusProbe::default();
                self.monitor
                    .enable(self.settings.auto_auth || self.settings.auto_reconnect);
                let mut message = "配置已导入并保存，已立即生效。\r\nWindows 开机启动和系统代理的当前开关状态保持原样。".to_owned();
                self.refresh_connection();
                if let Some(backup) = backup {
                    message.push_str(&format!(
                        "\r\n\r\n导入前的配置已备份到：\r\n{}",
                        backup.display()
                    ));
                }
                self.notice(&message);
            }
            Err(error) => self.notice(&format!("配置未能保存，现有设置已保留。\r\n{error}")),
        }
    }
    fn save_websites(&mut self, websites: Vec<Website>) {
        if let Err(error) = websites::validate_all(&websites) {
            self.log(&error, true);
            return;
        }
        let mut settings = self.settings.clone();
        settings.websites = websites;
        match self.store.save(&settings) {
            Ok(()) => {
                // Publish the new list only after the atomic configuration write succeeds.
                self.settings.websites = settings.websites;
                self.log("常用网址已保存。", true);
            }
            Err(error) => self.log(&format!("网址保存失败，原收藏已保留：{error}"), true),
        }
    }
    fn complete_scan(&mut self) {
        let Some(task) = self.scan_task.take() else {
            return;
        };
        let result = task.finish();
        // Drop the callback context only after WLAN has stopped invoking it.
        drop(task);
        self.view.scanning = false;
        self.view.scanned_at = chrono::Local::now().format("%H:%M:%S").to_string();
        match result {
            Ok(report) => {
                self.view.networks = report.networks;
                self.view.scan_cached = report.cached;
                self.view.scan_detail = report.detail.clone();
                self.log(&report.detail, false);
            }
            Err(error) => {
                self.view.scan_cached = true;
                self.view.scan_detail = error.clone();
                self.log(&error, false);
            }
        }
    }

    fn refresh_connection(&mut self) -> bool {
        self.next_local_check = std::time::Instant::now() + std::time::Duration::from_secs(60);
        match network::current_wifi_connection() {
            Ok((ssid, ip, index)) => {
                if self.wifi_index != index && self.view.ssid == ssid && self.view.ip == ip {
                    self.view.state = ConnectionState::Checking;
                    self.view.state_detail = "Wi-Fi 网卡已变化，等待重新确认认证".into();
                    self.view.checked_at = "尚未检测".into();
                    self.view.confirmed_at.clear();
                    self.view.online_session.clear();
                    self.pending_reconnect=None;
                    self.portal_seen = false;
                    self.probe = network::CampusProbe::default();
                    self.monitor.invalidate();
                }
                self.wifi_index = index;
                self.update_connection(ssid, ip);
                true
            }
            Err(error) => {
                self.view.state = ConnectionState::Checking;
                self.view.state_detail = error;
                false
            }
        }
    }

    fn update_connection(&mut self, ssid: String, ip: String) {
        let ip = if ssid.is_empty() {
            network::other_connection_address().unwrap_or_else(|| "—".into())
        } else {
            ip
        };
        if self.view.ssid != ssid || self.view.ip != ip {
            self.view.state = if ssid.is_empty() {
                if ip == "—" {
                    ConnectionState::Disconnected
                } else {
                    ConnectionState::OtherConnection
                }
            } else if ip == "—" {
                ConnectionState::WaitingAddress
            } else if self
                .settings
                .network_preferences
                .get(&ssid)
                .map_or(NetworkMode::Auto, |p| p.mode)
                .allows_campus(&ssid)
            {
                ConnectionState::Checking
            } else {
                ConnectionState::Connected
            };
            self.view.state_detail = "连接已变化，原认证结果已失效".into();
            self.view.checked_at = "尚未检测".into();
            self.view.confirmed_at.clear();
            self.view.online_session.clear();
            self.pending_reconnect=None;
            self.portal_seen = false;
            self.probe = network::CampusProbe::default();
            self.monitor.invalidate();
        }
        if !ssid.is_empty() && self.session_ssid != ssid {
            self.monitor.reset_network_history();
            self.session_ssid = ssid.clone();
            self.had_online = false;
            self.recovery = false;
            self.initial_attempts = 0;
            self.auto_auth_ssid.clear();
            self.monitor
                .enable(self.settings.auto_auth || self.settings.auto_reconnect);
            self.monitor.settle_connection();
            if self.paused_ssid != ssid {
                self.paused_ssid.clear();
            }
        }
        self.view.ssid = ssid;
        self.view.ip = ip;
        if self.view.ssid.is_empty() {
            self.view.state = if self.view.ip == "—" {
                ConnectionState::Disconnected
            } else {
                ConnectionState::OtherConnection
            };
            self.view.state_detail = "仅确认本机接口连接情况，不据此判断互联网是否可用".into();
        } else if !self.is_campus() {
            if network::is_enterprise_campus_ssid(&self.view.ssid) {
                self.view.state=if self.view.ip=="—" {ConnectionState::WaitingAddress} else {ConnectionState::EnterpriseConnected};
                self.view.state_detail="ShuWlan-1X 通过 Windows Wi-Fi 输入账号密码完成 802.1X 认证，不访问网页门户。".into();
            } else {
                self.view.state = ConnectionState::Connected;
                self.view.state_detail = "普通网络不执行校园认证；代理、测速和常用网址仍可使用".into();
            }
        }
    }
    fn authenticate(&mut self, automatic: bool) {
        if self.verification.is_some() { return; }
        if !automatic { self.pending_reconnect=None; }
        self.owns_auth_trace = crate::auth_trace::start_if_idle();
        self.authenticate_inner(automatic);
        if self.verification.is_none() { self.finish_auth_trace("本次认证结束"); }
    }

    fn finish_auth_trace(&mut self, result: &str) {
        if !self.owns_auth_trace { return; }
        self.owns_auth_trace = false;
        if let Some(verification) = &self.verification { crate::auth_trace::record("confirmation", verification.started.elapsed()); }
        if let Some(report) = crate::auth_trace::finish() {
            let requests: u64 = report.requests.values().sum();
            self.log(&format!("认证统计：{result}，共 {:.2} 秒，HTTP {requests} 次，登录提交 {} 次。",
                report.total_ms / 1000.0, report.requests.get("login").copied().unwrap_or(0)), false);
            let phase_ms = |name| report.phases.get(name).map_or(0.0, |phase| phase.milliseconds);
            self.log(&format!("认证分段：预检查 {:.0}ms，页面参数 {:.0}ms，运营商 {:.0}ms，加密 {:.0}ms，提交 {:.0}ms，在线确认 {:.0}ms。",
                phase_ms("preflight"), phase_ms("page_info"), phase_ms("services"), phase_ms("encryption"), phase_ms("submit"), phase_ms("confirmation")), false);
        }
    }

    fn authenticate_inner(&mut self, automatic: bool) {
        if automatic && !self.settings.status_detection {
            return;
        }
        if !self.is_campus() {
            return;
        }
        let guard = self.monitor.guard();
        self.view.busy = true;
        self.begin_query(false);
        let preflight_timing = crate::auth_trace::phase("preflight");
        let preflight = self.probe.prepare_auth(&guard, self.settings.operator);
        drop(preflight_timing);
        self.view.query.active = false;
        self.view.busy = false;
        match preflight {
            Ok(check) if matches!(check.state, CampusPortalState::Online) => {
                self.mark_online(check.session.as_ref());
                if !automatic {
                    self.notice("校园网已经认证，无需重复登录。");
                }
                return;
            }
            Ok(check) if matches!(check.state, CampusPortalState::Offline) => {}
            Ok(_) => {
                self.view.state = ConnectionState::Checking;
                self.view.state_detail = "连接或门户仍在同步，尚未提交认证".into();
                self.monitor.wait_for_address();
                self.log("校园网状态尚未确认，未提交账号密码。", true);
                return;
            }
            Err(error) => {
                self.view.state = ConnectionState::Checking;
                self.view.state_detail = error.clone();
                self.monitor.check_failed();
                self.log(&format!("认证前检查未确认下线，未提交凭据：{error}"), true);
                return;
            }
        }
        if self.settings.username.trim().is_empty() || self.settings.password.is_empty() {
            self.log("尚未配置校园账号，请从托盘菜单配置账号密码。", true);
            self.monitor.wait_for_network("等待配置账号");
            return;
        }
        if !self.is_campus() {
            self.view.status = "等待连接 SHU / CMCC 校园 Wi-Fi".into();
            return;
        }
        self.auto_auth_ssid = self.view.ssid.clone();
        self.view.busy = true;
        if automatic && !self.had_online {
            self.initial_attempts = self.initial_attempts.saturating_add(1);
        }
        self.view.state = ConnectionState::Authenticating;
        self.view.state_detail = "正在向校园门户提交认证；尚未确认成功".into();
        self.view.status = "正在认证校园网…".into();
        self.publish();
        let guard = self.monitor.guard();
        let result = network::portal_login_prepared(
            &self.view.ssid,
            &self.settings.username,
            &self.settings.password,
            self.settings.operator,
            &guard,
            self.probe.take_auth_context(),
        );
        self.view.busy = false;
        let reconnect_request = self.record_reconnect_request(automatic,&result);
        if guard.check().is_err() {
            self.pending_reconnect=None;
            self.view.state = ConnectionState::Checking;
            self.view.state_detail = "旧认证已取消，等待重新检测".into();
            self.view.status = "旧认证已取消，等待当前操作…".into();
            return;
        }
        match result {
            Ok(network::AuthOutcome::AlreadyOnline(evidence)) => {
                self.mark_online(Some(&evidence));
                if !automatic { self.notice("校园网已经认证，无需重复登录。"); }
            }
            Ok(outcome @ (network::AuthOutcome::Accepted | network::AuthOutcome::Unknown(_))) => {
                self.pending_reconnect=reconnect_request;
                self.view.online_session.submitted(self.settings.operator, matches!(outcome, network::AuthOutcome::Accepted));
                // One bounded read-only confirmation remains part of a manual
                // login, even when periodic status monitoring is disabled.
                self.manual_check_pending = false;
                self.verification = Some(Verification {
                    started: std::time::Instant::now(), guard: self.monitor.guard(),
                    wifi_generation: crate::wifi::connection_generation(), round: 0,
                });
                self.view.state = ConnectionState::Verifying;
                self.view.state_detail = "认证请求已发送，等待只读查询确认在线；不重复提交".into();
                self.monitor.authentication_submitted();
                self.view.status = "已提交认证，等待在线确认…".into();
            }
            Ok(network::AuthOutcome::Rejected(error)) => {
                self.pending_reconnect=None;
                self.view.state = ConnectionState::AuthenticationFailed;
                self.view.state_detail = error.clone();
                self.monitor.authentication_failed(&error);
                if !automatic || !self.recovery || self.monitor.needs_attention() {
                    self.log(&error, true);
                } else {
                    self.view.status = self.monitor.status.clone();
                }
            }
            Err(error) => {
                self.view.state = ConnectionState::Checking;
                self.view.state_detail = format!("认证请求未能确认：{error}");
                self.monitor.authentication_failed(&error);
                if automatic
                    && !self.had_online
                    && !self.monitor.needs_attention()
                    && self.initial_attempts < 3
                {
                    self.auto_auth_ssid.clear();
                }
                if self.monitor.needs_attention() {
                    self.view.state = ConnectionState::AuthenticationFailed;
                }
                self.log(&error, true);
            }
        }
    }
    fn poll(&mut self) {
        if let Some(verification) = &self.verification {
            let cancelled = verification.guard.check().is_err()
                || verification.wifi_generation != crate::wifi::connection_generation();
            let expired = verification.started.elapsed() >= std::time::Duration::from_secs(30);
            if cancelled || expired {
                self.finish_auth_trace(if cancelled { "本轮已取消" } else { "确认超时，结果待确认" });
                self.verification = None;
                self.manual_check_pending = false;
                self.view.state = ConnectionState::Checking;
                self.view.state_detail = if cancelled { "连接或设置已变化，本轮认证确认已取消" } else {
                    "30 秒内尚未确认在线，本轮确认已结束；后续须重新确认下线并遵守重试间隔"
                }.into();
                self.monitor.authentication_failed("认证结果未确认，等待后续独立检测");
            }
        }
        if !self.should_monitor() {
            return;
        }
        let Some(generation) = self.monitor.begin_if_due() else {
            return;
        };
        self.begin_query(self.manual_check_pending);
        self.poll_query(generation);
        // All result paths leave the query badge together with the committed
        // state; early returns never publish an idle icon with a pending result.
        self.view.query.active = false;
    }
    fn continue_verification(&mut self, result: &Result<network::CampusConnectionCheck, String>) -> bool {
        if result.as_ref().is_ok_and(|check| matches!(check.state, CampusPortalState::Online)) { return false; }
        let Some(verification) = self.verification.as_mut() else { return false; };
        let intervals = [2, 3, 5, 8];
        let delay = std::time::Duration::from_secs(intervals[verification.round.min(intervals.len() - 1)]);
        verification.round = verification.round.saturating_add(1);
        let remaining = std::time::Duration::from_secs(30).saturating_sub(verification.started.elapsed());
        self.monitor.verification_after(delay.min(remaining));
        self.view.state = ConnectionState::Verifying;
        self.view.checked_at = chrono::Local::now().format("%H:%M:%S").to_string();
        self.view.state_detail = "认证结果仍待确认，继续只读查询；本轮不重复提交账号密码".into();
        self.view.status = self.monitor.status.clone();
        true
    }

    fn poll_query(&mut self, generation: u64) {
        let local_state_known = self.refresh_connection();
        if generation != self.monitor.generation {
            self.monitor.finish_probe(generation);
            if self.manual_check_pending {
                self.log("手动查询遇到连接变化，等待当前连接稳定后继续。", false);
            }
            return; // Connection identity changed; wait for DHCP / roaming to settle.
        }
        if local_state_known && !self.is_campus() {
            self.manual_check_pending = false;
            self.monitor.finish_probe(generation);
            self.probe = network::CampusProbe::default();
            return;
        }
        let guard = self.verification.as_ref().map_or_else(|| self.monitor.guard(), |verification|
            verification.guard.with_deadline(verification.started + std::time::Duration::from_secs(30)));
        self.publish();
        let result = if local_state_known {
            self.probe.check(&guard, self.is_campus())
        } else {
            Err(self.view.state_detail.clone())
        };
        let requested_once = self.manual_check_pending;
        let manual_once = !self.settings.status_detection && requested_once;
        self.manual_check_pending = false;
        if !self.monitor.finish_probe(generation) || guard.check().is_err() {
            if requested_once {
                self.log("手动查询已取消：连接或设置发生变化，未采用旧结果。", false);
            }
            return;
        }
        if self.continue_verification(&result) { return; }
        if requested_once && !manual_once {
            let message = match &result {
                Ok(check) => match check.state {
                    CampusPortalState::Online => "手动查询完成：门户已确认认证在线。".to_owned(),
                    CampusPortalState::Offline => "手动查询完成：门户返回未认证，等待后续独立确认。".to_owned(),
                    CampusPortalState::Verifying if self.view.online_session.has_session() => "手动查询完成：门户返回 wait；此前已确认在线，继续只读复核，未重新认证。".to_owned(),
                    CampusPortalState::Verifying => "手动查询完成：门户仍在同步，本轮仅只读核对，未提交新的认证。".to_owned(),
                    CampusPortalState::WaitingAddress => "手动查询完成：等待 Wi-Fi 分配 IPv4 地址。".to_owned(),
                    CampusPortalState::NotCampus => "手动查询完成：当前无需查询校园门户。".to_owned(),
                },
                Err(error) => format!("手动查询未能确认：{error}"),
            };
            self.log(&message, false);
        }
        if manual_once {
            let message = match &result {
                Ok(check) => match check.state {
                    CampusPortalState::Online => {
                        "本次手动检测：门户已确认认证在线；定期检测仍关闭。".to_owned()
                    }
                    CampusPortalState::Offline => {
                        "本次手动检测：门户返回登录页（单次结果，尚未复查）；定期检测仍关闭。"
                            .to_owned()
                    }
                    CampusPortalState::Verifying => {
                        "本次手动检测：门户仍在同步，尚未确认在线；可稍后手动重新检测。".to_owned()
                    }
                    CampusPortalState::WaitingAddress => {
                        "本次手动检测：等待 Wi-Fi 分配 IPv4 地址；定期检测仍关闭。".to_owned()
                    }
                    CampusPortalState::NotCampus => {
                        "本次手动检测：Wi-Fi 未连接或未匹配校园门户；定期检测仍关闭。".to_owned()
                    }
                },
                Err(error) => format!("本次手动检测未能确认：{error}；定期检测仍关闭。"),
            };
            self.log(&message, true);
        }
        match result {
            Ok(check) => {
                self.update_connection(check.ssid, check.ip);
                if matches!(
                    check.state,
                    CampusPortalState::Online
                        | CampusPortalState::Offline
                        | CampusPortalState::Verifying
                ) {
                    self.portal_seen = true;
                }
                self.view.checked_at = chrono::Local::now().format("%H:%M:%S").to_string();
                match check.state {
                    CampusPortalState::Online => {
                        if self.recovery {
                            self.log("校园网已恢复在线。", false);
                            self.recovery = false;
                        }
                        self.mark_online(check.session.as_ref());
                    }
                    CampusPortalState::Offline => {
                        if self.is_paused() {
                            self.view.online_session.clear();
                            self.view.state = ConnectionState::Paused;
                            self.view.state_detail = "网关登录页已确认下线；自动认证仍暂停".into();
                            self.monitor.wait_for_network("校园网已下线 · 自动认证暂停");
                            return;
                        }
                        let allowed = self.settings.status_detection
                            && !requested_once
                            && ((self.settings.auto_reconnect && self.had_online)
                                || (self.settings.auto_auth
                                    && !self.had_online
                                    && self.auto_auth_ssid != self.view.ssid));
                        let credentials = !self.settings.username.trim().is_empty()
                            && !self.settings.password.is_empty();
                        let reconnect = self.monitor.offline(allowed && credentials);
                        if self.monitor.offline_confirmed() { self.view.online_session.clear(); }
                        self.view.state = if self.monitor.needs_attention() {
                            ConnectionState::AuthenticationFailed
                        } else if !self.monitor.offline_confirmed() {
                            ConnectionState::Checking
                        } else if self.had_online {
                            ConnectionState::Offline
                        } else {
                            ConnectionState::NeedsAuthentication
                        };
                        self.view.state_detail = if !credentials {
                            "门户返回当前 Wi-Fi 的登录页；尚未配置校园账号".into()
                        } else {
                            self.monitor.status.clone()
                        };
                        if reconnect {
                            if !self.recovery {
                                self.log("确认校园网下线，正在后台恢复。", false);
                            }
                            self.authenticate(true);
                            self.recovery = self.view.state != ConnectionState::Online;
                        }
                    }
                    CampusPortalState::NotCampus => {
                        self.view.state = if self.view.ssid.is_empty() {
                            ConnectionState::Disconnected
                        } else if network::is_enterprise_campus_ssid(&self.view.ssid) {
                            if self.view.ip=="—" {ConnectionState::WaitingAddress} else {ConnectionState::EnterpriseConnected}
                        } else {
                            ConnectionState::Connected
                        };
                        self.view.state_detail = if self.view.ssid.is_empty() {
                            "未检测到已连接的物理 Wi-Fi".into()
                        } else if network::is_enterprise_campus_ssid(&self.view.ssid) {
                            "ShuWlan-1X 通过 Windows Wi-Fi 输入账号密码完成 802.1X 认证，不访问网页门户。".into()
                        } else {
                            "当前 SSID 未匹配 SHU / CMCC 门户候选规则；不据此判断是否已认证或能否上网".into()
                        };
                        self.monitor.wait_for_network(if self.view.ssid.is_empty() {
                            "等待 Wi-Fi 恢复连接"
                        } else {
                            "Wi-Fi 已连接 · 未检测此网络的校园门户"
                        });
                    }
                    CampusPortalState::WaitingAddress => {
                        self.view.state = ConnectionState::WaitingAddress;
                        self.view.state_detail = "等待校园 Wi-Fi 获得可用 IPv4 地址".into();
                        self.monitor.wait_for_address();
                    }
                    CampusPortalState::Verifying => {
                        self.apply_portal_wait();
                    }
                }
                self.view.status = self.monitor.status.clone();
            }
            Err(error) => {
                self.view.checked_at = chrono::Local::now().format("%H:%M:%S").to_string();
                let known = self.refresh_connection();
                if known && self.view.ssid.is_empty() {
                    self.monitor.wait_for_network("等待 Wi-Fi 恢复连接");
                    return;
                }
                if self.monitor.check_failed() {
                    self.log(&error, false);
                }
                self.view.state = if self.portal_seen && self.monitor.probe_failures() >= 3 {
                    ConnectionState::GatewayUnavailable
                } else {
                    ConnectionState::Checking
                };
                self.view.state_detail = format!(
                    "{error}（连续 {} 次，未据此判定为认证下线）",
                    self.monitor.probe_failures()
                );
                self.view.status = self.monitor.status.clone();
            }
        }
    }

    fn apply_portal_wait(&mut self) {
        self.monitor.portal_waiting();
        // Active submissions are handled by continue_verification. A later
        // read-only `wait` is not a new authentication and must not turn orange.
        let had_session = self.view.online_session.has_session();
        let state = if had_session { ConnectionState::OnlineRechecking } else { ConnectionState::Checking };
        if self.view.state != state {
            self.log(if had_session {
                "在线状态复核：短暂复查后门户仍返回 wait；保留最近会话记录，继续只读核对。"
            } else {
                "门户状态尚在同步（wait）；本轮仅只读核对，不重复提交认证。"
            }, false);
        }
        self.view.state = state;
        self.view.state_detail = if had_session {
            format!("最近于 {} 确认在线；本次门户返回 wait，正在只读复核，未重新认证。", self.view.confirmed_at)
        } else {
            "门户在线接口返回 wait；正在只读核对，本轮没有提交新的认证请求。".into()
        };
    }
}
