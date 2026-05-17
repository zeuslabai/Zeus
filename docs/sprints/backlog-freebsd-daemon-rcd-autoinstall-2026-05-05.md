# Backlog — FreeBSD daemon rc.d auto-install in install.sh

**Filed:** 2026-05-05
**Owner suggested:** fbsd2 (operator persona, FreeBSD home turf) or zeus106
**Estimate:** 2-4h
**Priority:** P2 — bites every fresh FreeBSD titan node, but workaround exists.

---

## Problem

On a fresh FreeBSD titan node:

1. `scripts/install.sh --update` (line 723-727) tries `sudo service zeus_gateway restart`.
2. If the rc.d script `/usr/local/etc/rc.d/zeus_gateway` was never installed, `service ... restart` exits non-zero.
3. install.sh falls through to the `nohup zeus gateway` fallback. Gateway starts, but as a backgrounded user-process — NOT as an rc.d service.
4. Later, `zeus daemon restart` calls `stop_daemon()` → kills the nohup process (or warns if pkill is partial), then calls `start_daemon()` which is hardcoded to `service zeus_gateway start` (`src/daemon.rs:692`). Exits non-zero because rc.d still isn't installed.
5. User sees: `Warning: gateway process may still be running` + `Error: Failed to start daemon: ` (empty stderr).

The warning is from `stop_daemon()` verifying with `pgrep -f "zeus gateway"`. The error is from `service zeus_gateway start` failing because rc.d isn't installed.

## Witness incident — 2026-05-05

merakizzz, on a freshly-deployed FreeBSD jail (hostname `qtumz2`), ran `zeus daemon restart` and `sudo zeus daemon restart`. Both failed with the symptoms above. Workaround that resolved:

```bash
sudo zeus daemon install   # writes rc.d script + sets rc.conf zeus_gateway_enable="YES"
sudo pkill -9 -f "zeus gateway"   # kill the orphaned nohup process
sudo service zeus_gateway start
```

After that, `zeus daemon restart` works cleanly.

## Fix

In `scripts/install.sh:723-727`, the FreeBSD branch should be:

```bash
FreeBSD)
    # Ensure rc.d service is installed FIRST (prevents nohup orphan + later restart failures).
    if [ ! -f /usr/local/etc/rc.d/zeus_gateway ]; then
        sudo zeus daemon install 2>/dev/null || warn "zeus daemon install failed — service won't be registered"
    fi
    # Try service restart (now that rc.d should be in place).
    if sudo service zeus_gateway restart 2>/dev/null; then
        ok "Gateway restarted via rc.d service"
    else
        warn "service zeus_gateway restart failed — falling back to nohup"
        nohup zeus gateway > "$ZEUS_HOME/logs/gateway.out.log" 2> "$ZEUS_HOME/logs/gateway.err.log" &
        ok "Gateway started via nohup (rc.d service not active — run 'sudo zeus daemon install' to fix)"
    fi
    ;;
```

Same pattern as the launchd path on macOS (line 716-722) which calls `install_launchd_plist` first then falls back to nohup.

## Optional secondary fix

In `src/daemon.rs::start_daemon()` (FreeBSD branch, line 691-703), detect "service does not exist" stderr and emit a guidance line:

```rust
if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("does not exist") || stderr.contains("zeus_gateway is not enabled") {
        anyhow::bail!(
            "Failed to start daemon: rc.d service not registered. Run: sudo zeus daemon install"
        );
    }
    anyhow::bail!("Failed to start daemon: {}", stderr);
}
```

## Acceptance gate

- [ ] Fresh FreeBSD jail without `/usr/local/etc/rc.d/zeus_gateway`: `install.sh --update` registers rc.d + restarts via service (not nohup fallback).
- [ ] After install.sh exits, `zeus daemon restart` works without manual `sudo zeus daemon install`.
- [ ] `service zeus_gateway status` reports active.
- [ ] If rc.d install fails (perms, etc.), nohup fallback is taken with clear warning that points at the recovery command.
- [ ] `start_daemon()` error message mentions `sudo zeus daemon install` when service is missing.

## Out of scope

- Linux systemd path (already calls `systemctl --user start` cleanly).
- macOS launchd path (already calls `install_launchd_plist` first).
