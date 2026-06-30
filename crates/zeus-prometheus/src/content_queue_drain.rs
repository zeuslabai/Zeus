//! Content queue drain — bridges the B5 `ContentQueue` with the B6 `ContentPipeline`.
//!
//! Scheduled via `TaskType::ContentQueueDrain` (typically every 1–5 minutes).
//! On each tick:
//!   1. Opens the `ContentQueue` at the configured SQLite path
//!   2. Fetches all ready jobs (`status = queued` or `scheduled AND due`)
//!   3. For each job:
//!      a. Marks the job `Publishing`
//!      b. Runs `execute_content_pipeline` with the job's parameters
//!      c. Marks the job `Published` (with result URL) or `Failed` (with error)
//!
//! Returns `(success, summary)` matching the `execute_task` convention.
//! `success = true` even when individual jobs fail — the drain itself succeeded.
//! Job-level failures are persisted in the queue and visible via `list_jobs`.

use crate::content_pipeline::execute_content_pipeline;
use crate::content_queue::{ContentQueue, JobStatus};
use tracing::{info, warn};

/// Drain the content queue: process all ready jobs through the content pipeline.
///
/// Returns `(true, summary)` if the drain ran successfully (even with job-level
/// failures). Returns `(false, error)` only if the queue cannot be opened.
pub async fn execute_content_queue_drain(db_path: &str) -> (bool, String) {
    // Open the queue at the configured path.
    let queue = match ContentQueue::new(std::path::Path::new(db_path)) {
        Ok(q) => q,
        Err(e) => return (false, format!("content_queue_drain: failed to open queue at {db_path}: {e}")),
    };

    // Fetch all ready jobs.
    let jobs = match queue.get_ready_jobs().await {
        Ok(j) => j,
        Err(e) => return (false, format!("content_queue_drain: failed to fetch ready jobs: {e}")),
    };

    if jobs.is_empty() {
        info!("content_queue_drain: no ready jobs");
        return (true, "content_queue_drain: 0 jobs ready".to_string());
    }

    let total = jobs.len();
    let mut published = 0usize;
    let mut failed = 0usize;

    for job in jobs {
        let job_id = &job.id;
        let platform = job.platform.to_string();

        // Mark as Publishing.
        if let Err(e) = queue
            .update_status(job_id, JobStatus::Publishing, None, None)
            .await
        {
            warn!("content_queue_drain: job {job_id} — failed to mark Publishing: {e}");
            // Don't skip — attempt the pipeline anyway.
        }

        // Run the content pipeline.
        let (ok, msg) = execute_content_pipeline(
            &job.file_path,
            &platform,
            &job.title,
            &job.description,
            &None::<String>, // trim_start — handled pre-enqueue
            &None::<String>, // trim_end
            &None::<String>, // captions_srt
            &None::<String>, // media_url — Instagram: caller pre-processes
        )
        .await;

        if ok {
            let _ = queue
                .update_status(job_id, JobStatus::Published, Some(&msg), None)
                .await;
            info!("content_queue_drain: job {job_id} [{platform}] → Published");
            published += 1;
        } else {
            let _ = queue
                .update_status(job_id, JobStatus::Failed, None, Some(&msg))
                .await;
            warn!("content_queue_drain: job {job_id} [{platform}] → Failed: {msg}");
            failed += 1;
        }
    }

    let summary = format!(
        "content_queue_drain: {total} job(s) processed — {published} published, {failed} failed"
    );
    info!("{summary}");
    (true, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn drain_empty_queue_returns_ok() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        // Open to initialise schema.
        ContentQueue::new(std::path::Path::new(path)).unwrap();
        let (ok, msg) = execute_content_queue_drain(path).await;
        assert!(ok, "empty drain should succeed");
        assert!(msg.contains("0 jobs ready"), "unexpected msg: {msg}");
    }

    #[tokio::test]
    async fn drain_bad_path_returns_false() {
        let (ok, msg) = execute_content_queue_drain("/nonexistent/path/queue.db").await;
        assert!(!ok, "bad path should return false");
        assert!(msg.contains("failed to open queue"), "unexpected msg: {msg}");
    }

    #[tokio::test]
    async fn drain_serializes_correctly() {
        // Verify ContentQueueDrain roundtrips through serde (scheduler wiring).
        use crate::scheduler::TaskType;
        let drain = TaskType::ContentQueueDrain {
            db_path: "/tmp/test.db".to_string(),
        };
        let json = serde_json::to_string(&drain).unwrap();
        assert!(json.contains("content_queue_drain"), "variant tag: {json}");
        assert!(json.contains("/tmp/test.db"), "db_path: {json}");
        let roundtrip: TaskType = serde_json::from_str(&json).unwrap();
        match roundtrip {
            TaskType::ContentQueueDrain { db_path } => assert_eq!(db_path, "/tmp/test.db"),
            _ => panic!("expected ContentQueueDrain"),
        }
    }
}
