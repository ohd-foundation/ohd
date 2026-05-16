//! Config loading: file parse, defaults, client-secret resolution, and
//! `OHD_IDP_*` scalar overrides.
//!
//! Environment variables are process-global and Rust runs the tests in a
//! file in parallel, so anything that asserts an env-overridable field —
//! or mutates the environment — must not race a peer test. The whole
//! load/override surface is therefore exercised in one sequential
//! `#[test]`; the remaining tests touch neither env-overridable fields
//! nor the environment.

use ohd_idp::config;

const SAMPLE: &str = r#"
[server]
listen = "0.0.0.0:8447"
issuer = "https://accounts.ohd.dev"

[[client]]
id = "cord-web"
redirect_uris = ["https://cord.ohd.dev/v1/auth/callback"]
client_secret_env = "TEST_IDP_CORD_SECRET"

[[client]]
id = "connect-web"
redirect_uris = ["https://connect.ohd.dev/auth/callback"]
public = true
"#;

/// File parse + section defaults + client-secret resolution + every
/// `OHD_IDP_*` scalar override, run sequentially in one test so the
/// process-global environment is never observed mid-mutation by a peer.
#[test]
fn config_load_defaults_and_env_overrides() {
    // Make sure no override leaks in from the ambient environment.
    for v in [
        "OHD_IDP_ISSUER",
        "OHD_IDP_SIGNUP_OPEN",
        "OHD_IDP_LISTEN",
        "TEST_IDP_CORD_SECRET",
    ] {
        std::env::remove_var(v);
    }

    // --- file parse + section defaults ------------------------------------
    let cfg = config::from_str(SAMPLE).expect("config parses");
    assert_eq!(cfg.server.listen.to_string(), "0.0.0.0:8447");
    assert_eq!(cfg.server.issuer, "https://accounts.ohd.dev");
    assert_eq!(cfg.server.data_dir, "/var/lib/ohd-idp");
    assert_eq!(cfg.keys.signing_key_file, "/var/lib/ohd-idp/signing-key.pem");
    assert_eq!(cfg.keys.rotation_overlap_days, 7);
    assert_eq!(cfg.session.sso_ttl_hours, 12);
    assert_eq!(cfg.session.code_ttl_secs, 120);
    assert!(cfg.signup.open);
    assert!(cfg.recovery.enabled);
    assert_eq!(cfg.store.saas_db, "/var/lib/ohd-saas/ohd-saas.db");
    assert_eq!(cfg.clients.len(), 2);
    let connect = cfg.client("connect-web").expect("connect-web present");
    assert!(connect.public);
    assert!(connect.client_secret.is_empty());
    // A confidential client with no secret env set resolves to empty.
    let cord = cfg.client("cord-web").expect("cord-web present");
    assert!(!cord.public);
    assert!(cord.client_secret.is_empty());

    // --- confidential client secret resolves from its named env var ------
    std::env::set_var("TEST_IDP_CORD_SECRET", "cord-secret-xyz");
    let cfg = config::from_str(SAMPLE).expect("config parses");
    assert_eq!(
        cfg.client("cord-web").unwrap().client_secret,
        "cord-secret-xyz"
    );
    std::env::remove_var("TEST_IDP_CORD_SECRET");

    // --- OHD_IDP_ISSUER overrides the file value, trims trailing slash ---
    std::env::set_var("OHD_IDP_ISSUER", "https://accounts.clinic.example/");
    let cfg = config::from_str(SAMPLE).expect("config parses");
    assert_eq!(cfg.server.issuer, "https://accounts.clinic.example");
    std::env::remove_var("OHD_IDP_ISSUER");

    // --- OHD_IDP_LISTEN overrides the bind address -----------------------
    std::env::set_var("OHD_IDP_LISTEN", "127.0.0.1:9999");
    let cfg = config::from_str(SAMPLE).expect("config parses");
    assert_eq!(cfg.server.listen.to_string(), "127.0.0.1:9999");
    std::env::remove_var("OHD_IDP_LISTEN");

    // --- OHD_IDP_SIGNUP_OPEN overrides a boolean -------------------------
    std::env::set_var("OHD_IDP_SIGNUP_OPEN", "false");
    let cfg = config::from_str(SAMPLE).expect("config parses");
    assert!(!cfg.signup.open);
    std::env::remove_var("OHD_IDP_SIGNUP_OPEN");

    // --- with everything unset again, file/default values stand ----------
    let cfg = config::from_str(SAMPLE).expect("config parses");
    assert_eq!(cfg.server.issuer, "https://accounts.ohd.dev");
    assert_eq!(cfg.server.listen.to_string(), "0.0.0.0:8447");
    assert!(cfg.signup.open);
}

#[test]
fn rejects_a_malformed_listen_address() {
    let bad = r#"
[server]
listen = "not-an-address"
issuer = "https://accounts.ohd.dev"
"#;
    assert!(config::from_str(bad).is_err());
}

#[test]
fn shipped_default_idp_toml_parses() {
    // The repo's default idp.toml must always be valid TOML with the
    // expected RP clients. `listen` and `issuer` are env-overridable, so
    // this test does not assert them — a peer test mutating `OHD_IDP_*`
    // runs concurrently (see `config_load_defaults_*`).
    let text = include_str!("../../../idp.toml");
    let cfg = config::from_str(text).expect("shipped idp.toml parses");
    assert!(cfg.client("cord-web").is_some());
    assert!(cfg.client("connect-web").is_some());
    assert!(cfg.client("connect-web").unwrap().public);
}
