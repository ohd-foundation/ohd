//! End-to-end smoke test for `case-export`.
//!
//! Gated by `#[ignore]` because it spins up an actual `ohd-storage-server`
//! subprocess. Run with:
//!
//! ```bash
//! cd ../../storage && cargo build --bin ohd-storage-server
//! cd ../emergency/cli
//! cargo test --test case_export_smoke -- --ignored --nocapture
//! ```
//!
//! What it does:
//!
//! 1. Starts `ohd-storage-server serve` on a random port with a temp DB.
//! 2. Issues a self-token via the server's `issue-self-token` subcommand.
//! 3. Runs `ohd-emergency case-export --case-ulid X --output T.json`.
//!    Storage's `GetCase` is Unimplemented today, so the test asserts
//!    the CLI surfaces the error cleanly instead of producing a
//!    half-archive.
//! 4. When `GetCase` lands the test flips to assert archive contents
//!    (case header + events).
//!
//! This test deliberately doesn't reach into the storage core or its
//! workspace — it treats the storage server as a black-box subprocess.
//! That keeps emergency/cli's build independent.

#[test]
#[ignore = "requires ohd-storage-server binary; run with --ignored"]
fn case_export_against_real_storage() {
    use std::path::PathBuf;
    use std::process::Command;

    // Locate the storage server binary built by the workspace next door.
    // The path is intentionally relative to the test's CARGO_MANIFEST_DIR
    // so this works in CI where both crates are checked out side-by-side.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let storage_bin = manifest
        .parent() // emergency/
        .and_then(|p| p.parent()) // ohd/
        .map(|p| p.join("storage").join("target").join("debug").join("ohd-storage-server"))
        .expect("layout: <ohd>/emergency/cli");
    if !storage_bin.exists() {
        eprintln!(
            "skipping: storage server binary missing at {} \
             (run `cargo build --bin ohd-storage-server` in ../../storage first)",
            storage_bin.display()
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("storage.sqlite");

    // 1. Issue a self-token.
    let token_out = Command::new(&storage_bin)
        .args([
            "issue-self-token",
            "--db",
            db_path.to_str().unwrap(),
            "--label",
            "test-emergency-cli",
        ])
        .output()
        .expect("spawn issue-self-token");
    assert!(
        token_out.status.success(),
        "issue-self-token failed: {}",
        String::from_utf8_lossy(&token_out.stderr)
    );
    let token_stdout = String::from_utf8_lossy(&token_out.stdout);
    let token = token_stdout
        .lines()
        .find(|l| l.starts_with("ohds_"))
        .unwrap_or_else(|| panic!("no ohds_ token in:\n{token_stdout}"))
        .trim()
        .to_string();

    // 2. Find a free port + start the server.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut server = Command::new(&storage_bin)
        .args([
            "serve",
            "--db",
            db_path.to_str().unwrap(),
            "--listen",
            &format!("127.0.0.1:{port}"),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn serve");

    // Best-effort wait for the server to start. A real test would poll
    // the Health RPC; for the smoke we sleep briefly.
    std::thread::sleep(std::time::Duration::from_millis(800));

    // 3. Build the emergency CLI binary.
    let cli_bin = manifest
        .join("target")
        .join("debug")
        .join("ohd-emergency");
    assert!(
        cli_bin.exists(),
        "emergency CLI binary missing at {} (run `cargo build` first)",
        cli_bin.display()
    );

    // 4. Run case-export with a fake ULID. Storage returns Unimplemented
    //    on GetCase today, so the CLI must surface a non-zero exit with
    //    a clear error message.
    let out_path = tmp.path().join("archive.json");
    let cli_out = Command::new(&cli_bin)
        .args([
            "--storage",
            &format!("http://127.0.0.1:{port}"),
            "--token",
            &token,
            "case-export",
            "--case-ulid",
            "01JT00000000000000000000AB",
            "--output",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn ohd-emergency case-export");

    let _ = server.kill();
    let stderr = String::from_utf8_lossy(&cli_out.stderr).to_string();
    let stdout = String::from_utf8_lossy(&cli_out.stdout).to_string();

    // Today: storage's GetCase returns Unimplemented, so the CLI exits
    // non-zero and surfaces the error. Once the handler lands the
    // archive will be created and we'll assert on its contents instead.
    if cli_out.status.success() {
        assert!(out_path.exists(), "expected archive at {}", out_path.display());
        let archive = std::fs::read_to_string(&out_path).unwrap();
        assert!(archive.contains("ohd-emergency.case-export.v1"));
        assert!(archive.contains("\"case_ulid\""));
    } else {
        assert!(
            stderr.contains("Unimplemented")
                || stderr.contains("unimplement")
                || stderr.contains("GetCase"),
            "expected Unimplemented error from server; got:\nstdout={stdout}\nstderr={stderr}"
        );
    }
}
