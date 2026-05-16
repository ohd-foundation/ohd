//! Integration tests over the assembled router for the Phase 2
//! email/password OIDC authorization-code flow: `/authorize`, the login
//! UI, `/signup`, `/token`, `/userinfo` — plus PKCE enforcement and the
//! `id_token` claim shape.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use http_body_util::BodyExt;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use ohd_idp::store::AccountStore as AS;
use ohd_idp::{build_router, config, AccountStore, IdpStore, KeyStore, SigningKey};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const SAMPLE: &str = r#"
[server]
listen = "0.0.0.0:8447"
issuer = "https://accounts.ohd.dev"

[[client]]
id = "cord-web"
redirect_uris = ["https://cord.ohd.dev/v1/auth/callback"]
client_secret_env = "TEST_OIDC_CORD_SECRET"

[[client]]
id = "connect-web"
redirect_uris = ["https://connect.ohd.dev/auth/callback"]
public = true
"#;

struct Harness {
    router: axum::Router,
    accounts: AccountStore,
    signing_key: SigningKey,
}

fn harness() -> Harness {
    // Confidential client secret for `cord-web`.
    std::env::set_var("TEST_OIDC_CORD_SECRET", "cord-test-secret");
    let cfg = config::from_str(SAMPLE).expect("config parses");
    let key = SigningKey::generate().expect("key generates");
    let accounts = AccountStore::in_memory().expect("account store");
    let idp_store = IdpStore::in_memory().expect("idp store");
    let router = build_router(
        cfg,
        KeyStore::in_memory(key.clone()),
        accounts.clone(),
        idp_store,
    );
    Harness {
        router,
        accounts,
        signing_key: key,
    }
}

async fn get(router: &axum::Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let resp = router
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
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
) -> (StatusCode, axum::http::HeaderMap, String) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&bytes).into_owned())
}

/// PKCE S256 challenge for a verifier.
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Pull the `code` query parameter out of a redirect `Location`.
fn code_from_location(location: &str) -> String {
    let q = location.split_once('?').unwrap().1;
    form_urlencoded::parse(q.as_bytes())
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .expect("code in redirect")
}

#[tokio::test]
async fn authorize_renders_login_for_a_valid_request() {
    let h = harness();
    let uri = "/authorize?client_id=cord-web\
        &redirect_uri=https%3A%2F%2Fcord.ohd.dev%2Fv1%2Fauth%2Fcallback\
        &response_type=code&scope=openid%20email&state=st-1\
        &code_challenge=abc123&code_challenge_method=S256&nonce=n-1";
    let (status, _, body) = get(&h.router, uri).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Sign in"));
    assert!(body.contains("name=\"state\" value=\"st-1\""));
}

#[tokio::test]
async fn authorize_rejects_unknown_client_with_an_error_page_not_a_redirect() {
    let h = harness();
    let uri = "/authorize?client_id=evil-app\
        &redirect_uri=https%3A%2F%2Fevil.example%2Fcb\
        &response_type=code&scope=openid&state=s\
        &code_challenge=c&code_challenge_method=S256";
    let (status, headers, body) = get(&h.router, uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(headers.get("location").is_none());
    assert!(body.contains("not registered"));
}

#[tokio::test]
async fn authorize_rejects_a_missing_pkce_challenge() {
    let h = harness();
    let uri = "/authorize?client_id=cord-web\
        &redirect_uri=https%3A%2F%2Fcord.ohd.dev%2Fv1%2Fauth%2Fcallback\
        &response_type=code&scope=openid&state=s";
    let (status, _, body) = get(&h.router, uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("PKCE"));
}

#[tokio::test]
async fn authorize_rejects_a_redirect_uri_not_in_the_registry() {
    let h = harness();
    let uri = "/authorize?client_id=cord-web\
        &redirect_uri=https%3A%2F%2Fcord.ohd.dev%2Fwrong\
        &response_type=code&scope=openid&state=s\
        &code_challenge=c&code_challenge_method=S256";
    let (status, _, body) = get(&h.router, uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("not registered"));
}

/// The full happy path: an account exists → `POST /login` → redirect with
/// a code → `POST /token` → decode the `id_token` → `GET /userinfo`.
#[tokio::test]
async fn full_login_to_token_flow() {
    let h = harness();
    let created = h
        .accounts
        .create_account("flow@example.com", "a-good-password")
        .unwrap();

    let verifier = "verifier-0123456789-abcdefghijklmnop-QRSTUVWXYZ";
    let challenge = pkce_challenge(verifier);
    let redirect = "https://cord.ohd.dev/v1/auth/callback";

    // POST /login with the carried authorize params + credentials.
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid+email&state=st-9\
         &nonce=nonce-9&code_challenge={}&email=flow%40example.com&password=a-good-password",
        urlencoding::encode(redirect),
        urlencoding::encode(&challenge),
    );
    let (status, headers, _) = post_form(&h.router, "/login", &body).await;
    assert_eq!(status, StatusCode::FOUND);
    let location = headers.get("location").unwrap().to_str().unwrap();
    assert!(location.starts_with(redirect));
    assert!(location.contains("state=st-9"));
    let code = code_from_location(location);

    // POST /token — confidential client, presents its secret + verifier.
    let token_body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}\
         &client_id=cord-web&client_secret=cord-test-secret&code_verifier={}",
        urlencoding::encode(&code),
        urlencoding::encode(redirect),
        urlencoding::encode(verifier),
    );
    let (status, _, body) = post_form(&h.router, "/token", &token_body).await;
    assert_eq!(status, StatusCode::OK, "token response: {body}");
    let tok: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(tok["token_type"], "Bearer");
    assert!(tok["expires_in"].as_i64().unwrap() > 0);
    assert!(tok["refresh_token"].is_string());
    assert_eq!(tok["scope"], "openid email");

    // Decode + verify the id_token with the public JWK.
    let id_token = tok["id_token"].as_str().unwrap();
    let claims = verify_id_token(&h.signing_key, id_token);
    assert_eq!(claims.iss, "https://accounts.ohd.dev");
    assert_eq!(claims.sub, created.profile_ulid);
    assert_eq!(claims.aud, "cord-web");
    assert_eq!(claims.nonce.as_deref(), Some("nonce-9"));
    assert_eq!(claims.email, "flow@example.com");

    // GET /userinfo with the access token.
    let access = tok["access_token"].as_str().unwrap();
    let resp = h
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/userinfo")
                .header("authorization", format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let ui: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(ui["sub"], created.profile_ulid);
    assert_eq!(ui["email"], "flow@example.com");
}

