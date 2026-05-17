//! Session reset policies
//!
//! Controls when sessions should be automatically reset based on time, idle duration, or both.

use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};

/// Session reset policy
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[derive(Default)]
pub enum ResetPolicy {
    /// Only reset when explicitly requested
    #[default]
    Manual,
    /// Reset daily at a specific hour (0-23)
    Daily { hour: u8 },
    /// Reset after idle for N minutes
    Idle { timeout_minutes: u32 },
    /// Reset daily at hour OR after idle, whichever comes first
    Combined { hour: u8, idle_minutes: u32 },
}

/// Manages session reset timing
pub struct SessionResetManager {
    policy: ResetPolicy,
    last_activity: DateTime<Utc>,
    session_start: DateTime<Utc>,
}

impl SessionResetManager {
    /// Create a new manager with the given policy
    pub fn new(policy: ResetPolicy) -> Self {
        let now = Utc::now();
        Self {
            policy,
            last_activity: now,
            session_start: now,
        }
    }

    /// Create with a specific start time (for testing)
    pub fn with_start_time(policy: ResetPolicy, start: DateTime<Utc>) -> Self {
        Self {
            policy,
            last_activity: start,
            session_start: start,
        }
    }

    /// Check if the session should be reset
    pub fn should_reset(&self) -> bool {
        let now = Utc::now();
        self.should_reset_at(now)
    }

    /// Check if the session should be reset at a given time (for testing)
    pub fn should_reset_at(&self, now: DateTime<Utc>) -> bool {
        match &self.policy {
            ResetPolicy::Manual => false,
            ResetPolicy::Daily { hour } => self.is_past_daily_reset(*hour, now),
            ResetPolicy::Idle { timeout_minutes } => self.is_idle(*timeout_minutes, now),
            ResetPolicy::Combined { hour, idle_minutes } => {
                self.is_past_daily_reset(*hour, now) || self.is_idle(*idle_minutes, now)
            }
        }
    }

