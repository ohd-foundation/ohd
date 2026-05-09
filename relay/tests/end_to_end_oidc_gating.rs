//! End-to-end integration tests for per-OIDC registration gating.
//!
//! Exercises the full HTTP path:
//!
//! 1. Spin up a mock OIDC IdP (HTTP server serving `/.well-known/openid-
//!    configuration` + `/jwks.json`).
//! 2. Spin up the relay with an `[auth.registration]` allowlist
//!    pointing at the mock IdP.
//! 3. Register the storage with / without an `id_token`, valid + invalid
//!    tokens; assert the right HTTP status + JSON `code` field.
//! 4. Hit `GET /v1/auth/info` and assert it surfaces the configured
//!    issuers + the require_oidc flag.
//!
//! This is the integration counterpart to the unit tests in
//! `src/auth/oidc.rs`; together they cover the JWKS-cache primitive in
//! isolation AND its wiring through the registration RPC.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use ohd_relay::auth::{OidcVerifier, OidcVerifierConfig};
use ohd_relay::config::{AllowedIssuer, RegistrationAuthConfig};
use ohd_relay::push::PushDispatcher;
use ohd_relay::server::{build_router, AppState, RegistrationAuthState};
use ohd_relay::state::RelayState;
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::Serialize;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Mock IdP
// ---------------------------------------------------------------------------

struct TestKey {
    signing_pem: Vec<u8>,
    kid: String,
    n_b64: String,
    e_b64: String,
}

fn gen_rsa_key(kid: &str) -> TestKey {
    use base64::Engine;
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pem = priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
    let signing_pem = pem.as_bytes().to_vec();

    let pub_key = priv_key.to_public_key();
    let n = pub_key.n().to_bytes_be();
    let e = pub_key.e().to_bytes_be();
    let n_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&n);
    let e_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&e);
    TestKey {
        signing_pem,
        kid: kid.into(),
        n_b64,
        e_b64,
    }
}

fn jwks_doc(key: &TestKey) -> serde_json::Value {
    serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": key.kid,
            "n": key.n_b64,
            "e": key.e_b64,
        }]
    })
}

#[derive(Serialize)]
struct TestClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    exp: i64,
    nbf: i64,
    iat: i64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn sign_jwt(key: &TestKey, claims: &TestClaims) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(
        &header,
        claims,
        &EncodingKey::from_rsa_pem(&key.signing_pem).unwrap(),
    )
    .unwrap()
}

async fn spawn_idp(
    issuer: String,
    jwks: Arc<Mutex<serde_json::Value>>,
) -> (String, tokio::task::JoinHandle<()>) {
    use axum::{
        extract::State,
        response::{IntoResponse, Json},
        routing::get,
        Router,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let discovery_url = format!("{base}/.well-known/openid-configuration");
    let jwks_url = format!("{base}/jwks.json");

    #[derive(Clone)]
    struct St {
        issuer: String,
        jwks: Arc<Mutex<serde_json::Value>>,
        jwks_url: String,
    }

    async fn discovery(State(s): State<St>) -> impl IntoResponse {
        Json(serde_json::json!({
            "issuer": s.issuer,
            "jwks_uri": s.jwks_url,
        }))
    }
    async fn jwks_route(State(s): State<St>) -> impl IntoResponse {
        let v = s.jwks.lock().await.clone();
        Json(v)
    }

    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks.json", get(jwks_route))
        .with_state(St {
            issuer: issuer.clone(),
            jwks: jwks.clone(),
            jwks_url: jwks_url.clone(),
        });

    let h = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (discovery_url, h)
}

// ---------------------------------------------------------------------------
// Relay harness
// ---------------------------------------------------------------------------

async fn spawn_relay(reg_cfg: RegistrationAuthConfig, discovery_url: String) -> SocketAddr {
    let relay = RelayState::in_memory().await.expect("in-memory state");
    let mut over = HashMap::new();
    if let Some(first) = reg_cfg.allowed_issuers.first() {
        over.insert(first.issuer.clone(), discovery_url);
    }
    let verifier = OidcVerifier::new(OidcVerifierConfig {
        allowed_issuers: reg_cfg.allowed_issuers.clone(),
        jwks_cache_ttl: Duration::from_secs(reg_cfg.jwks_cache_ttl_secs),
        discovery_override: over,
    });
    let registration_auth = RegistrationAuthState {
        require_oidc: reg_cfg.require_oidc,
        allowed_issuers: reg_cfg.allowed_issuers.clone(),
        verifier,
    };
    let emergency = ohd_relay::emergency_endpoints::EmergencyStateTable::new(
        relay.registrations.conn_for_emergency(),
    );
    let app_state = AppState {
        relay,
        push: Arc::new(PushDispatcher::new()),
        public_host: "127.0.0.1:0".to_string(),
        registration_auth,
        #[cfg(feature = "authority")]
        authority: None,
        emergency,
        storage_tunnel: None,
    };
    let app = build_router(app_state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    addr
}

fn register_body(id_token: Option<&str>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "user_ulid": "0123456789abcdef0123456789abcdef",
        "storage_pubkey_spki_hex": "deadbeef".repeat(8),
        "user_label": "oidc-test",
    });
    if let Some(t) = id_token {
        v["id_token"] = serde_json::Value::String(t.to_string());
    }
    v
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn permissive_relay_accepts_register_without_id_token() {
    // No allowlist → fully permissive. id_token field is ignored.
    let relay_addr = spawn_relay(
        RegistrationAuthConfig {
            allowed_issuers: vec![],
            require_oidc: false,
            jwks_cache_ttl_secs: 3600,
        },
        // Discovery URL doesn't matter — verifier won't be invoked.
        "http://localhost:0/disco".to_string(),
    )
    .await;
    let url = format!("http://{relay_addr}/v1/register");
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&register_body(None))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
}

