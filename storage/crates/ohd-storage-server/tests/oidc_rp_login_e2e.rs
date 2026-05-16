//! End-to-end test for the storage server's **OIDC-RP login** flow — the
//! Phase 4 "Sign in with OHD" wiring.
//!
//! It stands up a self-contained mock upstream OIDC provider (discovery,
//! authorize, token, JWKS — RS256-signed id_tokens), points the storage AS at
//! it as a configured provider, and drives the full path an OHD client takes:
//!
//!  1. `GET /oauth/authorize` — the login page lists the configured provider.
//!  2. `POST /oauth/authorize` with `provider=…` — 302 to the provider's
//!     `/authorize` carrying the AS's RP `state` + PKCE.
//!  3. Simulate the user authenticating: the mock provider would redirect to
//!     `/oauth/oidc-callback?code=…&state=…`.
//!  4. The callback exchanges the code (mock `/token` → RS256 id_token),
//!     verifies it against the mock `/jwks`, resolves/creates the storage
//!     user, and 302s back to the OHD client's `redirect_uri` with an OHD
//!     authorization code.
//!  5. `POST /oauth/token` exchanges that code for a storage self-session.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::{get, post};
use axum::{Form, Json};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use jsonwebtoken::jwk::{
    AlgorithmParameters, CommonParameters, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
    RSAKeyParameters, RSAKeyType,
};
use ohd_storage_core::storage::{Storage, StorageConfig};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

// ---------------------------------------------------------------------------
// Mock upstream OIDC provider
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MockIdp {
    issuer: String,
    /// The fixed `sub` the mock mints — a Crockford ULID so the storage RP
    /// adopts it verbatim when the provider key is `ohd_account`.
    subject: String,
    kid: String,
    encoding_key: Arc<EncodingKey>,
    jwks: JwkSet,
    client_id: String,
}

impl MockIdp {
    fn new(issuer: &str, client_id: &str, subject: &str) -> Self {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("rsa gen");
        let public_key = private_key.to_public_key();
        use base64::Engine;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let kid = "mock-kid-1".to_string();
        let jwk = Jwk {
            common: CommonParameters {
                public_key_use: Some(PublicKeyUse::Signature),
                key_algorithm: Some(KeyAlgorithm::RS256),
                key_id: Some(kid.clone()),
                ..Default::default()
            },
            algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
                key_type: RSAKeyType::RSA,
                n: b64.encode(public_key.n().to_bytes_be()),
                e: b64.encode(public_key.e().to_bytes_be()),
            }),
        };
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
            .expect("pkcs1 pem");
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes()).expect("from_rsa_pem");
        Self {
            issuer: issuer.into(),
            subject: subject.into(),
            kid,
            encoding_key: Arc::new(encoding_key),
            jwks: JwkSet { keys: vec![jwk] },
            client_id: client_id.into(),
        }
    }

    fn id_token(&self) -> String {
        #[derive(Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            aud: &'a str,
            sub: &'a str,
            iat: i64,
            exp: i64,
            email: &'a str,
        }
        let now = ohd_storage_core::format::now_ms() / 1000;
        let claims = Claims {
            iss: &self.issuer,
            aud: &self.client_id,
            sub: &self.subject,
            iat: now,
            exp: now + 600,
            email: "user@ohd.test",
        };
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        encode(&header, &claims, &self.encoding_key).expect("encode")
    }
}

/// Spin the mock IdP on an ephemeral port; returns its issuer URL.
async fn boot_mock_idp(client_id: &str, subject: &str) -> (SocketAddr, MockIdp) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let issuer = format!("http://{addr}");
    let idp = MockIdp::new(&issuer, client_id, subject);

    let idp_disc = idp.clone();
    let idp_jwks = idp.clone();
    let idp_token = idp.clone();

    let app = axum::Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let i = idp_disc.issuer.clone();
                async move {
                    Json(serde_json::json!({
                        "issuer": i,
                        "authorization_endpoint": format!("{i}/authorize"),
                        "token_endpoint": format!("{i}/token"),
                        "jwks_uri": format!("{i}/jwks"),
                    }))
                }
            }),
        )
        .route(
            "/jwks",
            get(move || {
                let j = idp_jwks.jwks.clone();
                async move { Json(j) }
            }),
        )
        .route(
            "/token",
            post(move |Form(_f): Form<TokenForm>| {
                let t = idp_token.id_token();
                async move {
                    Json(serde_json::json!({
                        "access_token": "mock-access",
                        "token_type": "Bearer",
                        "expires_in": 3600,
                        "id_token": t,
                    }))
                }
            }),
        );

    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (addr, idp)
}

