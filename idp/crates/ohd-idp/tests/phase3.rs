//! Integration tests for Phase 3 — recovery-code login, password reset
//! via a recovery code, the bounded SSO session, RP-Initiated Logout, and
//! signing-key rotation.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use http_body_util::BodyExt;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use ohd_idp::store::{canonical_recovery, sha256_hex};
use ohd_idp::{build_router, config, AccountStore, IdpStore, KeyStore, SigningKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const SAMPLE: &str = r#"
[server]
listen = "0.0.0.0:8447"
issuer = "https://accounts.ohd.dev"

[[client]]
id = "cord-web"
redirect_uris = ["https://cord.ohd.dev/v1/auth/callback"]
client_secret_env = "TEST_P3_CORD_SECRET"

[[client]]
id = "connect-web"
redirect_uris = ["https://connect.ohd.dev/auth/callback"]
public = true
"#;

struct Harness {
    router: axum::Router,
    accounts: AccountStore,
}

fn harness() -> Harness {
    std::env::set_var("TEST_P3_CORD_SECRET", "cord-test-secret");
    let cfg = config::from_str(SAMPLE).expect("config parses");
    let key = SigningKey::generate().expect("key generates");
    let accounts = AccountStore::in_memory().expect("account store");
    let idp_store = IdpStore::in_memory().expect("idp store");
    let router = build_router(cfg, KeyStore::in_memory(key), accounts.clone(), idp_store);
    Harness { router, accounts }
}

const REDIRECT: &str = "https://cord.ohd.dev/v1/auth/callback";

/// PKCE S256 challenge for a verifier.
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

async fn get(
    router: &axum::Router,
    uri: &str,
    cookie: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut builder = Request::builder().uri(uri);
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    let resp = router
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&bytes).into_owned())
}

async fn post_form(
    router: &axum::Router,
    uri: &str,
    body: &str,
    cookie: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    let resp = router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&bytes).into_owned())
}

/// Pull the `code` query parameter out of a redirect `Location`.
fn code_from_location(location: &str) -> String {
    let q = location.split_once('?').unwrap().1;
    form_urlencoded::parse(q.as_bytes())
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .expect("code in redirect")
}

/// Extract the `ohd_idp_sso=<token>` cookie value from a `Set-Cookie`
/// header — the bare `name=value`, no attributes.
fn sso_cookie_from(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get("set-cookie")?.to_str().ok()?;
    let first = raw.split(';').next()?.trim();
    let value = first.strip_prefix("ohd_idp_sso=")?;
    if value.is_empty() {
        None
    } else {
        Some(format!("ohd_idp_sso={value}"))
    }
}

/// The standard authorize URL for the harness's `cord-web` client.
fn authorize_url(state: &str, challenge: &str) -> String {
    format!(
        "/authorize?client_id=cord-web&redirect_uri={}\
         &response_type=code&scope=openid%20email&state={}\
         &code_challenge={}&code_challenge_method=S256&nonce=n-1",
        urlencoding::encode(REDIRECT),
        urlencoding::encode(state),
        urlencoding::encode(challenge),
    )
}

// --- recovery-code login ---------------------------------------------------

#[tokio::test]
async fn recovery_code_login_succeeds_and_issues_a_code() {
    let h = harness();
    let created = h
        .accounts
        .create_account("rec@example.com", "a-good-password")
        .unwrap();

    let challenge = pkce_challenge("rec-verifier-aaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid+email&state=rec-1\
         &code_challenge={}&recovery_code={}",
        urlencoding::encode(REDIRECT),
        urlencoding::encode(&challenge),
        urlencoding::encode(&created.recovery_code),
    );
    let (status, headers, _) = post_form(&h.router, "/login", &body, None).await;
    assert_eq!(status, StatusCode::FOUND);
    let location = headers.get("location").unwrap().to_str().unwrap();
    assert!(location.starts_with(REDIRECT));
    assert!(location.contains("state=rec-1"));
    // A recovery-code login also mints the SSO session.
    assert!(sso_cookie_from(&headers).is_some());
}

