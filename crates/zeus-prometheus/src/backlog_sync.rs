//! Autonomous backlog-pull layer (Sprint #84).
//!
//! Periodically pulls work items from an external backlog source (a local
//! markdown file, GitHub issues, or both) and stages them as goal files in
//! `~/.zeus/workspace/goals/<slug>.md` for the existing autonomous_loop
//! hot-loader (see `src/gateway.rs:2978-3019`) to pick up.
//!
//! Design doc: `docs/design/autonomous-backlog-system-2026-05-21.md`.
//!
//! ## V1 scope
//! - `BacklogSource::LocalFile` — markdown checklist parser
//! - `BacklogSource::GithubIssues` — stubbed (returns empty)
//! - `BacklogSource::Hybrid` — stubbed (returns empty)
//! - Idempotency: skip items whose slug already exists as a goal file or has
//!   been seen this process lifetime (in-memory dedup set).
//! - Backpressure: respect `max_pending`; stop staging once the goals
//!   directory contains that many `.md` files.
//!
//! Anti-patterns avoided (from MAST taxonomy + design doc §2.2):
//! - **No self-generated tasks** — backlog is externally bounded.
//! - **No LLM self-judged completion** — goal files include an
//!   `acceptance:` block that downstream layers can verify.
//! - **Slug-based idempotency** — re-running the loop on the same source
//!   does not re-stage items already in `goals/`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};

/// Source of backlog items.
#[derive(Debug, Clone)]
pub enum BacklogSource {
    /// Markdown checklist on disk. Expected format per line:
    ///   `- [ ] [P0] Title — slug: my-task — body...`
    /// Checked items (`- [x]`) are skipped.
    LocalFile { path: PathBuf },

    /// GitHub issues, filtered by label. V1 stub — returns empty.
    GithubIssues {
        repo: String,
        labels: Vec<String>,
        token: String,
    },

    /// Both sources merged. V1 stub — returns empty.
    Hybrid {
        github: Box<BacklogSource>,
        local: Box<BacklogSource>,
    },
}

/// One backlog item — minimum fields needed to stage a goal file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacklogItem {
    pub slug: String,
    pub title: String,
    pub priority: String, // "P0" | "P1" | "P2" | "normal"
    pub body: String,
}

/// Configuration for the backlog sync loop.
#[derive(Debug, Clone)]
pub struct BacklogSyncConfig {
    pub source: BacklogSource,
    /// Poll interval in seconds (default 60).
    pub poll_interval_secs: u64,
    /// Cap on pending goal files in `goals/` before we stop staging (default 20).
    pub max_pending: usize,
    /// Titan role string, for future role-based filtering.
    pub titan_role: String,
}

impl Default for BacklogSyncConfig {
    fn default() -> Self {
        Self {
            source: BacklogSource::LocalFile {
                path: dirs::home_dir()
                    .unwrap_or_default()
                    .join(".zeus/workspace/BACKLOG.md"),
            },
            poll_interval_secs: 60,
            max_pending: 20,
            titan_role: "implementer".to_string(),
        }
    }
}

/// In-memory dedup set so re-polling the same source doesn't re-stage
/// items that have already been written this process lifetime.
#[derive(Default, Clone)]
pub struct SeenSet(Arc<Mutex<HashSet<String>>>);

impl SeenSet {
    pub fn new() -> Self {
        Self::default()
    }
    fn contains(&self, slug: &str) -> bool {
        self.0.lock().map(|s| s.contains(slug)).unwrap_or(false)
    }
    fn insert(&self, slug: &str) {
        if let Ok(mut s) = self.0.lock() {
            s.insert(slug.to_string());
        }
    }
}

