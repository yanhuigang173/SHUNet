//! User-owned Windows scheduled tasks. No power operation is ever run by tests.
use base64::Engine as _;
use chrono::{Datelike, Duration as TimeDelta, Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    hash::{Hash, Hasher},
    io::Write,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::*,
    System::{Power::*, Threading::*},
};

pub const WARNING_SECONDS: i64 = 60;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Shutdown,
    Restart,
    Sleep,
    Hibernate,
    Wake,
}
impl Action {
    pub const ALL: [Self; 5] = [
        Self::Shutdown,
        Self::Restart,
        Self::Sleep,
        Self::Hibernate,
        Self::Wake,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Shutdown => "关机",
            Self::Restart => "重启",
            Self::Sleep => "睡眠",
            Self::Hibernate => "休眠",
            Self::Wake => "唤醒（睡眠／休眠）",
        }
    }
    fn lead(self) -> i64 {
        if self == Self::Wake {
            0
        } else {
            WARNING_SECONDS
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    Once,
    Daily,
    Weekdays,
}
impl Repeat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Once => "仅一次",
            Self::Daily => "每天",
            Self::Weekdays => "工作日（周一至周五）",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Run {
    pub occurrence: i64,
    pub status: String,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub version: u8,
    pub id: String,
    pub action: Action,
    /// Local calendar time; repeating plans follow the computer's local timezone.
    pub start: NaiveDateTime,
    pub repeat: Repeat,
    pub on_battery: bool,
    pub enabled: bool,
    pub last: Option<Run>,
}
impl Plan {
    pub fn new(
        action: Action,
        start: NaiveDateTime,
        repeat: Repeat,
        on_battery: bool,
    ) -> Result<Self, String> {
        let mut bytes = [0u8; 16];
        unsafe {
            use windows_sys::Win32::Security::Cryptography::*;
            if BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                16,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            ) < 0
            {
                return Err("无法生成计划编号。".into());
            }
        }
        Ok(Self {
            version: 1,
            id: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
            action,
            start,
            repeat,
            on_battery,
            enabled: false,
            last: None,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || !valid_id(&self.id) || !(2020..=2200).contains(&self.start.year()) {
            return Err("计划格式无效，未执行电源操作。".into());
        }
        Ok(())
    }
    /// Find an occurrence at or after `from`, never earlier than the start date.
    pub fn next(&self, from: i64) -> Option<i64> {
        let from_local = Local.timestamp_opt(from, 0).single()?.naive_local();
        let date = from_local.date().max(self.start.date());
        if self.repeat == Repeat::Once {
            let at = local_timestamp(self.start).ok()?;
            return (at >= from).then_some(at);
        }
        for offset in 0..9 {
            let date = date.checked_add_signed(TimeDelta::days(offset))?;
            if self.repeat == Repeat::Weekdays && date.weekday().number_from_monday() > 5 {
                continue;
            }
            let datetime = date.and_time(self.start.time());
            // Nonexistent/ambiguous local times are skipped rather than guessed.
            let Ok(at) = local_timestamp(datetime) else {
                continue;
            };
            if at >= from {
                return Some(at);
            }
        }
        None
    }
    pub fn next_unclaimed(&self, now: i64) -> Option<i64> {
        if !self.enabled {
            return None;
        }
        self.next(
            now.max(
                self.last
                    .as_ref()
                    .map_or(i64::MIN, |last| last.occurrence.saturating_add(1)),
            ),
        )
    }
    pub fn summary(&self, now: i64) -> String {
        let (when, state) = self.display(now);
        let next = when.map(format_time).unwrap_or_else(|| "—".into());
        format!(
            "{}  ·  {}  ·  {}  ·  {}",
            self.action.label(),
            next,
            self.repeat.label(),
            state
        )
    }
    pub fn display(&self, now: i64) -> (Option<i64>, &'static str) {
        if !self.enabled {
            return (None, "已取消");
        }
        if let Some(last) = &self.last {
            if last.status == "countdown" && now <= last.occurrence + 15 {
                return (Some(last.occurrence), "倒计时中");
            }
        }
        if let Some(next) = self.next_unclaimed(now) {
            return (Some(next), "已安排");
        }
        let status = match self.last.as_ref().map(|last| last.status.as_str()) {
            Some("submitted") => "已提交 Windows",
            Some("cancelled") => "本次已取消",
            Some("skipped") => "本次已跳过",
            Some("failed") => "执行失败",
            Some("countdown") => "未确认执行",
            Some("completed") => "已处理",
            _ => "已错过",
        };
        (self.last.as_ref().map(|last| last.occurrence), status)
    }
}
pub fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub fn local_timestamp(value: NaiveDateTime) -> Result<i64, String> {
    Local
        .from_local_datetime(&value)
        .single()
        .map(|value| value.timestamp())
        .ok_or_else(|| "该本地时间不存在或处于夏令时重复时段，请选择其他时间。".into())
}
pub fn format_time(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "时间无效".into())
}
pub fn relative_time(minutes: &str, now: i64) -> Result<NaiveDateTime, String> {
    let minutes = minutes
        .trim()
        .parse::<i64>()
        .map_err(|_| "请输入 2～10080 的整数分钟。")?;
    if !(2..=10080).contains(&minutes) {
        return Err("倒计时支持 2 分钟～7 天；更久请使用指定时间。".into());
    }
    Local
        .timestamp_opt(now.checked_add(minutes * 60).ok_or("时间超出范围。")?, 0)
        .single()
        .map(|value| value.naive_local())
        .ok_or_else(|| "时间超出范围。".into())
}

#[derive(Clone)]
pub struct Store {
    pub folder: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanIssue {
    pub id: String,
    pub error: String,
}
#[derive(Default)]
pub struct Listing {
    pub plans: Vec<Plan>,
    pub issues: Vec<PlanIssue>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            folder: std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("CampusLinkTray")
                .join("power-plans"),
        }
    }
}
struct Lock(HANDLE);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}
impl Store {
    fn lock(&self) -> Result<Lock, String> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.folder.to_string_lossy().to_lowercase().hash(&mut hash);
        let name = crate::wide(&format!("Local\\SHUNet.Power.Store.{:x}", hash.finish()));
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err("无法锁定电源计划存储。".into());
            }
            match WaitForSingleObject(handle, 5000) {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Lock(handle)),
                _ => {
                    CloseHandle(handle);
                    Err("电源计划正被其他操作使用，请稍后重试。".into())
                }
            }
        }
    }
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_id(id) {
            return Err("计划编号无效。".into());
        }
        Ok(self.folder.join(format!("{id}.json")))
    }
    pub fn read(&self, id: &str) -> Result<Plan, String> {
        let path = self.path(id)?;
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 32768 {
            return Err("计划文件过大，未执行。".into());
        }
        let plan: Plan = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|_| "计划文件损坏，未执行。")?;
        plan.validate()?;
        if plan.id != id {
            return Err("计划编号不匹配，未执行。".into());
        }
        Ok(plan)
    }
    fn write(&self, plan: &Plan) -> Result<(), String> {
        plan.validate()?;
        fs::create_dir_all(&self.folder).map_err(|e| e.to_string())?;
        atomic_write(
            &self.path(&plan.id)?,
            &serde_json::to_vec_pretty(plan).map_err(|e| e.to_string())?,
        )
    }
    pub fn list(&self) -> Result<Vec<Plan>, String> {
        Ok(self.list_with_issues()?.plans)
    }
    pub fn stamp(&self) -> Result<Vec<(String, u64, std::time::SystemTime)>, String> {
        if !self.folder.exists() {
            return Ok(Vec::new());
        }
        let mut result = Vec::new();
        for entry in fs::read_dir(&self.folder).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.strip_suffix(".json").is_some_and(valid_id) {
                continue;
            }
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            result.push((
                name,
                meta.len(),
                meta.modified().map_err(|e| e.to_string())?,
            ));
        }
        result.sort();
        Ok(result)
    }
    pub fn list_with_issues(&self) -> Result<Listing, String> {
        if !self.folder.exists() {
            return Ok(Listing::default());
        }
        let mut result = Listing::default();
        for entry in fs::read_dir(&self.folder).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let file = entry.file_name();
            let name = file.to_string_lossy();
            let Some(id) = name.strip_suffix(".json").filter(|id| valid_id(id)) else {
                continue;
            };
            match self.read(id) {
                Ok(plan) => result.plans.push(plan),
                Err(error) => result.issues.push(PlanIssue {
                    id: id.into(),
                    error,
                }),
            }
            if result.plans.len() + result.issues.len() > 100 {
                return Err("计划超过 100 项，请先清理已完成的计划文件。".into());
            }
        }
        result.plans.sort_by_key(|plan| plan.start);
        result.issues.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }
    fn helper(&self) -> Result<PathBuf, String> {
        let folder = self.folder.join("runner").join(env!("CARGO_PKG_VERSION"));
        fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let target = folder.join("SHUNetPower.exe");
        // Version-specific stable path remains valid if the portable tray file moves.
        if !target.exists() {
            let temporary = target.with_extension("exe.tmp");
            let source = std::env::current_exe().map_err(|e| e.to_string())?;
            let result = (|| {
                fs::copy(&source, &temporary)?;
                fs::OpenOptions::new()
                    .write(true)
                    .open(&temporary)?
                    .sync_all()?;
                fs::rename(&temporary, &target)
            })();
            if let Err(error) = result {
                let _ = fs::remove_file(&temporary);
                return Err(format!("无法保存独立执行器：{error}"));
            }
        }
        Ok(target)
    }
    pub fn create(&self, mut plan: Plan) -> Result<Plan, String> {
        let _lock = self.lock()?;
        plan.validate()?;
        let now = Local::now().timestamp();
        let at = plan
            .next(now + 90)
            .ok_or("请选择至少 90 秒后的执行时间。")?;
        if plan.repeat == Repeat::Once && local_timestamp(plan.start)? < now + 90 {
            return Err("请选择至少 90 秒后的执行时间。".into());
        }
        if at > now + 366 * 86400 {
            return Err("首次执行时间请安排在一年以内。".into());
        }
        let caps = capabilities();
        if plan.action == Action::Hibernate && !caps.hibernate {
            return Err(
                "本机尚未启用或不支持休眠。请先在 Windows 电源设置中启用休眠，再创建计划。".into(),
            );
        }
        if plan.action == Action::Sleep && !caps.sleep {
            return Err("当前设备不支持可用的睡眠状态。".into());
        }
        let listing = self.list_with_issues()?;
        let plans = listing.plans;
        if plans.len() + listing.issues.len() >= 100 {
            return Err("已保留 100 项计划，请先移除已结束的计划。".into());
        }
        if plans
            .iter()
            .filter(|p| p.enabled && p.next_unclaimed(now).is_some())
            .count()
            >= 20
        {
            return Err("最多同时安排 20 项计划，请先取消部分计划。".into());
        }
        if plans
            .iter()
            .filter(|p| p.enabled && p.action != Action::Wake && plan.action != Action::Wake)
            .any(|p| conflicts(p, &plan, now))
        {
            return Err("附近两分钟内已有电源操作，请错开时间或先取消原计划。".into());
        }
        let helper = self.helper()?;
        let xml = task_xml(&plan, at, &helper)?;
        self.write(&plan)?; // Not armed until Windows confirms registration.
        if let Err(error) = scheduler("create", &plan.id, Some(&xml)) {
            let _ = scheduler("cancel", &plan.id, None);
            let _ = fs::remove_file(self.path(&plan.id)?);
            return Err(format!("Windows 未能保存计划：{error}"));
        }
        plan.enabled = true;
        if let Err(error) = self.write(&plan) {
            let _ = scheduler("cancel", &plan.id, None);
            return Err(format!("计划未启用，保存失败：{error}"));
        }
        Ok(plan)
    }
    pub fn cancel(&self, id: &str) -> Result<String, String> {
        let submitted = {
            let _lock = self.lock()?;
            let mut plan = match self.read(id) {
                Ok(plan) => plan,
                // Invalid files cannot grant runner permission. Still allow the
                // exact, ownership-checked Windows task to be stopped/removed.
                Err(error) if self.path(id)?.is_file() => {
                    drop(_lock);
                    scheduler("cancel", id, None).map_err(|e| {
                        format!("计划文件无效，执行器会拒绝运行；Windows 任务清理失败：{e}")
                    })?;
                    return Ok(format!(
                        "损坏计划对应的 Windows 任务已移除；可继续移除文件。{error}"
                    ));
                }
                Err(error) => return Err(error),
            };
            let submitted = plan.last.as_ref().is_some_and(|r| r.status == "submitted");
            plan.enabled = false;
            if let Some(last) = plan.last.as_mut().filter(|last| last.status == "countdown") {
                last.status = "cancelled".into();
                last.detail = "计划已取消，本次及后续执行已停用".into();
            }
            self.write(&plan)?; // Runner observes cancellation even if scheduler is unavailable.
            submitted
        };
        scheduler("cancel", id, None).map_err(|e| {
            format!("本机执行许可已撤销，Windows 任务清理未完成，可再次取消重试：{e}")
        })?;
        Ok(if submitted {
            "后续计划已取消；本次操作已提交 Windows，不能保证撤回。"
        } else {
            "计划已取消，Windows 定时任务已移除。"
        }
        .into())
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.cancel(id)?;
        let _lock = self.lock()?;
        fs::remove_file(self.path(id)?).map_err(|e| e.to_string())
    }
    pub fn inspect(&self, id: &str) -> Result<String, String> {
        let value = scheduler("inspect", id, None)?;
        let state = value["state"].as_str().unwrap_or("Unknown");
        Ok(match state {
            "Ready" => "Windows 任务就绪".into(),
            "Running" => "Windows 正在运行执行器".into(),
            "Disabled" => "Windows 任务已禁用".into(),
            "Missing" => "Windows 任务不存在，请移除后重新创建".into(),
            _ => format!("Windows 状态：{state}"),
        })
    }
    pub fn claim(&self, id: &str, now: i64) -> Result<Option<(Plan, i64)>, String> {
        let _lock = self.lock()?;
        let mut plan = self.read(id)?;
        if !plan.enabled {
            return Ok(None);
        }
        let Some(at) = plan.next(now - 15) else {
            return Ok(None);
        };
        if at - now > plan.action.lead() + 5
            || plan.last.as_ref().is_some_and(|last| last.occurrence >= at)
        {
            return Ok(None);
        }
        // Destructive actions need a useful cancellation window; missed actions are skipped.
        if plan.action != Action::Wake && at - now < 15 {
            return Ok(None);
        }
        plan.last = Some(Run {
            occurrence: at,
            status: "countdown".into(),
            detail: if plan.action == Action::Wake {
                "正在处理唤醒"
            } else {
                "执行前倒计时，可取消本次"
            }
            .into(),
        });
        self.write(&plan)?;
        Ok(Some((plan, at)))
    }
    pub fn record(&self, id: &str, at: i64, status: &str, detail: &str) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut plan = self.read(id)?;
        if plan.last.as_ref().is_some_and(|last| last.occurrence == at) {
            plan.last = Some(Run {
                occurrence: at,
                status: status.into(),
                detail: detail.into(),
            });
            self.write(&plan)?;
        }
        Ok(())
    }
    pub fn permitted(&self, id: &str, at: i64) -> bool {
        self.read(id).is_ok_and(|p| {
            p.enabled
                && p.last
                    .as_ref()
                    .is_some_and(|r| r.occurrence == at && r.status == "countdown")
        })
    }
    pub fn commit(&self, id: &str, at: i64, now: i64) -> Result<bool, String> {
        self.commit_with_power(id, at, now, ac_connected)
    }
    fn commit_with_power(
        &self,
        id: &str,
        at: i64,
        now: i64,
        power: impl FnOnce() -> Result<bool, String>,
    ) -> Result<bool, String> {
        let _lock = self.lock()?;
        let mut plan = self.read(id)?;
        if !plan.enabled
            || now < at
            || now > at + 15
            || !plan
                .last
                .as_ref()
                .is_some_and(|r| r.occurrence == at && r.status == "countdown")
        {
            return Ok(false);
        }
        if !plan.on_battery {
            let reason = match power() {
                Ok(true) => None,
                Ok(false) => Some("已切换到电池供电，按计划条件跳过本次操作。".to_owned()),
                Err(error) => Some(format!("无法确认已接通电源，跳过本次操作：{error}")),
            };
            if let Some(detail) = reason {
                plan.last = Some(Run {
                    occurrence: at,
                    status: "skipped".into(),
                    detail,
                });
                self.write(&plan)?;
                return Ok(false);
            }
        }
        plan.last = Some(Run {
            occurrence: at,
            status: "submitted".into(),
            detail: "已向 Windows 提交操作；实际结果以系统为准".into(),
        });
        self.write(&plan)?;
        Ok(true)
    }
}
fn ac_connected() -> Result<bool, String> {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return Err("Windows 电源状态读取失败".into());
    }
    match status.ACLineStatus {
        1 => Ok(true),
        0 => Ok(false),
        _ => Err("Windows 电源状态未知".into()),
    }
}
fn conflicts(a: &Plan, b: &Plan, now: i64) -> bool {
    let begin = local_timestamp(a.start)
        .unwrap_or(now)
        .max(local_timestamp(b.start).unwrap_or(now))
        .max(now);
    let mut left = a.next((begin - 120).max(now));
    let mut right = b.next((begin - 120).max(now));
    let end = begin + 9 * 86400;
    for _ in 0..32 {
        let (Some(x), Some(y)) = (left, right) else {
            break;
        };
        if x > end || y > end {
            break;
        }
        if (x - y).abs() < 120 {
            return true;
        }
        if x < y {
            left = a.next(x + 1);
        } else {
            right = b.next(y + 1);
        }
    }
    false
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(|e: std::io::Error| e.to_string())
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn task_xml(plan: &Plan, at: i64, helper: &Path) -> Result<String, String> {
    plan.validate()?;
    let trigger_time = Local
        .timestamp_opt(at - plan.action.lead(), 0)
        .single()
        .ok_or("触发时间无效。")?;
    // Local boundaries intentionally have no UTC suffix: calendar repeats follow local wall time.
    let boundary = trigger_time.format("%Y-%m-%dT%H:%M:%S");
    let trigger = match plan.repeat {
        Repeat::Once => format!(
            "<TimeTrigger><StartBoundary>{boundary}</StartBoundary><EndBoundary>{}</EndBoundary><Enabled>true</Enabled></TimeTrigger>",
            Local
                .timestamp_opt(at + 30, 0)
                .single()
                .unwrap()
                .format("%Y-%m-%dT%H:%M:%S")
        ),
        Repeat::Daily => format!(
            "<CalendarTrigger><StartBoundary>{boundary}</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>"
        ),
        Repeat::Weekdays => {
            // A 00:00 execution warns at 23:59 on the previous calendar day.
            let shift = if trigger_time.date_naive()
                < Local.timestamp_opt(at, 0).single().unwrap().date_naive()
            {
                6
            } else {
                0
            };
            let names = [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ];
            let days = (0..5)
                .map(|i| format!("<{}/>", names[(i + shift) % 7]))
                .collect::<String>();
            format!(
                "<CalendarTrigger><StartBoundary>{boundary}</StartBoundary><Enabled>true</Enabled><ScheduleByWeek><WeeksInterval>1</WeeksInterval><DaysOfWeek>{days}</DaysOfWeek></ScheduleByWeek></CalendarTrigger>"
            )
        }
    };
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?><Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><RegistrationInfo><Description>SHUNet.PowerPlan/1/{id}</Description></RegistrationInfo><Triggers>{trigger}</Triggers><Principals><Principal id="User"><UserId>__SHUNET_SID__</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>{battery}</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>{battery}</StopIfGoingOnBatteries><AllowHardTerminate>true</AllowHardTerminate><StartWhenAvailable>false</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><AllowStartOnDemand>false</AllowStartOnDemand><Enabled>true</Enabled><Hidden>false</Hidden><WakeToRun>{wake}</WakeToRun><ExecutionTimeLimit>PT0S</ExecutionTimeLimit></Settings><Actions Context="User"><Exec><Command>{helper}</Command><Arguments>--power-run {id}</Arguments></Exec></Actions></Task>"#,
        id = plan.id,
        battery = !plan.on_battery,
        wake = plan.action == Action::Wake,
        helper = escape(&helper.to_string_lossy())
    ))
}
pub fn system_file(relative: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32")
        .join(relative)
}
fn scheduler(operation: &str, id: &str, xml: Option<&str>) -> Result<serde_json::Value, String> {
    if !valid_id(id) {
        return Err("计划编号无效。".into());
    }
    let payload = base64::engine::general_purpose::STANDARD.encode(
        serde_json::to_vec(&serde_json::json!({"operation":operation,"id":id,"xml":xml}))
            .map_err(|e| e.to_string())?,
    );
    let script = format!(
        "$request = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{payload}')) | ConvertFrom-Json\n{}",
        include_str!("../assets/power-task.ps1")
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut process = Command::new(system_file("WindowsPowerShell\\v1.0\\powershell.exe"))
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        if process.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(25) {
            let _ = process.kill();
            let _ = process.wait();
            return Err("Windows 任务计划服务响应超时，请稍后刷新或重试。".into());
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let output = process.wait_with_output().map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        format!(
            "Windows 任务计划服务没有返回可识别的结果（退出码 {:?}）。",
            output.status.code()
        )
    })?;
    if !output.status.success() || value["ok"] != true {
        return Err(value["error"].as_str().unwrap_or("任务操作失败").to_owned());
    }
    Ok(value)
}

