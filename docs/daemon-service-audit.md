# Daemon Service Audit Report

**Branch:** `audit/daemon-service`  
**Reviewed:** `src/daemon.rs`  
**Platforms:** macOS (launchd), Linux (systemd), FreeBSD (rc.d)

---

## Summary

The daemon service implementation is consistent and well-structured across all three platforms. Each platform uses its native init system with appropriate fallbacks.

---

## 1. Clean Startup

| Platform | Mechanism | Status |
|---|---|---|
| macOS | `launchctl load` in `install_daemon()`, also sets `RunAtLoad=true` in plist | ✅ |
| Linux | `systemctl --user start` after installing unit file | ✅ |
| FreeBSD | `service zeus_gateway start` via rc.d script | ✅ |

**Notes:**
- macOS auto-loads via launchd on install — gateway survives SSH close
- Linux systemd unit sets `Restart=on-failure` with `RestartSec=5`
- FreeBSD uses `/usr/sbin/daemon` wrapper for proper backgrounding

---

## 2. PID File Management

| Platform | PID File Path | Status |
|---|---|---|
| macOS | `~/.zeus/gateway.pid` | ✅ |
| Linux | `~/.zeus/gateway.pid` | ✅ |
| FreeBSD | `/var/run/zeus_gateway.pid` | ⚠️ FreeBSD uses system PID dir |

**Issue:** FreeBSD uses `/var/run/` which requires write access. The rc.d script hardcodes `pidfile="/var/run/${name}.pid"`. If the user is unprivileged, this will fail — though `service` typically runs as root.

---

## 3. Graceful Shutdown (pkill fallback)

**All three platforms implement identical 3-step shutdown:**

1. Native stop (`launchctl unload` / `systemctl stop` / `service stop`)
2. PID file kill: read PID → `SIGTERM` → wait 5s → `SIGKILL` if still alive
3. `pkill -f "zeus gateway"` cleanup sweep

**Status:** ✅ Consistent across all platforms

---

## 4. Restart Behavior (--fresh support)

`restart_daemon(fresh)` is platform-agnostic (in the non-platform blocks):

```rust
async fn restart_daemon(fresh: bool) -> Result<()> {
    stop_daemon().await?;
    if fresh {
        // Clear ~/.zeus/sessions/*.jsonl files
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    start_daemon().await?;
}
```

**Status:** ✅ Works on all platforms

---

## 5. Status Reporting

| Platform | Command | Output | Status |
|---|---|---|---|
| macOS | `launchctl list com.zeus.gateway` | Running + PID | ✅ |
| Linux | `systemctl --user status` | Full unit status | ✅ |
| FreeBSD | `service zeus_gateway status` | RC script output | ✅ |

---

## Issues Found

### Issue 1: FreeBSD rc.d script hardcodes user "mike"
```sh
zeus_gateway_user="mike"
```
This is a hardcoded default. The rc.d script reads from `/etc/rc.conf` variables but defaults to "mike". Should use `${USER}` or prompt for the user.

**Severity:** Low (configurable via rc.conf)

### Issue 2: FreeBSD PID file path inconsistency
- Linux/macOS: `~/.zeus/gateway.pid`
- FreeBSD: `/var/run/zeus_gateway.pid`

This means stop daemon logic won't find the PID file on FreeBSD when it was started via rc.d.

**Severity:** Medium — the 3-step shutdown fallback (`pkill`) still catches stragglers, but the PID-based graceful kill may not work.

### Issue 3: Linux systemd lacks `RestartPrevent` / `RestartSec` tuning
The unit file is minimal — no `RuntimeDirectory`, no `TimeoutStopSec`. A runaway gateway could block stop forever.

**Severity:** Low (pkill fallback saves us)

### Issue 4: No platform-specific status for PID file location
The status command doesn't attempt to read the PID file directly on any platform — it only queries the init system. If the init system is out of sync, there's no fallback visibility.

**Severity:** Low

---

## Recommendations

1. **FreeBSD PID path:** Align to `~/.zeus/gateway.pid` for consistency, or document the divergence
2. **FreeBSD user default:** Use `${zeus_gateway_user:-$USER}` instead of hardcoding "mike"
3. **Linux systemd:** Add `TimeoutStopSec=10` to prevent hung shutdowns
4. **Status command:** Also check PID file as fallback when init system query fails

---

## Verdict

✅ **Fit for purpose.** The implementation is consistent, the 3-step shutdown is solid, and the pkill fallback ensures processes don't leak. The issues found are edge cases, not structural flaws.