#[tokio::test]
async fn gated_relay_rejects_missing_id_token_when_required() {
    let issuer = "https://idp.test/".to_string();
    let key = gen_rsa_key("kid-1");
    let jwks = Arc::new(Mutex::new(jwks_doc(&key)));
    let (discovery_url, _h) = spawn_idp(issuer.clone(), jwks).await;

    let cfg = RegistrationAuthConfig {
        allowed_issuers: vec![AllowedIssuer {
            issuer: issuer.clone(),
            expected_audience: "ohd-relay-cloud".into(),
        }],
        require_oidc: true,
        jwks_cache_ttl_secs: 3600,
    };
    let relay_addr = spawn_relay(cfg, discovery_url).await;

    let url = format!("http://{relay_addr}/v1/register");
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&register_body(None))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "OIDC_REQUIRED");
}

#[tokio::test]
async fn gated_relay_accepts_valid_id_token() {
    let issuer = "https://idp.test/".to_string();
    let key = gen_rsa_key("kid-1");
    let jwks = Arc::new(Mutex::new(jwks_doc(&key)));
    let (discovery_url, _h) = spawn_idp(issuer.clone(), jwks).await;

    let cfg = RegistrationAuthConfig {
        allowed_issuers: vec![AllowedIssuer {
            issuer: issuer.clone(),
            expected_audience: "ohd-relay-cloud".into(),
        }],
        require_oidc: true,
        jwks_cache_ttl_secs: 3600,
    };
    let relay_addr = spawn_relay(cfg, discovery_url).await;

    let now = now_secs();
    let claims = TestClaims {
        iss: &issuer,
        sub: "ops@example.invalid",
        aud: "ohd-relay-cloud",
        exp: now + 600,
        nbf: now - 30,
        iat: now,
    };
    let jwt = sign_jwt(&key, &claims);

    let url = format!("http://{relay_addr}/v1/register");
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&register_body(Some(&jwt)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("rendezvous_id").is_some());
    assert!(body.get("long_lived_credential").is_some());
}

#[tokio::test]
async fn gated_relay_rejects_invalid_signature_with_verify_failed_code() {
    let issuer = "https://idp.test/".to_string();
    let key = gen_rsa_key("kid-1");
    let intruder = gen_rsa_key("kid-1"); // same kid, different key material
    let jwks = Arc::new(Mutex::new(jwks_doc(&key)));
    let (discovery_url, _h) = spawn_idp(issuer.clone(), jwks).await;

    let cfg = RegistrationAuthConfig {
        allowed_issuers: vec![AllowedIssuer {
            issuer: issuer.clone(),
            expected_audience: "ohd-relay-cloud".into(),
        }],
        require_oidc: true,
        jwks_cache_ttl_secs: 3600,
    };
    let relay_addr = spawn_relay(cfg, discovery_url).await;

    let now = now_secs();
    let claims = TestClaims {
        iss: &issuer,
        sub: "ops",
        aud: "ohd-relay-cloud",
        exp: now + 600,
        nbf: now - 30,
        iat: now,
    };
    let jwt = sign_jwt(&intruder, &claims);

    let url = format!("http://{relay_addr}/v1/register");
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&register_body(Some(&jwt)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "OIDC_VERIFY_FAILED");
}

