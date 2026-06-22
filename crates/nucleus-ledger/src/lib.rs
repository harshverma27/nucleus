//! The Nucleus project ledger.
//!
//! Nucleus treats every project as a tracked repository. The ledger lives under
//! `.nucleus/` and content-addresses each **version** — the SHA-256 of
//! (`stm32.toml` + resolved config + firmware binary + toolchain identity) — and
//! records, per version, the verifier verdict, the solved assignment (for M4
//! auto-routed configs), and the per-backend test results.
//!
//! Two properties define the store:
//!
//! - **Content-addressing.** Re-recording an unchanged version is recognized as
//!   the *same* version; no duplicate entry is created. The hash is computed
//!   from the four identity inputs only — never from the test results — so a
//!   build (which has no results yet) and a later test run land on the same
//!   version.
//! - **Append-only identity.** A version's identity fields are written once and
//!   never modified. Test results are the one mutable facet: they are *outcomes*
//!   attached to an immutable identity, and a re-run replaces the prior result
//!   for the same `(name, backend)` pair via [`Ledger::record_tests`].
//!
//! On disk:
//!
//! ```text
//! .nucleus/
//!   ├── versions.json       # { "versions": [ { hash, timestamp, ..., tests } ] }
//!   └── artifacts/          # <hash>/firmware.bin, <hash>/nucleus_config.h, ...
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod trend;
pub use trend::{TrendData, TrendPoint, VerificationReport, REPORT_SCHEMA, TREND_SCHEMA};

/// Directory under the project root that holds the ledger.
pub const LEDGER_DIR: &str = ".nucleus";
/// File (inside [`LEDGER_DIR`]) that holds the version list.
pub const VERSIONS_FILE: &str = "versions.json";
/// Sub-directory (inside [`LEDGER_DIR`]) that holds per-version artifacts.
pub const ARTIFACTS_DIR: &str = "artifacts";
/// Number of leading hex characters used as a version's short id.
pub const SHORT_HASH_LEN: usize = 7;

/// The verifier's verdict for a version: either the config was approved
/// (conflict-free) or it carried conflicts, stored as their rendered strings so
/// the ledger stays decoupled from the compiler's `Conflict` type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", content = "conflicts", rename_all = "snake_case")]
pub enum Verdict {
    /// No error-level conflicts; the config is buildable.
    Approved,
    /// Error-level conflicts blocked the config; the rendered messages.
    Conflicts(Vec<String>),
}

/// One assertion's outcome on one backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRecord {
    /// The `[[test]]` block name.
    pub name: String,
    /// The backend the assertion ran on: `"qemu"` or `"hardware"`.
    pub backend: String,
    /// `"pass"`, `"fail"`, or `"skip"`.
    pub status: String,
    /// Human-readable detail (the runner's `detail` string).
    pub detail: String,
}

/// One tracked project version: an immutable identity plus its (mutable) test
/// results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Full lowercase-hex SHA-256 of the four identity inputs.
    pub hash: String,
    /// Unix epoch seconds when this version was first recorded. Not part of the
    /// hash, so it never affects identity.
    pub timestamp: u64,
    /// Device family (e.g. `STM32F446RE`), for display and grouping.
    pub family: String,
    /// Toolchain identity string (e.g. the `arm-none-eabi-gcc` version line).
    pub toolchain: String,
    /// The raw `stm32.toml` text that produced this version.
    pub stm32_toml: String,
    /// The resolved/auto-routed config text, when it differs from `stm32_toml`
    /// (M4). `None` when the config was already fully specified.
    pub solved_config: Option<String>,
    /// The verifier verdict.
    pub verdict: Verdict,
    /// Per-backend assertion outcomes, accumulated across test runs.
    pub tests: Vec<TestRecord>,
}

impl Version {
    /// The short id: the first [`SHORT_HASH_LEN`] hex characters of [`hash`].
    ///
    /// [`hash`]: Version::hash
    pub fn short(&self) -> &str {
        &self.hash[..SHORT_HASH_LEN.min(self.hash.len())]
    }
}

/// The four inputs that define a version's identity. Borrowed so the caller
/// keeps ownership of the (potentially large) firmware bytes.
pub struct VersionInputs<'a> {
    /// The raw `stm32.toml` text.
    pub stm32_toml: &'a str,
    /// The resolved config text. When the config was not auto-routed this is the
    /// same text as `stm32_toml`; it is always hashed so the identity is stable
    /// regardless of how the resolved form was obtained.
    pub resolved_config: &'a str,
    /// The release firmware binary bytes.
    pub firmware_bin: &'a [u8],
    /// The toolchain identity string.
    pub toolchain: &'a str,
}

