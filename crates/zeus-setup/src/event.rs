//! Event system for async operation progress reporting

use std::time::Duration;
use tokio::sync::mpsc;

/// Progress events sent from async operations to the TUI
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// A named step started
    StepStarted {
        name: String,
        index: usize,
        total: usize,
    },
    /// A step completed successfully
    StepCompleted { name: String, message: String },
    /// A step failed
    StepFailed { name: String, error: String },
    /// A step emitted a warning
    StepWarning { name: String, message: String },
    /// Log line from a subprocess
    LogLine(String),
    /// Progress update (0..100)
    Progress { percent: u8, message: String },
    /// All operations complete
    Finished {
        success: bool,
        elapsed: Duration,
        summary: String,
    },
    /// Deploy node result (for summary table)
    DeployResult {
        host: String,
        ip: String,
        os: String,
        status: String,
        duration: Duration,
    },
    /// Doctor check result
    DoctorCheck {
        name: String,
        ok: bool,
        detail: String,
    },
    /// Doctor repair action taken
    DoctorRepair {
        name: String,
        success: bool,
        detail: String,
    },
}

/// Create a progress channel pair
pub fn progress_channel() -> (mpsc::Sender<ProgressEvent>, mpsc::Receiver<ProgressEvent>) {
    mpsc::channel(256)
}
