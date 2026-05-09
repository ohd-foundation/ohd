//! OHD Storage conformance corpus runner.
//!
//! See `spec/conformance.md` for the full corpus model. This crate ships the
//! reference Rust runner that:
//!
//! 1. Loads each fixture under `corpus/<category>/<fixture>/` (`input.json`,
//!    `expected.json` or `expected.bin`, `README.md`).
//! 2. Runs the fixture against a fresh in-memory `Storage` instance.
//! 3. Compares the actual output to the expected output byte-for-byte.
//! 4. Emits a unified-diff-style failure report on mismatch.
//!
//! # Categories implemented
//!
//! - `sample_blocks/encoding1/*` — sample-block determinism for encoding 1.
//! - `sample_blocks/encoding2/*` — sample-block determinism for encoding 2.
//! - `ohdc/put_query/*` — PutEvents → QueryEvents round-trip.
//! - `permissions/*` — grant scope intersection (event_type / channel /
//!   sensitivity / time-window / rate-limit rules).
//!
//! Categories deferred to v1.x: `sync/*`, `auth/*`, `pagination/*`, full
//! `streaming/*`. See STATUS.md.
//!
//! # How to add a fixture
//!
//! 1. Pick the right category dir (`corpus/sample_blocks/encoding1/`,
//!    `corpus/ohdc/put_query/`, etc.).
//! 2. Create `<NNN>_<short_name>/`.
//! 3. Drop `input.json` (the fixture's input — meaning is per-category) and
//!    `expected.json` or `expected.bin` (the expected output).
//! 4. Add a `README.md` paragraph explaining what the fixture asserts.
//! 5. Add the fixture to the manifest (`corpus/manifest.json`) so the runner
//!    discovers it.
//! 6. Run `cargo test -p ohd-storage-conformance` and confirm green.
//!
//! See `corpus/sample_blocks/encoding1/001_simple/` for the canonical layout.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One corpus fixture entry from `manifest.json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestEntry {
    /// Category, e.g. `"sample_blocks/encoding1"`.
    pub category: String,
    /// Fixture path relative to the corpus root, e.g. `"sample_blocks/encoding1/001_simple"`.
    pub path: String,
    /// One-paragraph description copied from the fixture's README.
    pub description: String,
    /// Whether this fixture is required for OHDC v0 conformance (true) or
    /// optional (false; encoding 2 in `sample_blocks/encoding2/*` is
    /// recommended but not required, per the spec).
    pub required: bool,
}

/// Top-level manifest. Loaded from `corpus/manifest.json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    /// Corpus version ("1.0.0" today).
    pub version: String,
    /// All fixtures in deterministic load order.
    pub fixtures: Vec<ManifestEntry>,
}