/// A code minted with one PKCE challenge is rejected at `/token` when the
/// presented verifier does not match.
#[tokio::test]
async fn token_rejects_a_pkce_verifier_mismatch() {
    let h = harness();
    h.accounts
        .create_account("pkce@example.com", "a-good-password")
        .unwrap();

    let challenge = pkce_challenge("the-real-verifier-aaaaaaaaaaaaaaaaaaaaaaa");
    let redirect = "https://cord.ohd.dev/v1/auth/callback";
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=s\
         &code_challenge={}&email=pkce%40example.com&password=a-good-password",
        urlencoding::encode(redirect),
        urlencoding::encode(&challenge),
    );
    let (_, headers, _) = post_form(&h.router, "/login", &body).await;
    let code = code_from_location(headers.get("location").unwrap().to_str().unwrap());

    // Present a *wrong* verifier.
    let token_body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}\
         &client_id=cord-web&client_secret=cord-test-secret&code_verifier=wrong-verifier-xxxxxxxxxxxxxxxxxxxxxxxx",
        urlencoding::encode(&code),
        urlencoding::encode(redirect),
    );
    let (status, _, body) = post_form(&h.router, "/token", &token_body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("PKCE"));
}

/// A code is single-use — a second `/token` call with the same code fails.
#[tokio::test]
async fn token_rejects_a_replayed_code() {
    let h = harness();
    h.accounts
        .create_account("replay@example.com", "a-good-password")
        .unwrap();
    let verifier = "replay-verifier-bbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let challenge = pkce_challenge(verifier);
    let redirect = "https://cord.ohd.dev/v1/auth/callback";
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=s\
         &code_challenge={}&email=replay%40example.com&password=a-good-password",
        urlencoding::encode(redirect),
        urlencoding::encode(&challenge),
    );
    let (_, headers, _) = post_form(&h.router, "/login", &body).await;
    let code = code_from_location(headers.get("location").unwrap().to_str().unwrap());

    let token_body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}\
         &client_id=cord-web&client_secret=cord-test-secret&code_verifier={}",
        urlencoding::encode(&code),
        urlencoding::encode(redirect),
        urlencoding::encode(verifier),
    );
    let (first, _, _) = post_form(&h.router, "/token", &token_body).await;
    assert_eq!(first, StatusCode::OK);
    let (second, _, body) = post_form(&h.router, "/token", &token_body).await;
    assert_eq!(second, StatusCode::BAD_REQUEST);
    assert!(body.contains("already used"));
}