pub struct Capabilities {
    pub sleep: bool,
    pub hibernate: bool,
    pub text: String,
}
pub fn capabilities() -> Capabilities {
    let mut caps = SYSTEM_POWER_CAPABILITIES::default();
    if !unsafe { GetPwrCapabilities(&mut caps) } {
        return Capabilities {
            sleep: false,
            hibernate: false,
            text: "无法读取设备电源能力；定时唤醒仍受固件和电源策略限制。".into(),
        };
    }
    Capabilities {
        sleep: caps.SystemS1 || caps.SystemS2 || caps.SystemS3 || caps.AoAc,
        hibernate: caps.SystemS4 && caps.HiberFilePresent,
        text: format!(
            "设备支持：睡眠 {} · 休眠 {}。唤醒还需固件支持并允许唤醒计时器。",
            if caps.SystemS1 || caps.SystemS2 || caps.SystemS3 || caps.AoAc {
                "可用"
            } else {
                "不可用"
            },
            if caps.SystemS4 && caps.HiberFilePresent {
                "可用"
            } else {
                "未启用"
            }
        ),
    }
}
pub const HELP: &str = "定时唤醒适用于睡眠／休眠，不能保证从完全关机状态开机。完全关机后定时开机需在 BIOS/UEFI 中设置 RTC / Resume by Alarm，具体以电脑厂商为准。\r\n\r\nWindows 计划需保持当前用户登录（锁屏可以）；退出 SHUNet 或关闭此窗口不会取消计划。关机、重启、睡眠、休眠执行前约 60 秒弹出取消窗口，错过的操作不会在开机后补执行。默认只在接通电源时执行，可按需勾选电池供电也执行。\r\n\r\n关机和重启不强制关闭应用，未保存的文档可能阻止操作。唤醒可能保留锁屏，不会自动登录。休眠不可用时请在 Windows 中手动启用；本工具不会修改系统电源策略。\r\n\r\n每天／工作日按电脑本地时间执行，工作日指周一至周五，不含节假日调休。若电脑系统时区变化，请取消并重新创建计划。";