    /// Update last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Utc::now();
    }

    /// Update last activity to a specific time (for testing)
    pub fn touch_at(&mut self, time: DateTime<Utc>) {
        self.last_activity = time;
    }

    /// Get the current policy
    pub fn policy(&self) -> &ResetPolicy {
        &self.policy
    }

    /// Get last activity time
    pub fn last_activity(&self) -> DateTime<Utc> {
        self.last_activity
    }

    /// Get session start time
    pub fn session_start(&self) -> DateTime<Utc> {
        self.session_start
    }

    /// Reset the session start time (called after a reset occurs)
    pub fn mark_reset(&mut self) {
        let now = Utc::now();
        self.session_start = now;
        self.last_activity = now;
    }

    /// Check if the daily reset hour has passed since session start
    fn is_past_daily_reset(&self, hour: u8, now: DateTime<Utc>) -> bool {
        let hour = hour.min(23);
        // Get today's reset time
        let today = now.date_naive();
        let reset_time =
            today.and_time(NaiveTime::from_hms_opt(hour as u32, 0, 0).unwrap_or_default());
        let reset_utc = reset_time.and_utc();

        // Session should reset if:
        // 1. Current time is past today's reset hour
        // 2. Session started before the reset hour
        now >= reset_utc && self.session_start < reset_utc
    }

    /// Check if idle timeout has been exceeded
    fn is_idle(&self, timeout_minutes: u32, now: DateTime<Utc>) -> bool {
        let idle_duration = now.signed_duration_since(self.last_activity);
        idle_duration.num_minutes() >= timeout_minutes as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Helper: create a UTC datetime from components
    fn utc(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, sec)
            .unwrap()
    }

    #[test]
    fn test_manual_policy_never_resets() {
        let start = utc(2026, 1, 1, 0, 0, 0);
        let manager = SessionResetManager::with_start_time(ResetPolicy::Manual, start);

        // Even far into the future, manual policy should never trigger
        let far_future = utc(2030, 12, 31, 23, 59, 59);
        assert!(!manager.should_reset_at(far_future));

        // Right after start
        let just_after = utc(2026, 1, 1, 0, 0, 1);
        assert!(!manager.should_reset_at(just_after));
    }

    #[test]
    fn test_daily_policy_resets_after_hour_passes() {
        // Session starts at 08:00, reset hour is 9
        let start = utc(2026, 2, 5, 8, 0, 0);
        let manager = SessionResetManager::with_start_time(ResetPolicy::Daily { hour: 9 }, start);

        // At 09:00 the same day, the reset hour has passed and session started before it
        let after_reset = utc(2026, 2, 5, 9, 0, 0);
        assert!(manager.should_reset_at(after_reset));

        // At 10:00 the same day, still past the reset hour
        let well_after = utc(2026, 2, 5, 10, 0, 0);
        assert!(manager.should_reset_at(well_after));
    }

    #[test]
    fn test_daily_policy_does_not_reset_before_hour() {
        // Session starts at 08:00, reset hour is 9
        let start = utc(2026, 2, 5, 8, 0, 0);
        let manager = SessionResetManager::with_start_time(ResetPolicy::Daily { hour: 9 }, start);

        // At 08:30, before the reset hour
        let before_reset = utc(2026, 2, 5, 8, 30, 0);
        assert!(!manager.should_reset_at(before_reset));

        // At 08:59:59, just before the reset hour
        let just_before = utc(2026, 2, 5, 8, 59, 59);
        assert!(!manager.should_reset_at(just_before));
    }

    #[test]
    fn test_daily_policy_does_not_reset_if_session_started_after_hour() {
        // Session starts at 10:00, reset hour is 9
        // Since the session started after the reset hour today, it should NOT reset today
        let start = utc(2026, 2, 5, 10, 0, 0);
        let manager = SessionResetManager::with_start_time(ResetPolicy::Daily { hour: 9 }, start);

        // Even later the same day
        let later = utc(2026, 2, 5, 23, 0, 0);
        assert!(!manager.should_reset_at(later));
    }

    #[test]
    fn test_daily_policy_resets_next_day() {
        // Session starts at 10:00 on Feb 5, reset hour is 9
        // The next day at 09:00, the session should reset because session_start < next day 09:00
        let start = utc(2026, 2, 5, 10, 0, 0);
        let manager = SessionResetManager::with_start_time(ResetPolicy::Daily { hour: 9 }, start);

        let next_day = utc(2026, 2, 6, 9, 0, 0);
        assert!(manager.should_reset_at(next_day));
    }

    #[test]
    fn test_idle_policy_resets_after_timeout() {
        let start = utc(2026, 2, 5, 10, 0, 0);
        let manager = SessionResetManager::with_start_time(
            ResetPolicy::Idle {
                timeout_minutes: 30,
            },
            start,
        );

        // Exactly 30 minutes later
        let at_timeout = utc(2026, 2, 5, 10, 30, 0);
        assert!(manager.should_reset_at(at_timeout));

        // Well past the timeout
        let past_timeout = utc(2026, 2, 5, 11, 0, 0);
        assert!(manager.should_reset_at(past_timeout));
    }

    #[test]
    fn test_idle_policy_does_not_reset_before_timeout() {
        let start = utc(2026, 2, 5, 10, 0, 0);
        let manager = SessionResetManager::with_start_time(
            ResetPolicy::Idle {
                timeout_minutes: 30,
            },
            start,
        );

        // 15 minutes later (half the timeout)
        let before_timeout = utc(2026, 2, 5, 10, 15, 0);
        assert!(!manager.should_reset_at(before_timeout));

        // 29 minutes later (just under)
        let just_before = utc(2026, 2, 5, 10, 29, 0);
        assert!(!manager.should_reset_at(just_before));
    }

    #[test]
    fn test_combined_policy_resets_on_daily() {
        // Combined: reset at hour 9 OR after 60 minutes idle
        let start = utc(2026, 2, 5, 8, 0, 0);
        let manager = SessionResetManager::with_start_time(
            ResetPolicy::Combined {
                hour: 9,
                idle_minutes: 60,
            },
            start,
        );

        // At 09:00 (only 60 minutes since start, so idle hasn't triggered, but daily has)
        let at_daily = utc(2026, 2, 5, 9, 0, 0);
        assert!(manager.should_reset_at(at_daily));
    }

    #[test]
    fn test_combined_policy_resets_on_idle() {
        // Combined: reset at hour 9 OR after 30 minutes idle
        // Session starts at 10:00 (after the daily hour), so daily won't trigger today
        let start = utc(2026, 2, 5, 10, 0, 0);
        let manager = SessionResetManager::with_start_time(
            ResetPolicy::Combined {
                hour: 9,
                idle_minutes: 30,
            },
            start,
        );

        // 30 minutes idle
        let at_idle = utc(2026, 2, 5, 10, 30, 0);
        assert!(manager.should_reset_at(at_idle));
    }

    #[test]
    fn test_combined_policy_does_not_reset_when_neither_condition_met() {
        let start = utc(2026, 2, 5, 8, 0, 0);
        let manager = SessionResetManager::with_start_time(
            ResetPolicy::Combined {
                hour: 9,
                idle_minutes: 60,
            },
            start,
        );

        // 30 minutes later: not past daily (9), not idle for 60 min
        let neither = utc(2026, 2, 5, 8, 30, 0);
        assert!(!manager.should_reset_at(neither));
    }

    #[test]
    fn test_touch_updates_last_activity() {
        let start = utc(2026, 2, 5, 10, 0, 0);
        let mut manager = SessionResetManager::with_start_time(
            ResetPolicy::Idle {
                timeout_minutes: 30,
            },
            start,
        );

        // Without touch, 30 min idle triggers reset
        let at_timeout = utc(2026, 2, 5, 10, 30, 0);
        assert!(manager.should_reset_at(at_timeout));

        // Touch at 20 minutes, resetting the idle timer
        let touch_time = utc(2026, 2, 5, 10, 20, 0);
        manager.touch_at(touch_time);

        // Now at 30 min from original start, only 10 min since touch -> no reset
        assert!(!manager.should_reset_at(at_timeout));

        // 50 minutes from start (30 min since touch) -> should reset
        let after_touch_timeout = utc(2026, 2, 5, 10, 50, 0);
        assert!(manager.should_reset_at(after_touch_timeout));
    }

    #[test]
    fn test_mark_reset_resets_times() {
        let start = utc(2026, 2, 5, 8, 0, 0);
        let mut manager =
            SessionResetManager::with_start_time(ResetPolicy::Daily { hour: 9 }, start);

        // Should want to reset at 09:00
        let at_nine = utc(2026, 2, 5, 9, 0, 0);
        assert!(manager.should_reset_at(at_nine));

        // Mark reset (this sets session_start and last_activity to now)
        manager.mark_reset();

        // After mark_reset, session_start is now (close to current time)
        // Since session_start is now >= the 9am reset time, it should NOT want to reset
        // for the rest of today
        let later = utc(2026, 2, 5, 23, 0, 0);
        // The session_start is "now" (real clock), but for the daily check:
        // today's 09:00 has passed, and session_start (real now ~2026) is past it too.
        // So is_past_daily_reset returns false because session_start >= reset_utc.
        // This is correct behavior.
        assert!(!manager.should_reset_at(later));
    }

    #[test]
    fn test_default_policy_is_manual() {
        let policy = ResetPolicy::default();
        assert!(matches!(policy, ResetPolicy::Manual));
    }

    #[test]
    fn test_daily_hour_clamped_to_23() {
        // Hour value above 23 should be clamped
        let start = utc(2026, 2, 5, 22, 0, 0);
        let manager = SessionResetManager::with_start_time(ResetPolicy::Daily { hour: 99 }, start);

        // Hour 99 is clamped to 23, so reset should trigger at 23:00
        let at_23 = utc(2026, 2, 5, 23, 0, 0);
        assert!(manager.should_reset_at(at_23));
    }

    #[test]
    fn test_policy_serialization_roundtrip() {
        let policies = vec![
            ResetPolicy::Manual,
            ResetPolicy::Daily { hour: 9 },
            ResetPolicy::Idle {
                timeout_minutes: 30,
            },
            ResetPolicy::Combined {
                hour: 6,
                idle_minutes: 45,
            },
        ];

        for policy in policies {
            let json = serde_json::to_string(&policy).expect("should serialize to JSON");
            let deserialized: ResetPolicy =
                serde_json::from_str(&json).expect("should parse successfully");

            // Verify the roundtrip preserves the variant
            let original_json = serde_json::to_value(&policy).expect("should serialize to JSON");
            let roundtrip_json =
                serde_json::to_value(&deserialized).expect("should serialize to JSON");
            assert_eq!(original_json, roundtrip_json);
        }
    }

    #[test]
    fn test_accessors() {
        let start = utc(2026, 2, 5, 10, 0, 0);
        let manager = SessionResetManager::with_start_time(
            ResetPolicy::Idle {
                timeout_minutes: 30,
            },
            start,
        );

        assert_eq!(manager.session_start(), start);
        assert_eq!(manager.last_activity(), start);
        assert!(matches!(
            manager.policy(),
            ResetPolicy::Idle {
                timeout_minutes: 30
            }
        ));
    }
}
