//! Trend + verification views over the ledger (M9).
//!
//! Both are *pure, deterministic* projections of the ledger — given the same
//! `versions.json` they produce byte-identical JSON, with no wall-clock or
//! environment input. That is what lets the CI action treat them as
//! reproducible artifacts.
//!
//! - [`TrendData`] feeds the dashboard's history mode: one [`TrendPoint`] per
//!   version with per-backend pass/fail/skip counts.
//! - [`VerificationReport`] is the machine-checkable record for one version:
//!   what the verifier proved statically and what each backend proved at
//!   runtime.

use serde::{Deserialize, Serialize};

use crate::{Ledger, Verdict, Version};

/// Pass/fail/skip tally for one backend within a version. `total` excludes
/// skips, matching `nucleus test`'s "a skip is not a failure" rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCounts {
    pub backend: String,
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
}

impl BackendCounts {
    /// Assertions that actually ran (pass + fail; skips excluded).
    pub fn total(&self) -> usize {
        self.pass + self.fail
    }
}

/// One version on the history timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrendPoint {
    pub hash: String,
    pub short: String,
    pub timestamp: u64,
    pub family: String,
    /// Verifier approved (no error-level conflicts).
    pub approved: bool,
    /// Number of recorded conflicts (0 when approved).
    pub conflicts: usize,
    /// Per-backend counts, ordered by backend name for determinism.
    pub backends: Vec<BackendCounts>,
    /// Totals across all backends.
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
}

/// The full timeline: a deterministic projection of the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrendData {
    pub schema: String,
    pub points: Vec<TrendPoint>,
}

/// Schema id stamped into [`TrendData`], so consumers can version the shape.
pub const TREND_SCHEMA: &str = "nucleus.trend.v1";

fn point_for(version: &Version) -> TrendPoint {
    // Group records by backend deterministically.
    let mut by_backend: std::collections::BTreeMap<String, BackendCounts> =
        std::collections::BTreeMap::new();
    let (mut pass, mut fail, mut skip) = (0, 0, 0);
    for record in &version.tests {
        let entry = by_backend
            .entry(record.backend.clone())
            .or_insert_with(|| BackendCounts {
                backend: record.backend.clone(),
                pass: 0,
                fail: 0,
                skip: 0,
            });
        match record.status.as_str() {
            "pass" => {
                entry.pass += 1;
                pass += 1;
            }
            "skip" => {
                entry.skip += 1;
                skip += 1;
            }
            _ => {
                entry.fail += 1;
                fail += 1;
            }
        }
    }
    let (approved, conflicts) = match &version.verdict {
        Verdict::Approved => (true, 0),
        Verdict::Conflicts(c) => (false, c.len()),
    };
    TrendPoint {
        hash: version.hash.clone(),
        short: version.short().to_string(),
        timestamp: version.timestamp,
        family: version.family.clone(),
        approved,
        conflicts,
        backends: by_backend.into_values().collect(),
        pass,
        fail,
        skip,
    }
}

impl Ledger {
    /// Build the trend timeline. `last` keeps only the most recent N versions
    /// (the tail, since versions are stored oldest-first); `None` keeps all.
    pub fn trend(&self, last: Option<usize>) -> TrendData {
        let start = match last {
            Some(n) if n < self.versions.len() => self.versions.len() - n,
            _ => 0,
        };
        TrendData {
            schema: TREND_SCHEMA.to_string(),
            points: self.versions[start..].iter().map(point_for).collect(),
        }
    }
}

/// The verifier's static verdict, restated for the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticCheck {
    pub approved: bool,
    pub conflicts: Vec<String>,
}

/// One assertion's outcome, as it appears in a [`VerificationReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionResult {
    pub name: String,
    pub status: String,
    pub detail: String,
}

/// What one backend proved for a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendReport {
    pub backend: String,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub results: Vec<AssertionResult>,
}

/// Machine-checkable verification record for a single version: exactly what was
/// proven statically and on which backend at runtime. Deterministic — a pure
/// function of the version's ledger entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReport {
    pub schema: String,
    pub hash: String,
    pub short: String,
    pub family: String,
    pub toolchain: String,
    pub timestamp: u64,
    pub static_check: StaticCheck,
    pub backends: Vec<BackendReport>,
    /// Backends on which every assertion that ran passed *and* at least one
    /// ran — i.e. the backends that actually proved something. A backend with
    /// only skips, or any failure, is excluded.
    pub proven_on: Vec<String>,
}

/// Schema id stamped into [`VerificationReport`].
pub const REPORT_SCHEMA: &str = "nucleus.verification.v1";

