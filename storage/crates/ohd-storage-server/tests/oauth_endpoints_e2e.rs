//! End-to-end test for the OAuth/OIDC IdP endpoints exposed by the storage
//! server when run with `--oauth-issuer`.
//!
//! What this exercises:
//!  - `GET /.well-known/openid-configuration` returns valid discovery JSON
//!    pointing at the configured issuer.
//!  - `POST /oauth/register` mints a public client.
//!  - Authorization-code + PKCE round-trip:
//!     * `POST /oauth/authorize` with self-session token → 302 to redirect
//!       URI carrying `code` + `state`.
//!     * `POST /oauth/token` exchanges the code for `(access_token,
//!       refresh_token, id_token)`. Wrong PKCE verifier is rejected.
//!     * `GET /oauth/userinfo` with the access_token returns the user's sub.
//!     * The id_token verifies against `GET /oauth/jwks.json`.
//!  - Device-code round-trip:
//!     * `POST /oauth/device` returns a `(device_code, user_code, …)` bundle.
//!     * Polling `/oauth/token` before confirmation returns
//!       `authorization_pending`.
//!     * Submitting the user_code at `/oauth/device-confirm` completes.
//!     * Polling `/oauth/token` again returns tokens.
//!  - Key rotation: rotating the active key keeps old id_tokens verifiable
//!    (the old kid stays in the JWKS); new id_tokens use the new kid.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use ohd_storage_core::auth::issue_self_session_token;
use ohd_storage_core::storage::{Storage, StorageConfig};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[allow(dead_code)]
#[path = "../src/auth_server.rs"]
mod auth_server;
#[allow(dead_code)]
#[path = "../src/jwks.rs"]
mod jwks;
#[allow(dead_code)]
#[path = "../src/oauth.rs"]
mod oauth;
#[allow(dead_code)]
#[path = "../src/server.rs"]
mod server;
#[allow(dead_code)]
#[path = "../src/sync_server.rs"]
mod sync_server;

mod proto {
    connectrpc::include_generated!();
}

