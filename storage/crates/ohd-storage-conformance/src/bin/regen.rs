//! Regenerate `expected.bin` for every sample-block fixture in the corpus.
//!
//! Run once after adding/modifying a sample-block fixture's `input.json`.
//!
//! `cargo run -p ohd-storage-conformance --bin regen`

fn main() -> anyhow::Result<()> {
    let root = ohd_storage_conformance::default_corpus_root();
    ohd_storage_conformance::regenerate_sample_block_expected_bins(&root)?;
    println!(
        "regenerated sample-block expected.bin under {}",
        root.display()
    );
    Ok(())
}
