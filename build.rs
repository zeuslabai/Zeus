use std::process::Command;

fn main() {
    // Try to get the short git SHA at build time.
    // Falls back to "unknown" if git is unavailable or we're not in a repo.
    let git_sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Re-run this build script when HEAD changes so the embedded SHA stays fresh.
    println!("cargo:rerun-if-changed=.git/HEAD");
    // Also watch the ref HEAD points to, so the SHA refreshes on a new commit
    // (not just on branch switches / clean builds).
    if let Ok(head) = std::fs::read_to_string(".git/HEAD") {
        if let Some(r) = head.strip_prefix("ref: ").map(str::trim) {
            println!("cargo:rerun-if-changed=.git/{r}");
        }
    }

    println!("cargo:rustc-env=GIT_SHA={}", git_sha);

    // #332 ③ boot fingerprint: embed the build timestamp (unix epoch secs)
    // so boot can sanity-check the system clock against it. A system clock
    // EARLIER than the build date is physically impossible and means the
    // clock is wrong (dead CMOS battery, unsynced VM, minibsd's 8-day skew
    // class) — timestamps in every log line downstream would be lies.
    let build_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=ZEUS_BUILD_EPOCH={}", build_epoch);
}