/// Deterministic content hash of a version's identity inputs.
///
/// Each field is length-prefixed (8-byte little-endian length, then bytes) so
/// no concatenation can be ambiguous — two different field splits can never
/// produce the same byte stream. The result is lowercase hex.
pub fn compute_hash(inputs: &VersionInputs) -> String {
    let mut hasher = Sha256::new();
    for field in [
        inputs.stm32_toml.as_bytes(),
        inputs.resolved_config.as_bytes(),
        inputs.firmware_bin,
        inputs.toolchain.as_bytes(),
    ] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Why a short-hash lookup failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// No version's hash starts with the query.
    NotFound(String),
    /// More than one version's hash starts with the query; the matching short
    /// ids are returned so the caller can ask the user to disambiguate.
    Ambiguous(String, Vec<String>),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::NotFound(q) => write!(f, "no version matches {q:?}"),
            ResolveError::Ambiguous(q, matches) => {
                write!(f, "{q:?} is ambiguous: matches {}", matches.join(", "))
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// The on-disk ledger: an append-only list of versions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ledger {
    /// Versions in the order they were first recorded.
    #[serde(default)]
    pub versions: Vec<Version>,
}

impl Ledger {
    /// Load the ledger rooted at `project_root`. A missing store is not an
    /// error — it yields an empty ledger so first-run callers need no special
    /// case.
    pub fn load(project_root: &Path) -> std::io::Result<Ledger> {
        let path = Self::versions_path(project_root);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
            Err(e) => Err(e),
        }
    }

    /// Persist the ledger under `project_root/.nucleus/versions.json`, creating
    /// the directory if needed. JSON is pretty-printed so the file diffs cleanly
    /// when a user opts to commit `.nucleus/`.
    pub fn save(&self, project_root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(project_root.join(LEDGER_DIR))?;
        let mut json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        json.push('\n');
        std::fs::write(Self::versions_path(project_root), json)
    }

    /// Path to the versions file under a project root.
    pub fn versions_path(project_root: &Path) -> PathBuf {
        project_root.join(LEDGER_DIR).join(VERSIONS_FILE)
    }

    /// Whether a version with this full hash is already recorded.
    pub fn contains(&self, hash: &str) -> bool {
        self.versions.iter().any(|v| v.hash == hash)
    }

    /// Content-addressed insert. Computes the identity hash, and:
    ///
    /// - if a version with that hash already exists, leaves its immutable
    ///   identity untouched and returns `(hash, false)`;
    /// - otherwise appends a new version (with empty test results) and returns
    ///   `(hash, true)`.
    ///
    /// `timestamp` is the unix-epoch-seconds stamp to use for a *new* version
    /// (ignored for an existing one, preserving append-only identity).
    pub fn record(
        &mut self,
        inputs: &VersionInputs,
        family: &str,
        verdict: Verdict,
        timestamp: u64,
    ) -> (String, bool) {
        let hash = compute_hash(inputs);
        if self.contains(&hash) {
            return (hash, false);
        }
        let solved_config = if inputs.resolved_config == inputs.stm32_toml {
            None
        } else {
            Some(inputs.resolved_config.to_string())
        };
        self.versions.push(Version {
            hash: hash.clone(),
            timestamp,
            family: family.to_string(),
            toolchain: inputs.toolchain.to_string(),
            stm32_toml: inputs.stm32_toml.to_string(),
            solved_config,
            verdict,
            tests: Vec::new(),
        });
        (hash, true)
    }

    /// Attach test outcomes to the version with this full hash, replacing any
    /// prior outcome for the same `(name, backend)` pair so a re-run reflects
    /// the latest result. Returns `false` if no such version exists.
    ///
    /// This is the one operation that mutates a version after creation — and it
    /// touches only the results, never the identity.
    pub fn record_tests(&mut self, hash: &str, records: Vec<TestRecord>) -> bool {
        let Some(version) = self.versions.iter_mut().find(|v| v.hash == hash) else {
            return false;
        };
        for record in records {
            if let Some(existing) = version
                .tests
                .iter_mut()
                .find(|t| t.name == record.name && t.backend == record.backend)
            {
                *existing = record;
            } else {
                version.tests.push(record);
            }
        }
        true
    }

    /// Resolve a (possibly short) hash query to a single version. Matches any
    /// version whose full hash starts with `query`.
    pub fn find(&self, query: &str) -> Result<&Version, ResolveError> {
        let matches: Vec<&Version> = self
            .versions
            .iter()
            .filter(|v| v.hash.starts_with(query))
            .collect();
        match matches.as_slice() {
            [] => Err(ResolveError::NotFound(query.to_string())),
            [one] => Ok(one),
            many => Err(ResolveError::Ambiguous(
                query.to_string(),
                many.iter().map(|v| v.short().to_string()).collect(),
            )),
        }
    }

    /// Per-backend pass/total summary for a version, e.g.
    /// `{ "qemu": (3, 3), "hardware": (2, 3) }`. A `skip` counts toward neither
    /// pass nor total, matching `nucleus test`'s "skip is not a failure" rule.
    /// The map is ordered for deterministic display.
    pub fn test_summary(version: &Version) -> BTreeMap<String, (usize, usize)> {
        let mut summary: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for record in &version.tests {
            match record.status.as_str() {
                "skip" => {
                    summary.entry(record.backend.clone()).or_default();
                }
                "pass" => {
                    let entry = summary.entry(record.backend.clone()).or_default();
                    entry.0 += 1;
                    entry.1 += 1;
                }
                _ => {
                    let entry = summary.entry(record.backend.clone()).or_default();
                    entry.1 += 1;
                }
            }
        }
        summary
    }
}