#[tokio::test]
async fn gated_relay_rejects_unknown_issuer() {
    // Allowlist contains issuer A; token is from issuer B.
    let allowed = "https://idp-allowed.test/".to_string();
    let other = "https://idp-other.test/".to_string();
    let key = gen_rsa_key("kid-1");
    let jwks = Arc::new(Mutex::new(jwks_doc(&key)));
    let (discovery_url, _h) = spawn_idp(allowed.clone(), jwks).await;

    let cfg = RegistrationAuthConfig {
        allowed_issuers: vec![AllowedIssuer {
            issuer: allowed,
            expected_audience: "ohd-relay-cloud".into(),
        }],
        require_oidc: true,
        jwks_cache_ttl_secs: 3600,
    };
    let relay_addr = spawn_relay(cfg, discovery_url).await;

    let now = now_secs();
    let claims = TestClaims {
        iss: &other,
        sub: "ops",
        aud: "ohd-relay-cloud",
        exp: now + 600,
        nbf: now - 30,
        iat: now,
    };
    let jwt = sign_jwt(&key, &claims);

    let url = format!("http://{relay_addr}/v1/register");
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&register_body(Some(&jwt)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "OIDC_VERIFY_FAILED");
}

#[tokio::test]
async fn soft_gated_relay_accepts_no_token_but_verifies_when_present() {
    // require_oidc=false: no token = ok, but a presented bad token still
    // fails. This is the migration / rollout setting.
    let issuer = "https://idp.test/".to_string();
    let key = gen_rsa_key("kid-1");
    let jwks = Arc::new(Mutex::new(jwks_doc(&key)));
    let (discovery_url, _h) = spawn_idp(issuer.clone(), jwks).await;

    let cfg = RegistrationAuthConfig {
        allowed_issuers: vec![AllowedIssuer {
            issuer: issuer.clone(),
            expected_audience: "ohd-relay-cloud".into(),
        }],
        require_oidc: false,
        jwks_cache_ttl_secs: 3600,
    };
    let relay_addr = spawn_relay(cfg, discovery_url).await;

    let url = format!("http://{relay_addr}/v1/register");
    let client = reqwest::Client::new();

    // No token: accepted.
    let resp = client
        .post(&url)
        .json(&register_body(None))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);

    // Expired token: rejected (still verified when presented). Use a
    // generous expiry-in-the-past so the verifier's 30s leeway doesn't
    // accidentally accept it.
    let now = now_secs();
    let claims = TestClaims {
        iss: &issuer,
        sub: "ops",
        aud: "ohd-relay-cloud",
        exp: now - 600,
        nbf: now - 1200,
        iat: now - 1200,
    };
    let jwt = sign_jwt(&key, &claims);
    let resp = client
        .post(&url)
        .json(&register_body(Some(&jwt)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "OIDC_VERIFY_FAILED");
}

#[tokio::test]
async fn auth_info_surfaces_configured_issuers() {
    let issuer = "https://idp.test/".to_string();
    let cfg = RegistrationAuthConfig {
        allowed_issuers: vec![
            AllowedIssuer {
                issuer: issuer.clone(),
                expected_audience: "ohd-relay-cloud".into(),
            },
            AllowedIssuer {
                issuer: "https://other.test/".into(),
                expected_audience: "ohd-relay-cloud".into(),
            },
        ],
        require_oidc: true,
        jwks_cache_ttl_secs: 3600,
    };
    let relay_addr = spawn_relay(cfg, "http://unused/disco".into()).await;

    let url = format!("http://{relay_addr}/v1/auth/info");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["registration_oidc_required"], true);
    let issuers = body["allowed_issuers"].as_array().unwrap();
    assert_eq!(issuers.len(), 2);
    assert_eq!(issuers[0]["issuer"], issuer);
    assert_eq!(issuers[0]["expected_audience"], "ohd-relay-cloud");
}

#[tokio::test]
async fn auth_info_permissive_relay_returns_empty_list() {
    let relay_addr = spawn_relay(
        RegistrationAuthConfig::default(),
        "http://unused/disco".into(),
    )
    .await;
    let url = format!("http://{relay_addr}/v1/auth/info");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["registration_oidc_required"], false);
    let issuers = body["allowed_issuers"].as_array().unwrap();
    assert!(issuers.is_empty());
}