/// Run the sync loop forever. Returns only on unrecoverable error.
///
/// The loop:
/// 1. Fetches items from `config.source`.
/// 2. Filters out items already staged (by slug → filename).
/// 3. Writes one goal file per new item, up to `max_pending`.
/// 4. Sleeps `poll_interval_secs` and repeats.
pub async fn sync_loop(config: BacklogSyncConfig, goals_dir: PathBuf) -> Result<()> {
    let seen = SeenSet::new();
    let interval = Duration::from_secs(config.poll_interval_secs.max(1));
    loop {
        match fetch_backlog(&config.source).await {
            Ok(items) => {
                match stage_new_items(&items, &goals_dir, &config, &seen).await {
                    Ok(n) if n > 0 => {
                        tracing::info!(
                            staged = n,
                            source = ?source_kind(&config.source),
                            "backlog_sync: staged new goal files"
                        );
                    }
                    Ok(_) => {} // nothing to do
                    Err(e) => tracing::warn!("backlog_sync: stage error: {e:#}"),
                }
            }
            Err(e) => tracing::warn!("backlog_sync: fetch error: {e:#}"),
        }
        tokio::time::sleep(interval).await;
    }
}

fn source_kind(s: &BacklogSource) -> &'static str {
    match s {
        BacklogSource::LocalFile { .. } => "local",
        BacklogSource::GithubIssues { .. } => "github",
        BacklogSource::Hybrid { .. } => "hybrid",
    }
}

/// Fetch all candidate items from a source. Pure (no side effects on disk).
pub async fn fetch_backlog(source: &BacklogSource) -> Result<Vec<BacklogItem>> {
    match source {
        BacklogSource::LocalFile { path } => {
            if !path.exists() {
                return Ok(Vec::new());
            }
            let content = tokio::fs::read_to_string(path)
                .await
                .with_context(|| format!("reading backlog file {}", path.display()))?;
            Ok(parse_local_backlog(&content))
        }
        // V1: stubs. Real impls land in Phase A C-3.
        BacklogSource::GithubIssues { .. } => Ok(Vec::new()),
        BacklogSource::Hybrid { .. } => Ok(Vec::new()),
    }
}

/// Parse a markdown backlog file.
///
/// Expected per-line format (loose — extra text after the body is preserved):
///   `- [ ] [P0] Title goes here — slug: my-slug — optional body...`
///
/// Rules:
/// - Lines starting with `- [x]` (case-insensitive) are skipped (done).
/// - Lines without a `slug:` token are skipped (no stable id → can't dedup).
/// - Priority defaults to `"normal"` if no `[Pn]` tag is present.
pub fn parse_local_backlog(content: &str) -> Vec<BacklogItem> {
    let mut out = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        // Must look like a checklist item.
        let body = if let Some(rest) = line.strip_prefix("- [ ]") {
            rest.trim()
        } else if let Some(rest) = line.strip_prefix("- [X]").or_else(|| line.strip_prefix("- [x]")) {
            // checked → skip
            let _ = rest;
            continue;
        } else {
            continue;
        };

        // Find slug token: `slug: <id>` — required.
        let Some(slug_idx) = body.find("slug:") else { continue };
        let after_slug = &body[slug_idx + "slug:".len()..];
        // Slug ends at next whitespace or em-dash (NOT ASCII '-', which is
        // a legal slug character — `hb-fix` must remain intact).
        let slug: String = after_slug
            .trim_start()
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '—')
            .collect();
        if slug.is_empty() {
            continue;
        }

        // Extract priority `[P0]` style — optional.
        let priority = if let (Some(s), Some(e)) = (body.find('['), body.find(']')) {
            if e > s + 1 {
                body[s + 1..e].to_string()
            } else {
                "normal".to_string()
            }
        } else {
            "normal".to_string()
        };

        // Title: everything between the `]` and the first `—` or `slug:`.
        let title_start = body.find(']').map(|i| i + 1).unwrap_or(0);
        let title_end = body
            .find('—')
            .or_else(|| body.find("slug:"))
            .unwrap_or(body.len());
        let title = if title_end > title_start {
            body[title_start..title_end].trim().to_string()
        } else {
            slug.clone()
        };

        out.push(BacklogItem {
            slug,
            title,
            priority,
            body: body.to_string(),
        });
    }
    out
}

