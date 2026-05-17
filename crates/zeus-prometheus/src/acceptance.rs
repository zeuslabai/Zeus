//! Acceptance-check runner — Phase 3d of task-driven autonomy.
//!
//! Verifies whether a task's deliverables actually exist and build.
//! Small, composable primitives so a cooking session can answer
//! "is this task done yet?" before marking it complete.
//!
//! # Primitives
//!
//! - [`cargo_check`] — run `cargo check --workspace`, return pass/fail.
//! - [`branch_exists`] — verify a git branch exists.
//! - [`file_exists`] — verify a file was created/modified.
//! - [`file_matches_regex`] — verify file contents match a regex.
//! - [`tests_pass`] — run a specific cargo test filter.
//!
//! All checks return [`CheckResult`] with structured pass/fail + stderr snippet
//! for diagnostics. They never panic; missing repos / binaries map to fail.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Outcome of a single acceptance check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Human-readable name of the check (e.g. "cargo_check").
    pub check: String,
    /// True if the check passed.
    pub passed: bool,
    /// Short human-readable detail (e.g. "branch exists", "3 errors").
    pub detail: String,
    /// Truncated stderr/stdout for diagnostics (max 2 KB).
    pub output: String,
}

impl CheckResult {
    fn pass(check: &str, detail: impl Into<String>) -> Self {
        Self {
            check: check.to_string(),
            passed: true,
            detail: detail.into(),
            output: String::new(),
        }
    }

    fn fail(check: &str, detail: impl Into<String>, output: impl Into<String>) -> Self {
        let mut out = output.into();
        if out.len() > 2048 {
            out.truncate(2048);
            out.push_str("\n…[truncated]");
        }
        Self {
            check: check.to_string(),
            passed: false,
            detail: detail.into(),
            output: out,
        }
    }
}

/// An individual acceptance check, serializable for storage in `scope_json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcceptanceCheck {
    /// `cargo check --workspace` must pass in `repo`.
    CargoCheck { repo: PathBuf },
    /// Git branch `name` must exist in `repo`.
    BranchExists { repo: PathBuf, name: String },
    /// File `path` must exist.
    FileExists { path: PathBuf },
    /// File `path` must exist and contain a regex match.
    FileMatchesRegex { path: PathBuf, regex: String },
    /// `cargo test <filter>` must pass in `repo`.
    TestsPass {
        repo: PathBuf,
        filter: Option<String>,
    },
}

impl AcceptanceCheck {
    /// Execute this check. Never panics.
    pub fn run(&self) -> CheckResult {
        match self {
            AcceptanceCheck::CargoCheck { repo } => cargo_check(repo),
            AcceptanceCheck::BranchExists { repo, name } => branch_exists(repo, name),
            AcceptanceCheck::FileExists { path } => file_exists(path),
            AcceptanceCheck::FileMatchesRegex { path, regex } => {
                file_matches_regex(path, regex)
            }
            AcceptanceCheck::TestsPass { repo, filter } => {
                tests_pass(repo, filter.as_deref())
            }
        }
    }
}

/// Run `cargo check --workspace` in `repo`. Passes iff exit status is success.
pub fn cargo_check(repo: &Path) -> CheckResult {
    let output = Command::new("cargo")
        .arg("check")
        .arg("--workspace")
        .arg("--quiet")
        .current_dir(repo)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            CheckResult::pass("cargo_check", "workspace compiles")
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            CheckResult::fail(
                "cargo_check",
                format!("exit {}", out.status.code().unwrap_or(-1)),
                stderr,
            )
        }
        Err(e) => CheckResult::fail(
            "cargo_check",
            "failed to spawn cargo",
            e.to_string(),
        ),
    }
}

/// Check whether a git branch exists (local or remote) in `repo`.
pub fn branch_exists(repo: &Path, name: &str) -> CheckResult {
    // `git rev-parse --verify refs/heads/<name>` — exits 0 if the ref exists.
    let local = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{}", name))
        .current_dir(repo)
        .output();

    if let Ok(out) = &local {
        if out.status.success() {
            return CheckResult::pass("branch_exists", format!("local branch `{}`", name));
        }
    }

    // Fallback: check remote-tracking branches.
    let remote = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/remotes/origin/{}", name))
        .current_dir(repo)
        .output();

    match remote {
        Ok(out) if out.status.success() => CheckResult::pass(
            "branch_exists",
            format!("remote branch `origin/{}`", name),
        ),
        Ok(_) => CheckResult::fail(
            "branch_exists",
            format!("branch `{}` not found", name),
            String::new(),
        ),
        Err(e) => CheckResult::fail("branch_exists", "failed to spawn git", e.to_string()),
    }
}