#[tokio::test]
async fn recovery_code_login_rejects_an_invalid_code() {
    let h = harness();
    h.accounts
        .create_account("badrec@example.com", "a-good-password")
        .unwrap();
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=s\
         &code_challenge=ch&recovery_code=WRONG+CODE+THAT+DOES+NOT+EXIST",
        urlencoding::encode(REDIRECT),
    );
    let (status, headers, page) = post_form(&h.router, "/login", &body, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get("location").is_none());
    assert!(page.contains("recovery code is not valid"));
}

// --- password reset via recovery code --------------------------------------

#[tokio::test]
async fn password_reset_via_recovery_code_round_trips() {
    let h = harness();
    let created = h
        .accounts
        .create_account("resetme@example.com", "old-password")
        .unwrap();

    let challenge = pkce_challenge("reset-verifier-bbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid+email&state=rst-1\
         &code_challenge={}&recovery_code={}\
         &password=a-fresh-password&confirm=a-fresh-password",
        urlencoding::encode(REDIRECT),
        urlencoding::encode(&challenge),
        urlencoding::encode(&created.recovery_code),
    );
    let (status, headers, _) = post_form(&h.router, "/reset", &body, None).await;
    assert_eq!(status, StatusCode::FOUND, "reset should resume the flow");
    let location = headers.get("location").unwrap().to_str().unwrap();
    assert!(location.starts_with(REDIRECT));
    assert!(sso_cookie_from(&headers).is_some());

    // The password is now the new one.
    let acct = h
        .accounts
        .find_by_email("resetme@example.com")
        .unwrap()
        .unwrap();
    assert!(acct.verify_password("a-fresh-password"));
    assert!(!acct.verify_password("old-password"));
}

#[tokio::test]
async fn password_reset_rejects_a_mismatched_confirmation() {
    let h = harness();
    let created = h
        .accounts
        .create_account("mismatch@example.com", "old-password")
        .unwrap();
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=s&code_challenge=ch\
         &recovery_code={}&password=one-password&confirm=other-password",
        urlencoding::encode(REDIRECT),
        urlencoding::encode(&created.recovery_code),
    );
    let (status, _, page) = post_form(&h.router, "/reset", &body, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("do not match"));
}

// --- bounded SSO session ---------------------------------------------------

#[tokio::test]
async fn first_authorize_renders_login_second_with_cookie_skips_to_a_code() {
    let h = harness();
    h.accounts
        .create_account("sso@example.com", "a-good-password")
        .unwrap();
    let challenge = pkce_challenge("sso-verifier-ccccccccccccccccccccccccccccc");

    // First /authorize, no cookie → the login page.
    let (status, _, page) = get(&h.router, &authorize_url("sso-1", &challenge), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Sign in"));

    // Log in → get the SSO cookie.
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid+email&state=sso-1\
         &nonce=n-1&code_challenge={}&email=sso%40example.com&password=a-good-password",
        urlencoding::encode(REDIRECT),
        urlencoding::encode(&challenge),
    );
    let (status, headers, _) = post_form(&h.router, "/login", &body, None).await;
    assert_eq!(status, StatusCode::FOUND);
    let cookie = sso_cookie_from(&headers).expect("login sets the SSO cookie");

    // Second /authorize WITH the cookie → straight to a code, no login.
    let (status, headers, page) =
        get(&h.router, &authorize_url("sso-2", &challenge), Some(&cookie)).await;
    assert_eq!(status, StatusCode::FOUND, "SSO hit should redirect, page: {page}");
    let location = headers.get("location").unwrap().to_str().unwrap();
    assert!(location.starts_with(REDIRECT));
    assert!(location.contains("state=sso-2"));
    let _ = code_from_location(location);
}