/// Write goal files for items not yet staged. Returns the count staged.
///
/// Idempotency: an item with slug `X` is skipped if either
///   (a) `<goals_dir>/<X>.md` already exists, or
///   (b) `seen` already contains `X` (this-process dedup).
///
/// Backpressure: if the goals directory already contains `max_pending`
/// `.md` files, we stop staging.
pub async fn stage_new_items(
    items: &[BacklogItem],
    goals_dir: &Path,
    config: &BacklogSyncConfig,
    seen: &SeenSet,
) -> Result<usize> {
    tokio::fs::create_dir_all(goals_dir)
        .await
        .with_context(|| format!("creating goals dir {}", goals_dir.display()))?;

    let mut current = count_md_files(goals_dir).await?;
    let mut staged = 0;
    for item in items {
        if current >= config.max_pending {
            tracing::debug!(
                pending = current,
                max = config.max_pending,
                "backlog_sync: max_pending reached, deferring"
            );
            break;
        }
        if seen.contains(&item.slug) {
            continue;
        }
        let target = goals_dir.join(format!("{}.md", sanitize_slug(&item.slug)));
        if target.exists() {
            // Already staged on a previous run — record and skip.
            seen.insert(&item.slug);
            continue;
        }
        let goal_md = render_goal_file(item, &config.titan_role);
        tokio::fs::write(&target, goal_md)
            .await
            .with_context(|| format!("writing goal file {}", target.display()))?;
        seen.insert(&item.slug);
        current += 1;
        staged += 1;
        tracing::info!(slug = %item.slug, path = %target.display(), "backlog_sync: staged");
    }
    Ok(staged)
}

async fn count_md_files(dir: &Path) -> Result<usize> {
    let mut count = 0;
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(_) => return Ok(0),
    };
    while let Some(e) = rd.next_entry().await? {
        if e.path().extension().map_or(false, |x| x == "md") {
            count += 1;
        }
    }
    Ok(count)
}

