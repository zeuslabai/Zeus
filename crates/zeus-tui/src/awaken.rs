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
    // Port-check guard: never double-launch if a gateway is already serving.
    if std::net::TcpStream::connect(("127.0.0.1", 8080)).is_ok() {
        tracing::info!("AWAKEN: gateway already serving on :8080 — skipping launch");
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("AWAKEN: cannot resolve current exe to launch gateway: {e}");
            return;
        }
    };

    let log_dir = dirs::home_dir().unwrap_or_default().join(".zeus").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("gateway.out.log"));
    let err = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("gateway.err.log"));

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

    match cmd.spawn() {
        Ok(child) => tracing::info!(
            "AWAKEN: gateway launched detached (pid {}) — titan going live",
            child.id()
        ),
        Err(e) => eprintln!("AWAKEN: failed to spawn gateway: {e}"),
    }
}