/// Only this explicit command-line entry may request a real power operation.
pub fn runner(id: &str) -> Result<(), String> {
    let store = Store::default();
    let now = Local::now().timestamp();
    let Some((plan, at)) = store.claim(id, now)? else {
        return Ok(());
    };
    if plan.action != Action::Wake && !crate::power_dialog::countdown(&store, &plan, at) {
        let _ = store.record(
            id,
            at,
            "cancelled",
            "本次执行已取消；重复计划的后续执行仍有效",
        );
        return Ok(());
    }
    let now = Local::now().timestamp();
    if !store.commit(id, at, now)? {
        if store.permitted(id, at) {
            let _ = store.record(
                id,
                at,
                "skipped",
                "计划被取消、时间发生变化或错过执行窗口，已跳过",
            );
        }
        return Ok(());
    }
    match execute(plan.action) {
        Ok(()) => store.record(
            id,
            at,
            "submitted",
            if plan.action == Action::Wake {
                "唤醒任务已运行；以设备实际状态为准"
            } else {
                "已向 Windows 提交操作；实际结果以系统为准"
            },
        ),
        Err(error) => {
            let _ = store.record(id, at, "failed", &error);
            Err(error)
        }
    }
}
fn execute(action: Action) -> Result<(), String> {
    match action {
        Action::Shutdown | Action::Restart => {
            // /t > 0 implies /f. The warning is implemented by our own cancellable UI.
            let status = Command::new(system_file("shutdown.exe"))
                .args([
                    if action == Action::Shutdown {
                        "/s"
                    } else {
                        "/r"
                    },
                    "/t",
                    "0",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(|e| e.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "Windows 拒绝电源操作（退出码 {:?}），可能存在权限或未保存的工作。",
                    status.code()
                ))
            }
        }
        Action::Sleep | Action::Hibernate => {
            let _privilege = ShutdownPrivilege::enable()?;
            if unsafe { SetSuspendState(action == Action::Hibernate, false, false) } {
                Ok(())
            } else {
                Err(format!(
                    "Windows 无法进入{}：{}",
                    action.label(),
                    std::io::Error::last_os_error()
                ))
            }
        }
        Action::Wake => {
            unsafe {
                SetThreadExecutionState(ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
            }
            Ok(())
        }
    }
}
struct ShutdownPrivilege {
    token: HANDLE,
    previous: TOKEN_PRIVILEGES,
}
impl ShutdownPrivilege {
    fn enable() -> Result<Self, String> {
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            ) == 0
            {
                return Err("无法读取电源操作权限。".into());
            }
            let mut luid = LUID::default();
            if LookupPrivilegeValueW(
                std::ptr::null(),
                crate::wide("SeShutdownPrivilege").as_ptr(),
                &mut luid,
            ) == 0
            {
                CloseHandle(token);
                return Err("无法查询电源操作权限。".into());
            }
            let requested = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let mut previous = TOKEN_PRIVILEGES::default();
            let mut size = 0;
            SetLastError(0);
            let ok = AdjustTokenPrivileges(
                token,
                0,
                &requested,
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut previous,
                &mut size,
            );
            if ok == 0 || GetLastError() == ERROR_NOT_ALL_ASSIGNED {
                CloseHandle(token);
                return Err("当前 Windows 用户没有休眠／睡眠所需权限。".into());
            }
            Ok(Self { token, previous })
        }
    }
}
impl Drop for ShutdownPrivilege {
    fn drop(&mut self) {
        unsafe {
            AdjustTokenPrivileges(
                self.token,
                0,
                &self.previous,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            CloseHandle(self.token);
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    pub struct Temporary(pub Store);
    impl Temporary {
        pub fn new() -> Self {
            static SERIAL: AtomicU64 = AtomicU64::new(0);
            let folder = std::env::temp_dir().join(format!(
                "SHUNetPowerTest-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            assert!(folder.is_absolute() && folder.starts_with(std::env::temp_dir()));
            assert!(
                folder
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("SHUNetPowerTest-")
            );
            Self(Store { folder })
        }
        pub fn save(&self, plan: &Plan) {
            self.0.write(plan).unwrap();
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.folder);
        }
    }
    fn datetime(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
    }
    fn timestamp(text: &str) -> i64 {
        local_timestamp(datetime(text)).unwrap()
    }
    fn plan(action: Action, time: &str, repeat: Repeat) -> Plan {
        Plan::new(action, datetime(time), repeat, false).unwrap()
    }
    #[test]
    fn repeat_calendar_handles_weekends_start_dates_and_midnight() {
        let work = plan(Action::Shutdown, "2026-10-16 00:00:00", Repeat::Weekdays);
        assert_eq!(
            work.next(timestamp("2026-10-17 10:00:00")),
            Some(timestamp("2026-10-19 00:00:00"))
        );
        let later = plan(Action::Sleep, "2026-11-10 18:30:00", Repeat::Daily);
        assert_eq!(
            later.next(timestamp("2026-10-18 10:00:00")),
            Some(timestamp("2026-11-10 18:30:00"))
        );
        let one = plan(Action::Wake, "2026-10-18 08:00:00", Repeat::Once);
        assert!(one.next(timestamp("2026-10-18 08:00:01")).is_none());
        assert!(relative_time("1", 0).is_err());
        assert!(relative_time("10081", 0).is_err());
        assert!(relative_time("1;shutdown", 0).is_err());
        assert_eq!(
            local_timestamp(relative_time("60", timestamp("2026-10-18 23:45:00")).unwrap())
                .unwrap(),
            timestamp("2026-10-19 00:45:00")
        );
    }
    #[test]
    fn task_xml_reserves_warning_and_shifts_weekdays_before_midnight() {
        let plan = plan(Action::Shutdown, "2026-10-19 00:00:00", Repeat::Weekdays);
        let xml = task_xml(
            &plan,
            timestamp("2026-10-19 00:00:00"),
            Path::new("C:\\A&B\\校园网\\power.exe"),
        )
        .unwrap();
        assert!(xml.contains("2026-10-18T23:59:00"));
        assert!(xml.contains("<Sunday/>"));
        assert!(!xml.contains("<Friday/>"));
        assert!(xml.contains("A&amp;B"));
        assert!(xml.contains("<WakeToRun>false</WakeToRun>"));
        assert!(xml.contains("<StartWhenAvailable>false</StartWhenAvailable>"));
        assert!(xml.contains("<AllowStartOnDemand>false</AllowStartOnDemand>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>true</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(!xml.contains("/f"));
        let wake = plan_with_wake();
        let xml = task_xml(
            &wake,
            timestamp("2026-10-19 08:00:00"),
            Path::new("C:\\power.exe"),
        )
        .unwrap();
        assert!(xml.contains("2026-10-19T08:00:00"));
        assert!(xml.contains("<WakeToRun>true</WakeToRun>"));
    }
    fn plan_with_wake() -> Plan {
        plan(Action::Wake, "2026-10-19 08:00:00", Repeat::Daily)
    }
    #[test]
    fn conflicts_detect_future_weekday_overlap_and_midnight_neighbours() {
        let a = plan(Action::Restart, "2026-10-18 08:00:00", Repeat::Daily);
        let b = plan(Action::Shutdown, "2026-10-19 08:01:00", Repeat::Weekdays);
        assert!(conflicts(&a, &b, timestamp("2026-10-18 00:00:00")));
        let midnight = plan(Action::Sleep, "2026-10-18 23:59:30", Repeat::Daily);
        let next = plan(Action::Shutdown, "2026-10-19 00:00:00", Repeat::Once);
        assert!(conflicts(
            &midnight,
            &next,
            timestamp("2026-10-18 00:00:00")
        ));
    }
    #[test]
    fn claim_commit_and_cancel_are_fail_closed_and_deduplicated() {
        let temporary = Temporary::new();
        let store = &temporary.0;
        let mut plan = plan(Action::Shutdown, "2026-10-19 08:00:00", Repeat::Daily);
        plan.enabled = true;
        plan.on_battery = true; // Test execution permission independently of this PC's AC state.
        temporary.save(&plan);
        let at = timestamp("2026-10-19 08:00:00");
        assert!(store.claim(&plan.id, at - 120).unwrap().is_none());
        assert!(store.claim(&plan.id, at - 60).unwrap().is_some());
        assert!(store.claim(&plan.id, at - 59).unwrap().is_none());
        assert!(store.permitted(&plan.id, at));
        assert!(!store.commit(&plan.id, at, at - 1).unwrap());
        assert!(store.commit(&plan.id, at, at).unwrap());
        assert!(!store.commit(&plan.id, at, at).unwrap());
        assert!(!store.permitted(&plan.id, at));
        // Next recurrence is independent; local revocation prevents execution immediately.
        let next = at + 86400;
        assert!(store.claim(&plan.id, next - 60).unwrap().is_some());
        let mut saved = store.read(&plan.id).unwrap();
        saved.enabled = false;
        temporary.save(&saved);
        assert!(!store.permitted(&plan.id, next));
        assert!(!store.commit(&plan.id, next, next).unwrap());
    }
    #[test]
    fn countdown_summary_and_battery_commit_follow_actual_permission() {
        let temporary = Temporary::new();
        let store = &temporary.0;
        let mut plan = plan(Action::Shutdown, "2026-10-19 08:00:00", Repeat::Once);
        plan.enabled = true;
        let at = timestamp("2026-10-19 08:00:00");
        for ac in [Some(false), None, Some(true)] {
            temporary.save(&plan);
            let (claimed, _) = store.claim(&plan.id, at - 60).unwrap().unwrap();
            assert_eq!(claimed.display(at - 60), (Some(at), "倒计时中"));
            assert!(claimed.summary(at - 60).contains("倒计时中"));
            assert_eq!(claimed.display(at + 16).1, "未确认执行");
            let committed = store
                .commit_with_power(&plan.id, at, at, || ac.ok_or("unknown AC".into()))
                .unwrap();
            assert_eq!(committed, ac == Some(true));
            let saved = store.read(&plan.id).unwrap();
            assert_eq!(
                saved.last.unwrap().status,
                if committed { "submitted" } else { "skipped" }
            );
            assert!(!store.permitted(&plan.id, at));
        }
    }

    #[test]
    fn corrupt_plan_is_isolated_and_metadata_detects_changes() {
        let temporary = Temporary::new();
        let store = &temporary.0;
        let valid = plan(Action::Wake, "2026-10-19 08:00:00", Repeat::Daily);
        temporary.save(&valid);
        let stamp = store.stamp().unwrap();
        let bad_id = "1".repeat(32);
        fs::write(store.path(&bad_id).unwrap(), "invalid").unwrap();
        fs::write(store.folder.join("unrelated.json"), "leave alone").unwrap();
        let listing = store.list_with_issues().unwrap();
        assert_eq!(listing.plans.len(), 1);
        assert_eq!(listing.plans[0].id, valid.id);
        assert_eq!(listing.issues[0].id, bad_id);
        assert_eq!(listing.issues.len(), 1);
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(!store.permitted(&bad_id, 0));
        assert_ne!(store.stamp().unwrap(), stamp);
        fs::remove_file(store.path(&bad_id).unwrap()).unwrap();
        assert_eq!(store.stamp().unwrap(), stamp);
    }

    #[test]
    fn missed_and_corrupt_plans_never_become_power_operations() {
        let temporary = Temporary::new();
        let store = &temporary.0;
        let mut plan = plan(Action::Hibernate, "2026-10-19 08:00:00", Repeat::Once);
        plan.enabled = true;
        temporary.save(&plan);
        let at = timestamp("2026-10-19 08:00:00");
        assert!(store.claim(&plan.id, at + 1).unwrap().is_none());
        assert!(store.claim(&plan.id, at + 86400).unwrap().is_none());
        assert!(!store.permitted(&plan.id, at));
        assert!(store.read("../settings").is_err());
        fs::write(store.path(&plan.id).unwrap(), "not json").unwrap();
        assert!(store.claim(&plan.id, at - 60).is_err());
    }
    #[test]
    #[ignore = "Registers and immediately removes only a uniquely named wake task 30 days in the future; never runs a task or power action"]
    fn native_power_task_registration_and_cleanup() {
        let temporary = Temporary::new();
        let store = &temporary.0;
        let start = (Local::now() + TimeDelta::days(30)).naive_local();
        for repeat in [Repeat::Once, Repeat::Daily, Repeat::Weekdays] {
            let plan = Plan::new(Action::Wake, start, repeat, false).unwrap();
            let id = plan.id.clone();
            struct Cleanup(String);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    let _ = scheduler("cancel", &self.0, None);
                }
            }
            let _cleanup = Cleanup(id.clone());
            let saved = store.create(plan).unwrap();
            assert!(saved.enabled);
            assert!(store.inspect(&id).unwrap().contains("就绪"));
            if repeat == Repeat::Weekdays {
                fs::write(store.path(&id).unwrap(), "corrupt fixture").unwrap();
                assert_eq!(store.list_with_issues().unwrap().issues.len(), 1);
                assert!(store.cancel(&id).unwrap().contains("Windows 任务已移除"));
                assert!(store.read(&id).is_err());
            } else {
                assert!(store.cancel(&id).unwrap().contains("已取消"));
                assert!(!store.read(&id).unwrap().enabled);
            }
            assert!(store.inspect(&id).unwrap().contains("不存在"));
            store.remove(&id).unwrap();
            assert!(store.list().unwrap().is_empty());
        }
    }
}
