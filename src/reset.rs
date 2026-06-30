//! `zeus reset` — fleet-state reset orchestrator (#55 B-shape).
//!
//! Wipes a curated set of fleet-state surfaces under `~/.zeus/` while
//! preserving an **explicit allow-list** of credentials/identity/install-class
//! artifacts. Designed to recover from fabrication-contamination or scaffolding
//! bleed-through without bricking the agent (credentials + SOUL/AGENTS/USER
//! survive any invocation).
//!
//! ## Wipe set (10 surfaces, when `--all`):
//!  1. `~/.zeus/memory.db`             (via `mnemosyne-cleanup --apply --vacuum` subprocess)
//!  2. `~/.zeus/scheduler.db`
//!  3. `~/.zeus/cooking_checkpoints.db`
//!  4. `~/.zeus/goals.db`
//!  5. `~/.zeus/plan_outcomes.db`
//!  6. `~/.zeus/learning.db`
//!  7. `~/.zeus/sessions/*.jsonl`
//!  8. `~/.zeus/workspace/memory/`     (scratch fabrication-layer)
//!  9. `~/.zeus/workspace/daily/`      (scratch daily-notes)
//! 10. `~/.zeus/.skills_tmp/`          (tmp-class)
//!
//! ## Preserve allow-list (15 surfaces, INVARIANT):
//!   - `config.toml`, `config.toml.bak`, `skill_permissions.json`, `zeus.log`
//!   - `standing_orders.db`
//!   - `wallet/`, `skills/`, `agents/`, `completions/`, `.mcp_servers/`,
//!     `.community_skills/`, `economy/`, `logs/`
//!   - `workspace/{SOUL,AGENTS,USER,IDENTITY,HEARTBEAT,CAPABILITIES}.md`
//!
//! ## Selective scopes:
//!   - `--all`              (full 10-surface wipe; default if no scope flag)
//!   - `--memory-only`      (1 only)
//!   - `--scheduler-only`   (2 only)
//!   - `--sessions-only`    (7 only)
//!   - `--dry-run`          (print plan, no IO)
//!   - `--yes` / `--hard`   (skip interactive double-confirm)
//!
//! ## Safety invariants:
//!   - Double-confirm on destructive paths unless `--yes` AND `--hard`.
//!   - Preserve allow-list checked AFTER plan compute, BEFORE any IO.
//!   - SQLite WAL/SHM siblings removed alongside their `.db`.
//!   - mnemosyne-cleanup invoked as **subprocess** (clean boundary, no
//!     shared mutable state with the binary's internals).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

// ---------------------------------------------------------------------------
// Scope flags (mirror Commands::Reset args; kept as struct for testability).
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct ResetArgs {
    pub all: bool,
    pub memory_only: bool,
    pub scheduler_only: bool,
    pub sessions_only: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub hard: bool,
}

// ---------------------------------------------------------------------------
// Wipe-set + preserve-allow-list constants (load-bearing, see regression test).
// ---------------------------------------------------------------------------

/// Relative paths under `~/.zeus/` that `--all` wipes. Filesystem dirs end `/`.
const WIPE_SET: &[&str] = &[
    "memory.db",                 // (1) handled via subprocess
    "scheduler.db",              // (2)
    "cooking_checkpoints.db",    // (3)
    "goals.db",                  // (4)
    "plan_outcomes.db",          // (5)
    "learning.db",               // (6)
    "sessions/",                 // (7) *.jsonl removed inside
    "workspace/memory/",         // (8)
    "workspace/daily/",          // (9)
    ".skills_tmp/",              // (10)
];

/// Top-level files under `~/.zeus/` that MUST survive any reset.
const PRESERVE_FILES: &[&str] = &[
    "config.toml",
    "config.toml.bak",
    "skill_permissions.json",
    "zeus.log",
    "standing_orders.db",
];

/// Top-level dirs under `~/.zeus/` that MUST survive any reset.
const PRESERVE_DIRS: &[&str] = &[
    "wallet/",
    "skills/",
    "agents/",
    "completions/",
    ".mcp_servers/",
    ".community_skills/",
    "economy/",
    "logs/",
];

/// Identity-class files under `~/.zeus/workspace/` that MUST survive.
const PRESERVE_WORKSPACE_FILES: &[&str] = &[
    "SOUL.md",
    "AGENTS.md",
    "USER.md",
    "IDENTITY.md",
    "HEARTBEAT.md",
    "CAPABILITIES.md",
];

// ---------------------------------------------------------------------------
// Plan computation (pure, no IO — testable).
// ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub paths: Vec<String>,
    pub run_mnemosyne_cleanup: bool,
}

