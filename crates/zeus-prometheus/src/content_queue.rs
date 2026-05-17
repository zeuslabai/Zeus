//! Content Batch Queue — persistent scheduling for social media publishing
//!
//! Provides a SQLite-backed queue for content publishing jobs across platforms
//! (YouTube, TikTok, Instagram). Jobs can be scheduled for immediate or future
//! publishing and are processed by the cron scheduler.
//!
//! ## Job lifecycle
//!
//! ```text
//! Queued → Scheduled → Publishing → Published
//!                         ↓
//!                       Failed → (retry) → Queued
//! ```

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

/// Supported publishing platforms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    YouTube,
    TikTok,
    Instagram,
    X,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::YouTube => write!(f, "youtube"),
            Self::TikTok => write!(f, "tiktok"),
            Self::Instagram => write!(f, "instagram"),
            Self::X => write!(f, "x"),
        }
    }
}

impl Platform {
    /// Parse from string (case-insensitive).
    /// Accepts "x", "twitter", and "x.com" as aliases for the X platform.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "youtube" => Some(Self::YouTube),
            "tiktok" => Some(Self::TikTok),
            "instagram" => Some(Self::Instagram),
            "x" | "twitter" | "x.com" => Some(Self::X),
            _ => None,
        }
    }
}

/// Status of a content publishing job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Waiting in queue.
    Queued,
    /// Assigned a scheduled time, waiting for cron to fire.
    Scheduled,
    /// Currently being published.
    Publishing,
    /// Successfully published.
    Published,
    /// Publishing failed.
    Failed,
    /// Cancelled by user.
    Cancelled,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Scheduled => write!(f, "scheduled"),
            Self::Publishing => write!(f, "publishing"),
            Self::Published => write!(f, "published"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl JobStatus {
    fn from_str_db(s: &str) -> Self {
        match s {
            "queued" => Self::Queued,
            "scheduled" => Self::Scheduled,
            "publishing" => Self::Publishing,
            "published" => Self::Published,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

/// A content publishing job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentJob {
    /// Unique job ID.
    pub id: String,
    /// Target platform.
    pub platform: Platform,
    /// Local file path to the content.
    pub file_path: String,
    /// Title / caption.
    pub title: String,
    /// Description (optional).
    pub description: String,
    /// Tags (JSON array string).
    pub tags: Vec<String>,
    /// Platform-specific privacy setting.
    pub privacy: String,
    /// Current status.
    pub status: JobStatus,
    /// When to publish (None = immediately when next processed).
    pub scheduled_at: Option<DateTime<Utc>>,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job was last updated.
    pub updated_at: DateTime<Utc>,
    /// Platform-specific result (e.g., video URL, publish_id).
    pub result: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Number of retry attempts.
    pub retries: u32,
}

/// The Content Queue — persistent SQLite-backed job queue.
pub struct ContentQueue {
    db: Arc<Mutex<Connection>>,
}

impl ContentQueue {
    /// Create or open a content queue at the given SQLite database path.
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open content queue DB: {e}"))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| format!("Failed to set PRAGMA: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS content_jobs (
                id TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                file_path TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                tags TEXT NOT NULL DEFAULT '[]',
                privacy TEXT NOT NULL DEFAULT 'private',
                status TEXT NOT NULL DEFAULT 'queued',
                scheduled_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                result TEXT,
                error TEXT,
                retries INTEGER NOT NULL DEFAULT 0
            )",
        )
        .map_err(|e| format!("Failed to create content_jobs table: {e}"))?;

        info!("Content queue initialized at {}", db_path.display());

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory queue (for testing).
    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory DB: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS content_jobs (
                id TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                file_path TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                tags TEXT NOT NULL DEFAULT '[]',
                privacy TEXT NOT NULL DEFAULT 'private',
                status TEXT NOT NULL DEFAULT 'queued',
                scheduled_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                result TEXT,
                error TEXT,
                retries INTEGER NOT NULL DEFAULT 0
            )",
        )
        .map_err(|e| format!("Failed to create table: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Add a new content publishing job to the queue.
    #[allow(clippy::too_many_arguments)]
    pub async fn enqueue(
        &self,
        platform: Platform,
        file_path: &str,
        title: &str,
        description: &str,
        tags: Vec<String>,
        privacy: &str,
        scheduled_at: Option<DateTime<Utc>>,
    ) -> Result<ContentJob, String> {
        let now = Utc::now();
        let id = ulid::Ulid::new().to_string();
        let status = if scheduled_at.is_some() {
            JobStatus::Scheduled
        } else {
            JobStatus::Queued
        };
        let tags_json =
            serde_json::to_string(&tags).map_err(|e| format!("Failed to serialize tags: {e}"))?;

        let job = ContentJob {
            id: id.clone(),
            platform: platform.clone(),
            file_path: file_path.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            tags,
            privacy: privacy.to_string(),
            status,
            scheduled_at,
            created_at: now,
            updated_at: now,
            result: None,
            error: None,
            retries: 0,
        };

        let db = self.db.lock().await;
        db.execute(
            "INSERT INTO content_jobs (id, platform, file_path, title, description, tags, privacy, status, scheduled_at, created_at, updated_at, retries)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
            params![
                id,
                platform.to_string(),
                file_path,
                title,
                description,
                tags_json,
                privacy,
                job.status.to_string(),
                scheduled_at.map(|t| t.to_rfc3339()),
                now.to_rfc3339(),
                now.to_rfc3339(),
            ],
        )
        .map_err(|e| format!("Failed to insert job: {e}"))?;

        debug!(job_id = %id, platform = %platform, "Content job enqueued");
        Ok(job)
    }

    /// Get all jobs ready for publishing (queued with no future schedule, or scheduled and past due).
    pub async fn get_ready_jobs(&self) -> Result<Vec<ContentJob>, String> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let mut stmt = db
            .prepare(
                "SELECT id, platform, file_path, title, description, tags, privacy, status, scheduled_at, created_at, updated_at, result, error, retries
                 FROM content_jobs
                 WHERE (status = 'queued') OR (status = 'scheduled' AND scheduled_at <= ?1)
                 ORDER BY COALESCE(scheduled_at, created_at) ASC",
            )
            .map_err(|e| format!("Failed to prepare query: {e}"))?;

        let jobs = stmt
            .query_map(params![now], |row| Ok(Self::row_to_job(row)))
            .map_err(|e| format!("Failed to query ready jobs: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(jobs)
    }

    /// List all jobs, optionally filtered by status.
    pub async fn list_jobs(&self, status_filter: Option<&JobStatus>) -> Result<Vec<ContentJob>, String> {
        let db = self.db.lock().await;
        let jobs = if let Some(status) = status_filter {
            let mut stmt = db
                .prepare(
                    "SELECT id, platform, file_path, title, description, tags, privacy, status, scheduled_at, created_at, updated_at, result, error, retries
                     FROM content_jobs WHERE status = ?1 ORDER BY created_at DESC",
                )
                .map_err(|e| format!("Failed to prepare query: {e}"))?;
            stmt.query_map(params![status.to_string()], |row| Ok(Self::row_to_job(row)))
                .map_err(|e| format!("Query failed: {e}"))?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            let mut stmt = db
                .prepare(
                    "SELECT id, platform, file_path, title, description, tags, privacy, status, scheduled_at, created_at, updated_at, result, error, retries
                     FROM content_jobs ORDER BY created_at DESC",
                )
                .map_err(|e| format!("Failed to prepare query: {e}"))?;
            stmt.query_map([], |row| Ok(Self::row_to_job(row)))
                .map_err(|e| format!("Query failed: {e}"))?
                .filter_map(|r| r.ok())
                .collect()
        };
        Ok(jobs)
    }

    /// Get a job by ID.
    pub async fn get_job(&self, job_id: &str) -> Result<Option<ContentJob>, String> {
        let db = self.db.lock().await;
        let mut stmt = db
            .prepare(
                "SELECT id, platform, file_path, title, description, tags, privacy, status, scheduled_at, created_at, updated_at, result, error, retries
                 FROM content_jobs WHERE id = ?1",
            )
            .map_err(|e| format!("Failed to prepare query: {e}"))?;

        let mut rows = stmt
            .query_map(params![job_id], |row| Ok(Self::row_to_job(row)))
            .map_err(|e| format!("Query failed: {e}"))?;

        Ok(rows.next().and_then(|r| r.ok()))
    }

    /// Update job status.
    pub async fn update_status(
        &self,
        job_id: &str,
        status: JobStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), String> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        db.execute(
            "UPDATE content_jobs SET status = ?1, result = ?2, error = ?3, updated_at = ?4 WHERE id = ?5",
            params![status.to_string(), result, error, now, job_id],
        )
        .map_err(|e| format!("Failed to update job: {e}"))?;

        debug!(job_id = %job_id, status = %status, "Job status updated");
        Ok(())
    }

    /// Cancel a job (only if queued or scheduled).
    pub async fn cancel_job(&self, job_id: &str) -> Result<bool, String> {
        let db = self.db.lock().await;
        let rows = db
            .execute(
                "UPDATE content_jobs SET status = 'cancelled', updated_at = ?1 WHERE id = ?2 AND status IN ('queued', 'scheduled')",
                params![Utc::now().to_rfc3339(), job_id],
            )
            .map_err(|e| format!("Failed to cancel job: {e}"))?;
        Ok(rows > 0)
    }

    /// Retry a failed job by resetting its status to Queued.
    pub async fn retry_job(&self, job_id: &str) -> Result<bool, String> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let rows = db
            .execute(
                "UPDATE content_jobs SET status = 'queued', error = NULL, retries = retries + 1, updated_at = ?1 WHERE id = ?2 AND status = 'failed'",
                params![now, job_id],
            )
            .map_err(|e| format!("Failed to retry job: {e}"))?;
        Ok(rows > 0)
    }

    /// Get queue statistics.
    pub async fn stats(&self) -> Result<QueueStats, String> {
        let db = self.db.lock().await;
        let mut stmt = db
            .prepare("SELECT status, COUNT(*) FROM content_jobs GROUP BY status")
            .map_err(|e| format!("Failed to prepare stats query: {e}"))?;

        let mut stats = QueueStats::default();
        let rows = stmt
            .query_map([], |row| {
                let status: String = row.get(0)?;
                let count: u32 = row.get(1)?;
                Ok((status, count))
            })
            .map_err(|e| format!("Stats query failed: {e}"))?;

        for row in rows.flatten() {
            match row.0.as_str() {
                "queued" => stats.queued = row.1,
                "scheduled" => stats.scheduled = row.1,
                "publishing" => stats.publishing = row.1,
                "published" => stats.published = row.1,
                "failed" => stats.failed = row.1,
                "cancelled" => stats.cancelled = row.1,
                _ => {}
            }
            stats.total += row.1;
        }
        Ok(stats)
    }

    /// Parse a database row into a ContentJob.
    fn row_to_job(row: &rusqlite::Row<'_>) -> ContentJob {
        let tags_json: String = row.get(5).unwrap_or_default();
        let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
        let scheduled_str: Option<String> = row.get(8).unwrap_or(None);
        let scheduled_at = scheduled_str
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        ContentJob {
            id: row.get(0).unwrap_or_default(),
            platform: Platform::from_str_loose(&row.get::<_, String>(1).unwrap_or_default())
                .unwrap_or(Platform::YouTube),
            file_path: row.get(2).unwrap_or_default(),
            title: row.get(3).unwrap_or_default(),
            description: row.get(4).unwrap_or_default(),
            tags,
            privacy: row.get(6).unwrap_or_default(),
            status: JobStatus::from_str_db(&row.get::<_, String>(7).unwrap_or_default()),
            scheduled_at,
            created_at: row
                .get::<_, String>(9)
                .ok()
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
            updated_at: row
                .get::<_, String>(10)
                .ok()
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
            result: row.get(11).unwrap_or(None),
            error: row.get(12).unwrap_or(None),
            retries: row.get::<_, u32>(13).unwrap_or(0),
        }
    }
}

/// Queue statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueStats {
    pub total: u32,
    pub queued: u32,
    pub scheduled: u32,
    pub publishing: u32,
    pub published: u32,
    pub failed: u32,
    pub cancelled: u32,
}

impl std::fmt::Display for QueueStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} jobs: {} queued, {} scheduled, {} publishing, {} published, {} failed",
            self.total, self.queued, self.scheduled, self.publishing, self.published, self.failed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_enqueue_immediate() {
        let queue = ContentQueue::in_memory().unwrap();
        let job = queue
            .enqueue(
                Platform::YouTube,
                "/tmp/video.mp4",
                "Test Video",
                "A test video",
                vec!["test".to_string()],
                "private",
                None,
            )
            .await
            .unwrap();
        assert_eq!(job.status, JobStatus::Queued);
        assert!(job.scheduled_at.is_none());
        assert!(!job.id.is_empty());
    }

    #[tokio::test]
    async fn test_enqueue_scheduled() {
        let queue = ContentQueue::in_memory().unwrap();
        let future = Utc::now() + chrono::Duration::hours(2);
        let job = queue
            .enqueue(
                Platform::TikTok,
                "/tmp/clip.mp4",
                "Scheduled Post",
                "",
                vec![],
                "SELF_ONLY",
                Some(future),
            )
            .await
            .unwrap();
        assert_eq!(job.status, JobStatus::Scheduled);
        assert!(job.scheduled_at.is_some());
    }

    #[tokio::test]
    async fn test_get_ready_jobs_immediate() {
        let queue = ContentQueue::in_memory().unwrap();
        queue
            .enqueue(Platform::YouTube, "/tmp/v.mp4", "Ready", "", vec![], "private", None)
            .await
            .unwrap();

        let ready = queue.get_ready_jobs().await.unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].title, "Ready");
    }

    #[tokio::test]
    async fn test_get_ready_jobs_future_not_ready() {
        let queue = ContentQueue::in_memory().unwrap();
        let future = Utc::now() + chrono::Duration::hours(2);
        queue
            .enqueue(Platform::TikTok, "/tmp/v.mp4", "Future", "", vec![], "private", Some(future))
            .await
            .unwrap();

        let ready = queue.get_ready_jobs().await.unwrap();
        assert!(ready.is_empty());
    }

    #[tokio::test]
    async fn test_get_ready_jobs_past_scheduled() {
        let queue = ContentQueue::in_memory().unwrap();
        let past = Utc::now() - chrono::Duration::hours(1);
        queue
            .enqueue(Platform::Instagram, "/tmp/v.mp4", "Past Due", "", vec![], "private", Some(past))
            .await
            .unwrap();

        let ready = queue.get_ready_jobs().await.unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].title, "Past Due");
    }

    #[tokio::test]
    async fn test_update_status() {
        let queue = ContentQueue::in_memory().unwrap();
        let job = queue
            .enqueue(Platform::YouTube, "/tmp/v.mp4", "Status Test", "", vec![], "private", None)
            .await
            .unwrap();

        queue
            .update_status(&job.id, JobStatus::Publishing, None, None)
            .await
            .unwrap();

        let fetched = queue.get_job(&job.id).await.unwrap().unwrap();
        assert_eq!(fetched.status, JobStatus::Publishing);

        queue
            .update_status(
                &job.id,
                JobStatus::Published,
                Some("https://youtube.com/watch?v=abc"),
                None,
            )
            .await
            .unwrap();

        let fetched = queue.get_job(&job.id).await.unwrap().unwrap();
        assert_eq!(fetched.status, JobStatus::Published);
        assert_eq!(fetched.result.as_deref(), Some("https://youtube.com/watch?v=abc"));
    }

    #[tokio::test]
    async fn test_cancel_job() {
        let queue = ContentQueue::in_memory().unwrap();
        let job = queue
            .enqueue(Platform::TikTok, "/tmp/v.mp4", "Cancel Me", "", vec![], "private", None)
            .await
            .unwrap();

        let cancelled = queue.cancel_job(&job.id).await.unwrap();
        assert!(cancelled);

        let fetched = queue.get_job(&job.id).await.unwrap().unwrap();
        assert_eq!(fetched.status, JobStatus::Cancelled);

        // Can't cancel again
        let cancelled_again = queue.cancel_job(&job.id).await.unwrap();
        assert!(!cancelled_again);
    }

    #[tokio::test]
    async fn test_retry_failed_job() {
        let queue = ContentQueue::in_memory().unwrap();
        let job = queue
            .enqueue(Platform::Instagram, "/tmp/v.mp4", "Retry Me", "", vec![], "private", None)
            .await
            .unwrap();

        // Mark as failed
        queue
            .update_status(&job.id, JobStatus::Failed, None, Some("timeout"))
            .await
            .unwrap();

        // Retry
        let retried = queue.retry_job(&job.id).await.unwrap();
        assert!(retried);

        let fetched = queue.get_job(&job.id).await.unwrap().unwrap();
        assert_eq!(fetched.status, JobStatus::Queued);
        assert_eq!(fetched.retries, 1);
        assert!(fetched.error.is_none());
    }

    #[tokio::test]
    async fn test_retry_non_failed_job() {
        let queue = ContentQueue::in_memory().unwrap();
        let job = queue
            .enqueue(Platform::YouTube, "/tmp/v.mp4", "Not Failed", "", vec![], "private", None)
            .await
            .unwrap();

        // Can't retry a queued job
        let retried = queue.retry_job(&job.id).await.unwrap();
        assert!(!retried);
    }

    #[tokio::test]
    async fn test_list_jobs_all() {
        let queue = ContentQueue::in_memory().unwrap();
        queue.enqueue(Platform::YouTube, "/tmp/a.mp4", "A", "", vec![], "private", None).await.unwrap();
        queue.enqueue(Platform::TikTok, "/tmp/b.mp4", "B", "", vec![], "private", None).await.unwrap();

        let all = queue.list_jobs(None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_list_jobs_filtered() {
        let queue = ContentQueue::in_memory().unwrap();
        let job = queue.enqueue(Platform::YouTube, "/tmp/a.mp4", "A", "", vec![], "private", None).await.unwrap();
        queue.enqueue(Platform::TikTok, "/tmp/b.mp4", "B", "", vec![], "private", None).await.unwrap();

        queue.update_status(&job.id, JobStatus::Published, Some("done"), None).await.unwrap();

        let published = queue.list_jobs(Some(&JobStatus::Published)).await.unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].title, "A");

        let queued = queue.list_jobs(Some(&JobStatus::Queued)).await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].title, "B");
    }

    #[tokio::test]
    async fn test_stats() {
        let queue = ContentQueue::in_memory().unwrap();
        let j1 = queue.enqueue(Platform::YouTube, "/tmp/a.mp4", "A", "", vec![], "private", None).await.unwrap();
        queue.enqueue(Platform::TikTok, "/tmp/b.mp4", "B", "", vec![], "private", None).await.unwrap();
        queue.enqueue(Platform::Instagram, "/tmp/c.mp4", "C", "", vec![], "private", None).await.unwrap();

        queue.update_status(&j1.id, JobStatus::Published, Some("ok"), None).await.unwrap();

        let stats = queue.stats().await.unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.queued, 2);
        assert_eq!(stats.published, 1);
    }

    #[tokio::test]
    async fn test_get_nonexistent_job() {
        let queue = ContentQueue::in_memory().unwrap();
        let result = queue.get_job("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_platform_display() {
        assert_eq!(Platform::YouTube.to_string(), "youtube");
        assert_eq!(Platform::TikTok.to_string(), "tiktok");
        assert_eq!(Platform::Instagram.to_string(), "instagram");
    }

    #[test]
    fn test_platform_from_str() {
        assert_eq!(Platform::from_str_loose("YouTube"), Some(Platform::YouTube));
        assert_eq!(Platform::from_str_loose("TIKTOK"), Some(Platform::TikTok));
        assert_eq!(Platform::from_str_loose("instagram"), Some(Platform::Instagram));
        assert_eq!(Platform::from_str_loose("unknown"), None);
    }

    #[test]
    fn test_queue_stats_display() {
        let stats = QueueStats {
            total: 10,
            queued: 3,
            scheduled: 2,
            publishing: 1,
            published: 3,
            failed: 1,
            cancelled: 0,
        };
        let s = stats.to_string();
        assert!(s.contains("10 jobs"));
        assert!(s.contains("3 queued"));
    }

    #[test]
    fn test_job_serialization() {
        let job = ContentJob {
            id: "test-123".to_string(),
            platform: Platform::YouTube,
            file_path: "/tmp/v.mp4".to_string(),
            title: "Test".to_string(),
            description: "Desc".to_string(),
            tags: vec!["tag1".to_string()],
            privacy: "private".to_string(),
            status: JobStatus::Queued,
            scheduled_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            result: None,
            error: None,
            retries: 0,
        };
        let json = serde_json::to_string(&job).unwrap();
        let deser: ContentJob = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.id, "test-123");
        assert_eq!(deser.platform, Platform::YouTube);
    }
}