/// Write per-version artifacts under `project_root/.nucleus/artifacts/<hash>/`.
/// `files` is a list of `(filename, bytes)`. Existing files are overwritten
/// (re-recording the same version re-writes identical bytes — harmless).
pub fn write_artifacts(
    project_root: &Path,
    hash: &str,
    files: &[(&str, &[u8])],
) -> std::io::Result<()> {
    let dir = project_root.join(LEDGER_DIR).join(ARTIFACTS_DIR).join(hash);
    std::fs::create_dir_all(&dir)?;
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(toml: &'a str, bin: &'a [u8]) -> VersionInputs<'a> {
        VersionInputs {
            stm32_toml: toml,
            resolved_config: toml,
            firmware_bin: bin,
            toolchain: "arm-none-eabi-gcc 13.2.0",
        }
    }

    #[test]
    fn hash_is_deterministic() {
        let a = compute_hash(&inputs("x", b"bin"));
        let b = compute_hash(&inputs("x", b"bin"));
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // sha-256 hex
    }

    #[test]
    fn hash_changes_with_each_input() {
        let base = compute_hash(&inputs("toml", b"bin"));
        assert_ne!(base, compute_hash(&inputs("TOML", b"bin")));
        assert_ne!(base, compute_hash(&inputs("toml", b"BIN")));
        let mut diff_toolchain = inputs("toml", b"bin");
        diff_toolchain.toolchain = "gcc 14";
        assert_ne!(base, compute_hash(&diff_toolchain));
        let mut diff_resolved = inputs("toml", b"bin");
        diff_resolved.resolved_config = "resolved";
        assert_ne!(base, compute_hash(&diff_resolved));
    }

    #[test]
    fn length_prefix_disambiguates_boundaries() {
        // Without length-prefixing, ("ab","c") and ("a","bc") would collide.
        let mut left = inputs("ab", b"");
        left.resolved_config = "c";
        let mut right = inputs("a", b"");
        right.resolved_config = "bc";
        assert_ne!(compute_hash(&left), compute_hash(&right));
    }

    #[test]
    fn record_is_content_addressed_no_duplicates() {
        let mut ledger = Ledger::default();
        let (h1, new1) = ledger.record(&inputs("t", b"b"), "STM32F446RE", Verdict::Approved, 100);
        assert!(new1);
        let (h2, new2) = ledger.record(&inputs("t", b"b"), "STM32F446RE", Verdict::Approved, 200);
        assert!(!new2, "re-recording unchanged version must not duplicate");
        assert_eq!(h1, h2);
        assert_eq!(ledger.versions.len(), 1);
        // Timestamp of the original is preserved (append-only identity).
        assert_eq!(ledger.versions[0].timestamp, 100);
    }

    #[test]
    fn changed_config_appends_new_version() {
        let mut ledger = Ledger::default();
        ledger.record(&inputs("a", b"b"), "STM32F446RE", Verdict::Approved, 1);
        ledger.record(&inputs("b", b"b"), "STM32F446RE", Verdict::Approved, 2);
        assert_eq!(ledger.versions.len(), 2);
    }

    #[test]
    fn solved_config_only_stored_when_it_differs() {
        let mut ledger = Ledger::default();
        ledger.record(&inputs("same", b"b"), "STM32F446RE", Verdict::Approved, 1);
        assert_eq!(ledger.versions[0].solved_config, None);

        let routed = VersionInputs {
            stm32_toml: "intent",
            resolved_config: "routed",
            firmware_bin: b"b",
            toolchain: "tc",
        };
        ledger.record(&routed, "STM32F446RE", Verdict::Approved, 2);
        assert_eq!(ledger.versions[1].solved_config.as_deref(), Some("routed"));
    }

    #[test]
    fn record_tests_replaces_same_name_backend() {
        let mut ledger = Ledger::default();
        let (hash, _) = ledger.record(&inputs("t", b"b"), "STM32F446RE", Verdict::Approved, 1);
        assert!(ledger.record_tests(
            &hash,
            vec![TestRecord {
                name: "echo".into(),
                backend: "qemu".into(),
                status: "fail".into(),
                detail: "no reply".into(),
            }]
        ));
        // Re-run: same (name, backend) now passes -> replace, not append.
        ledger.record_tests(
            &hash,
            vec![TestRecord {
                name: "echo".into(),
                backend: "qemu".into(),
                status: "pass".into(),
                detail: "ok".into(),
            }],
        );
        assert_eq!(ledger.versions[0].tests.len(), 1);
        assert_eq!(ledger.versions[0].tests[0].status, "pass");
    }

    #[test]
    fn record_tests_unknown_hash_is_false() {
        let mut ledger = Ledger::default();
        assert!(!ledger.record_tests("deadbeef", vec![]));
    }

    #[test]
    fn find_resolves_short_hash_and_detects_ambiguity() {
        let mut ledger = Ledger::default();
        ledger.versions.push(Version {
            hash: "abc111".into(),
            timestamp: 0,
            family: "F4".into(),
            toolchain: "tc".into(),
            stm32_toml: String::new(),
            solved_config: None,
            verdict: Verdict::Approved,
            tests: vec![],
        });
        ledger.versions.push(Version {
            hash: "abc222".into(),
            timestamp: 0,
            family: "F4".into(),
            toolchain: "tc".into(),
            stm32_toml: String::new(),
            solved_config: None,
            verdict: Verdict::Approved,
            tests: vec![],
        });
        assert_eq!(ledger.find("abc111").unwrap().hash, "abc111");
        assert!(matches!(
            ledger.find("abc"),
            Err(ResolveError::Ambiguous(..))
        ));
        assert!(matches!(ledger.find("zzz"), Err(ResolveError::NotFound(_))));
    }

    #[test]
    fn test_summary_counts_pass_over_total_skips_excluded() {
        let version = Version {
            hash: "h".into(),
            timestamp: 0,
            family: "F4".into(),
            toolchain: "tc".into(),
            stm32_toml: String::new(),
            solved_config: None,
            verdict: Verdict::Approved,
            tests: vec![
                rec("a", "qemu", "pass"),
                rec("b", "qemu", "fail"),
                rec("c", "qemu", "pass"),
                rec("d", "hardware", "pass"),
                rec("e", "hardware", "skip"),
            ],
        };
        let summary = Ledger::test_summary(&version);
        assert_eq!(summary["qemu"], (2, 3));
        assert_eq!(summary["hardware"], (1, 1)); // skip excluded from total
    }

    fn rec(name: &str, backend: &str, status: &str) -> TestRecord {
        TestRecord {
            name: name.into(),
            backend: backend.into(),
            status: status.into(),
            detail: String::new(),
        }
    }

    #[test]
    fn verdict_conflicts_round_trips_through_json() {
        let v = Verdict::Conflicts(vec![
            "TIM2 clock too fast".into(),
            "PA5 double-booked".into(),
        ]);
        let json = serde_json::to_string(&v).unwrap();
        // Adjacently tagged: a Vec lives under "conflicts".
        assert!(json.contains("\"verdict\":\"conflicts\""), "{json}");
        assert_eq!(serde_json::from_str::<Verdict>(&json).unwrap(), v);
        // Approved stays a bare tag.
        let a = serde_json::to_string(&Verdict::Approved).unwrap();
        assert_eq!(a, "{\"verdict\":\"approved\"}");
    }

    #[test]
    fn load_missing_is_empty_ledger() {
        let dir = tempdir();
        let ledger = Ledger::load(&dir).unwrap();
        assert!(ledger.versions.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir();
        let mut ledger = Ledger::default();
        let (hash, _) = ledger.record(&inputs("t", b"b"), "STM32F446RE", Verdict::Approved, 42);
        ledger.record_tests(&hash, vec![rec("echo", "qemu", "pass")]);
        ledger.save(&dir).unwrap();

        let loaded = Ledger::load(&dir).unwrap();
        assert_eq!(loaded.versions, ledger.versions);
        // File actually lives where we promised.
        assert!(Ledger::versions_path(&dir).exists());
        cleanup(&dir);
    }

    #[test]
    fn write_artifacts_lands_under_hash_dir() {
        let dir = tempdir();
        write_artifacts(
            &dir,
            "abc123",
            &[("firmware.bin", b"\x00\x01"), ("config.h", b"#define")],
        )
        .unwrap();
        let base = dir.join(LEDGER_DIR).join(ARTIFACTS_DIR).join("abc123");
        assert_eq!(
            std::fs::read(base.join("firmware.bin")).unwrap(),
            b"\x00\x01"
        );
        assert!(base.join("config.h").exists());
        cleanup(&dir);
    }

    // Minimal unique temp dir without pulling in a dev-dependency.
    fn tempdir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("nucleus-ledger-test-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