#[tokio::test]
async fn authorize_with_an_unknown_cookie_re_prompts() {
    let h = harness();
    h.accounts
        .create_account("unknown@example.com", "a-good-password")
        .unwrap();
    let challenge = pkce_challenge("unknown-verifier-ddddddddddddddddddddddddd");
    // A cookie whose token was never minted → SSO miss → the login page.
    let (status, _, page) = get(
        &h.router,
        &authorize_url("u-1", &challenge),
        Some("ohd_idp_sso=never-a-real-session-token"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Sign in"));
}

#[tokio::test]
async fn logout_clears_the_session_so_the_next_authorize_re_prompts() {
    let h = harness();
    h.accounts
        .create_account("logout@example.com", "a-good-password")
        .unwrap();
    let challenge = pkce_challenge("logout-verifier-eeeeeeeeeeeeeeeeeeeeeeeeee");

    // Log in → cookie.
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=lo-1\
         &code_challenge={}&email=logout%40example.com&password=a-good-password",
        urlencoding::encode(REDIRECT),
        urlencoding::encode(&challenge),
    );
    let (_, headers, _) = post_form(&h.router, "/login", &body, None).await;
    let cookie = sso_cookie_from(&headers).expect("SSO cookie set");

    // Sanity: the cookie is live — /authorize skips login.
    let (status, _, _) = get(&h.router, &authorize_url("lo-2", &challenge), Some(&cookie)).await;
    assert_eq!(status, StatusCode::FOUND);

    // Log out — the response clears the cookie.
    let (status, lo_headers, page) = post_form(&h.router, "/logout", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Signed out"));
    let cleared = lo_headers.get("set-cookie").unwrap().to_str().unwrap();
    assert!(cleared.contains("ohd_idp_sso=;"));
    assert!(cleared.contains("Max-Age=0"));

    // The session is gone server-side — re-presenting the old cookie at
    // /authorize now re-prompts.
    let (status, _, page) =
        get(&h.router, &authorize_url("lo-3", &challenge), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Sign in"));
}

#[tokio::test]
async fn logout_honours_a_registered_post_logout_redirect_uri_only() {
    let h = harness();

    // A registered redirect URI is honoured.
    let (status, headers, _) = get(
        &h.router,
        &format!(
            "/logout?post_logout_redirect_uri={}&state=bye",
            urlencoding::encode(REDIRECT)
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let location = headers.get("location").unwrap().to_str().unwrap();
    assert!(location.starts_with(REDIRECT));
    assert!(location.contains("state=bye"));

    // An unregistered target is ignored — the plain signed-out page shows.
    let (status, _, page) = get(
        &h.router,
        "/logout?post_logout_redirect_uri=https%3A%2F%2Fevil.example%2Fgrab",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Signed out"));
}

#[tokio::test]
async fn sso_cookie_is_secure_httponly_and_samesite_lax() {
    let h = harness();
    h.accounts
        .create_account("cookie@example.com", "a-good-password")
        .unwrap();
    let challenge = pkce_challenge("cookie-verifier-fffffffffffffffffffffffff");
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=ck-1\
         &code_challenge={}&email=cookie%40example.com&password=a-good-password",
        urlencoding::encode(REDIRECT),
        urlencoding::encode(&challenge),
    );
    let (_, headers, _) = post_form(&h.router, "/login", &body, None).await;
    let raw = headers.get("set-cookie").unwrap().to_str().unwrap();
    assert!(raw.contains("HttpOnly"));
    assert!(raw.contains("Secure"));
    assert!(raw.contains("SameSite=Lax"));
}

// --- signing-key rotation --------------------------------------------------

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
}

#[tokio::test]
async fn key_rotation_keeps_old_id_tokens_verifiable_and_drops_expired_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join("signing-key.pem");

    // Mint an id_token-equivalent under the original key.
    let mut ks = KeyStore::load(&key_path).unwrap();
    let old_kid = ks.active().kid().to_string();
    let old_token = ohd_idp::token::mint_id_token(
        ks.active(),
        "https://accounts.ohd.dev",
        "01PROFILEULID00000000000000",
        "cord-web",
        "u@e.com",
        false,
        None,
        1_700_000_000,
    )
    .unwrap();

    // Rotate — the new key signs, the old key lingers in the JWKS.
    let (reported_old, new_kid) = ks.rotate(7).unwrap();
    assert_eq!(reported_old, old_kid);
    assert_ne!(new_kid, old_kid);

    let jwks = ks.jwks();
    assert_eq!(jwks.keys.len(), 2, "active + one overlap key");
    let kids: Vec<_> = jwks.keys.iter().map(|k| k.kid.clone()).collect();
    assert!(kids.contains(&old_kid));
    assert!(kids.contains(&new_kid));

    // The old id_token still verifies against its (overlap) JWK.
    let header = decode_header(&old_token).unwrap();
    assert_eq!(header.kid.as_deref(), Some(old_kid.as_str()));
    let old_jwk = jwks.keys.iter().find(|k| k.kid == old_kid).unwrap();
    let decoding = DecodingKey::from_rsa_components(&old_jwk.n, &old_jwk.e).unwrap();
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://accounts.ohd.dev"]);
    validation.set_audience(&["cord-web"]);
    let data = decode::<Claims>(&old_token, &decoding, &validation).unwrap();
    assert_eq!(data.claims.sub, "01PROFILEULID00000000000000");

    // Rotating again with a zero-day overlap → that retired key expires
    // immediately; a reload prunes it, leaving only active + the most
    // recent overlap.
    let mut ks2 = KeyStore::load(&key_path).unwrap();
    ks2.rotate(0).unwrap();
    let reloaded = KeyStore::load(&key_path).unwrap();
    let kids: Vec<_> = reloaded.jwks().keys.iter().map(|k| k.kid.clone()).collect();
    // The zero-day key is gone; the 7-day overlap key from the first
    // rotation is also expired-or-not depending on timing, but the
    // just-expired zero-day retiree must not appear.
    assert!(reloaded.jwks().keys.len() <= 2);
    assert!(kids.contains(&reloaded.active().kid().to_string()));
}

#[tokio::test]
async fn jwks_endpoint_serves_rotation_overlap_keys() {
    // A live router whose KeyStore has already rotated once serves both
    // keys at /jwks.
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join("signing-key.pem");
    let mut ks = KeyStore::load(&key_path).unwrap();
    let old_kid = ks.active().kid().to_string();
    ks.rotate(7).unwrap();
    let new_kid = ks.active().kid().to_string();

    let cfg = config::from_str(SAMPLE).expect("config parses");
    let accounts = AccountStore::in_memory().unwrap();
    let idp_store = IdpStore::in_memory().unwrap();
    let router = build_router(cfg, ks, accounts, idp_store);

    let (status, _, body) = get(&router, "/jwks", None).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let kids: Vec<String> = v["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kid"].as_str().unwrap().to_string())
        .collect();
    assert!(kids.contains(&old_kid));
    assert!(kids.contains(&new_kid));
    assert_eq!(kids.len(), 2);
}

#[tokio::test]
async fn discovery_advertises_the_end_session_endpoint() {
    let h = harness();
    let (status, _, body) = get(
        &h.router,
        "/.well-known/openid-configuration",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["end_session_endpoint"],
        "https://accounts.ohd.dev/logout"
    );
}

/// Keep the `canonical_recovery` + `sha256_hex` re-exports meaningfully
/// imported — they are the hashing the recovery flow relies on.
#[test]
fn recovery_hashing_is_the_saas_canonical_form() {
    assert_eq!(canonical_recovery("ab cd-ef"), "ABCDEF");
    assert_eq!(sha256_hex("ABCDEF").len(), 64);
}