/// Wrong password re-renders the login page with an error — no redirect.
#[tokio::test]
async fn login_with_wrong_password_re_renders_with_an_error() {
    let h = harness();
    h.accounts
        .create_account("badpass@example.com", "a-good-password")
        .unwrap();
    let redirect = "https://cord.ohd.dev/v1/auth/callback";
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=s\
         &code_challenge=challenge-x&email=badpass%40example.com&password=WRONG",
        urlencoding::encode(redirect),
    );
    let (status, headers, body) = post_form(&h.router, "/login", &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get("location").is_none());
    assert!(body.contains("Incorrect email or password"));
}

/// `/token` rejects a confidential client presenting a wrong secret.
#[tokio::test]
async fn token_rejects_a_bad_client_secret() {
    let h = harness();
    h.accounts
        .create_account("secret@example.com", "a-good-password")
        .unwrap();
    let verifier = "secret-verifier-cccccccccccccccccccccccccccc";
    let challenge = pkce_challenge(verifier);
    let redirect = "https://cord.ohd.dev/v1/auth/callback";
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=s\
         &code_challenge={}&email=secret%40example.com&password=a-good-password",
        urlencoding::encode(redirect),
        urlencoding::encode(&challenge),
    );
    let (_, headers, _) = post_form(&h.router, "/login", &body).await;
    let code = code_from_location(headers.get("location").unwrap().to_str().unwrap());

    let token_body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}\
         &client_id=cord-web&client_secret=WRONG-SECRET&code_verifier={}",
        urlencoding::encode(&code),
        urlencoding::encode(redirect),
        urlencoding::encode(verifier),
    );
    let (status, _, body) = post_form(&h.router, "/token", &token_body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("invalid_client"));
}

/// The sign-up path: `POST /signup` creates an account, shows the recovery
/// code once, and `/continue` resumes the flow with a usable code.
#[tokio::test]
async fn signup_flow_creates_account_shows_recovery_code_and_continues() {
    let h = harness();
    let verifier = "signup-verifier-dddddddddddddddddddddddddddd";
    let challenge = pkce_challenge(verifier);
    let redirect = "https://cord.ohd.dev/v1/auth/callback";

    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid+email&state=su-1\
         &nonce=su-nonce&code_challenge={}\
         &email=newuser%40example.com&password=brand-new-pass&confirm=brand-new-pass",
        urlencoding::encode(redirect),
        urlencoding::encode(&challenge),
    );
    let (status, _, page) = post_form(&h.router, "/signup", &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Save your recovery code"));
    assert!(page.contains("action=\"/continue\""));
    assert!(page.contains("name=\"token\""));

    // The account now exists in the shared store.
    assert!(h
        .accounts
        .find_by_email("newuser@example.com")
        .unwrap()
        .is_some());

    // Follow the flow as a browser would: a GET form submit to /continue
    // carrying the hidden `token` field (the form action has no query).
    let cont_uri = {
        let marker = "name=\"token\" value=\"";
        let start = page.find(marker).unwrap() + marker.len();
        let rest = &page[start..];
        let end = rest.find('"').unwrap();
        format!("/continue?token={}", &rest[..end])
    };
    let (status, headers, _) = get(&h.router, &cont_uri).await;
    assert_eq!(status, StatusCode::FOUND);
    let location = headers.get("location").unwrap().to_str().unwrap();
    assert!(location.starts_with(redirect));
    assert!(location.contains("state=su-1"));
}

#[tokio::test]
async fn signup_rejects_a_duplicate_email() {
    let h = harness();
    h.accounts
        .create_account("dup@example.com", "a-good-password")
        .unwrap();
    let redirect = "https://cord.ohd.dev/v1/auth/callback";
    let body = format!(
        "client_id=cord-web&redirect_uri={}&scope=openid&state=s\
         &code_challenge=ch&email=dup%40example.com&password=another-pass&confirm=another-pass",
        urlencoding::encode(redirect),
    );
    let (status, _, page) = post_form(&h.router, "/signup", &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("already exists"));
}

/// Reusing `AccountStore` directly keeps the import meaningful even though
/// the harness wraps it — and documents the alias.
#[test]
fn account_store_alias_is_the_store_type() {
    let _: AS = AccountStore::in_memory().unwrap();
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: String,
    #[serde(default)]
    nonce: Option<String>,
    email: String,
}

/// Decode + verify an `id_token` against the signing key's public JWK.
fn verify_id_token(key: &SigningKey, jwt: &str) -> Claims {
    let header = decode_header(jwt).unwrap();
    assert_eq!(header.alg, Algorithm::RS256);
    assert_eq!(header.kid.as_deref(), Some(key.kid()));
    let decoding =
        DecodingKey::from_rsa_components(&key.jwk_modulus(), &key.jwk_exponent()).unwrap();
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://accounts.ohd.dev"]);
    validation.set_audience(&["cord-web"]);
    decode::<Claims>(jwt, &decoding, &validation).unwrap().claims
}