fn sanitize_slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Render a goal file: YAML front-matter + markdown body that the hot-loader
/// at `src/gateway.rs:2978` parses via `zeus_agent::tools::parse_goal_front_matter`.
pub fn render_goal_file(item: &BacklogItem, titan_role: &str) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    format!(
        "---\n\
         title: {title:?}\n\
         priority: {priority}\n\
         source: backlog_sync\n\
         slug: {slug}\n\
         titan_role: {role}\n\
         staged_at: {now}\n\
         ---\n\
         \n\
         # Goal\n\
         \n\
         {body}\n\
         \n\
         # Acceptance\n\
         \n\
         - Item completed per backlog description above.\n\
         - Verifiable artifact produced (commit, file, or message).\n",
        title = item.title,
        priority = item.priority,
        slug = item.slug,
        role = titan_role,
        now = now,
        body = item.body,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_backlog() {
        let src = "\
- [ ] [P0] Fix the heartbeat — slug: hb-fix — heartbeat dies on cold start
- [x] [P1] Done item — slug: done-thing — should be skipped
- [ ] [P2] No slug here — should be skipped
- [ ] [P1] Another — slug: another-task — second item
not a checklist line
";
        let items = parse_local_backlog(src);
        assert_eq!(items.len(), 2, "got: {items:#?}");
        assert_eq!(items[0].slug, "hb-fix");
        assert_eq!(items[0].priority, "P0");
        assert!(items[0].title.contains("Fix the heartbeat"));
        assert_eq!(items[1].slug, "another-task");
    }

    #[test]
    fn checked_items_are_skipped() {
        let src = "- [x] [P0] Done — slug: done-1 — finished\n- [X] [P0] Also done — slug: done-2 — also finished\n";
        assert!(parse_local_backlog(src).is_empty());
    }

    #[test]
    fn items_without_slug_are_skipped() {
        let src = "- [ ] [P0] Anonymous task with no slug at all\n";
        assert!(parse_local_backlog(src).is_empty());
    }

    #[tokio::test]
    async fn stages_items_to_goal_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BacklogSyncConfig {
            source: BacklogSource::LocalFile {
                path: tmp.path().join("BACKLOG.md"),
            },
            poll_interval_secs: 60,
            max_pending: 20,
            titan_role: "implementer".to_string(),
        };
        let seen = SeenSet::new();
        let items = vec![BacklogItem {
            slug: "test-task-1".to_string(),
            title: "Test task".to_string(),
            priority: "P0".to_string(),
            body: "do the thing".to_string(),
        }];
        let n = stage_new_items(&items, tmp.path(), &cfg, &seen).await.unwrap();
        assert_eq!(n, 1);
        let staged = tmp.path().join("test-task-1.md");
        assert!(staged.exists(), "goal file not written");
        let content = std::fs::read_to_string(&staged).unwrap();
        assert!(content.starts_with("---\n"), "missing front-matter");
        assert!(content.contains("slug: test-task-1"));
        assert!(content.contains("# Goal"));
        assert!(content.contains("# Acceptance"));
    }

    #[tokio::test]
    async fn idempotent_on_repeat() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BacklogSyncConfig {
            source: BacklogSource::LocalFile { path: tmp.path().join("BACKLOG.md") },
            poll_interval_secs: 60,
            max_pending: 20,
            titan_role: "implementer".to_string(),
        };
        let seen = SeenSet::new();
        let items = vec![BacklogItem {
            slug: "dedup-test".to_string(),
            title: "T".to_string(),
            priority: "P0".to_string(),
            body: "b".to_string(),
        }];
        let first = stage_new_items(&items, tmp.path(), &cfg, &seen).await.unwrap();
        let second = stage_new_items(&items, tmp.path(), &cfg, &seen).await.unwrap();
        let third = stage_new_items(&items, tmp.path(), &cfg, &seen).await.unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 0, "second pass must not re-stage");
        assert_eq!(third, 0, "third pass must not re-stage");
    }

    #[tokio::test]
    async fn idempotent_across_process_restart() {
        // Simulate: process restart → seen set cleared, but file on disk persists.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BacklogSyncConfig {
            source: BacklogSource::LocalFile { path: tmp.path().join("BACKLOG.md") },
            poll_interval_secs: 60,
            max_pending: 20,
            titan_role: "implementer".to_string(),
        };
        let items = vec![BacklogItem {
            slug: "persist-test".to_string(),
            title: "T".to_string(),
            priority: "P0".to_string(),
            body: "b".to_string(),
        }];
        // First "process":
        let seen1 = SeenSet::new();
        assert_eq!(stage_new_items(&items, tmp.path(), &cfg, &seen1).await.unwrap(), 1);
        // Second "process" — fresh seen set, but file still on disk:
        let seen2 = SeenSet::new();
        assert_eq!(stage_new_items(&items, tmp.path(), &cfg, &seen2).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn respects_max_pending_backpressure() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BacklogSyncConfig {
            source: BacklogSource::LocalFile { path: tmp.path().join("BACKLOG.md") },
            poll_interval_secs: 60,
            max_pending: 2,
            titan_role: "implementer".to_string(),
        };
        let seen = SeenSet::new();
        let items: Vec<BacklogItem> = (0..5)
            .map(|i| BacklogItem {
                slug: format!("bp-{i}"),
                title: format!("Task {i}"),
                priority: "P1".to_string(),
                body: "body".to_string(),
            })
            .collect();
        let n = stage_new_items(&items, tmp.path(), &cfg, &seen).await.unwrap();
        assert_eq!(n, 2, "must stop at max_pending");
        assert_eq!(count_md_files(tmp.path()).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn fetch_localfile_missing_returns_empty() {
        let src = BacklogSource::LocalFile {
            path: PathBuf::from("/nonexistent/path/to/backlog.md"),
        };
        let items = fetch_backlog(&src).await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn fetch_github_stub_returns_empty() {
        let src = BacklogSource::GithubIssues {
            repo: "z/x".to_string(),
            labels: vec!["backlog".into()],
            token: "fake".into(),
        };
        assert!(fetch_backlog(&src).await.unwrap().is_empty());
    }

    #[test]
    fn render_goal_file_has_required_keys() {
        let item = BacklogItem {
            slug: "render-test".into(),
            title: "Render Test".into(),
            priority: "P0".into(),
            body: "the body".into(),
        };
        let md = render_goal_file(&item, "implementer");
        assert!(md.starts_with("---\n"));
        assert!(md.contains("slug: render-test"));
        assert!(md.contains("priority: P0"));
        assert!(md.contains("source: backlog_sync"));
        assert!(md.contains("# Goal"));
        assert!(md.contains("# Acceptance"));
    }

    #[test]
    fn sanitize_slug_filters_path_chars() {
        assert_eq!(sanitize_slug("ok-slug_123"), "ok-slug_123");
        assert_eq!(sanitize_slug("../etc/passwd"), "---etc-passwd");
        assert_eq!(sanitize_slug("a/b/c"), "a-b-c");
    }
}
