use std::time::{Duration, Instant};

use crate::operator::Operator;

/// Session identifiers stay in memory and are never included in display text.
#[derive(Clone)]
pub struct SessionEvidence {
    pub id: String,
    pub operator: Option<Operator>,
}

#[derive(Clone)]
struct Clock {
    id: String,
    started: Instant,
    confirmed: Instant,
    operator: Option<Operator>,
    observed_only: bool,
}

#[derive(Clone, Default)]
pub struct OnlineSession {
    current: Option<Clock>,
    pending: Option<(Instant, Operator, bool)>,
}

impl OnlineSession {
    pub fn submitted(&mut self, operator: Operator, accepted: bool) {
        self.current = None;
        self.pending = Some((Instant::now(), operator, accepted));
    }

    pub fn confirm(&mut self, evidence: Option<&SessionEvidence>) {
        self.confirm_at(evidence, Instant::now());
    }

    fn confirm_at(&mut self, evidence: Option<&SessionEvidence>, now: Instant) {
        let id = evidence.map_or("", |session| session.id.as_str());
        if let Some(current) = self.current.as_mut().filter(|current| current.id == id) {
            current.confirmed = now;
            if let Some(operator) = evidence.and_then(|session| session.operator) {
                current.operator = Some(operator);
            }
            return;
        }
        let pending = self.pending.take();
        self.current = Some(Clock {
            id: id.to_owned(),
            started: pending.filter(|(_, _, accepted)| *accepted).map_or(now, |(start, _, _)| start),
            confirmed: now,
            operator: evidence.and_then(|session| session.operator).or_else(|| pending.map(|(_, operator, _)| operator)),
            observed_only: pending.is_none_or(|(_, _, accepted)| !accepted),
        });
    }

    pub fn clear(&mut self) { *self = Self::default(); }

    pub fn operator(&self) -> Option<Operator> { self.current.as_ref().and_then(|clock| clock.operator) }

    pub fn has_session(&self) -> bool { self.current.is_some() }

    pub fn duration_text(&self, live: bool) -> String {
        self.duration_at(live, Instant::now())
    }

    fn duration_at(&self, live: bool, now: Instant) -> String {
        let Some(clock) = &self.current else { return "本次在线时长：—".into(); };
        let end = if live { now } else { clock.confirmed };
        let elapsed = end.saturating_duration_since(clock.started);
        let qualifier = match (clock.observed_only, live) {
            (true, true) => "（首检起）",
            (true, false) => "（首检起，待确认）",
            (false, true) => "",
            (false, false) => "（待确认）",
        };
        format!("本次在线时长：{}{qualifier}", format_duration(elapsed))
    }
}

fn format_duration(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let hours = seconds / 3600;
    let clock = format!("{:02}:{:02}:{:02}", hours % 24, seconds / 60 % 60, seconds % 60);
    if hours >= 24 { format!("{}天 {clock}", hours / 24) } else { clock }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(id: &str, operator: Option<Operator>) -> SessionEvidence {
        SessionEvidence { id: id.into(), operator }
    }

    #[test]
    fn repeated_confirmation_preserves_start_and_unknown_status_freezes_at_confirmation() {
        let mut session = OnlineSession::default();
        let start = Instant::now();
        session.pending = Some((start, Operator::Campus, true));
        let info = evidence("session-a", Some(Operator::Campus));
        session.confirm_at(Some(&info), start + Duration::from_secs(3));
        session.confirm_at(Some(&info), start + Duration::from_secs(30));
        assert_eq!(session.duration_at(true, start + Duration::from_secs(75)), "本次在线时长：00:01:15");
        assert_eq!(session.duration_at(false, start + Duration::from_secs(75)), "本次在线时长：00:00:30（待确认）");
        assert_eq!(session.operator(), Some(Operator::Campus));
    }

    #[test]
    fn newly_observed_or_changed_session_never_invents_prior_authentication_time() {
        let mut session = OnlineSession::default();
        let start = Instant::now();
        session.confirm_at(Some(&evidence("a", Some(Operator::Telecom))), start);
        assert_eq!(session.duration_at(true, start + Duration::from_secs(10)), "本次在线时长：00:00:10（首检起）");
        session.confirm_at(Some(&evidence("b", Some(Operator::Mobile))), start + Duration::from_secs(20));
        assert_eq!(session.duration_at(true, start + Duration::from_secs(22)), "本次在线时长：00:00:02（首检起）");
        assert_eq!(session.operator(), Some(Operator::Mobile));
        session.clear();
        assert_eq!(session.duration_text(true), "本次在线时长：—");
        assert_eq!(session.operator(), None);
    }

    #[test]
    fn pending_authentication_keeps_its_operator_until_online_and_unknown_reply_is_estimated() {
        let mut session = OnlineSession::default();
        session.submitted(Operator::Campus, true);
        assert!(!session.has_session());
        assert_eq!(session.operator(), None);
        session.confirm(Some(&evidence("a", None)));
        // A different next-login setting is not an input to this clock.
        assert_eq!(session.operator(), Some(Operator::Campus));
        assert!(!session.duration_text(true).contains("首检起"));
        session.submitted(Operator::Unicom, false);
        session.confirm(Some(&evidence("b", Some(Operator::Unicom))));
        assert!(session.duration_text(true).contains("首检起"));
        assert_eq!(session.operator(), Some(Operator::Unicom));
    }

    #[test]
    fn long_sessions_include_days_without_wrapping_to_zero() {
        assert_eq!(format_duration(Duration::from_secs(90_061)), "1天 01:01:01");
    }
}
