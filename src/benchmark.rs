//! `zeus benchmark` — Run task benchmarks and score pass/fail.
//!
//! Reads TOML task files from `~/.zeus/benchmarks/` or falls back to 3 hardcoded tasks.
//! Each task is run through Agent.run(), scored against expected output, and summarized.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;
use zeus_core::Config;
use zeus_llm::LlmClient;
use zeus_memory::Workspace;
use zeus_prometheus::BenchmarkStore;
use zeus_session::Session;

/// A single benchmark task definition (TOML format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTask {
    /// Human-readable task name
    pub name: String,
    /// The prompt/task to send to the agent
    pub task: String,
    /// Expected substring or keyword in the output (case-insensitive match)
    pub expected: String,
    /// Timeout in seconds (default: 60)
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    60
}

/// Result of running a single benchmark task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub task_name: String,
    pub passed: bool,
    pub score: f64,
    pub duration_ms: u64,
    pub tokens: u64,
    pub output: String,
    pub expected: String,
    pub timestamp: String,
}

/// File containing multiple benchmark tasks.
#[derive(Debug, Deserialize)]
struct BenchmarkFile {
    #[serde(rename = "task")]
    tasks: Vec<BenchmarkTask>,
}

/// Load tasks from `~/.zeus/benchmarks/` directory.
/// Each `.toml` file can contain one or more `[[task]]` entries.
fn load_tasks_from_dir(dir: &PathBuf) -> Result<Vec<BenchmarkTask>> {
    let mut tasks = Vec::new();

    if !dir.exists() {
        return Ok(tasks);
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            let content = std::fs::read_to_string(&path)?;
            match toml::from_str::<BenchmarkFile>(&content) {
                Ok(file) => tasks.extend(file.tasks),
                Err(e) => {
                    eprintln!("  ⚠ Failed to parse {}: {}", path.display(), e);
                }
            }
        }
    }

    Ok(tasks)
}

/// Returns 3 hardcoded test tasks as a baseline.
fn default_tasks() -> Vec<BenchmarkTask> {
    vec![
        BenchmarkTask {
            name: "arithmetic".to_string(),
            task: "What is 7 * 8? Reply with just the number.".to_string(),
            expected: "56".to_string(),
            timeout: 30,
        },
        BenchmarkTask {
            name: "file_creation".to_string(),
            task: "Create a file at /tmp/zeus_bench_test.txt containing 'hello benchmark'. Then read it back and confirm the contents.".to_string(),
            expected: "hello benchmark".to_string(),
            timeout: 60,
        },
        BenchmarkTask {
            name: "reasoning".to_string(),
            task: "If a train travels 120 miles in 2 hours, what is its average speed in mph? Reply with just the number.".to_string(),
            expected: "60".to_string(),
            timeout: 30,
        },
    ]
}

/// Score an agent's output against the expected string.
/// Returns (passed, score) where score is 0.0 or 1.0 for now.
fn score_output(output: &str, expected: &str) -> (bool, f64) {
    let output_lower = output.to_lowercase();
    let expected_lower = expected.to_lowercase();

    // Check if expected substring is present in output
    let passed = output_lower.contains(&expected_lower);
    let score = if passed { 1.0 } else { 0.0 };

    (passed, score)
}

