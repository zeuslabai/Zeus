use std::process::Command;

fn main() {
    // Git SHA (short)
    let git_sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "dev".to_string());

    // Build time (UTC)
    let build_time = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();

    println!("cargo:rustc-env=GIT_SHA={}", git_sha);
    println!("cargo:rustc-env=BUILD_TIME={}", build_time);
}
