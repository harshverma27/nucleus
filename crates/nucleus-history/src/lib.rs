//! Nucleus test history.
//!
//! A deliberately small record of every `nucleus test` run. `nucleus test`
//! creates `tests/test_history.json` on the first run and appends one [`Run`]
//! on every run afterwards. Each run stores its timestamp and one [`TestEntry`]
//! per assertion-on-a-backend (name, backend, status, detail) — extensible: add
//! more per-test fields here as needed.
//!
//! There is no hashing, no content-addressing, no artifacts — just an
//! append-on-run JSON file that travels with the repo. The dashboard's history
//! graph reads the per-run summary ([`TestHistory::summary`]); `nucleus history`
//! and `nucleus show` read the runs directly.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Directory (under the project root) that holds the history file.
pub const HISTORY_DIR: &str = "tests";
/// History file name (inside [`HISTORY_DIR`]).
pub const HISTORY_FILE: &str = "test_history.json";

/// One assertion's outcome on one backend within a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestEntry {
    /// The `[[test]]` block name.
    pub name: String,
    /// Backend the assertion ran on: `"qemu"` or `"hardware"`.
    pub backend: String,
    /// `"pass"`, `"fail"`, or `"skip"`.
    pub status: String,
    /// Human-readable detail (the runner's `detail` string).
    pub detail: String,
}

/// One `nucleus test` invocation: when it ran and what each assertion did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Unix epoch seconds when the run started.
    pub timestamp: u64,
    /// One entry per assertion-on-a-backend.
    pub tests: Vec<TestEntry>,
}

impl Run {
    /// Assertions with the given status in this run.
    fn count(&self, status: &str) -> usize {
        self.tests.iter().filter(|t| t.status == status).count()
    }
    /// Number of passing assertions.
    pub fn passed(&self) -> usize {
        self.count("pass")
    }
    /// Number of failing assertions.
    pub fn failed(&self) -> usize {
        self.count("fail")
    }
    /// Number of skipped assertions.
    pub fn skipped(&self) -> usize {
        self.tests.len() - self.passed() - self.failed()
    }
}

/// The whole history: runs in the order they happened (oldest first).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestHistory {
    #[serde(default)]
    pub runs: Vec<Run>,
}

/// Per-run pass/fail/skip tally — the shape the dashboard graph renders, so the
/// counting stays in Rust and the TypeScript only draws bars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub timestamp: u64,
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
}

/// A list of [`RunSummary`], schema-tagged so the dashboard can version it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySummary {
    pub schema: String,
    pub runs: Vec<RunSummary>,
}

/// Schema id stamped into [`HistorySummary`].
pub const SUMMARY_SCHEMA: &str = "nucleus.history.v1";

impl TestHistory {
    /// Path to the history file under a project root.
    pub fn path(project_root: &Path) -> PathBuf {
        project_root.join(HISTORY_DIR).join(HISTORY_FILE)
    }

    /// Load the history for `project_root`. A missing file is not an error — it
    /// yields an empty history so the first run needs no special case.
    pub fn load(project_root: &Path) -> std::io::Result<TestHistory> {
        match std::fs::read_to_string(Self::path(project_root)) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TestHistory::default()),
            Err(e) => Err(e),
        }
    }

    /// Persist under `project_root/tests/test_history.json`, creating the
    /// `tests/` directory if needed. Pretty-printed so it diffs cleanly in git.
    pub fn save(&self, project_root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(project_root.join(HISTORY_DIR))?;
        let mut json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        json.push('\n');
        std::fs::write(Self::path(project_root), json)
    }

    /// Append a run.
    pub fn push(&mut self, run: Run) {
        self.runs.push(run);
    }

    /// Per-run pass/fail/skip summary for the graph. `last` keeps only the most
    /// recent N runs (the tail); `None` keeps all.
    pub fn summary(&self, last: Option<usize>) -> HistorySummary {
        let start = match last {
            Some(n) if n < self.runs.len() => self.runs.len() - n,
            _ => 0,
        };
        HistorySummary {
            schema: SUMMARY_SCHEMA.to_string(),
            runs: self.runs[start..]
                .iter()
                .map(|r| RunSummary {
                    timestamp: r.timestamp,
                    pass: r.passed(),
                    fail: r.failed(),
                    skip: r.skipped(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, backend: &str, status: &str) -> TestEntry {
        TestEntry {
            name: name.into(),
            backend: backend.into(),
            status: status.into(),
            detail: format!("{name}/{backend}"),
        }
    }

    fn run(ts: u64, entries: Vec<TestEntry>) -> Run {
        Run {
            timestamp: ts,
            tests: entries,
        }
    }

    fn tempdir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nucleus-history-test-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn run_counts_pass_fail_skip() {
        let r = run(
            1,
            vec![
                entry("a", "qemu", "pass"),
                entry("b", "qemu", "fail"),
                entry("c", "hardware", "pass"),
                entry("d", "hardware", "skip"),
            ],
        );
        assert_eq!(r.passed(), 2);
        assert_eq!(r.failed(), 1);
        assert_eq!(r.skipped(), 1);
    }

    #[test]
    fn load_missing_is_empty() {
        let dir = tempdir();
        assert!(TestHistory::load(&dir).unwrap().runs.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_creates_tests_dir_and_round_trips() {
        let dir = tempdir();
        let mut h = TestHistory::default();
        h.push(run(10, vec![entry("echo", "qemu", "pass")]));
        h.save(&dir).unwrap();

        // Lands in tests/test_history.json.
        assert!(dir.join("tests").join("test_history.json").exists());
        let back = TestHistory::load(&dir).unwrap();
        assert_eq!(back, h);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn push_appends_each_run() {
        let dir = tempdir();
        let mut h = TestHistory::load(&dir).unwrap();
        h.push(run(1, vec![entry("a", "qemu", "pass")]));
        h.save(&dir).unwrap();

        // Simulate a second `nucleus test`: load, push, save.
        let mut h2 = TestHistory::load(&dir).unwrap();
        h2.push(run(2, vec![entry("a", "qemu", "fail")]));
        h2.save(&dir).unwrap();

        let final_h = TestHistory::load(&dir).unwrap();
        assert_eq!(final_h.runs.len(), 2);
        assert_eq!(final_h.runs[0].timestamp, 1);
        assert_eq!(final_h.runs[1].tests[0].status, "fail");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn summary_counts_and_last_n() {
        let mut h = TestHistory::default();
        h.push(run(
            1,
            vec![entry("a", "qemu", "pass"), entry("b", "qemu", "fail")],
        ));
        h.push(run(2, vec![entry("a", "qemu", "pass")]));
        h.push(run(3, vec![entry("a", "qemu", "skip")]));

        let all = h.summary(None);
        assert_eq!(all.schema, SUMMARY_SCHEMA);
        assert_eq!(all.runs.len(), 3);
        assert_eq!((all.runs[0].pass, all.runs[0].fail), (1, 1));

        let last2 = h.summary(Some(2));
        assert_eq!(last2.runs.len(), 2);
        assert_eq!(last2.runs[0].timestamp, 2);
        assert_eq!(last2.runs[1].skip, 1);
    }
}