/// Check whether a file (or directory) exists at `path`.
pub fn file_exists(path: &Path) -> CheckResult {
    if path.exists() {
        CheckResult::pass("file_exists", format!("{} exists", path.display()))
    } else {
        CheckResult::fail(
            "file_exists",
            format!("{} missing", path.display()),
            String::new(),
        )
    }
}

/// Check whether `path` exists and its contents match `regex`.
pub fn file_matches_regex(path: &Path, regex: &str) -> CheckResult {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return CheckResult::fail(
                "file_matches_regex",
                format!("cannot read {}", path.display()),
                e.to_string(),
            );
        }
    };

    let re = match regex::Regex::new(regex) {
        Ok(r) => r,
        Err(e) => {
            return CheckResult::fail(
                "file_matches_regex",
                format!("invalid regex `{}`", regex),
                e.to_string(),
            );
        }
    };

    if re.is_match(&contents) {
        CheckResult::pass(
            "file_matches_regex",
            format!("{} matches /{}/", path.display(), regex),
        )
    } else {
        CheckResult::fail(
            "file_matches_regex",
            format!("{} does not match /{}/", path.display(), regex),
            String::new(),
        )
    }
}

/// Run `cargo test` (optionally with a filter) in `repo`. Passes iff exit status is success.
pub fn tests_pass(repo: &Path, filter: Option<&str>) -> CheckResult {
    let mut cmd = Command::new("cargo");
    cmd.arg("test").arg("--workspace").arg("--quiet");
    if let Some(f) = filter {
        cmd.arg("--").arg(f);
    }
    cmd.current_dir(repo);

    match cmd.output() {
        Ok(out) if out.status.success() => CheckResult::pass(
            "tests_pass",
            filter.map(|f| format!("filter `{}`", f)).unwrap_or_else(|| "all tests".into()),
        ),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            CheckResult::fail(
                "tests_pass",
                format!("exit {}", out.status.code().unwrap_or(-1)),
                stderr,
            )
        }
        Err(e) => CheckResult::fail("tests_pass", "failed to spawn cargo", e.to_string()),
    }
}

/// Run a batch of checks and return a summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceReport {
    pub results: Vec<CheckResult>,
    pub all_passed: bool,
}

/// Run every check in `checks`, returning a full report.
pub fn run_all(checks: &[AcceptanceCheck]) -> AcceptanceReport {
    let results: Vec<CheckResult> = checks.iter().map(|c| c.run()).collect();
    let all_passed = results.iter().all(|r| r.passed);
    AcceptanceReport {
        results,
        all_passed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn file_exists_pass() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let r = file_exists(tmp.path());
        assert!(r.passed, "detail: {}", r.detail);
    }

    #[test]
    fn file_exists_fail() {
        let r = file_exists(Path::new("/nonexistent/zeus/acceptance/path"));
        assert!(!r.passed);
    }

    #[test]
    fn file_matches_regex_pass() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "hello zeus autonomy").unwrap();
        let r = file_matches_regex(tmp.path(), r"zeus\s+autonomy");
        assert!(r.passed, "detail: {}", r.detail);
    }

    #[test]
    fn file_matches_regex_no_match() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "nothing to see").unwrap();
        let r = file_matches_regex(tmp.path(), r"zeus\s+autonomy");
        assert!(!r.passed);
    }

    #[test]
    fn file_matches_regex_invalid_regex() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let r = file_matches_regex(tmp.path(), "[unclosed");
        assert!(!r.passed);
        assert_eq!(r.check, "file_matches_regex");
    }

    #[test]
    fn branch_exists_missing_repo() {
        let r = branch_exists(Path::new("/nonexistent/zeus/repo"), "main");
        assert!(!r.passed);
    }

    #[test]
    fn run_all_mixed() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let checks = vec![
            AcceptanceCheck::FileExists {
                path: tmp.path().to_path_buf(),
            },
            AcceptanceCheck::FileExists {
                path: PathBuf::from("/nonexistent/zeus/acceptance/path"),
            },
        ];
        let report = run_all(&checks);
        assert_eq!(report.results.len(), 2);
        assert!(!report.all_passed);
        assert!(report.results[0].passed);
        assert!(!report.results[1].passed);
    }

    #[test]
    fn serde_roundtrip_check() {
        let check = AcceptanceCheck::FileMatchesRegex {
            path: PathBuf::from("/tmp/foo"),
            regex: "bar".into(),
        };
        let json = serde_json::to_string(&check).unwrap();
        let back: AcceptanceCheck = serde_json::from_str(&json).unwrap();
        match back {
            AcceptanceCheck::FileMatchesRegex { path, regex } => {
                assert_eq!(path, PathBuf::from("/tmp/foo"));
                assert_eq!(regex, "bar");
            }
            _ => panic!("wrong variant"),
        }
    }
}