/// Compute the wipe-plan from `ResetArgs`. Pure — no IO. Selectivity:
/// `--memory-only` and `--scheduler-only` and `--sessions-only` are mutually
/// exclusive shortcuts; if multiple set, `--all` semantics win.
pub fn compute_plan(args: &ResetArgs) -> Plan {
    let selective_count = [args.memory_only, args.scheduler_only, args.sessions_only]
        .iter()
        .filter(|x| **x)
        .count();
    let use_all = args.all || selective_count == 0 || selective_count > 1;

    if use_all {
        let paths: Vec<String> = WIPE_SET
            .iter()
            .filter(|p| **p != "memory.db") // handled by subprocess
            .map(|s| s.to_string())
            .collect();
        return Plan {
            paths,
            run_mnemosyne_cleanup: true,
        };
    }

    if args.memory_only {
        return Plan {
            paths: vec![],
            run_mnemosyne_cleanup: true,
        };
    }
    if args.scheduler_only {
        return Plan {
            paths: vec!["scheduler.db".into()],
            run_mnemosyne_cleanup: false,
        };
    }
    if args.sessions_only {
        return Plan {
            paths: vec!["sessions/".into()],
            run_mnemosyne_cleanup: false,
        };
    }

    Plan::default()
}

// ---------------------------------------------------------------------------
// Path-safety guards (preserve-allow-list enforcement).
// ---------------------------------------------------------------------------

/// Returns `Err` if the relative path would touch a preserve-class artifact.
/// Defense-in-depth: even if WIPE_SET drifts, this guards the invariant.
pub fn assert_preserve_safe(rel: &str) -> Result<()> {
    let norm = rel.trim_start_matches("./");

    for p in PRESERVE_FILES {
        if norm == *p {
            return Err(anyhow!("refusing to wipe preserve-file: {}", p));
        }
    }
    for d in PRESERVE_DIRS {
        if norm.starts_with(d) || norm == d.trim_end_matches('/') {
            return Err(anyhow!("refusing to wipe preserve-dir: {}", d));
        }
    }
    for f in PRESERVE_WORKSPACE_FILES {
        let ws_path = format!("workspace/{}", f);
        if norm == ws_path {
            return Err(anyhow!("refusing to wipe identity-file: {}", ws_path));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive double-confirm.
// ---------------------------------------------------------------------------

fn double_confirm() -> Result<bool> {
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    print!("This will wipe fleet-state. Type 'yes' to continue: ");
    stdout.flush()?;
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    if line.trim() != "yes" {
        return Ok(false);
    }

    print!("Confirm again — type 'RESET' to proceed: ");
    stdout.flush()?;
    line.clear();
    stdin.lock().read_line(&mut line)?;
    Ok(line.trim() == "RESET")
}

// ---------------------------------------------------------------------------
// IO execution.
// ---------------------------------------------------------------------------

/// Resolve `~/.zeus/` root (or override via $ZEUS_HOME for testing).
pub fn zeus_home() -> Result<PathBuf> {
    if let Ok(h) = std::env::var("ZEUS_HOME") {
        return Ok(PathBuf::from(h));
    }
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join(".zeus"))
}

/// Wipe a single rel-path under `root`. Handles dirs (recursive), files,
/// and SQLite WAL/SHM siblings. For `sessions/`, removes only `*.jsonl`
/// (preserves the directory itself).
fn wipe_one(root: &Path, rel: &str, dry_run: bool) -> Result<()> {
    assert_preserve_safe(rel)?;
    let target = root.join(rel);

    if rel == "sessions/" {
        if !target.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&target)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                if dry_run {
                    println!("[dry-run] rm {}", path.display());
                } else {
                    std::fs::remove_file(&path)
                        .with_context(|| format!("rm {}", path.display()))?;
                }
            }
        }
        return Ok(());
    }

    let is_dir_marker = rel.ends_with('/');
    if !target.exists() {
        return Ok(());
    }

    if dry_run {
        println!("[dry-run] rm -rf {}", target.display());
        return Ok(());
    }

    if is_dir_marker || target.is_dir() {
        std::fs::remove_dir_all(&target)
            .with_context(|| format!("rm -rf {}", target.display()))?;
    } else {
        std::fs::remove_file(&target)
            .with_context(|| format!("rm {}", target.display()))?;
        // SQLite WAL/SHM siblings.
        for ext in &["-wal", "-shm"] {
            let sib = root.join(format!("{}{}", rel, ext));
            if sib.exists() {
                let _ = std::fs::remove_file(&sib);
            }
        }
    }
    Ok(())
}

/// Invoke `mnemosyne-cleanup --apply --vacuum` as a subprocess (clean boundary).
fn run_mnemosyne_cleanup(dry_run: bool) -> Result<()> {
    if dry_run {
        println!("[dry-run] mnemosyne-cleanup --apply --vacuum");
        return Ok(());
    }
    let status = Command::new("mnemosyne-cleanup")
        .arg("--apply")
        .arg("--vacuum")
        .status()
        .context("invoke mnemosyne-cleanup (is it on PATH?)")?;
    if !status.success() {
        return Err(anyhow!(
            "mnemosyne-cleanup exited non-zero: {:?}",
            status.code()
        ));
    }
    Ok(())
}