#[derive(Deserialize)]
struct TokenForm {
    #[allow(dead_code)]
    grant_type: Option<String>,
    #[allow(dead_code)]
    code: Option<String>,
}

// ---------------------------------------------------------------------------
// Storage server boot
// ---------------------------------------------------------------------------

async fn boot_storage(
    storage: Arc<Storage>,
    providers: Vec<oauth::OidcProvider>,
) -> (SocketAddr, String) {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    let issuer = format!("http://{addr}");
    drop(std_listener);

    let issuer_for_serve = issuer.clone();
    tokio::spawn(async move {
        server::serve_with_providers(
            storage,
            addr,
            false,
            Some(issuer_for_serve),
            Arc::new(providers),
        )
        .await
        .ok();
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (addr, issuer)
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_pair() -> (String, String) {
    use rand::RngCore;
    use sha2::{Digest, Sha256};
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = b64url(&bytes);
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    (verifier, b64url(&h.finalize()))
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return Some(urldecode(v));
        }
    }
    None
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16).unwrap_or(0);
                let lo = (bytes[i + 2] as char).to_digit(16).unwrap_or(0);
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn sign_in_with_ohd_account_creates_session() {
    // The mock IdP mints id_tokens with this `sub` — a Crockford ULID so the
    // RP adopts it verbatim as the storage user_ulid (the profile_ulid case).
    let profile_ulid = "01HF8K2PXYZ3W4Q5R6S7T8V9AB";
    // The storage RP authenticates to the provider as this client_id.
    let rp_client_id = "ohd-storage";
    let (idp_addr, idp) = boot_mock_idp(rp_client_id, profile_ulid).await;
    let idp_issuer = format!("http://{idp_addr}");

    // Configure storage with the mock as the `ohd_account` provider.
    let provider = oauth::OidcProvider {
        key: "ohd_account".into(),
        display_name: "OHD Account".into(),
        issuer: idp_issuer.clone(),
        client_id: rp_client_id.into(),
        client_secret: None,
        scopes: "openid email".into(),
    };
    let _ = &idp; // silence unused if assertions trimmed

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oidc_rp.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    oauth::bootstrap(&storage).unwrap();
    let (_addr, storage_issuer) = boot_storage(Arc::clone(&storage), vec![provider]).await;

    let client = http_client();

    // The OHD client (SPA) registers itself + computes its own PKCE.
    let reg: Value = client
        .post(format!("{storage_issuer}/oauth/register"))
        // register requires a self-session bearer — mint one out of band.
        .bearer_auth(
            storage
                .with_conn(|conn| {
                    ohd_storage_core::auth::issue_self_session_token(
                        conn,
                        storage.user_ulid(),
                        Some("test"),
                        None,
                    )
                })
                .unwrap(),
        )
        .json(&serde_json::json!({
            "client_name": "spa",
            "redirect_uris": ["https://spa.test/cb"],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let spa_client_id = reg["client_id"].as_str().unwrap().to_string();
    let (spa_verifier, spa_challenge) = pkce_pair();

    // Step 1: the login page lists the provider.
    let page = client
        .get(format!("{storage_issuer}/oauth/authorize"))
        .query(&[
            ("response_type", "code"),
            ("client_id", &spa_client_id),
            ("redirect_uri", "https://spa.test/cb"),
            ("scope", "openid"),
            ("state", "spa-state-xyz"),
            ("code_challenge", &spa_challenge),
            ("code_challenge_method", "S256"),
        ])
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        page.contains("Sign in with OHD Account"),
        "login page should advertise the configured provider: {page}"
    );

    // Step 2: pick the provider → 302 to the provider's /authorize.
    let resp = client
        .post(format!("{storage_issuer}/oauth/authorize"))
        .form(&[
            ("response_type", "code"),
            ("client_id", &spa_client_id),
            ("redirect_uri", "https://spa.test/cb"),
            ("scope", "openid"),
            ("state", "spa-state-xyz"),
            ("code_challenge", &spa_challenge),
            ("code_challenge_method", "S256"),
            ("provider", "ohd_account"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 303, "expected redirect to provider");
    let to_provider = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        to_provider.starts_with(&format!("{idp_issuer}/authorize")),
        "should redirect to the provider's authorize endpoint: {to_provider}"
    );
    let oidc_state = query_param(&to_provider, "state").expect("state on provider redirect");
    let rp_redirect = query_param(&to_provider, "redirect_uri").expect("redirect_uri");
    assert_eq!(rp_redirect, format!("{storage_issuer}/oauth/oidc-callback"));

    // Step 3+4: simulate the provider redirecting the user back to the
    // storage RP callback with an authorization code. The storage callback
    // exchanges it at the mock /token, verifies the id_token, resolves the
    // user, and 302s back to the SPA's redirect_uri.
    let cb = client
        .get(format!("{storage_issuer}/oauth/oidc-callback"))
        .query(&[("code", "mock-provider-code"), ("state", &oidc_state)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        cb.status().as_u16(),
        303,
        "callback should redirect back to the SPA"
    );
    let back_to_spa = cb
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        back_to_spa.starts_with("https://spa.test/cb"),
        "should redirect to the SPA redirect_uri: {back_to_spa}"
    );
    assert_eq!(
        query_param(&back_to_spa, "state").as_deref(),
        Some("spa-state-xyz"),
        "the SPA's own state must round-trip"
    );
    let ohd_code = query_param(&back_to_spa, "code").expect("OHD authorization code");

    // Step 5: exchange the OHD code for a storage self-session.
    let tokens: Value = client
        .post(format!("{storage_issuer}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &ohd_code),
            ("redirect_uri", "https://spa.test/cb"),
            ("client_id", &spa_client_id),
            ("code_verifier", &spa_verifier),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access = tokens["access_token"].as_str().expect("access_token");
    assert!(access.starts_with("ohds_"), "self-session token: {access}");

    // The id_token's sub (a ULID) became the storage user_ulid; userinfo
    // should report it.
    let userinfo: Value = client
        .get(format!("{storage_issuer}/oauth/userinfo"))
        .bearer_auth(access)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        userinfo["sub"].as_str(),
        Some(profile_ulid),
        "the storage user identity should be the profile_ulid from the id_token"
    );

    // The `(provider, sub)` binding is recorded in `_oidc_identities`.
    let parsed = ohd_storage_core::ulid::parse_crockford(profile_ulid).unwrap();
    let identities = storage
        .with_conn(|conn| ohd_storage_core::identities::list_identities(conn, parsed))
        .unwrap();
    assert_eq!(identities.len(), 1, "one linked identity recorded");
    assert_eq!(identities[0].provider, idp_issuer);
    assert_eq!(identities[0].subject, profile_ulid);

    // A second login with the same identity resolves to the *same* user (no
    // duplicate user, no duplicate identity row).
    let (v2, c2) = pkce_pair();
    let resp2 = client
        .post(format!("{storage_issuer}/oauth/authorize"))
        .form(&[
            ("response_type", "code"),
            ("client_id", &spa_client_id),
            ("redirect_uri", "https://spa.test/cb"),
            ("scope", "openid"),
            ("state", "second"),
            ("code_challenge", &c2),
            ("code_challenge_method", "S256"),
            ("provider", "ohd_account"),
        ])
        .send()
        .await
        .unwrap();
    let to_provider2 = resp2
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let oidc_state2 = query_param(&to_provider2, "state").unwrap();
    let cb2 = client
        .get(format!("{storage_issuer}/oauth/oidc-callback"))
        .query(&[("code", "mock-code-2"), ("state", &oidc_state2)])
        .send()
        .await
        .unwrap();
    let back2 = cb2
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let code2 = query_param(&back2, "code").unwrap();
    let tokens2: Value = client
        .post(format!("{storage_issuer}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code2),
            ("redirect_uri", "https://spa.test/cb"),
            ("client_id", &spa_client_id),
            ("code_verifier", &v2),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access2 = tokens2["access_token"].as_str().unwrap();
    let userinfo2: Value = client
        .get(format!("{storage_issuer}/oauth/userinfo"))
        .bearer_auth(access2)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        userinfo2["sub"].as_str(),
        Some(profile_ulid),
        "second login resolves to the same storage user"
    );
    let identities_after = storage
        .with_conn(|conn| ohd_storage_core::identities::list_identities(conn, parsed))
        .unwrap();
    assert_eq!(
        identities_after.len(),
        1,
        "no duplicate identity row on re-login"
    );

    // Reused login `state` is rejected (single-use).
    let replay = client
        .get(format!("{storage_issuer}/oauth/oidc-callback"))
        .query(&[("code", "mock-code-2"), ("state", &oidc_state2)])
        .send()
        .await
        .unwrap();
    assert_eq!(
        replay.status().as_u16(),
        400,
        "replayed oidc state must be rejected"
    );
}
