//! #332 ③ — boot-time clock sanity.
//!
//! Every log line's usefulness depends on the system clock being roughly
//! right. When it isn't, forensics actively lie: minibsd's gateway logged an
//! 8-day-stale "last line" that read as a zombie hang, and a box that boots
//! with a pre-build-date clock stamps every event in the past. This module
//! cross-checks three independent time sources at boot and WARNs loudly
//! (target=boot, greppable) when they disagree:
//!
//! 1. **System clock vs build date** — the binary cannot have been built in
//!    the future; `now < build_epoch` proves the clock is behind (dead CMOS
//!    battery, unsynced VM, container without NTP).
//! 2. **System clock vs last log write** — the previous gateway.log mtime
//!    cannot be in the future either; `last_log > now` proves the clock
//!    moved backwards since the last run.
//!
//! Detection + WARN only. Nothing is auto-corrected — clock policy belongs
//! to the operator/NTP, not to us.

/// A single clock-sanity finding, rendered as one WARN line at boot.
#[derive(Debug, PartialEq, Eq)]
pub enum ClockAnomaly {
    /// System clock is EARLIER than the binary's build timestamp.
    /// `behind_secs` = build_epoch - now.
    BehindBuildDate { behind_secs: u64 },
    /// The existing gateway.log was last written IN THE FUTURE relative to
    /// the current clock. `ahead_secs` = last_log_epoch - now.
    LastLogInFuture { ahead_secs: u64 },
}

/// Pure verdict: compare the three time sources. `last_log_epoch` is `None`
/// on first boot (no prior gateway.log).
///
/// A small slack window absorbs benign causes: build-machine/host clock
/// differences of a few minutes (binary built on another seat, distributed
/// via fleet update) and mtime granularity. Anomalies smaller than the slack
/// are noise; larger ones are real skew.
pub fn clock_anomalies(
    now_epoch: u64,
    build_epoch: u64,
    last_log_epoch: Option<u64>,
) -> Vec<ClockAnomaly> {
    /// 10 minutes of slack: generous for cross-seat build clocks, far below
    /// any skew that would mislead forensics.
    const SLACK_SECS: u64 = 600;
    let mut findings = Vec::new();
    // build_epoch == 0 means the build script couldn't read the clock —
    // nothing to compare against.
    if build_epoch > 0 && now_epoch + SLACK_SECS < build_epoch {
        findings.push(ClockAnomaly::BehindBuildDate {
            behind_secs: build_epoch - now_epoch,
        });
    }
    if let Some(log_epoch) = last_log_epoch
        && log_epoch > now_epoch + SLACK_SECS
    {
        findings.push(ClockAnomaly::LastLogInFuture {
            ahead_secs: log_epoch - now_epoch,
        });
    }
    findings
}

/// Epoch seconds of `logs/gateway.log`'s last modification, if it exists.
pub fn last_log_epoch(zeus_home: &std::path::Path) -> Option<u64> {
    std::fs::metadata(zeus_home.join("logs").join("gateway.log"))
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Run the check and emit one WARN per anomaly under `target=boot`.
/// Called once from `run_gateway` right after the boot banner.
pub fn warn_on_clock_skew(zeus_home: &std::path::Path) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let build: u64 = env!("ZEUS_BUILD_EPOCH").parse().unwrap_or(0);
    for anomaly in clock_anomalies(now, build, last_log_epoch(zeus_home)) {
        match anomaly {
            ClockAnomaly::BehindBuildDate { behind_secs } => tracing::warn!(
                target: "boot",
                event = "clock_skew",
                kind = "behind_build_date",
                behind_secs,
                "system clock is EARLIER than this binary's build date — \
                 clock is wrong (unsynced VM / dead CMOS / no NTP); all log \
                 timestamps are unreliable until fixed"
            ),
            ClockAnomaly::LastLogInFuture { ahead_secs } => tracing::warn!(
                target: "boot",
                event = "clock_skew",
                kind = "last_log_in_future",
                ahead_secs,
                "previous gateway.log was written IN THE FUTURE relative to \
                 the current clock — the clock moved backwards since the \
                 last run; log-age forensics will mislead"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3600;

    #[test]
    fn healthy_clock_yields_no_findings() {
        // Now is after build, last log slightly in the past: all good.
        assert!(clock_anomalies(1_000_000, 900_000, Some(999_000)).is_empty());
        // First boot (no prior log).
        assert!(clock_anomalies(1_000_000, 900_000, None).is_empty());
    }

    #[test]
    fn clock_behind_build_date_detected() {
        let f = clock_anomalies(1_000_000, 1_000_000 + 8 * 24 * HOUR, None);
        assert_eq!(
            f,
            vec![ClockAnomaly::BehindBuildDate {
                behind_secs: 8 * 24 * HOUR
            }]
        );
    }

    #[test]
    fn last_log_in_future_detected() {
        let f = clock_anomalies(1_000_000, 900_000, Some(1_000_000 + 2 * HOUR));
        assert_eq!(
            f,
            vec![ClockAnomaly::LastLogInFuture {
                ahead_secs: 2 * HOUR
            }]
        );
    }

    #[test]
    fn slack_absorbs_benign_cross_seat_build_clocks() {
        // 5 minutes "in the future" build date: benign, no finding.
        assert!(clock_anomalies(1_000_000, 1_000_000 + 300, None).is_empty());
        // 5-minute future log mtime: benign.
        assert!(clock_anomalies(1_000_000, 900_000, Some(1_000_000 + 300)).is_empty());
    }

    #[test]
    fn zero_build_epoch_never_fires() {
        // Build script couldn't read the clock — comparison disabled.
        assert!(clock_anomalies(1_000, 0, None).is_empty());
    }

    #[test]
    fn both_anomalies_reported_together() {
        let f = clock_anomalies(1_000_000, 2_000_000, Some(3_000_000));
        assert_eq!(f.len(), 2);
    }
}