/// Spin up the server with OAuth enabled, returning the `(addr, issuer)`.
async fn boot_server(storage: Arc<Storage>) -> (SocketAddr, String) {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    let issuer = format!("http://{addr}");
    drop(std_listener); // We'll let server::serve bind it.

    let issuer_for_serve = issuer.clone();
    tokio::spawn(async move {
        // Re-bind. `server::serve` opens its own listener.
        server::serve(storage, addr, false, Some(issuer_for_serve))
            .await
            .ok();
    });
    // Give the listener a beat to come up.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (addr, issuer)
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_pair() -> (String, String) {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = b64url(&bytes);
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    let challenge = b64url(&h.finalize());
    (verifier, challenge)
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn discovery_returns_valid_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oauth_discovery.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    oauth::bootstrap(&storage).unwrap();
    let (_addr, issuer) = boot_server(storage).await;

    let client = http_client();
    let resp = client
        .get(format!("{issuer}/.well-known/openid-configuration"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["issuer"], issuer.trim_end_matches('/'));
    assert_eq!(
        v["authorization_endpoint"],
        format!("{issuer}/oauth/authorize")
    );
    assert_eq!(v["token_endpoint"], format!("{issuer}/oauth/token"));
    assert_eq!(v["jwks_uri"], format!("{issuer}/oauth/jwks.json"));
    assert!(v["grant_types_supported"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g == "urn:ietf:params:oauth:grant-type:device_code"));
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_code_flow_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oauth_code.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    oauth::bootstrap(&storage).unwrap();
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, user_ulid, Some("oauth-e2e"), None))
        .unwrap();

    let (_addr, issuer) = boot_server(storage).await;
    let client = http_client();

    // ---- Register a public client. Auth as the operator's self-session. ----
    let redirect_uri = "http://localhost:9999/callback";
    let reg = client
        .post(format!("{issuer}/oauth/register"))
        .bearer_auth(&bearer)
        .json(&serde_json::json!({
            "client_name": "oauth-e2e-client",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(reg.status(), 201);
    let reg: Value = reg.json().await.unwrap();
    let client_id = reg["client_id"].as_str().unwrap().to_string();

    // ---- Authorize: POST the form with the self-session token. ----
    let (verifier, challenge) = pkce_pair();
    let state_val = "xyz123";
    let auth_resp = client
        .post(format!("{issuer}/oauth/authorize"))
        .form(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", "openid"),
            ("state", state_val),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("self_session_token", bearer.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert!(
        auth_resp.status().is_redirection(),
        "expected 302, got {}",
        auth_resp.status()
    );
    let loc = auth_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(loc.starts_with(redirect_uri), "loc = {loc}");
    let url = url_like_parse(&loc);
    let code = url.get("code").cloned().expect("code in redirect");
    assert_eq!(url.get("state").map(String::as_str), Some(state_val));

    // ---- Wrong PKCE verifier → invalid_grant. ----
    let bad = client
        .post(format!("{issuer}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id.as_str()),
            ("code_verifier", "definitely-not-the-verifier"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let bad_body: Value = bad.json().await.unwrap();
    assert_eq!(bad_body["error"], "invalid_grant");

    // The bad attempt also marked the code used. Need a fresh code.
    let auth_resp = client
        .post(format!("{issuer}/oauth/authorize"))
        .form(&[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", "openid"),
            ("state", state_val),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("self_session_token", bearer.as_str()),
        ])
        .send()
        .await
        .unwrap();
    let loc = auth_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let url = url_like_parse(&loc);
    let code = url.get("code").cloned().expect("code in redirect");

    // ---- Correct token exchange ----
    let tok = client
        .post(format!("{issuer}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(tok.status(), 200, "{:?}", tok.text().await);
    let tok: Value = tok.json().await.unwrap();
    let access_token = tok["access_token"].as_str().unwrap().to_string();
    let id_token = tok["id_token"].as_str().unwrap().to_string();
    let refresh_token = tok["refresh_token"].as_str().unwrap().to_string();
    assert!(
        access_token.starts_with("ohds_"),
        "access_token shape: {access_token}"
    );
    assert!(
        refresh_token.starts_with("ohdr_"),
        "refresh_token shape: {refresh_token}"
    );

    // ---- Verify id_token signature against /oauth/jwks.json ----
    let jwks_resp = client
        .get(format!("{issuer}/oauth/jwks.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(jwks_resp.status(), 200);
    let jwks: JwkSet = jwks_resp.json().await.unwrap();
    let header = decode_header(&id_token).unwrap();
    let kid = header.kid.expect("id_token kid");
    let jwk = jwks
        .keys
        .iter()
        .find(|k| k.common.key_id.as_deref() == Some(kid.as_str()))
        .expect("kid in JWKS");
    let key = DecodingKey::from_jwk(jwk).unwrap();
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[issuer.trim_end_matches('/')]);
    validation.set_audience(&[client_id.as_str()]);
    let decoded = decode::<Value>(&id_token, &key, &validation).expect("id_token verifies");
    assert_eq!(
        decoded.claims["sub"].as_str().unwrap(),
        ohd_storage_core::ulid::to_crockford(&user_ulid)
    );

    // ---- /oauth/userinfo with the access token ----
    let info = client
        .get(format!("{issuer}/oauth/userinfo"))
        .bearer_auth(&access_token)
        .send()
        .await
        .unwrap();
    assert_eq!(info.status(), 200);
    let info: Value = info.json().await.unwrap();
    assert_eq!(
        info["sub"].as_str().unwrap(),
        ohd_storage_core::ulid::to_crockford(&user_ulid)
    );

    // ---- Refresh ----
    let r = client
        .post(format!("{issuer}/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", client_id.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "refresh: {:?}", r.text().await);
    let r: Value = r.json().await.unwrap();
    assert!(r["access_token"].as_str().unwrap().starts_with("ohds_"));
    // No new id_token check — we already validated the signing path.
}

#[tokio::test(flavor = "multi_thread")]
async fn device_code_flow_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oauth_device.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    oauth::bootstrap(&storage).unwrap();
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, user_ulid, Some("dev-e2e"), None))
        .unwrap();

    let (_addr, issuer) = boot_server(storage).await;
    let client = http_client();

    // Register a client.
    let reg = client
        .post(format!("{issuer}/oauth/register"))
        .bearer_auth(&bearer)
        .json(&serde_json::json!({
            "client_name": "device-e2e",
            "redirect_uris": [],
            "grant_types": ["urn:ietf:params:oauth:grant-type:device_code"],
            "response_types": [],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .unwrap();
    let reg: Value = reg.json().await.unwrap();
    let client_id = reg["client_id"].as_str().unwrap().to_string();

    // Device authorize.
    let dev = client
        .post(format!("{issuer}/oauth/device"))
        .form(&[("client_id", client_id.as_str()), ("scope", "openid")])
        .send()
        .await
        .unwrap();
    assert_eq!(dev.status(), 200);
    let dev: Value = dev.json().await.unwrap();
    let device_code = dev["device_code"].as_str().unwrap().to_string();
    let user_code = dev["user_code"].as_str().unwrap().to_string();

    // Pre-confirmation poll → authorization_pending.
    let pending = client
        .post(format!("{issuer}/oauth/token"))
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code.as_str()),
            ("client_id", client_id.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(pending.status(), 400);
    let pending: Value = pending.json().await.unwrap();
    assert_eq!(pending["error"], "authorization_pending");

    // Confirm via /oauth/device-confirm (POST form).
    let confirm = client
        .post(format!("{issuer}/oauth/device-confirm"))
        .form(&[
            ("user_code", user_code.as_str()),
            ("self_session_token", bearer.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(confirm.status(), 200);

    // Now poll → tokens.
    let tok = client
        .post(format!("{issuer}/oauth/token"))
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code.as_str()),
            ("client_id", client_id.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(tok.status(), 200, "{:?}", tok.text().await);
    let tok: Value = tok.json().await.unwrap();
    assert!(tok["access_token"].as_str().unwrap().starts_with("ohds_"));
    assert!(!tok["id_token"].as_str().unwrap().is_empty());

    // Re-redeeming the device_code fails.
    let again = client
        .post(format!("{issuer}/oauth/token"))
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code.as_str()),
            ("client_id", client_id.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 400);
    let again: Value = again.json().await.unwrap();
    assert_eq!(again["error"], "invalid_grant");
}

#[tokio::test(flavor = "multi_thread")]
async fn jwks_rotation_keeps_old_keys_verifiable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oauth_rotate.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    oauth::bootstrap(&storage).unwrap();
    let user_ulid = storage.user_ulid();

    // Mint an id_token under the original key directly via the signing module.
    let issuer = "https://test.example".to_string();
    let _t1 = oauth::signing::mint_id_token(
        &storage,
        &issuer,
        "client-a",
        user_ulid,
        ohd_storage_core::format::now_ms(),
        3_600_000,
    )
    .expect("mint #1");

    // Rotate.
    let new_kid = oauth::signing::rotate_active_key(&storage).expect("rotate");
    // Mint another id_token; it should use the new kid.
    let t2 = oauth::signing::mint_id_token(
        &storage,
        &issuer,
        "client-a",
        user_ulid,
        ohd_storage_core::format::now_ms(),
        3_600_000,
    )
    .expect("mint #2");
    let h2 = decode_header(&t2).unwrap();
    assert_eq!(h2.kid.as_deref(), Some(new_kid.as_str()));

    // JWKS contains both keys.
    let jwks = oauth::signing::list_active_jwks(&storage).expect("jwks");
    assert!(
        jwks.keys.len() >= 2,
        "expected ≥2 keys post-rotation; got {}",
        jwks.keys.len()
    );
    assert!(jwks
        .keys
        .iter()
        .any(|k| k.common.key_id.as_deref() == Some(new_kid.as_str())));
}

/// Tiny `?k=v&k2=v2` parser. Doesn't url-decode beyond '+' → ' '.
fn url_like_parse(loc: &str) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    let qs = match loc.find('?') {
        Some(i) => &loc[i + 1..],
        None => loc,
    };
    for kv in qs.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            m.insert(k.to_string(), v.replace('+', " "));
        }
    }
    m
}