/// Top-level entry point. Called from `Commands::Reset` dispatch arm.
pub fn run(args: ResetArgs) -> Result<()> {
    let plan = compute_plan(&args);

    println!("zeus reset — plan:");
    if plan.run_mnemosyne_cleanup {
        println!("  - mnemosyne-cleanup --apply --vacuum   (memory.db row-purge + VACUUM)");
    }
    for p in &plan.paths {
        println!("  - rm -rf ~/.zeus/{}", p);
    }
    if plan.paths.is_empty() && !plan.run_mnemosyne_cleanup {
        println!("  (nothing to do)");
        return Ok(());
    }

    if args.dry_run {
        println!("dry-run — no IO performed.");
        // Still walk paths to surface any preserve-violation as error.
        let root = zeus_home()?;
        for p in &plan.paths {
            wipe_one(&root, p, true)?;
        }
        if plan.run_mnemosyne_cleanup {
            run_mnemosyne_cleanup(true)?;
        }
        return Ok(());
    }

    // Destructive path — double-confirm unless explicitly bypassed.
    if !(args.yes && args.hard) {
        if !double_confirm()? {
            println!("aborted.");
            return Ok(());
        }
    }

    let root = zeus_home()?;

    // Filesystem wipes first; subprocess last (so a subprocess failure leaves
    // memory.db intact for forensics).
    for p in &plan.paths {
        wipe_one(&root, p, false)?;
        println!("wiped: ~/.zeus/{}", p);
    }
    if plan.run_mnemosyne_cleanup {
        run_mnemosyne_cleanup(false)?;
        println!("mnemosyne-cleanup --apply --vacuum: ok");
    }

    println!("reset complete.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — regression-fence on preserve-allow-list invariant + scope shapes.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_plan_excludes_memory_db_path_uses_subprocess() {
        let args = ResetArgs { all: true, ..Default::default() };
        let plan = compute_plan(&args);
        assert!(plan.run_mnemosyne_cleanup);
        assert!(!plan.paths.iter().any(|p| p == "memory.db"));
        // All other 9 wipe-set surfaces present:
        assert!(plan.paths.contains(&"scheduler.db".into()));
        assert!(plan.paths.contains(&"sessions/".into()));
        assert!(plan.paths.contains(&"workspace/memory/".into()));
        assert!(plan.paths.contains(&".skills_tmp/".into()));
    }

    #[test]
    fn memory_only_plan_runs_subprocess_only() {
        let args = ResetArgs { memory_only: true, ..Default::default() };
        let plan = compute_plan(&args);
        assert!(plan.run_mnemosyne_cleanup);
        assert!(plan.paths.is_empty());
    }

    #[test]
    fn scheduler_only_plan_is_one_path() {
        let args = ResetArgs { scheduler_only: true, ..Default::default() };
        let plan = compute_plan(&args);
        assert!(!plan.run_mnemosyne_cleanup);
        assert_eq!(plan.paths, vec!["scheduler.db".to_string()]);
    }

    #[test]
    fn sessions_only_plan_is_sessions_dir() {
        let args = ResetArgs { sessions_only: true, ..Default::default() };
        let plan = compute_plan(&args);
        assert!(!plan.run_mnemosyne_cleanup);
        assert_eq!(plan.paths, vec!["sessions/".to_string()]);
    }

    #[test]
    fn no_flags_defaults_to_all() {
        let args = ResetArgs::default();
        let plan = compute_plan(&args);
        assert!(plan.run_mnemosyne_cleanup);
        assert!(plan.paths.len() >= 8);
    }

    /// LOAD-BEARING regression fence: every preserve-class artifact must be
    /// rejected by `assert_preserve_safe`, even if WIPE_SET drifts.
    #[test]
    fn reset_preserve_allow_list_survives_all() {
        for f in PRESERVE_FILES {
            assert!(
                assert_preserve_safe(f).is_err(),
                "preserve-file slipped past guard: {}",
                f
            );
        }
        for d in PRESERVE_DIRS {
            assert!(
                assert_preserve_safe(d).is_err(),
                "preserve-dir slipped past guard: {}",
                d
            );
            // And sub-path inside:
            let sub = format!("{}some-child", d);
            assert!(
                assert_preserve_safe(&sub).is_err(),
                "preserve-dir child slipped past guard: {}",
                sub
            );
        }
        for f in PRESERVE_WORKSPACE_FILES {
            let path = format!("workspace/{}", f);
            assert!(
                assert_preserve_safe(&path).is_err(),
                "identity-file slipped past guard: {}",
                path
            );
        }
    }

    #[test]
    fn reset_wipe_paths_are_preserve_safe() {
        // Every WIPE_SET entry must pass the preserve guard — otherwise the
        // two lists have drifted into contradiction.
        for w in WIPE_SET {
            if *w == "memory.db" {
                continue; // handled by subprocess, not direct path-wipe
            }
            assert!(
                assert_preserve_safe(w).is_ok(),
                "wipe-set entry blocked by preserve guard: {}",
                w
            );
        }
    }
}
