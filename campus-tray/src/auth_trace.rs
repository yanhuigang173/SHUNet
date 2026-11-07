use std::{cell::RefCell, collections::BTreeMap, time::Instant};
use serde::Serialize;

#[derive(Default, Serialize)]
pub struct Timing {
    pub calls: u64,
    pub milliseconds: f64,
}

#[derive(Default, Serialize)]
pub struct Report {
    pub total_ms: f64,
    pub phases: BTreeMap<&'static str, Timing>,
    pub requests: BTreeMap<&'static str, u64>,
}

thread_local! {
    static ACTIVE: RefCell<Option<(Instant, Report)>> = const { RefCell::new(None) };
}

pub fn start() {
    ACTIVE.with(|slot| *slot.borrow_mut() = Some((Instant::now(), Report::default())));
}

pub fn start_if_idle() -> bool {
    ACTIVE.with(|slot| {
        let mut active = slot.borrow_mut();
        if active.is_some() { return false; }
        *active = Some((Instant::now(), Report::default()));
        true
    })
}

pub fn finish() -> Option<Report> {
    ACTIVE.with(|slot| slot.borrow_mut().take().map(|(start, mut report)| {
        report.total_ms = start.elapsed().as_secs_f64() * 1000.0;
        report
    }))
}

pub struct Phase(&'static str, Option<Instant>);

pub fn record(name: &'static str, elapsed: std::time::Duration) {
    ACTIVE.with(|slot| {
        if let Some((_, report)) = slot.borrow_mut().as_mut() {
            let timing = report.phases.entry(name).or_default();
            timing.calls += 1;
            timing.milliseconds += elapsed.as_secs_f64() * 1000.0;
        }
    });
}

pub fn phase(name: &'static str) -> Phase {
    Phase(name, ACTIVE.with(|slot| slot.borrow().as_ref().map(|_| Instant::now())))
}

impl Drop for Phase {
    fn drop(&mut self) {
        if let Some(start) = self.1 {
            ACTIVE.with(|slot| {
                if let Some((_, report)) = slot.borrow_mut().as_mut() {
                    let timing = report.phases.entry(self.0).or_default();
                    timing.calls += 1;
                    timing.milliseconds += start.elapsed().as_secs_f64() * 1000.0;
                }
            });
        }
    }
}

pub trait PortalRequest {
    fn send_traced(self, kind: &'static str) -> Result<reqwest::blocking::Response, reqwest::Error>;
}

impl PortalRequest for reqwest::blocking::RequestBuilder {
    fn send_traced(self, kind: &'static str) -> Result<reqwest::blocking::Response, reqwest::Error> {
        ACTIVE.with(|slot| {
            if let Some((_, report)) = slot.borrow_mut().as_mut() {
                *report.requests.entry(kind).or_default() += 1;
            }
        });
        self.send()
    }
}
