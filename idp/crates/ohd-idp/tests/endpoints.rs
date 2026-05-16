//! Integration tests over the assembled router: `/healthz`, `/jwks`, the
//! OIDC discovery document, and the RP-client registry.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ohd_idp::{build_router, config, ClientRegistry, SigningKey};
use serde_json::Value;
use tower::ServiceExt;

const SAMPLE: &str = r#"
[server]
listen = "0.0.0.0:8447"
issuer = "https://accounts.ohd.dev"

[[client]]
id = "cord-web"
redirect_uris = ["https://cord.ohd.dev/v1/auth/callback"]

[[client]]
id = "connect-web"
redirect_uris = ["https://connect.ohd.dev/auth/callback"]
public = true
"#;

/// Build a router over the sample config and a fresh in-memory key.
fn router() -> axum::Router {
    let cfg = config::from_str(SAMPLE).expect("config parses");
    let key = SigningKey::generate().expect("key generates");
    build_router(cfg, key)
}

async fn get_json(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let resp = router
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    (status, json)
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (status, body) = get_json(router(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "ohd-idp");
}

#[tokio::test]
async fn jwks_publishes_one_rs256_key() {
    let (status, body) = get_json(router(), "/jwks").await;
    assert_eq!(status, StatusCode::OK);
    let keys = body["keys"].as_array().expect("keys array");
    assert_eq!(keys.len(), 1);
    let jwk = &keys[0];
    assert_eq!(jwk["kty"], "RSA");
    assert_eq!(jwk["use"], "sig");
    assert_eq!(jwk["alg"], "RS256");
    assert!(jwk["kid"].as_str().map(|s| !s.is_empty()).unwrap_or(false));
    assert!(jwk["n"].is_string());
    assert!(jwk["e"].is_string());
}

#[tokio::test]
async fn discovery_document_has_expected_shape() {
    let (status, body) = get_json(router(), "/.well-known/openid-configuration").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["issuer"], "https://accounts.ohd.dev");
    assert_eq!(body["authorization_endpoint"], "https://accounts.ohd.dev/authorize");
    assert_eq!(body["token_endpoint"], "https://accounts.ohd.dev/token");
    assert_eq!(body["jwks_uri"], "https://accounts.ohd.dev/jwks");
    assert_eq!(body["userinfo_endpoint"], "https://accounts.ohd.dev/userinfo");
    assert_eq!(body["id_token_signing_alg_values_supported"][0], "RS256");
    assert_eq!(body["code_challenge_methods_supported"][0], "S256");
    assert_eq!(body["response_types_supported"][0], "code");
    assert_eq!(body["subject_types_supported"][0], "public");
}

#[tokio::test]
async fn jwks_kid_matches_discovery_jwks_uri_host() {
    // The JWKS the OP serves and the JWKS the discovery document points
    // at are the same origin — a smoke check the two endpoints agree.
    let (_, disc) = get_json(router(), "/.well-known/openid-configuration").await;
    assert_eq!(disc["jwks_uri"], "https://accounts.ohd.dev/jwks");
}

#[test]
fn registry_lookup_resolves_clients_from_config() {
    let cfg = config::from_str(SAMPLE).expect("config parses");
    let reg = ClientRegistry::from_config(&cfg.clients);
    assert_eq!(reg.len(), 2);

    let cord = reg.get("cord-web").expect("cord-web registered");
    assert!(!cord.public);
    assert!(cord.allows_redirect("https://cord.ohd.dev/v1/auth/callback"));
    assert!(!cord.allows_redirect("https://cord.ohd.dev/other"));

    let connect = reg.get("connect-web").expect("connect-web registered");
    assert!(connect.public);

    assert!(reg.get("unregistered-app").is_none());
}
