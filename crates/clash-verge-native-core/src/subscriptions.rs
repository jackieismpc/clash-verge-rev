use anyhow::{Result, bail};
use std::collections::BTreeMap;

pub const DEFAULT_INTERVAL_MINUTES: u64 = 1440;
pub const MAX_INTERVAL_MINUTES: u64 = 525_600;
pub const MAX_RETRIES: u8 = 2;

pub fn validate_interval(minutes: Option<u64>) -> Result<()> {
    if minutes.is_some_and(|value| value > MAX_INTERVAL_MINUTES) {
        bail!("update interval must be 0 through 525600 minutes; 0 disables automatic updates");
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleProfile {
    pub id: String,
    pub url: String,
    pub updated_at: u64,
    pub interval_minutes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateTicket {
    pub profile: ScheduleProfile,
    pub epoch: u64,
}

#[derive(Clone, Debug)]
struct Entry {
    profile: ScheduleProfile,
    next_due: u64,
    failures: u8,
}

#[derive(Default)]
pub struct Scheduler {
    entries: BTreeMap<String, Entry>,
    in_flight: Option<UpdateTicket>,
    epoch: u64,
}

impl Scheduler {
    pub fn claim_due(&mut self, profiles: &[ScheduleProfile], now: u64) -> Option<UpdateTicket> {
        self.entries
            .retain(|id, _| profiles.iter().any(|p| &p.id == id && enabled(p)));
        for profile in profiles.iter().filter(|p| enabled(p)) {
            if self
                .entries
                .get(&profile.id)
                .is_none_or(|entry| entry.profile != *profile)
            {
                self.entries.insert(
                    profile.id.clone(),
                    Entry {
                        profile: profile.clone(),
                        next_due: profile.updated_at.saturating_add(interval(profile)),
                        failures: 0,
                    },
                );
            }
        }
        // The caller cancels a stale network task before claiming another ticket.
        if self.in_flight.is_some() {
            return None;
        }
        let entry = self
            .entries
            .values()
            .filter(|entry| entry.next_due <= now)
            .min_by_key(|entry| (entry.next_due, &entry.profile.id))?;
        self.epoch = self.epoch.wrapping_add(1);
        let ticket = UpdateTicket {
            profile: entry.profile.clone(),
            epoch: self.epoch,
        };
        self.in_flight = Some(ticket.clone());
        Some(ticket)
    }

    pub fn is_current(&self, ticket: &UpdateTicket, profiles: &[ScheduleProfile]) -> bool {
        self.in_flight.as_ref() == Some(ticket)
            && profiles
                .iter()
                .any(|profile| profile == &ticket.profile && enabled(profile))
    }

    pub fn finish(&mut self, ticket: &UpdateTicket, now: u64, succeeded: bool) {
        if self.in_flight.as_ref() != Some(ticket) {
            return;
        }
        self.in_flight = None;
        if let Some(entry) = self.entries.get_mut(&ticket.profile.id) {
            if entry.profile != ticket.profile {
                return;
            }
            if succeeded {
                entry.failures = 0;
                entry.next_due = now.saturating_add(interval(&entry.profile));
            } else if entry.failures < MAX_RETRIES {
                entry.failures += 1;
                entry.next_due = now.saturating_add(30 * u64::from(entry.failures));
            } else {
                entry.failures = 0;
                entry.next_due = now.saturating_add(interval(&entry.profile));
            }
        }
    }

    pub fn cancel(&mut self, id: &str) {
        self.entries.remove(id);
        if self.in_flight.as_ref().is_some_and(|ticket| ticket.profile.id == id) {
            self.in_flight = None;
        }
    }

    pub fn cancel_all(&mut self) {
        self.entries.clear();
        self.in_flight = None;
    }
}

fn enabled(profile: &ScheduleProfile) -> bool {
    profile.interval_minutes != Some(0) && validate_interval(profile.interval_minutes).is_ok()
}
fn interval(profile: &ScheduleProfile) -> u64 {
    profile
        .interval_minutes
        .unwrap_or(DEFAULT_INTERVAL_MINUTES)
        .saturating_mul(60)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> ScheduleProfile {
        ScheduleProfile {
            id: "sample".into(),
            url: "http://127.0.0.1/subscription".into(),
            updated_at: 100,
            interval_minutes: Some(1),
        }
    }
    #[test]
    fn bounded_retries_and_single_in_flight() {
        let mut scheduler = Scheduler::default();
        let profiles = [profile()];
        assert!(scheduler.claim_due(&profiles, 159).is_none());
        let first = scheduler.claim_due(&profiles, 160).unwrap();
        assert!(scheduler.claim_due(&profiles, 1000).is_none());
        scheduler.finish(&first, 160, false);
        assert!(scheduler.claim_due(&profiles, 189).is_none());
        let second = scheduler.claim_due(&profiles, 190).unwrap();
        scheduler.finish(&second, 190, false);
        let third = scheduler.claim_due(&profiles, 250).unwrap();
        scheduler.finish(&third, 250, false);
        assert!(scheduler.claim_due(&profiles, 309).is_none());
        assert!(scheduler.claim_due(&profiles, 310).is_some());
    }
    #[test]
    fn changed_or_deleted_profiles_and_cancelled_tickets_are_stale() {
        let mut scheduler = Scheduler::default();
        let original = profile();
        let ticket = scheduler.claim_due(std::slice::from_ref(&original), 160).unwrap();
        assert!(scheduler.is_current(&ticket, std::slice::from_ref(&original)));
        let mut changed = original.clone();
        changed.url = "http://127.0.0.1/changed".into();
        assert!(!scheduler.is_current(&ticket, &[changed]));
        assert!(!scheduler.is_current(&ticket, &[]));
        scheduler.cancel(&original.id);
        let replacement = scheduler.claim_due(std::slice::from_ref(&original), 160).unwrap();
        scheduler.finish(&ticket, 160, true);
        assert!(scheduler.is_current(&replacement, &[original]));
        scheduler.cancel_all();
        assert!(!scheduler.is_current(&replacement, &[profile()]));
    }
    #[test]
    fn interval_default_disable_and_validation() {
        assert!(validate_interval(Some(MAX_INTERVAL_MINUTES + 1)).is_err());
        let mut p = profile();
        p.interval_minutes = None;
        let mut scheduler = Scheduler::default();
        assert!(scheduler.claim_due(&[p.clone()], 100 + 86400 - 1).is_none());
        assert!(scheduler.claim_due(&[p.clone()], 100 + 86400).is_some());
        scheduler.cancel_all();
        p.interval_minutes = Some(0);
        assert!(scheduler.claim_due(&[p], u64::MAX).is_none());
    }
}
