//! Run the full conformance corpus and assert all required fixtures pass.
//!
//! Iterates `corpus/manifest.json` end-to-end. Failures emit a diff
//! per-fixture and the test asserts `report.all_pass()`.

use ohd_storage_conformance::{
    default_corpus_root, regenerate_sample_block_expected_bins, run_all,
};

#[test]
fn corpus_passes() {
    let root = default_corpus_root();
    if std::env::var("OHD_CONFORMANCE_REGEN").as_deref() == Ok("1") {
        regenerate_sample_block_expected_bins(&root).expect("regen failed");
    }
    let report = run_all(&root).expect("run_all should not error catastrophically");
    if !report.all_pass() {
        for f in &report.failures {
            eprintln!("FAIL: {f}");
        }
        panic!(
            "{} corpus fixtures failed (passed {})",
            report.failed, report.passed
        );
    }
    eprintln!("{} corpus fixtures passed", report.passed);
}