impl VerificationReport {
    /// Build the report for one recorded version.
    pub fn for_version(version: &Version) -> VerificationReport {
        let mut by_backend: std::collections::BTreeMap<String, BackendReport> =
            std::collections::BTreeMap::new();
        for record in &version.tests {
            let entry = by_backend
                .entry(record.backend.clone())
                .or_insert_with(|| BackendReport {
                    backend: record.backend.clone(),
                    passed: 0,
                    failed: 0,
                    skipped: 0,
                    results: Vec::new(),
                });
            match record.status.as_str() {
                "pass" => entry.passed += 1,
                "skip" => entry.skipped += 1,
                _ => entry.failed += 1,
            }
            entry.results.push(AssertionResult {
                name: record.name.clone(),
                status: record.status.clone(),
                detail: record.detail.clone(),
            });
        }

        let proven_on: Vec<String> = by_backend
            .values()
            .filter(|b| b.failed == 0 && b.passed > 0)
            .map(|b| b.backend.clone())
            .collect();

        let static_check = match &version.verdict {
            Verdict::Approved => StaticCheck {
                approved: true,
                conflicts: Vec::new(),
            },
            Verdict::Conflicts(c) => StaticCheck {
                approved: false,
                conflicts: c.clone(),
            },
        };

        VerificationReport {
            schema: REPORT_SCHEMA.to_string(),
            hash: version.hash.clone(),
            short: version.short().to_string(),
            family: version.family.clone(),
            toolchain: version.toolchain.clone(),
            timestamp: version.timestamp,
            static_check,
            backends: by_backend.into_values().collect(),
            proven_on,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TestRecord, Verdict, Version};

    fn version(hash: &str, ts: u64, verdict: Verdict, tests: Vec<TestRecord>) -> Version {
        Version {
            hash: hash.to_string(),
            timestamp: ts,
            family: "STM32F446RE".into(),
            toolchain: "gcc 14".into(),
            stm32_toml: String::new(),
            solved_config: None,
            verdict,
            tests,
        }
    }

    fn rec(name: &str, backend: &str, status: &str) -> TestRecord {
        TestRecord {
            name: name.into(),
            backend: backend.into(),
            status: status.into(),
            detail: format!("{name}/{backend}"),
        }
    }

    #[test]
    fn trend_point_counts_per_backend_and_totals() {
        let v = version(
            &"a".repeat(64),
            10,
            Verdict::Approved,
            vec![
                rec("x", "qemu", "pass"),
                rec("y", "qemu", "fail"),
                rec("z", "hardware", "pass"),
                rec("w", "hardware", "skip"),
            ],
        );
        let p = point_for(&v);
        assert_eq!(p.pass, 2);
        assert_eq!(p.fail, 1);
        assert_eq!(p.skip, 1);
        // Ordered by backend name: hardware before qemu.
        assert_eq!(p.backends[0].backend, "hardware");
        assert_eq!(p.backends[0].pass, 1);
        assert_eq!(p.backends[0].skip, 1);
        assert_eq!(p.backends[0].total(), 1);
        assert_eq!(p.backends[1].backend, "qemu");
        assert_eq!(p.backends[1].total(), 2);
        assert!(p.approved);
        assert_eq!(p.conflicts, 0);
    }

    #[test]
    fn trend_point_records_conflicts() {
        let v = version(
            &"b".repeat(64),
            1,
            Verdict::Conflicts(vec!["TIM2 too fast".into(), "PA5 clash".into()]),
            vec![],
        );
        let p = point_for(&v);
        assert!(!p.approved);
        assert_eq!(p.conflicts, 2);
    }

    #[test]
    fn trend_last_n_keeps_most_recent_tail() {
        let mut ledger = Ledger::default();
        for i in 0..5 {
            ledger.versions.push(version(
                &format!("{i}{}", "0".repeat(63)),
                i,
                Verdict::Approved,
                vec![],
            ));
        }
        let all = ledger.trend(None);
        assert_eq!(all.points.len(), 5);
        assert_eq!(all.schema, TREND_SCHEMA);

        let last2 = ledger.trend(Some(2));
        assert_eq!(last2.points.len(), 2);
        assert_eq!(last2.points[0].timestamp, 3);
        assert_eq!(last2.points[1].timestamp, 4);

        // last larger than len -> all.
        assert_eq!(ledger.trend(Some(99)).points.len(), 5);
    }

    #[test]
    fn trend_data_round_trips_json() {
        let mut ledger = Ledger::default();
        ledger.versions.push(version(
            &"c".repeat(64),
            7,
            Verdict::Approved,
            vec![rec("e", "qemu", "pass")],
        ));
        let trend = ledger.trend(None);
        let json = serde_json::to_string(&trend).unwrap();
        let back: TrendData = serde_json::from_str(&json).unwrap();
        assert_eq!(back, trend);
    }

    #[test]
    fn report_marks_proven_backends() {
        let v = version(
            &"d".repeat(64),
            5,
            Verdict::Approved,
            vec![
                rec("a", "qemu", "pass"),
                rec("b", "qemu", "pass"),
                rec("c", "hardware", "fail"), // hardware not proven
                rec("d", "skiponly", "skip"), // skip-only not proven
            ],
        );
        let report = VerificationReport::for_version(&v);
        assert_eq!(report.schema, REPORT_SCHEMA);
        assert!(report.static_check.approved);
        assert_eq!(report.proven_on, vec!["qemu".to_string()]);

        // Backends ordered; counts correct.
        let qemu = report
            .backends
            .iter()
            .find(|b| b.backend == "qemu")
            .unwrap();
        assert_eq!((qemu.passed, qemu.failed, qemu.skipped), (2, 0, 0));
        let hw = report
            .backends
            .iter()
            .find(|b| b.backend == "hardware")
            .unwrap();
        assert_eq!((hw.passed, hw.failed, hw.skipped), (0, 1, 0));
    }

    #[test]
    fn report_carries_conflicts_when_not_approved() {
        let v = version(
            &"e".repeat(64),
            5,
            Verdict::Conflicts(vec!["bad clock".into()]),
            vec![],
        );
        let report = VerificationReport::for_version(&v);
        assert!(!report.static_check.approved);
        assert_eq!(report.static_check.conflicts, vec!["bad clock".to_string()]);
        assert!(report.proven_on.is_empty());
    }

    #[test]
    fn report_is_deterministic() {
        let v = version(
            &"f".repeat(64),
            5,
            Verdict::Approved,
            vec![rec("a", "qemu", "pass"), rec("b", "hardware", "pass")],
        );
        let a = serde_json::to_string(&VerificationReport::for_version(&v)).unwrap();
        let b = serde_json::to_string(&VerificationReport::for_version(&v)).unwrap();
        assert_eq!(a, b);
    }
}
