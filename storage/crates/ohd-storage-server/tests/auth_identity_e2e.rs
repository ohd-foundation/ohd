//! End-to-end test for the AuthService multi-identity link RPCs over
//! Connect-RPC. Exercises the full wire path:
//!
//! 1. Create storage, mint a self-session token.
//! 2. Bootstrap the user's first identity (issuer A) directly in the DB
//!    (mirrors what the OIDC sign-in flow would do at first contact).
//! 3. Open Connect-RPC server with a `StaticJwksResolver` carrying mock JWKS
//!    for issuer A and issuer B.
//! 4. AuthService.LinkIdentityStart → returns link_token.
//! 5. AuthService.CompleteIdentityLink with a freshly-minted issuer-B id_token
//!    → returns the new Identity.
//! 6. AuthService.ListIdentities → 2 rows, primary first.
//! 7. AuthService.UnlinkIdentity (issuer B) → success.
//! 8. AuthService.UnlinkIdentity (issuer A — last) → PermissionDenied with
//!    `LAST_IDENTITY_PROTECTED` in the message.
//! 9. WhoAmI's response includes `linked_identities` with the surviving row.

use std::sync::Arc;

use connectrpc::client::{ClientConfig, Http2Connection};
use connectrpc::ConnectError;
use jsonwebtoken::jwk::{
    AlgorithmParameters, CommonParameters, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
    RSAKeyParameters, RSAKeyType,
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use ohd_storage_core::auth::issue_self_session_token;
use ohd_storage_core::identities::{self, JwksResolver, StaticJwksResolver};
use ohd_storage_core::storage::{Storage, StorageConfig};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::Serialize;

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

use proto::ohdc::v0 as pb;
use proto::ohdc::v0::AuthServiceClient;
use proto::ohdc::v0::OhdcServiceClient;

struct MockIssuer {
    issuer: String,
    audience: String,
    kid: String,
    encoding_key: EncodingKey,
    jwks: JwkSet,
}

impl MockIssuer {
    fn new(issuer: &str, audience: &str, kid: &str) -> Self {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("rsa gen");
        let public_key = private_key.to_public_key();
        let n_bytes = public_key.n().to_bytes_be();
        let e_bytes = public_key.e().to_bytes_be();
        use base64::Engine;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let jwk = Jwk {
            common: CommonParameters {
                public_key_use: Some(PublicKeyUse::Signature),
                key_algorithm: Some(KeyAlgorithm::RS256),
                key_id: Some(kid.into()),
                ..Default::default()
            },
            algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
                key_type: RSAKeyType::RSA,
                n: b64.encode(&n_bytes),
                e: b64.encode(&e_bytes),
            }),
        };
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
            .expect("pkcs1 pem");
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes()).expect("from_rsa_pem");
        Self {
            issuer: issuer.into(),
            audience: audience.into(),
            kid: kid.into(),
            encoding_key,
            jwks: JwkSet { keys: vec![jwk] },
        }
    }

    fn id_token(&self, subject: &str) -> String {
        #[derive(Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            aud: &'a str,
            sub: &'a str,
            iat: i64,
            exp: i64,
        }
        let now = ohd_storage_core::format::now_ms() / 1000;
        let claims = Claims {
            iss: &self.issuer,
            aud: &self.audience,
            sub: subject,
            iat: now,
            exp: now + 600,
        };
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        encode(&header, &claims, &self.encoding_key).expect("encode")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_identity_link_flow_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth_e2e.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, user_ulid, Some("e2e-auth"), None))
        .unwrap();

    let issuer_a = MockIssuer::new("https://google.test", "ohd-aud-a", "kid-a");
    let issuer_b = MockIssuer::new("https://facebook.test", "ohd-aud-b", "kid-b");

    // Bootstrap initial identity (issuer A).
    storage
        .with_conn(|conn| {
            identities::bootstrap_first_identity(
                conn,
                user_ulid,
                &issuer_a.issuer,
                "sub-alice-google",
                Some("alice@example.com"),
                Some("Personal Google"),
            )
            .map(|_| ())
        })
        .unwrap();

    let resolver = Arc::new(StaticJwksResolver::new());
    resolver.insert(issuer_a.issuer.clone(), issuer_a.jwks.clone());
    resolver.insert(issuer_b.issuer.clone(), issuer_b.jwks.clone());
    let resolver: Arc<dyn JwksResolver> = resolver;

    // Bind ephemeral port, build router with auth wired in.
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();

    let router = server::router_with_auth(storage.clone(), Some(resolver));
    let _server_handle = tokio::spawn(async move {
        let bound = connectrpc::Server::from_listener(listener);
        bound.serve(router).await.expect("server died");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    let conn = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("h2")
        .shared(64);
    let auth_cfg = ClientConfig::new(uri.clone())
        .protocol(connectrpc::Protocol::Grpc)
        .default_header("authorization", format!("Bearer {bearer}"));
    let auth_client = AuthServiceClient::new(conn.clone(), auth_cfg.clone());
    let ohdc_client = OhdcServiceClient::new(conn, auth_cfg);

    // ---- LinkIdentityStart ----
    let start = auth_client
        .link_identity_start(pb::LinkIdentityStartRequest {
            provider_hint: "facebook".into(),
            ..Default::default()
        })
        .await
        .expect("link_identity_start")
        .into_owned();
    assert!(!start.link_token.is_empty());
    assert!(start.expires_at_ms > 0);

    // ---- CompleteIdentityLink ----
    let id_token = issuer_b.id_token("sub-alice-facebook");
    let complete = auth_client
        .complete_identity_link(pb::CompleteIdentityLinkRequest {
            link_token: start.link_token.clone(),
            id_token,
            issuer: issuer_b.issuer.clone(),
            audiences: vec![issuer_b.audience.clone()],
            display_label: Some("Personal Facebook".into()),
            ..Default::default()
        })
        .await
        .expect("complete_identity_link")
        .into_owned();
    let identity = complete.identity.into_option().expect("identity field");
    assert_eq!(identity.provider, issuer_b.issuer);
    assert_eq!(identity.subject, "sub-alice-facebook");
    assert!(!identity.is_primary);
    assert_eq!(identity.display_label.as_deref(), Some("Personal Facebook"));

    // ---- ListIdentities ----
    let list = auth_client
        .list_identities(pb::ListIdentitiesRequest::default())
        .await
        .expect("list_identities")
        .into_owned();
    assert_eq!(list.identities.len(), 2);
    assert!(list.identities[0].is_primary);
    assert_eq!(list.identities[0].provider, issuer_a.issuer);

    // ---- WhoAmI includes linked_identities ----
    let who = ohdc_client
        .who_am_i(pb::WhoAmIRequest::default())
        .await
        .expect("whoami")
        .into_owned();
    assert_eq!(who.token_kind, "self_session");
    assert_eq!(who.linked_identities.len(), 2);
    let providers: Vec<&str> = who
        .linked_identities
        .iter()
        .map(|i| i.provider.as_str())
        .collect();
    assert!(providers.contains(&issuer_a.issuer.as_str()));
    assert!(providers.contains(&issuer_b.issuer.as_str()));
    // Subject is intentionally NOT exposed in WhoAmI's summary view.
    // (LinkedIdentitySummary has no `subject` field; WhoAmI doesn't leak it.)

    // ---- UnlinkIdentity (issuer B) ----
    let unlink = auth_client
        .unlink_identity(pb::UnlinkIdentityRequest {
            provider: issuer_b.issuer.clone(),
            subject: "sub-alice-facebook".into(),
            ..Default::default()
        })
        .await
        .expect("unlink B")
        .into_owned();
    assert!(unlink.unlinked_at_ms > 0);

    // ---- UnlinkIdentity (issuer A — last) → LAST_IDENTITY_PROTECTED ----
    let res = auth_client
        .unlink_identity(pb::UnlinkIdentityRequest {
            provider: issuer_a.issuer.clone(),
            subject: "sub-alice-google".into(),
            ..Default::default()
        })
        .await;
    let err = res.expect_err("last identity must be protected");
    let msg = format!("{err}");
    assert!(
        msg.contains("LAST_IDENTITY_PROTECTED"),
        "expected LAST_IDENTITY_PROTECTED in error, got: {msg}"
    );

    // ---- ListIdentities final state — exactly 1 row, the primary ----
    let list = auth_client
        .list_identities(pb::ListIdentitiesRequest::default())
        .await
        .expect("list_identities final")
        .into_owned();
    assert_eq!(list.identities.len(), 1);
    assert!(list.identities[0].is_primary);
    assert_eq!(list.identities[0].provider, issuer_a.issuer);
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_rpcs_reject_grant_token() {
    use ohd_storage_core::auth::{issue_grant_token, TokenKind};
    use ohd_storage_core::grants::{create_grant, NewGrant, RuleEffect};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth_grant.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    let user_ulid = storage.user_ulid();

    // Create a grant + issue a grant bearer; AuthService should refuse.
    let new_grant = NewGrant {
        grantee_label: "Dr. Test".into(),
        grantee_kind: "human".into(),
        purpose: None,
        default_action: RuleEffect::Allow,
        approval_mode: "never_required".into(),
        expires_at_ms: None,
        event_type_rules: vec![],
        channel_rules: vec![],
        sensitivity_rules: vec![],
        write_event_type_rules: vec![],
        auto_approve_event_types: vec![],
        aggregation_only: false,
        strip_notes: false,
        notify_on_access: false,
        require_approval_per_query: false,
        max_queries_per_day: None,
        max_queries_per_hour: None,
        rolling_window_days: None,
        absolute_window: None,
        delegate_for_user_ulid: None,
        grantee_recovery_pubkey: None,
    };
    let (grant_id, _) = storage
        .with_conn_mut(|conn| create_grant(conn, &new_grant))
        .unwrap();
    let grant_bearer = storage
        .with_conn(|conn| issue_grant_token(conn, user_ulid, grant_id, TokenKind::Grant, None))
        .unwrap();

    let resolver: Arc<dyn JwksResolver> = Arc::new(StaticJwksResolver::new());

    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
    let router = server::router_with_auth(storage.clone(), Some(resolver));
    let _h = tokio::spawn(async move {
        connectrpc::Server::from_listener(listener)
            .serve(router)
            .await
            .expect("server died");
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    let conn = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("h2")
        .shared(64);
    let cfg = ClientConfig::new(uri.clone())
        .protocol(connectrpc::Protocol::Grpc)
        .default_header("authorization", format!("Bearer {grant_bearer}"));
    let auth_client = AuthServiceClient::new(conn, cfg);

    let res = auth_client
        .list_identities(pb::ListIdentitiesRequest::default())
        .await;
    let err = res.expect_err("grant token must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("WRONG_TOKEN_KIND") || msg.contains("self-session"),
        "expected WRONG_TOKEN_KIND in error, got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_rpcs_reject_no_bearer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth_unauth.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());

    let resolver: Arc<dyn JwksResolver> = Arc::new(StaticJwksResolver::new());

    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
    let router = server::router_with_auth(storage.clone(), Some(resolver));
    let _h = tokio::spawn(async move {
        connectrpc::Server::from_listener(listener)
            .serve(router)
            .await
            .expect("server died");
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    let conn = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("h2")
        .shared(64);
    let cfg = ClientConfig::new(uri).protocol(connectrpc::Protocol::Grpc);
    let auth_client = AuthServiceClient::new(conn, cfg);

    let res = auth_client
        .list_identities(pb::ListIdentitiesRequest::default())
        .await;
    let _err: ConnectError = res.expect_err("must be unauthenticated");
}