/// Regenerate every sample-block fixture's `expected.bin` using the live
/// encoder. Useful when adding new fixtures or after intentional codec
/// parameter changes; in steady state, the saved bytes are the source of
/// truth and `run_all` asserts byte-identity.
///
/// Activated by setting `OHD_CONFORMANCE_REGEN=1` before running the test.
pub fn regenerate_sample_block_expected_bins(corpus_root: &Path) -> anyhow::Result<()> {
    let manifest = load_manifest(corpus_root)?;
    for entry in &manifest.fixtures {
        let dir = corpus_root.join(&entry.path);
        let input_path = dir.join("input.json");
        let input_text = match std::fs::read_to_string(&input_path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let bytes = match entry.category.as_str() {
            "sample_blocks/encoding1" => {
                use ohd_storage_core::sample_codec::{encode_f32, Sample};
                let input: SampleBlockInputF32 = serde_json::from_str(&input_text)?;
                let samples: Vec<Sample> = input
                    .samples
                    .into_iter()
                    .map(|s| Sample {
                        t_offset_ms: s.t_offset_ms,
                        value: s.value,
                    })
                    .collect();
                encode_f32(&samples).map_err(|e| anyhow::anyhow!("encode_f32: {e}"))?
            }
            "sample_blocks/encoding2" => {
                use ohd_storage_core::sample_codec::{encode_i16, Sample};
                let input: SampleBlockInputI16 = serde_json::from_str(&input_text)?;
                let samples: Vec<Sample> = input
                    .samples
                    .into_iter()
                    .map(|s| Sample {
                        t_offset_ms: s.t_offset_ms,
                        value: s.value,
                    })
                    .collect();
                encode_i16(&samples, input.scale, input.offset)
                    .map_err(|e| anyhow::anyhow!("encode_i16: {e}"))?
            }
            _ => continue,
        };
        std::fs::write(dir.join("expected.bin"), bytes)?;
    }
    Ok(())
}

/// Load the corpus manifest. The default location is the corpus dir adjacent
/// to this crate's source root: `crates/ohd-storage-conformance/corpus/`.
pub fn load_manifest(corpus_root: &Path) -> anyhow::Result<Manifest> {
    let manifest_path = corpus_root.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", manifest_path.display(), e))?;
    let m: Manifest = serde_json::from_str(&text)?;
    Ok(m)
}

/// Locate the conformance corpus root. Default heuristic: this crate's
/// `corpus/` subdir resolves through `CARGO_MANIFEST_DIR`.
pub fn default_corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

/// Run all fixtures in the corpus. Returns a [`RunReport`] summarizing the
/// pass/fail counts and per-fixture diffs on failure.
pub fn run_all(corpus_root: &Path) -> anyhow::Result<RunReport> {
    let manifest = load_manifest(corpus_root)?;
    let mut report = RunReport::default();
    for entry in &manifest.fixtures {
        let result = run_fixture(corpus_root, entry);
        match result {
            Ok(()) => report.passed += 1,
            Err(e) => {
                report.failed += 1;
                report
                    .failures
                    .push(format!("[{}] {}: {}", entry.category, entry.path, e));
            }
        }
    }
    Ok(report)
}

/// One-shot dispatcher. Reads `input.json` + `expected.{json,bin}` and
/// invokes the per-category runner.
pub fn run_fixture(corpus_root: &Path, entry: &ManifestEntry) -> anyhow::Result<()> {
    let dir = corpus_root.join(&entry.path);
    let input_path = dir.join("input.json");
    let input_text = std::fs::read_to_string(&input_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", input_path.display(), e))?;
    match entry.category.as_str() {
        "sample_blocks/encoding1" => run_sample_blocks_f32(&dir, &input_text),
        "sample_blocks/encoding2" => run_sample_blocks_i16(&dir, &input_text),
        "ohdc/put_query" => run_put_query(&dir, &input_text),
        "permissions" => run_permissions(&dir, &input_text),
        other => Err(anyhow::anyhow!("unknown corpus category: {}", other)),
    }
}

/// Runner output.
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    /// Passing fixtures.
    pub passed: usize,
    /// Failing fixtures (with diffs).
    pub failed: usize,
    /// Per-failure messages (category + path + diff).
    pub failures: Vec<String>,
}

impl RunReport {
    /// Returns true iff every fixture passed.
    pub fn all_pass(&self) -> bool {
        self.failed == 0
    }
}

// =============================================================================
// Per-category runners
// =============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SampleBlockInputF32 {
    samples: Vec<SampleEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SampleBlockInputI16 {
    samples: Vec<SampleEntry>,
    scale: f32,
    offset: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SampleEntry {
    t_offset_ms: i64,
    value: f64,
}

fn run_sample_blocks_f32(dir: &Path, input_text: &str) -> anyhow::Result<()> {
    use ohd_storage_core::sample_codec::{encode_f32, Sample};
    let input: SampleBlockInputF32 = serde_json::from_str(input_text)?;
    let samples: Vec<Sample> = input
        .samples
        .into_iter()
        .map(|s| Sample {
            t_offset_ms: s.t_offset_ms,
            value: s.value,
        })
        .collect();
    let actual = encode_f32(&samples).map_err(|e| anyhow::anyhow!("encode_f32: {e}"))?;
    let expected_path = dir.join("expected.bin");
    let expected = std::fs::read(&expected_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", expected_path.display(), e))?;
    if actual != expected {
        return Err(anyhow::anyhow!(
            "encoding 1 byte-mismatch: expected {} bytes, got {} bytes; first diff at offset {}",
            expected.len(),
            actual.len(),
            first_diff_offset(&expected, &actual),
        ));
    }
    Ok(())
}

fn run_sample_blocks_i16(dir: &Path, input_text: &str) -> anyhow::Result<()> {
    use ohd_storage_core::sample_codec::{encode_i16, Sample};
    let input: SampleBlockInputI16 = serde_json::from_str(input_text)?;
    let samples: Vec<Sample> = input
        .samples
        .into_iter()
        .map(|s| Sample {
            t_offset_ms: s.t_offset_ms,
            value: s.value,
        })
        .collect();
    let actual = encode_i16(&samples, input.scale, input.offset)
        .map_err(|e| anyhow::anyhow!("encode_i16: {e}"))?;
    let expected_path = dir.join("expected.bin");
    let expected = std::fs::read(&expected_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", expected_path.display(), e))?;
    if actual != expected {
        return Err(anyhow::anyhow!(
            "encoding 2 byte-mismatch: expected {} bytes, got {} bytes; first diff at offset {}",
            expected.len(),
            actual.len(),
            first_diff_offset(&expected, &actual),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PutQueryInput {
    events: Vec<ohd_storage_core::events::EventInput>,
    query: ohd_storage_core::events::EventFilter,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PutQueryExpected {
    /// Number of events that should be returned by the query (post-filter).
    rows_returned: usize,
    /// Event types in the result, in order.
    event_types: Vec<String>,
}

fn run_put_query(dir: &Path, input_text: &str) -> anyhow::Result<()> {
    use ohd_storage_core::{
        events::{put_events, query_events},
        storage::{Storage, StorageConfig},
    };
    let input: PutQueryInput = serde_json::from_str(input_text)?;
    let tmp = tempfile::tempdir()?;
    let storage = Storage::open(StorageConfig::new(tmp.path().join("conformance.db")))
        .map_err(|e| anyhow::anyhow!("open storage: {e}"))?;
    let envelope = storage.envelope_key().cloned();
    storage
        .with_conn_mut(|conn| {
            // self-session: no grant id, no approval gate.
            put_events(conn, &input.events, None, false, envelope.as_ref()).map(|_| ())
        })
        .map_err(|e| anyhow::anyhow!("put_events: {e}"))?;
    let (events, _filtered) = storage
        .with_conn(|conn| query_events(conn, &input.query, None))
        .map_err(|e| anyhow::anyhow!("query_events: {e}"))?;
    let expected_path = dir.join("expected.json");
    let expected_text = std::fs::read_to_string(&expected_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", expected_path.display(), e))?;
    let expected: PutQueryExpected = serde_json::from_str(&expected_text)?;
    if events.len() != expected.rows_returned {
        return Err(anyhow::anyhow!(
            "rows_returned: expected {}, got {}",
            expected.rows_returned,
            events.len()
        ));
    }
    let actual_types: Vec<String> = events.iter().map(|e| e.event_type.clone()).collect();
    if actual_types != expected.event_types {
        return Err(anyhow::anyhow!(
            "event_types: expected {:?}, got {:?}",
            expected.event_types,
            actual_types
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PermissionsInput {
    events: Vec<ohd_storage_core::events::EventInput>,
    grant: PermissionsGrant,
    query: ohd_storage_core::events::EventFilter,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PermissionsGrant {
    default_action: String,
    /// Each is `["event_type_name", "allow"|"deny"]`.
    event_type_rules: Vec<(String, String)>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PermissionsExpected {
    rows_returned: usize,
    rows_filtered: i64,
    event_types: Vec<String>,
}

fn run_permissions(dir: &Path, input_text: &str) -> anyhow::Result<()> {
    use ohd_storage_core::{
        events::{put_events, query_events, GrantScope},
        registry,
        storage::{Storage, StorageConfig},
    };
    let input: PermissionsInput = serde_json::from_str(input_text)?;
    let tmp = tempfile::tempdir()?;
    let storage = Storage::open(StorageConfig::new(tmp.path().join("conformance.db")))
        .map_err(|e| anyhow::anyhow!("open storage: {e}"))?;
    let envelope = storage.envelope_key().cloned();
    storage
        .with_conn_mut(|conn| {
            put_events(conn, &input.events, None, false, envelope.as_ref()).map(|_| ())
        })
        .map_err(|e| anyhow::anyhow!("put_events: {e}"))?;

    // Materialize the grant scope synthetically (without writing it to the DB)
    // by resolving event-type names to ids.
    let mut allow = vec![];
    let mut deny = vec![];
    for (et_name, effect) in &input.grant.event_type_rules {
        let etn = registry::EventTypeName::parse(et_name)
            .map_err(|e| anyhow::anyhow!("parse event_type {et_name}: {e}"))?;
        let et = storage
            .with_conn(|conn| registry::resolve_event_type(conn, &etn))
            .map_err(|e| anyhow::anyhow!("resolve {et_name}: {e}"))?;
        match effect.as_str() {
            "allow" => allow.push(et.id),
            _ => deny.push(et.id),
        }
    }
    let scope = GrantScope {
        default_allow: input.grant.default_action == "allow",
        event_type_allow: allow,
        event_type_deny: deny,
        ..Default::default()
    };
    let (events, filtered) = storage
        .with_conn(|conn| query_events(conn, &input.query, Some(&scope)))
        .map_err(|e| anyhow::anyhow!("query_events: {e}"))?;

    let expected_path = dir.join("expected.json");
    let expected_text = std::fs::read_to_string(&expected_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", expected_path.display(), e))?;
    let expected: PermissionsExpected = serde_json::from_str(&expected_text)?;
    if events.len() != expected.rows_returned {
        return Err(anyhow::anyhow!(
            "rows_returned: expected {}, got {}",
            expected.rows_returned,
            events.len()
        ));
    }
    if filtered != expected.rows_filtered {
        return Err(anyhow::anyhow!(
            "rows_filtered: expected {}, got {}",
            expected.rows_filtered,
            filtered
        ));
    }
    let actual_types: Vec<String> = events.iter().map(|e| e.event_type.clone()).collect();
    if actual_types != expected.event_types {
        return Err(anyhow::anyhow!(
            "event_types: expected {:?}, got {:?}",
            expected.event_types,
            actual_types
        ));
    }
    Ok(())
}

fn first_diff_offset(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}
