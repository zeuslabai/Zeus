//! AWAKEN-B: post-onboarding gateway launch from inside the TUI.
//!
//! When onboarding completes, the dwell-flip in [`crate::app::App::tick`]
//! transitions `launching → onboarding_complete` and lands the user directly
//! in the live in-process production UI. For that UI to have a backend to poll,
//! the gateway must be running — so we spawn `zeus gateway` DETACHED here, at
//! the flip, before the prod UI renders.
//!
//! This is the same machinery the root `zeus` binary uses post-`run()`
//! (`src/main.rs spawn_gateway_detached`), lifted verbatim into the lib so the
//! TUI can fire it directly on the tick-driven handoff path (which never
//! returns from `run()`). The `:8080` guard makes it idempotent — if the root
//! bin's residual call also fires, the second is a no-op.

/// Launch `zeus gateway` as a detached child that survives this process exiting.
///
/// AWAKEN-B (approach B): post-onboarding the TUI relaunches the `zeus` binary
/// with the `gateway` subcommand. No privilege escalation (unlike the launchd
/// daemon path, which is a CLI no-op + `sudo` bail on macOS), so it works from
/// the plain terminal where `zeus onboard` ran. stdout/stderr → log files;
/// `setsid` (new session/process-group) detaches it from the dying parent.
pub fn spawn_gateway_detached() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("AWAKEN: cannot resolve current exe to launch gateway: {e}");
            return;
        }
    };

    // #310: classic onboarding installed the native boot service (rc.d /
    // systemd unit); the AWAKEN path must too, or the gateway dies with the
    // box. Runs BEFORE the port guard on purpose — re-onboarding against an
    // already-running gateway still needs boot persistence installed.
    install_native_service(&exe);

    // Port-check guard: never double-launch if a gateway is already serving.
    if std::net::TcpStream::connect(("127.0.0.1", 8080)).is_ok() {
        tracing::info!("AWAKEN: gateway already serving on :8080 — skipping launch");
        return;
    }

    let log_dir = dirs::home_dir().unwrap_or_default().join(".zeus").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    // #321: raw stdout/stderr go to the same two stable files the gateway's
    // tracing sinks use — gateway.log for stdout, error.log for panics/stderr.
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("gateway.log"));
    let err = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("error.log"));

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("gateway");
    if let Ok(f) = out {
        cmd.stdout(std::process::Stdio::from(f));
    }
    if let Ok(f) = err {
        cmd.stderr(std::process::Stdio::from(f));
    }
    cmd.stdin(std::process::Stdio::null());

    // Detach: new session so the child is not killed when this parent exits
    // and is not in the terminal's foreground process group.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            // SAFETY: async-signal-safe libc call in the forked child.
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // Windows equivalent (#308): CREATE_NEW_PROCESS_GROUP detaches the child
    // from this console's Ctrl+C group (the setsid analog for signal
    // delivery) and CREATE_NO_WINDOW keeps a console window from flashing
    // up — stdio is already redirected to the log files / null above.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    match cmd.spawn() {
        Ok(child) => tracing::info!(
            "AWAKEN: gateway launched detached (pid {}) — titan going live",
            child.id()
        ),
        Err(e) => eprintln!("AWAKEN: failed to spawn gateway: {e}"),
    }
}

/// #310: install the native boot service by re-invoking the `zeus` binary's
/// own `daemon install` subcommand (`src/daemon.rs`) as a child process.
///
/// Why a subprocess and not a direct call: the install logic lives in the
/// root bin crate (platform-cfg'd rc.d/systemd/launchd handling, idempotent
/// stale-script rewrite, sysrc enable) and is not linkable from this lib —
/// and it's already the tested path `zeus daemon install` users hit directly.
///
/// Per-platform behavior (all non-interactive):
/// - **FreeBSD**: writes `/usr/local/etc/rc.d/zeus_gateway`, chmod 755,
///   `sysrc zeus_gateway_enable=YES`. Needs root for rc.d — on
///   `PermissionDenied` the subcommand bails with the exact
///   `sudo zeus daemon install` remediation, which we surface in the log.
/// - **Linux**: writes the `zeus-gateway` systemd user unit +
///   `systemctl --user daemon-reload`. No privilege needed.
/// - **macOS**: `daemon install` is a guidance no-op (Dispatch 3 moved the
///   system LaunchDaemon to `scripts/install.sh`, sudo-gated) — harmless.
///
/// Failures are logged, never fatal: the detached AWAKEN-B spawn still gives
/// this session a live gateway; the service only adds boot persistence.
fn install_native_service(exe: &std::path::Path) {
    match std::process::Command::new(exe).args(["daemon", "install"]).output() {
        Ok(out) if out.status.success() => {
            tracing::info!(
                "AWAKEN: native boot service installed (zeus daemon install): {}",
                String::from_utf8_lossy(&out.stdout).trim().replace('\n', " | ")
            );
        }
        Ok(out) => {
            // Likely a privilege failure (rc.d needs root on FreeBSD). The
            // TUI terminal is in raw mode so an interactive sudo prompt is
            // impossible — but `sudo -n` (never-prompt) succeeds on NOPASSWD
            // boxes and fails instantly everywhere else.
            let retried = std::process::Command::new("sudo")
                .args(["-n", &exe.to_string_lossy(), "daemon", "install"])
                .output();
            if let Ok(r) = retried
                && r.status.success()
            {
                tracing::info!(
                    "AWAKEN: native boot service installed via sudo -n: {}",
                    String::from_utf8_lossy(&r.stdout).trim().replace('\n', " | ")
                );
                return;
            }
            tracing::warn!(
                "AWAKEN: native service install failed ({}): {} {} — run `sudo zeus daemon install` to enable boot persistence",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim(),
                String::from_utf8_lossy(&out.stdout).trim()
            );
        }
        Err(e) => tracing::warn!("AWAKEN: could not run `zeus daemon install`: {e}"),
    }
}