/// Run all benchmark tasks and print results.
pub async fn run_benchmark(config: Config) -> Result<()> {
    println!("Zeus Benchmark Runner");
    println!("=====================\n");

    // Load tasks
    let benchmarks_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("benchmarks");

    let tasks = {
        let loaded = load_tasks_from_dir(&benchmarks_dir)?;
        if loaded.is_empty() {
            println!("No task files found in {}.", benchmarks_dir.display());
            println!("Using 3 built-in test tasks.\n");
            // Create the benchmarks dir and write a sample file for next time
            std::fs::create_dir_all(&benchmarks_dir).ok();
            let sample = r#"# Zeus Benchmark Tasks
# Add [[task]] entries to define benchmarks.

[[task]]
name = "arithmetic"
task = "What is 7 * 8? Reply with just the number."
expected = "56"
timeout = 30

[[task]]
name = "file_creation"
task = "Create a file at /tmp/zeus_bench_test.txt containing 'hello benchmark'. Then read it back and confirm the contents."
expected = "hello benchmark"
timeout = 60

[[task]]
name = "reasoning"
task = "If a train travels 120 miles in 2 hours, what is its average speed in mph? Reply with just the number."
expected = "60"
timeout = 30
"#;
            std::fs::write(benchmarks_dir.join("default.toml"), sample).ok();
            default_tasks()
        } else {
            println!(
                "Loaded {} task(s) from {}\n",
                loaded.len(),
                benchmarks_dir.display()
            );
            loaded
        }
    };

    let (provider, model) = config.parse_model();
    println!("Model: {:?}/{}", provider, model);
    println!("Tasks: {}\n", tasks.len());

    let llm = LlmClient::from_config(&config)?;
    let workspace = Workspace::from_config(&config);
    workspace.init().await?;

    let mut results: Vec<BenchmarkResult> = Vec::new();
    let mut passed_count = 0;
    let total = tasks.len();

    for (i, task) in tasks.iter().enumerate() {
        print!("[{}/{}] {} ... ", i + 1, total, task.name);

        // Create a fresh session for each benchmark task
        let session = Session::new(&config.sessions);

        let mut agent =
            zeus_agent::Agent::new(config.clone(), llm.clone(), workspace.clone(), session, None);

        let start = Instant::now();

        // Run with timeout
        let run_result = tokio::time::timeout(
            std::time::Duration::from_secs(task.timeout),
            agent.run(&task.task),
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        let (output, timed_out) = match run_result {
            Ok(Ok(output)) => (output, false),
            Ok(Err(e)) => (format!("ERROR: {}", e), false),
            Err(_) => (format!("TIMEOUT after {}s", task.timeout), true),
        };

        let (passed, score) = if timed_out {
            (false, 0.0)
        } else {
            score_output(&output, &task.expected)
        };

        if passed {
            passed_count += 1;
            println!("✅ PASS ({:.1}s)", duration_ms as f64 / 1000.0);
        } else {
            println!("❌ FAIL ({:.1}s)", duration_ms as f64 / 1000.0);
            // Show first 200 chars of output for debugging
            let preview = if output.len() > 200 {
                format!("{}...", &output[..zeus_core::floor_char_boundary(&output, 200)])
            } else {
                output.clone()
            };
            println!("       Expected: \"{}\"", task.expected);
            println!("       Got: \"{}\"", preview.replace('\n', " "));
        }

        let result = BenchmarkResult {
            task_name: task.name.clone(),
            passed,
            score,
            duration_ms,
            tokens: 0, // TODO: wire token counting from LLM response
            output: output.clone(),
            expected: task.expected.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        results.push(result);
    }

    // Summary
    println!("\n─────────────────────────");
    println!(
        "Result: {}/{} passed ({}%)",
        passed_count,
        total,
        if total > 0 {
            (passed_count * 100) / total
        } else {
            0
        }
    );

    let total_duration: u64 = results.iter().map(|r| r.duration_ms).sum();
    println!("Total time: {:.1}s", total_duration as f64 / 1000.0);

    // Persist results to SQLite via zeus-prometheus BenchmarkStore
    let run_id = format!("run-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    match BenchmarkStore::open_default() {
        Ok(store) => {
            let prometheus_results: Vec<zeus_prometheus::BenchmarkResult> = results
                .iter()
                .map(|r| zeus_prometheus::BenchmarkResult {
                    task_name: r.task_name.clone(),
                    passed: r.passed,
                    score: r.score,
                    duration_ms: r.duration_ms,
                    tokens: r.tokens,
                    timestamp: chrono::Utc::now(),
                    metadata: Some(serde_json::json!({
                        "expected": r.expected,
                        "output_preview": if r.output.len() > 500 { &r.output[..zeus_core::floor_char_boundary(&r.output, 500)] } else { &r.output },
                    }).to_string()),
                })
                .collect();

            match store.store_run(&run_id, &prometheus_results) {
                Ok(_) => {
                    println!("\nResults persisted: {} ({} tasks)", run_id, prometheus_results.len());

                    // Show comparison with previous run if one exists
                    if let Ok(runs) = store.list_runs() {
                        if runs.len() >= 2 {
                            let prev_id = &runs[1]; // runs[0] is current
                            if let Ok(cmp) = store.compare_runs(prev_id, &run_id) {
                                let delta = cmp.pass_rate_delta * 100.0;
                                let arrow = if delta > 0.0 { "↑" } else if delta < 0.0 { "↓" } else { "=" };
                                println!("  vs {}: {}{:.1}% pass rate", prev_id, arrow, delta.abs());
                                if !cmp.regressions.is_empty() {
                                    println!("  ⚠ Regressions: {}", cmp.regressions.join(", "));
                                }
                                if !cmp.improvements.is_empty() {
                                    println!("  ✓ Improvements: {}", cmp.improvements.join(", "));
                                }
                            }
                        }
                    }
                }
                Err(e) => eprintln!("  ⚠ Failed to persist results: {}", e),
            }
        }
        Err(e) => eprintln!("  ⚠ Could not open benchmark store: {}", e),
    }

    // Write results JSON as backup
    let results_path = benchmarks_dir.join("last_run.json");
    if let Ok(json) = serde_json::to_string_pretty(&results) {
        std::fs::write(&results_path, &json).ok();
        println!("JSON backup: {}", results_path.display());
    }

    // Clean up bench temp files
    std::fs::remove_file("/tmp/zeus_bench_test.txt").ok();

    if passed_count == total {
        println!("\n🎯 All benchmarks passed!");
    }

    Ok(())
}
