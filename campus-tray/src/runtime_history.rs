//! Persistent, rolling 72-hour local history. No credential/session identifiers.
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const RETENTION_MS: i64 = 72 * 60 * 60 * 1000;
const HOUR_MS: i64 = 60 * 60 * 1000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Message,
    AutoReconnectRequest,
    AutoReconnectOnline,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub timestamp: i64,
    pub event: Event,
    pub attempt: Option<u64>,
    pub text: String,
}
impl Entry {
    pub fn line(&self) -> String {
        let category = match self.event {
            Event::Message => "记录",
            Event::AutoReconnectRequest => "重连",
            Event::AutoReconnectOnline => "恢复",
        };
        format!(
            "[{}] [{category}] {}",
            shanghai_time(self.timestamp, "%m-%d %H:%M:%S"),
            self.text
        )
    }
}
#[derive(Clone, Copy, Default)]
pub struct DailyStats {
    pub attempts: u32,
    pub recovered: u32,
}
// Snapshots share sealed blocks. Appending copies at most one 128-entry tail,
// instead of every String in the retained 72-hour history.
const BLOCK: usize = 128;
#[derive(Clone, Default)]
pub struct Entries {
    blocks: Vec<Arc<Vec<Entry>>>,
    len: usize,
    stats: Arc<Mutex<Option<(i64, DailyStats)>>>,
}
impl Entries {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Entry> + Clone {
        self.blocks.iter().flat_map(|block| block.iter())
    }
    fn push(&mut self, entry: Entry) {
        if entry.event != Event::Message {
            self.stats = Arc::default();
        }
        if self.len % BLOCK == 0 {
            self.blocks.push(Arc::new(Vec::with_capacity(BLOCK)));
        }
        Arc::make_mut(self.blocks.last_mut().unwrap()).push(entry);
        self.len += 1;
    }
    fn retain(&mut self, predicate: impl Fn(&Entry) -> bool) {
        *self = self
            .iter()
            .filter(|entry| predicate(entry))
            .cloned()
            .collect::<Vec<_>>()
            .into();
    }
    pub fn stats(&self, now: i64) -> DailyStats {
        let today = (now + 8 * 3_600_000).div_euclid(86_400_000);
        let mut cache = self.stats.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((cached_day, stats)) = *cache {
            if cached_day == today {
                return stats;
            }
        }
        let stats = daily_stats(self.iter(), now);
        *cache = Some((today, stats));
        stats
    }
}
impl From<Vec<Entry>> for Entries {
    fn from(entries: Vec<Entry>) -> Self {
        let mut result = Self::default();
        for entry in entries {
            result.push(entry);
        }
        result
    }
}
impl std::ops::Index<usize> for Entries {
    type Output = Entry;
    fn index(&self, index: usize) -> &Entry {
        &self.blocks[index / BLOCK][index % BLOCK]
    }
}
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
pub fn shanghai_time(timestamp: i64, format: &str) -> String {
    DateTime::<Utc>::from_timestamp_millis(timestamp)
        .map(|time| {
            time.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
                .format(format)
                .to_string()
        })
        .unwrap_or_default()
}
pub fn day(timestamp: i64) -> String {
    shanghai_time(timestamp, "%Y-%m-%d")
}
pub fn daily_stats<'a>(entries: impl IntoIterator<Item = &'a Entry>, now: i64) -> DailyStats {
    let today = (now + 8 * 3_600_000).div_euclid(86_400_000);
    let (mut attempts, mut recovered) = (BTreeSet::new(), BTreeSet::new());
    for entry in entries {
        match entry.event {
            Event::AutoReconnectRequest
                if (entry.timestamp + 8 * 3_600_000).div_euclid(86_400_000) == today =>
            {
                attempts.insert(entry.id);
            }
            Event::AutoReconnectOnline => {
                if let Some(id) = entry.attempt {
                    recovered.insert(id);
                }
            }
            _ => {}
        }
    }
    DailyStats {
        attempts: attempts.len().min(u32::MAX as usize) as u32,
        recovered: recovered
            .intersection(&attempts)
            .count()
            .min(u32::MAX as usize) as u32,
    }
}
pub fn folder() -> PathBuf {
    crate::settings::SettingsStore::new()
        .path()
        .parent()
        .unwrap()
        .join("runtime-logs")
}
pub struct History {
    path: Option<PathBuf>,
    entries: Arc<Entries>,
    next_id: u64,
    next_cleanup: i64,
    pub warning: Option<String>,
}
impl History {
    pub fn memory() -> Self {
        Self {
            path: None,
            entries: Arc::default(),
            next_id: 1,
            next_cleanup: 0,
            warning: None,
        }
    }
    pub fn open(path: PathBuf) -> Self {
        let now = now_ms();
        let mut history = Self {
            path: Some(path),
            ..Self::memory()
        };
        if let Err(error) = history.load(now) {
            history.warning = Some(error);
        }
        history.maintain(now);
        history
    }
    fn load(&mut self, now: i64) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        fs::create_dir_all(path).map_err(|_| "无法创建运行日志目录，当前记录暂存内存。")?;
        let files = log_files(path)?;
        let mut entries = Vec::new();
        for file in files {
            let input = fs::File::open(file).map_err(|_| "部分历史日志无法读取。")?;
            for line in BufReader::new(input).lines() {
                let Ok(line) = line else {
                    self.warning = Some("部分历史日志读取失败，已保留原文件。".into());
                    break;
                };
                if line.len() > 8192 {
                    continue;
                }
                let Ok(mut entry) = serde_json::from_str::<Entry>(&line) else {
                    continue;
                };
                entry.text = clean(&entry.text);
                if entry.timestamp >= now - RETENTION_MS && entry.timestamp <= now + 5 * 60 * 1000 {
                    entries.push(entry);
                }
            }
        }
        entries.sort_by_key(|entry| (entry.timestamp, entry.id));
        self.next_id = entries
            .iter()
            .map(|entry| entry.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.entries = Arc::new(entries.into());
        Ok(())
    }
    pub fn snapshot(&self) -> Arc<Entries> {
        self.entries.clone()
    }
    pub fn stats(&self) -> DailyStats {
        self.entries.stats(now_ms())
    }
    pub fn record(&mut self, event: Event, attempt: Option<u64>, text: &str) -> u64 {
        self.record_at(event, attempt, text, now_ms())
    }
    fn record_at(&mut self, event: Event, attempt: Option<u64>, text: &str, now: i64) -> u64 {
        self.maintain(now);
        let entry = Entry {
            id: self.next_id,
            timestamp: now,
            event,
            attempt,
            text: clean(text),
        };
        self.next_id = self.next_id.saturating_add(1);
        if let Some(path) = &self.path {
            let written = (|| -> std::io::Result<()> {
                fs::create_dir_all(path)?;
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path.join(format!("shunet-log-{}.jsonl", day(now))))?;
                serde_json::to_writer(&mut file, &entry)?;
                file.write_all(b"\n")?;
                file.flush()
            })();
            if written.is_err() {
                self.warning = Some("运行日志暂时无法保存，当前记录仍保留在内存中。".into());
            }
        }
        let id = entry.id;
        Arc::make_mut(&mut self.entries).push(entry);
        id
    }
    pub fn maintain(&mut self, now: i64) {
        if now < self.next_cleanup {
            return;
        }
        self.next_cleanup = now.saturating_add(HOUR_MS);
        let cutoff = now - RETENTION_MS;
        if self.entries.iter().any(|entry| entry.timestamp < cutoff) {
            Arc::make_mut(&mut self.entries).retain(|entry| entry.timestamp >= cutoff);
        }
        if let Some(path) = &self.path {
            if let Err(error) = prune_files(path, cutoff) {
                self.warning = Some(error);
            }
        }
    }
}
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(400).collect()
}
fn log_files(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(folder).map_err(|_| "无法读取运行日志目录。")? {
        let entry = entry.map_err(|_| "无法读取部分日志文件。")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(date) = name
            .strip_prefix("shunet-log-")
            .and_then(|name| name.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if date.len() == 10
            && chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
            && entry.file_type().is_ok_and(|kind| kind.is_file())
        {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}
fn prune_files(folder: &Path, cutoff: i64) -> Result<(), String> {
    if !folder.exists() {
        return Ok(());
    }
    let cutoff_day = day(cutoff);
    for path in log_files(folder)? {
        let name = path.file_name().unwrap().to_string_lossy();
        let date = &name["shunet-log-".len().."shunet-log-".len() + 10];
        if date < cutoff_day.as_str() {
            fs::remove_file(&path).map_err(|_| "过期日志清理失败，将稍后重试。")?;
        } else if date == cutoff_day {
            let input =
                fs::read_to_string(&path).map_err(|_| "边界日期日志读取失败，暂未清理。")?;
            let mut retained = String::new();
            let mut changed = false;
            for line in input.lines() {
                if serde_json::from_str::<Entry>(line).is_ok_and(|entry| entry.timestamp < cutoff) {
                    changed = true;
                } else {
                    retained.push_str(line);
                    retained.push('\n');
                }
            }
            if changed {
                let temporary = path.with_extension("jsonl.tmp");
                fs::write(&temporary, retained).map_err(|_| "过期日志整理失败。")?;
                fs::rename(&temporary, &path).map_err(|_| "过期日志替换失败，将稍后重试。")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_snapshots_keep_sealed_strings_and_stats_independent() {
        let now = timestamp("2026-10-18T23:59:59+08:00");
        let mut history = History::memory();
        for _ in 0..1024 {
            history.record_at(Event::Message, None, "retained text", now);
        }
        let attempt = history.record_at(Event::AutoReconnectRequest, None, "request", now);
        let old = history.snapshot();
        let original = old[0].text.as_ptr();
        assert_eq!(old.stats(now).attempts, 1);
        history.record_at(Event::AutoReconnectOnline, Some(attempt), "online", now);
        let new = history.snapshot();
        assert_eq!(new[0].text.as_ptr(), original);
        assert_eq!(old.len(), 1025);
        assert_eq!(new.len(), 1026);
        assert_eq!(old.stats(now).recovered, 0);
        assert_eq!(new.stats(now).recovered, 1);
        assert_eq!(new.stats(now + 2000).attempts, 0);
        assert_eq!(new.iter().rev().next().unwrap().text, "online");
        history.next_cleanup = 0;
        history.maintain(now + RETENTION_MS + 1);
        assert!(history.snapshot().is_empty());
        assert_eq!(old.len(), 1025);
    }
    fn timestamp(value: &str) -> i64 {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn today_uses_shanghai_midnight_and_counts_unique_confirmed_attempts() {
        let now = timestamp("2026-10-18T00:00:05+08:00");
        let entries = vec![
            Entry {
                id: 1,
                timestamp: now - 10_000,
                event: Event::AutoReconnectRequest,
                attempt: None,
                text: String::new(),
            },
            Entry {
                id: 2,
                timestamp: now - 1000,
                event: Event::AutoReconnectRequest,
                attempt: None,
                text: String::new(),
            },
            Entry {
                id: 3,
                timestamp: now,
                event: Event::AutoReconnectOnline,
                attempt: Some(2),
                text: String::new(),
            },
            Entry {
                id: 4,
                timestamp: now,
                event: Event::AutoReconnectOnline,
                attempt: Some(2),
                text: String::new(),
            },
            Entry {
                id: 5,
                timestamp: now,
                event: Event::Message,
                attempt: None,
                text: "手动认证".into(),
            },
        ];
        let stats = daily_stats(entries.iter(), now);
        assert_eq!(stats.attempts, 1);
        assert_eq!(stats.recovered, 1);
    }
    #[test]
    fn retention_survives_restart_and_only_removes_owned_expired_files() {
        let path = std::env::temp_dir().join(format!(
            "shunet-history-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&path).unwrap();
        let now = timestamp("2026-10-18T17:00:00+08:00");
        let mut history = History {
            path: Some(path.clone()),
            ..History::memory()
        };
        history.record_at(Event::Message, None, "old", now - RETENTION_MS - 1);
        let attempt = history.record_at(Event::AutoReconnectRequest, None, "request", now - 1000);
        history.record_at(Event::AutoReconnectOnline, Some(attempt), "online", now);
        fs::write(path.join("keep.txt"), "untouched").unwrap();
        history.next_cleanup = 0;
        history.maintain(now);
        assert!(
            history.warning.is_none(),
            "cleanup must actually succeed on Windows"
        );
        for file in log_files(&path).unwrap() {
            assert!(
                !fs::read_to_string(file).unwrap().contains("\"old\""),
                "expired records must be removed from disk too"
            );
        }
        assert!(history.snapshot().iter().all(|entry| entry.text != "old"));
        let mut restored = History {
            path: Some(path.clone()),
            ..History::memory()
        };
        restored.load(now).unwrap();
        let stats = restored.snapshot().stats(now);
        assert_eq!(stats.attempts, 1);
        assert_eq!(stats.recovered, 1);
        assert_eq!(
            fs::read_to_string(path.join("keep.txt")).unwrap(),
            "untouched"
        );
        for file in fs::read_dir(&path).unwrap() {
            fs::remove_file(file.unwrap().path()).unwrap();
        }
        fs::remove_dir(path).unwrap();
    }
}
